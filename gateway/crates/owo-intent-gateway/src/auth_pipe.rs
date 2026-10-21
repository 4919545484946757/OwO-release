//! Opt-in authenticated connected-pipe adapter. Not enabled by the product listener.
use crate::{
    frame,
    pairing::PairingRegistry,
    pipe::{FrameHandler, PipeError},
    proof_frame::ProofFrame,
};
use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::net::windows::named_pipe::NamedPipeServer;

/// Caller must create a local-only listener with the same ACL and exclusive-name
/// policy as the existing pipe server. No fallback to its legacy JSON mode.
/// Enrollment is exclusively host-side; requests cannot create pairing grants.
/// Revocation prevents NEW dispatch, not already-dispatched model cancellation.
pub async fn serve_authenticated_connection(
    server: NamedPipeServer,
    handler: Arc<dyn FrameHandler>,
    registry: Arc<Mutex<PairingRegistry>>,
    timeout: Duration,
) -> Result<(), PipeError> {
    let exchange = async {
        // Keep synchronous executable IO off Tokio's async executor. On timeout a
        // started blocking probe may finish, but it owns no business handler/grant.
        let (mut server, binding) = tokio::task::spawn_blocking(move || {
            let binding = crate::peer::connected_connector_binding(&server);
            (server, binding)
        })
        .await
        .map_err(|_| io::Error::other("client identity probe failed"))?;
        let binding = binding?;
        let bytes = frame::read_frame(&mut server).await?;
        {
            let mut guard = registry
                .lock()
                .map_err(|_| io::Error::other("pairing registry unavailable"))?;
            guard
                .authorize_frame(&binding, &bytes, Instant::now())
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "pipe request authentication rejected",
                    )
                })?;
        }
        let payload = ProofFrame::decode(&bytes)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "invalid authenticated frame",
                )
            })?
            .payload
            .to_vec();
        let origin = crate::source_capsule::VerifiedOrigin::authenticated(binding, payload.clone());
        if let Some(response) = handler.handle_verified(payload, origin).await {
            frame::write_frame(&mut server, &response).await?;
        }
        Ok::<(), PipeError>(())
    };
    tokio::time::timeout(timeout, exchange).await.map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "authenticated pipe exchange timed out",
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
    struct Probe(AtomicUsize);
    #[async_trait::async_trait]
    impl FrameHandler for Probe {
        async fn handle(&self, _payload: Vec<u8>) -> Option<Vec<u8>> {
            panic!("authenticated adapter must preserve verified origin");
        }
        async fn handle_verified(
            &self,
            payload: Vec<u8>,
            origin: crate::source_capsule::VerifiedOrigin,
        ) -> Option<Vec<u8>> {
            assert!(origin.matches(&payload));
            assert!(crate::protocol::AgentIpcRequest::decode(&payload).is_ok());
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(b"synthetic response".to_vec())
        }
    }
    fn envelope(payload: &[u8], nonce: [u8; 32]) -> Vec<u8> {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&[3; 32]).unwrap();
        mac.update(b"OwO/IME/pipe-request/v1\0");
        mac.update(&[1; 32]);
        mac.update(&nonce);
        mac.update(&(payload.len() as u64).to_le_bytes());
        mac.update(payload);
        let mut out = b"OwOPRF01".to_vec();
        out.extend_from_slice(&[1; 32]);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&mac.finalize().into_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }
    #[tokio::test]
    async fn real_pipe_authenticated_dispatch_and_denials() {
        let registry = Arc::new(Mutex::new(PairingRegistry::default()));
        let handler = Arc::new(Probe(AtomicUsize::new(0)));
        let payload =
            include_bytes!("../../../../contracts/agent-ipc/fixtures/request.submit.valid.json");
        // Valid, replay, bad tag, legacy JSON, revoked with a fresh nonce.
        for case in 0..5 {
            let name = format!(r"\\.\pipe\OwO.AuthProof.{}.{}", std::process::id(), case);
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(&name)
                .unwrap();
            let mut client = ClientOptions::new().open(&name).unwrap();
            server.connect().await.unwrap();
            if case == 0 {
                let binding = crate::peer::connected_connector_binding(&server).unwrap();
                registry
                    .lock()
                    .unwrap()
                    .approve_with_key_from_trusted_host(
                        [1; 32],
                        binding,
                        Instant::now(),
                        Duration::from_secs(60),
                        zeroize::Zeroizing::new([3; 32]),
                    )
                    .unwrap();
            }
            if case == 4 {
                registry.lock().unwrap().revoke(&[1; 32]).unwrap();
            }
            let mut bytes = envelope(payload, if case == 4 { [4; 32] } else { [2; 32] });
            if case == 2 {
                bytes[72] ^= 1;
            }
            if case == 3 {
                bytes = payload.to_vec();
            }
            let h: Arc<dyn FrameHandler> = handler.clone();
            let r = registry.clone();
            let task = tokio::spawn(serve_authenticated_connection(
                server,
                h,
                r,
                Duration::from_secs(10),
            ));
            frame::write_frame(&mut client, &bytes).await.unwrap();
            if case == 0 {
                assert_eq!(
                    frame::read_frame(&mut client).await.unwrap(),
                    b"synthetic response"
                );
            }
            let result = task.await.unwrap();
            if case == 0 {
                assert!(result.is_ok());
            } else {
                assert!(
                    matches!(result,Err(PipeError::Io(ref e)) if e.kind()==io::ErrorKind::PermissionDenied)
                );
            }
            assert_eq!(handler.0.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn child_process_client() {
        let Ok(name) = std::env::var("OWO_SYNTHETIC_AUTH_CHILD_PIPE") else {
            return;
        };
        assert!(name.starts_with(r"\\.\pipe\OwO.AuthChild."));
        let denial = std::env::var("OWO_SYNTHETIC_AUTH_CHILD_DENY").as_deref() == Ok("1");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut client = ClientOptions::new().open(&name).unwrap();
                let payload = include_bytes!(
                    "../../../../contracts/agent-ipc/fixtures/request.submit.valid.json"
                );
                frame::write_frame(&mut client, &envelope(payload, [2; 32]))
                    .await
                    .unwrap();
                let response =
                    tokio::time::timeout(Duration::from_secs(10), frame::read_frame(&mut client))
                        .await
                        .unwrap();
                if denial {
                    assert!(response.is_err());
                } else {
                    assert_eq!(response.unwrap(), b"synthetic response");
                }
            });
    }
    #[tokio::test]
    async fn separate_process_kernel_identity_and_install_digest_control_dispatch() {
        use sha2::{Digest, Sha256};
        use std::os::windows::io::AsRawHandle;
        let expected_image: [u8; 32] =
            Sha256::digest(std::fs::read(std::env::current_exe().unwrap()).unwrap()).into();
        for denial in [false, true] {
            let name = format!(
                r"\\.\pipe\OwO.AuthChild.{}.{}",
                std::process::id(),
                u8::from(denial)
            );
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(&name)
                .unwrap();
            let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "auth_pipe::tests::child_process_client",
                    "--test-threads=1",
                ])
                .env("OWO_SYNTHETIC_AUTH_CHILD_PIPE", &name)
                .env(
                    "OWO_SYNTHETIC_AUTH_CHILD_DENY",
                    if denial { "1" } else { "0" },
                )
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), server.connect())
                .await
                .unwrap()
                .unwrap();
            let mut kernel_pid = 0;
            assert_ne!(
                unsafe {
                    windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(
                        server.as_raw_handle().cast(),
                        &mut kernel_pid,
                    )
                },
                0
            );
            assert_eq!(Some(kernel_pid), child.id());
            assert_ne!(kernel_pid, std::process::id());
            let mut binding = crate::peer::connected_connector_binding(&server).unwrap();
            assert!(binding.executable_sha256 == expected_image);
            if denial {
                binding.executable_sha256[0] ^= 1;
            }
            let mut registry = PairingRegistry::default();
            registry
                .approve_with_key_from_trusted_host(
                    [1; 32],
                    binding,
                    Instant::now(),
                    Duration::from_secs(30),
                    zeroize::Zeroizing::new([3; 32]),
                )
                .unwrap();
            let handler = Arc::new(Probe(AtomicUsize::new(0)));
            let h: Arc<dyn FrameHandler> = handler.clone();
            let result = serve_authenticated_connection(
                server,
                h,
                Arc::new(Mutex::new(registry)),
                Duration::from_secs(10),
            )
            .await;
            if denial {
                assert!(
                    matches!(result,Err(PipeError::Io(ref e)) if e.kind()==io::ErrorKind::PermissionDenied)
                );
                assert_eq!(handler.0.load(Ordering::SeqCst), 0);
            } else {
                assert!(result.is_ok());
                assert_eq!(handler.0.load(Ordering::SeqCst), 1);
            }
            assert!(tokio::time::timeout(Duration::from_secs(10), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success());
        }
    }
    #[tokio::test]
    #[ignore = "requires separately built C++ owo_agent_auth_tests executable"]
    async fn cpp_credential_dpapi_hmac_and_authenticated_pipe_interoperate() {
        use crate::{credential_store::CredentialStore, pairing_key::WrappedPairingKey};
        use sha2::{Digest, Sha256};
        let executable = std::path::PathBuf::from(
            std::env::var_os("OWO_CPP_AUTH_CLIENT_TEST_EXE").expect("native client path required"),
        );
        assert!(executable.is_file());
        let parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.work-logs/cpp-proof-tests");
        std::fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let root = parent.join(format!("session-{}", std::process::id()));
        let store = CredentialStore::create_new(&root).unwrap();
        let id = [1; 32];
        let key = zeroize::Zeroizing::new([3; 32]);
        let wrapped = WrappedPairingKey::protect(&key, &id).unwrap();
        store.persist_new(&id, &wrapped).unwrap();
        let credential = store.credential_path(&id).unwrap();
        let name = format!(r"\\.\pipe\OwO.CppProof.{}", std::process::id());
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .unwrap();
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../contracts/agent-ipc/fixtures/request.submit.valid.json")
            .canonicalize()
            .unwrap();
        let mut child = tokio::process::Command::new(&executable)
            .arg(&credential)
            .arg(&name)
            .arg(&fixture)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), server.connect())
            .await
            .unwrap()
            .unwrap();
        let binding = crate::peer::connected_connector_binding(&server).unwrap();
        let expected: [u8; 32] = Sha256::digest(std::fs::read(&executable).unwrap()).into();
        assert!(binding.executable_sha256 == expected);
        let mut registry = PairingRegistry::default();
        registry
            .approve_with_key_from_trusted_host(
                id,
                binding,
                Instant::now(),
                Duration::from_secs(30),
                key,
            )
            .unwrap();
        let handler = Arc::new(Probe(AtomicUsize::new(0)));
        let h: Arc<dyn FrameHandler> = handler.clone();
        let registry = Arc::new(Mutex::new(registry));
        serve_authenticated_connection(server, h, registry.clone(), Duration::from_secs(10))
            .await
            .unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
        assert_eq!(handler.0.load(Ordering::SeqCst), 1);
        // The human approved one process instance, not every copy of this EXE.
        // A second process can read the same user-protected credential but must
        // not inherit the first process enrollment, even with the same image hash.
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .unwrap();
        let mut other = tokio::process::Command::new(&executable)
            .arg(&credential)
            .arg(&name)
            .arg(&fixture)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), server.connect())
            .await
            .unwrap()
            .unwrap();
        let denied = serve_authenticated_connection(
            server,
            handler.clone(),
            registry,
            Duration::from_secs(10),
        )
        .await;
        assert!(
            matches!(denied, Err(PipeError::Io(ref e)) if e.kind() == io::ErrorKind::PermissionDenied)
        );
        assert_eq!(handler.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), other.wait())
                .await
                .unwrap()
                .unwrap()
                .code(),
            Some(11)
        );
        // A malformed protected credential must fail in C++ before connecting.
        let corrupt_id = [2; 32];
        let corrupt =
            WrappedPairingKey::from_encrypted_bytes(b"synthetic invalid protected blob").unwrap();
        store.persist_new(&corrupt_id, &corrupt).unwrap();
        let absent = format!(r"\\.\pipe\OwO.CppProofAbsent.{}", std::process::id());
        let absent_server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&absent)
            .unwrap();
        let mut child = tokio::process::Command::new(&executable)
            .arg(store.credential_path(&corrupt_id).unwrap())
            .arg(&absent)
            .arg(&fixture)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.code(), Some(10));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), absent_server.connect())
                .await
                .is_err()
        );
        // Even a valid DPAPI blob is refused if its file ACL becomes non-private.
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Security::{
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
                    SDDL_REVISION_1, SE_FILE_OBJECT,
                },
                GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION,
                PROTECTED_DACL_SECURITY_INFORMATION,
            };
            let sddl: Vec<u16> = "D:P(A;;FA;;;WD)".encode_utf16().chain(Some(0)).collect();
            let mut descriptor = std::ptr::null_mut();
            assert_ne!(
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        sddl.as_ptr(),
                        SDDL_REVISION_1,
                        &mut descriptor,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            let mut acl = std::ptr::null_mut();
            let mut present = 0;
            let mut defaulted = 0;
            assert_ne!(
                unsafe {
                    GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted)
                },
                0
            );
            let path: Vec<u16> = credential
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            assert_eq!(
                unsafe {
                    SetNamedSecurityInfoW(
                        path.as_ptr(),
                        SE_FILE_OBJECT,
                        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        acl,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            unsafe {
                windows_sys::Win32::Foundation::LocalFree(descriptor);
            }
        }
        let mut child = tokio::process::Command::new(&executable)
            .arg(&credential)
            .arg(&absent)
            .arg(&fixture)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), child.wait())
                .await
                .unwrap()
                .unwrap()
                .code(),
            Some(10)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(300), absent_server.connect())
                .await
                .is_err()
        );
        store.remove(&id).unwrap();
        store.remove(&corrupt_id).unwrap();
        drop(store);
        std::fs::remove_dir(&root).unwrap();
    }
    #[tokio::test]
    async fn authenticated_listener_never_falls_back_to_plain_json() {
        let name = format!(r"\\.\pipe\OwO.AuthListener.{}", std::process::id());
        let binding = crate::peer::process_connector_binding(std::process::id()).unwrap();
        let mut registry = PairingRegistry::default();
        registry
            .approve_with_key_from_trusted_host(
                [1; 32],
                binding,
                Instant::now(),
                Duration::from_secs(30),
                zeroize::Zeroizing::new([3; 32]),
            )
            .unwrap();
        let registry = Arc::new(Mutex::new(registry));
        let handler = Arc::new(Probe(AtomicUsize::new(0)));
        let h: Arc<dyn FrameHandler> = handler.clone();
        let (tx, rx) = tokio::sync::watch::channel(false);
        let endpoint = name.clone();
        let task = tokio::spawn(async move {
            crate::pipe::run_authenticated_pipe_server(
                &endpoint,
                Duration::from_secs(10),
                h,
                rx,
                registry,
            )
            .await
        });
        let payload =
            include_bytes!("../../../../contracts/agent-ipc/fixtures/request.submit.valid.json");
        for authenticated in [false, true] {
            let mut client = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match ClientOptions::new().open(&name) {
                        Ok(client) => break client,
                        Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                    }
                }
            })
            .await
            .unwrap();
            let bytes = if authenticated {
                envelope(payload, [2; 32])
            } else {
                payload.to_vec()
            };
            frame::write_frame(&mut client, &bytes).await.unwrap();
            let result =
                tokio::time::timeout(Duration::from_secs(5), frame::read_frame(&mut client))
                    .await
                    .unwrap();
            if authenticated {
                assert_eq!(result.unwrap(), b"synthetic response");
            } else {
                assert!(result.is_err());
                assert_eq!(handler.0.load(Ordering::SeqCst), 0);
            }
        }
        assert_eq!(handler.0.load(Ordering::SeqCst), 1);
        tx.send(true).unwrap();
        task.await.unwrap().unwrap();
    }
}
