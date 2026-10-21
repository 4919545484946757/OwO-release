//! Fresh per-session user-only credential directory under a trusted existing parent.
//! No restoration of runtime grants; restart requires a new trusted enrollment.
use crate::pairing_key::{generate_key, WrappedPairingKey};
use std::{
    fs::File,
    io::{self, Write},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{LocalFree, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::*,
};
const MAX_BLOB: usize = 16 * 1024;
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut v: Vec<u16> = path.as_os_str().encode_wide().collect();
    if v.contains(&0) {
        return Err(io::Error::other("invalid credential path"));
    }
    v.push(0);
    Ok(v)
}
struct Security(*mut std::ffi::c_void);
impl Drop for Security {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
impl Security {
    fn user_only() -> io::Result<Self> {
        let identity = crate::peer::current_process_identity()?;
        let mut raw = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(identity.user_sid.as_ptr().cast_mut().cast(), &mut raw) }
            == 0
        {
            return Err(io::Error::other("credential SID unavailable"));
        }
        let mut n = 0;
        while unsafe { *raw.add(n) } != 0 {
            n += 1;
        }
        let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(raw, n) });
        unsafe {
            LocalFree(raw.cast());
        }
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::other("credential ACL unavailable"));
        }
        Ok(Self(descriptor))
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}
fn filename(id: &[u8; 32]) -> io::Result<String> {
    if id == &[0; 32] {
        return Err(io::Error::other("invalid pairing identifier"));
    }
    Ok(id.iter().map(|b| format!("{b:02x}")).collect::<String>() + ".owokey")
}
fn verify_private_acl(handle: windows_sys::Win32::Foundation::HANDLE) -> io::Result<()> {
    use windows_sys::Win32::Security::{
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        EqualSid, GetAce, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
    let rejected = || io::Error::other("credential owner or ACL not private");
    let mut owner = ptr::null_mut();
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        )
    } != 0
    {
        return Err(rejected());
    }
    let _guard = Security(descriptor);
    let mut control = 0;
    let mut revision = 0;
    if acl.is_null()
        || unsafe { (*acl).AceCount } != 1
        || unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0
        || control & SE_DACL_PROTECTED == 0
    {
        return Err(rejected());
    }
    let identity = crate::peer::current_process_identity()?;
    if unsafe { EqualSid(owner, identity.user_sid.as_ptr().cast_mut().cast()) } == 0 {
        return Err(rejected());
    }
    let mut raw = ptr::null_mut();
    if unsafe { GetAce(acl, 0, &mut raw) } == 0 {
        return Err(rejected());
    }
    let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
    if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8
        || ace.Mask & FILE_ALL_ACCESS != FILE_ALL_ACCESS
        || unsafe {
            EqualSid(
                (&ace.SidStart as *const u32).cast_mut().cast(),
                identity.user_sid.as_ptr().cast_mut().cast(),
            )
        } == 0
    {
        return Err(rejected());
    }
    Ok(())
}

