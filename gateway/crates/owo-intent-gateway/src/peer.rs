//! Kernel-derived process identity for the local pipe entrance.
//! This is not connector pairing or executable-integrity verification.
use std::{io, mem::size_of, os::windows::io::AsRawHandle};
use tokio::net::windows::named_pipe::NamedPipeServer;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Security::{
        GetLengthSid, GetTokenInformation, IsValidSid, TokenSessionId, TokenStatistics, TokenUser,
        TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER,
    },
    System::{
        Pipes::GetNamedPipeClientProcessId,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

// Never derive Debug: account/session identifiers must not enter ordinary logs.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProcessIdentity {
    pub(crate) user_sid: Vec<u8>,
    pub(crate) session_id: u32,
    pub(crate) authentication_id: (u32, i32),
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn process_identity(process: HANDLE) -> io::Result<ProcessIdentity> {
    let mut raw_token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw_token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(raw_token);
    let mut required = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut required);
    }
    if required < size_of::<TOKEN_USER>() as u32 || required > 65536 {
        return Err(io::Error::other("invalid token identity size"));
    }
    // TOKEN_USER contains a pointer; keep the backing allocation aligned.
    let mut storage = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    if unsafe { IsValidSid(user.User.Sid) } == 0 {
        return Err(io::Error::other("invalid token identity"));
    }
    let sid_length = unsafe { GetLengthSid(user.User.Sid) } as usize;
    let user_sid =
        unsafe { std::slice::from_raw_parts(user.User.Sid.cast::<u8>(), sid_length) }.to_vec();
    let mut session_id = 0u32;
    let mut returned = 0;
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenSessionId,
            (&mut session_id as *mut u32).cast(),
            size_of::<u32>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if returned != size_of::<u32>() as u32 {
        return Err(io::Error::other("invalid session identity"));
    }
    let mut statistics: TOKEN_STATISTICS = unsafe { std::mem::zeroed() };
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenStatistics,
            (&mut statistics as *mut TOKEN_STATISTICS).cast(),
            size_of::<TOKEN_STATISTICS>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if returned != size_of::<TOKEN_STATISTICS>() as u32 {
        return Err(io::Error::other("invalid authentication identity"));
    }
    Ok(ProcessIdentity {
        user_sid,
        session_id,
        authentication_id: (
            statistics.AuthenticationId.LowPart,
            statistics.AuthenticationId.HighPart,
        ),
    })
}
pub(crate) fn current_process_identity() -> io::Result<ProcessIdentity> {
    process_identity(unsafe { GetCurrentProcess() })
}
pub(crate) fn verify_connected_client(
    server: &NamedPipeServer,
    expected: &ProcessIdentity,
) -> io::Result<()> {
    let mut pid = 0;
    if unsafe { GetNamedPipeClientProcessId(server.as_raw_handle().cast(), &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let raw_process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw_process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = Handle(raw_process);
    let actual = process_identity(process.0)?;
    if actual != *expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pipe client identity mismatch",
        ));
    }
    Ok(())
}

/// Acquire the connected process on-disk image digest, not loaded memory/signatures.
/// Not production admission: compare with trusted installation metadata before granting.
/// Blocking file IO must be dispatched off the async executor by the host.
pub fn connected_image_digest(server: &NamedPipeServer) -> io::Result<[u8; 32]> {
    use std::os::windows::{ffi::OsStringExt, fs::OpenOptionsExt};
    use windows_sys::Win32::System::Threading::{QueryFullProcessImageNameW, PROCESS_NAME_WIN32};
    verify_connected_client(server, &current_process_identity()?)?;
    let mut pid = 0;
    if unsafe { GetNamedPipeClientProcessId(server.as_raw_handle().cast(), &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = Handle(raw);
    // Recheck using this held process handle rather than relying on a prior PID lookup.
    if process_identity(process.0)? != current_process_identity()? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pipe identity mismatch",
        ));
    }
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    if unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            path.as_mut_ptr(),
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if length == 0 || length as usize >= path.len() {
        return Err(io::Error::other("invalid image path"));
    }
    let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&path[..length as usize]));
    // Share READ only: refuse concurrent write/delete access during the digest.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|_| io::Error::other("cannot open client image"))?;
    digest_image(&mut file)
}
fn digest_image(reader: &mut impl std::io::Read) -> io::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut remaining = 128 * 1024 * 1024usize;
    let mut buffer = [0u8; 65536];
    let mut nonempty = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        remaining = remaining
            .checked_sub(count)
            .ok_or_else(|| io::Error::other("image too large"))?;
        nonempty = true;
        hash.update(&buffer[..count]);
    }
    if !nonempty {
        return Err(io::Error::other("empty image"));
    }
    Ok(hash.finalize().into())
}
#[cfg(test)]
mod image_tests {
    use super::*;
    #[test]
    fn known_digest_and_failures() {
        assert_eq!(
            digest_image(&mut &b"abc"[..]).unwrap(),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
        assert!(digest_image(&mut &b""[..]).is_err());
        struct Broken;
        impl std::io::Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected"))
            }
        }
        assert!(digest_image(&mut Broken).is_err());
        assert!(digest_image(&mut std::io::repeat(0)).is_err());
    }
    #[tokio::test]
    async fn actual_connected_image_matches_test_executable() {
        use sha2::{Digest, Sha256};
        let name = format!(r"\\.\pipe\OwO.ImageProbe.{}", std::process::id());
        let server = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .unwrap();
        assert!(connected_image_digest(&server).is_err());
        let _client = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(&name)
            .unwrap();
        server.connect().await.unwrap();
        let actual = connected_image_digest(&server).unwrap();
        let expected: [u8; 32] =
            Sha256::digest(std::fs::read(std::env::current_exe().unwrap()).unwrap()).into();
        assert_eq!(actual, expected);
    }
}
