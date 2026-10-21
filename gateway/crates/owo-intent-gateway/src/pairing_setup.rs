//! Explicit interactive setup. No dialog is shown by ordinary requests or tests.
use crate::{
    credential_store::CredentialStore, enrollment::EnrollmentHost, pairing::PairingRegistry,
};
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub struct SetupOptions {
    pub connector_pid: u32,
    pub approved_sha256: [u8; 32],
    pub directory: PathBuf,
}
/// Lifetime guard: revoke before deleting the published credential on normal exit.
/// A crashed process may leave an encrypted file, but no runtime grant is restored.
pub struct PairingSession {
    pub registry: Arc<Mutex<PairingRegistry>>,
    store: CredentialStore,
    id: [u8; 32],
}
impl Drop for PairingSession {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            let _ = registry.revoke(&self.id);
        }
        if self.store.remove(&self.id).is_err() {
            tracing::warn!(
                "Could not remove expired pairing credential; runtime grant remains revoked"
            );
        }
    }
}
impl SetupOptions {
    pub fn parse(args: &[String]) -> io::Result<Self> {
        if args.len() != 6 {
            return Err(io::Error::other("required: --pair-connector-pid PID --pair-image-sha256 SHA256 --pair-credential-directory NEW_ABSOLUTE_DIRECTORY"));
        }
        let mut pid = None;
        let mut hash = None;
        let mut directory = None;
        for pair in args.chunks_exact(2) {
            match pair[0].as_str() {
                "--pair-connector-pid" if pid.is_none() => {
                    pid = Some(
                        pair[1]
                            .parse::<u32>()
                            .map_err(|_| io::Error::other("invalid connector PID"))?,
                    );
                }
                "--pair-image-sha256" if hash.is_none() => {
                    if pair[1].len() != 64 || !pair[1].bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(io::Error::other("invalid installation SHA256"));
                    }
                    let mut value = [0u8; 32];
                    for (i, b) in value.iter_mut().enumerate() {
                        *b = u8::from_str_radix(&pair[1][i * 2..i * 2 + 2], 16)
                            .map_err(|_| io::Error::other("invalid installation SHA256"))?;
                    }
                    hash = Some(value);
                }
                "--pair-credential-directory" if directory.is_none() => {
                    if pair[1].chars().any(char::is_control) {
                        return Err(io::Error::other("invalid credential directory"));
                    }
                    directory = Some(PathBuf::from(&pair[1]));
                }
                _ => return Err(io::Error::other("unknown or duplicate pairing option")),
            }
        }
        let value = Self {
            connector_pid: pid.ok_or_else(|| io::Error::other("connector PID required"))?,
            approved_sha256: hash
                .ok_or_else(|| io::Error::other("installation SHA256 required"))?,
            directory: directory
                .ok_or_else(|| io::Error::other("credential directory required"))?,
        };
        if value.connector_pid == 0
            || value.approved_sha256 == [0; 32]
            || !value.directory.is_absolute()
        {
            return Err(io::Error::other("invalid pairing options"));
        }
        Ok(value)
    }
}
/// Must be explicitly invoked locally. Actual human confirmation is never simulated.
/// The expected installation digest is supplied by the operator's reviewed build receipt,
/// not the connector. This is developer enrollment, not signed-installer attestation.
pub fn confirm_interactively(options: SetupOptions) -> io::Result<PairingSession> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO,
    };
    let binding = crate::peer::process_connector_binding(options.connector_pid)?;
    let mut host = EnrollmentHost::default();
    let challenge = host
        .prepare(binding, options.approved_sha256, Instant::now())
        .map_err(|_| io::Error::other("connector does not match the reviewed installation"))?;
    let digest: String = options
        .approved_sha256
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let text=format!("允许此 OwO 连接器提交你明确确认的文本请求吗？有效期一小时。\n\n连接器进程：{}\n已核对的程序 SHA256：{}\n加密凭据目录：{}\n\n改写、翻译等表达请求可能将文本发送给已配置的云端模型。本次配对不会授予工具、MCP、hooks 或自主执行权限。凭据仅为当前 Windows 用户加密保护。如果不是你主动发起，请选“否”。",options.connector_pid,digest,options.directory.display());
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "OwO - 确认连接器配对"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    if unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    } != IDYES
    {
        return Err(io::Error::other("pairing not approved"));
    }
    let current = crate::peer::process_connector_binding(options.connector_pid)?;
    let store = CredentialStore::create_new(&options.directory)?;
    let mut registry = PairingRegistry::default();
    let issued = host
        .confirm_and_persist_from_trusted_ui(
            challenge,
            current,
            options.approved_sha256,
            Instant::now(),
            Duration::from_secs(3600),
            &mut registry,
            &store,
        )
        .map_err(|_| io::Error::other("pairing enrollment failed"))?;
    // Only the credential location, never plaintext key or encrypted blob.
    println!(
        "Pairing active for this gateway process; configure connector credential: {}",
        store.credential_path(&issued.id)?.display()
    );
    Ok(PairingSession {
        registry: Arc::new(Mutex::new(registry)),
        store,
        id: issued.id,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_is_explicit_bounded_and_rejects_missing_duplicate_or_bad_options() {
        assert!(SetupOptions::parse(&[]).is_err());
        let valid = vec![
            "--pair-connector-pid".into(),
            "12".into(),
            "--pair-image-sha256".into(),
            "04".repeat(32),
            "--pair-credential-directory".into(),
            r"D:\synthetic\new-pair".into(),
        ];
        assert!(SetupOptions::parse(&valid).is_ok());
        for (index, value) in [
            (1, "0"),
            (3, "not-a-digest"),
            (5, "relative"),
            (4, "--pair-image-sha256"),
        ] {
            let mut invalid = valid.clone();
            invalid[index] = value.into();
            assert!(SetupOptions::parse(&invalid).is_err());
        }
    }
    #[test]
    fn normal_session_exit_revokes_grant_and_removes_credential() {
        let parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.work-logs/setup-session-tests");
        std::fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let random = *crate::pairing_key::generate_key().unwrap();
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root = parent.join(suffix);
        let store = CredentialStore::create_new(&root).unwrap();
        let id = [1; 32];
        let key = zeroize::Zeroizing::new([7; 32]);
        let protected = crate::pairing_key::WrappedPairingKey::protect(&key, &id).unwrap();
        store.persist_new(&id, &protected).unwrap();
        let file = store.credential_path(&id).unwrap();
        let binding = crate::peer::process_connector_binding(std::process::id()).unwrap();
        let mut registry = PairingRegistry::default();
        registry
            .approve_with_key_from_trusted_host(
                id,
                binding.clone(),
                Instant::now(),
                Duration::from_secs(10),
                key,
            )
            .unwrap();
        let registry = Arc::new(Mutex::new(registry));
        let session = PairingSession {
            registry: registry.clone(),
            store,
            id,
        };
        assert!(file.is_file());
        drop(session);
        assert!(!file.exists());
        assert_eq!(
            registry
                .lock()
                .unwrap()
                .check(&id, &binding, Instant::now()),
            Err(crate::pairing::PairingError::Revoked)
        );
        std::fs::remove_dir(root).unwrap();
    }
}