// No Debug: do not leak credential locations/content to routine diagnostics.
pub struct CredentialStore {
    root: PathBuf,
    _directory: File,
}
impl CredentialStore {
    /// Parent must already be trusted and stable; never reuse an existing root.
    /// Keeping the root handle open without DELETE sharing prevents ordinary renames.
    pub fn create_new(root: &Path) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(io::Error::other("absolute credential directory required"));
        }
        let security = Security::user_only()?;
        let attributes = security.attributes();
        let path = wide(root)?;
        if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let directory = unsafe { File::from_raw_handle(handle) };
        verify_private_acl(directory.as_raw_handle().cast())?;
        let metadata = std::fs::symlink_metadata(root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::other("invalid credential directory"));
        }
        Ok(Self {
            root: root.to_path_buf(),
            _directory: directory,
        })
    }
    /// Publish only a complete DPAPI blob, with no overwrite of existing records.
    /// A write failure leaves no final file. A rename failure removes our own temp.
    pub fn persist_new(&self, id: &[u8; 32], key: &WrappedPairingKey) -> io::Result<()> {
        let name = filename(id)?;
        let blob = key.encrypted_bytes();
        if blob.is_empty() || blob.len() > MAX_BLOB {
            return Err(io::Error::other("invalid wrapped key"));
        }
        let random = *generate_key()?;
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let temporary = self.root.join(format!("{name}.{suffix}.pending"));
        let final_path = self.root.join(name);
        let security = Security::user_only()?;
        let attributes = security.attributes();
        let temp_wide = wide(&temporary)?;
        let handle = unsafe {
            CreateFileW(
                temp_wide.as_ptr(),
                GENERIC_WRITE | READ_CONTROL,
                0,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut file = unsafe { File::from_raw_handle(handle) };
        let written = (|| -> io::Result<()> {
            verify_private_acl(file.as_raw_handle().cast())?;
            file.write_all(b"OwOKEY01")?;
            file.write_all(id)?;
            file.write_all(&(blob.len() as u32).to_le_bytes())?;
            file.write_all(blob)?;
            file.sync_all()
        })();
        drop(file);
        let result = written.and_then(|_| {
            let final_wide = wide(&final_path)?;
            if unsafe {
                MoveFileExW(
                    temp_wide.as_ptr(),
                    final_wide.as_ptr(),
                    MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
    /// Host-only cleanup. Revoke the in-memory grant first; failed deletion must not
    /// restore authorization. Files are not accepted as grants after restart.
    pub fn remove(&self, id: &[u8; 32]) -> io::Result<()> {
        std::fs::remove_file(self.root.join(filename(id)?))
    }
    pub fn credential_path(&self, id: &[u8; 32]) -> io::Result<PathBuf> {
        Ok(self.root.join(filename(id)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{
        Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
        EqualSid, GetAce, ACCESS_ALLOWED_ACE, DACL_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
    fn private_acl(path: &Path) {
        let path = wide(path).unwrap();
        let mut owner = ptr::null_mut();
        let mut acl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        assert_eq!(
            unsafe {
                GetNamedSecurityInfoW(
                    path.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                    &mut owner,
                    ptr::null_mut(),
                    &mut acl,
                    ptr::null_mut(),
                    &mut descriptor,
                )
            },
            0
        );
        let _guard = Security(descriptor);
        assert!(!acl.is_null());
        assert_eq!(unsafe { (*acl).AceCount }, 1);
        let identity = crate::peer::current_process_identity().unwrap();
        assert_ne!(
            unsafe { EqualSid(owner, identity.user_sid.as_ptr().cast_mut().cast()) },
            0
        );
        let mut ace = ptr::null_mut();
        assert_ne!(unsafe { GetAce(acl, 0, &mut ace) }, 0);
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        assert_eq!(ace.Header.AceType, ACCESS_ALLOWED_ACE_TYPE as u8);
        assert_ne!(
            unsafe {
                EqualSid(
                    (&ace.SidStart as *const u32).cast_mut().cast(),
                    identity.user_sid.as_ptr().cast_mut().cast(),
                )
            },
            0
        );
    }
    #[test]
    fn actual_private_acl_atomic_publish_no_overwrite_and_cleanup() {
        let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.work-logs/credential-store-tests");
        std::fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let random = *generate_key().unwrap();
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root = parent.join(suffix);
        let store = CredentialStore::create_new(&root).unwrap();
        private_acl(&root);
        assert!(CredentialStore::create_new(&root).is_err());
        let id = [1; 32];
        let key = WrappedPairingKey::protect(&zeroize::Zeroizing::new([7; 32]), &id).unwrap();
        store.persist_new(&id, &key).unwrap();
        let path = store.credential_path(&id).unwrap();
        private_acl(&path);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], b"OwOKEY01");
        assert_eq!(&bytes[8..40], &id);
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize,
            bytes.len() - 44
        );
        let unwrapped = WrappedPairingKey::from_encrypted_bytes(&bytes[44..])
            .unwrap()
            .unprotect(&id)
            .unwrap();
        assert!(*unwrapped == [7; 32]);
        assert!(store.persist_new(&id, &key).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        store.remove(&id).unwrap();
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        drop(store);
        std::fs::remove_dir(&root).unwrap();
    }
}
