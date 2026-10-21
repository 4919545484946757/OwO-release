//! Host-only enrollment transaction. Not a network route or a substitute for user consent.
//! The host must obtain the binding from the kernel and digest from trusted install metadata.
use crate::{
    pairing::{ConnectorBinding, PairingRegistry, MAX_PAIRING_LIFETIME},
    pairing_key::{generate_key, WrappedPairingKey},
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
const CONSENT_TTL: Duration = Duration::from_secs(60);
const MAX_PENDING: usize = 32;

// Deliberately not Debug/Serialize: do not place pending consent data in ordinary logs.
#[derive(Clone)]
pub struct EnrollmentChallenge([u8; 32]);
struct Pending {
    binding: ConnectorBinding,
    issued: Instant,
    expires: Instant,
}
#[derive(Default)]
pub struct EnrollmentHost {
    pending: HashMap<[u8; 32], Pending>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum EnrollmentError {
    UntrustedInstallation,
    InvalidLifetime,
    Capacity,
    UnknownChallenge,
    Expired,
    IdentityChanged,
    Randomness,
    KeyProtection,
    RegistryRejected,
    Persistence,
}
pub struct IssuedPairing {
    pub id: [u8; 32],
    pub protected_key: WrappedPairingKey,
}
impl EnrollmentHost {
    /// Prepare only from local trusted UI/host, not a connector request.
    /// This creates no grant. Host UI must show the reviewed identity/install details.
    pub fn prepare(
        &mut self,
        binding: ConnectorBinding,
        approved_install_sha256: [u8; 32],
        now: Instant,
    ) -> Result<EnrollmentChallenge, EnrollmentError> {
        if approved_install_sha256 == [0; 32]
            || binding.executable_sha256 != approved_install_sha256
            || binding.user_sid.is_empty()
        {
            return Err(EnrollmentError::UntrustedInstallation);
        }
        self.pending
            .retain(|_, p| now >= p.issued && now < p.expires);
        if self.pending.len() >= MAX_PENDING {
            return Err(EnrollmentError::Capacity);
        }
        let challenge = *generate_key().map_err(|_| EnrollmentError::Randomness)?;
        if self.pending.contains_key(&challenge) {
            return Err(EnrollmentError::Randomness);
        }
        let expires = now
            .checked_add(CONSENT_TTL)
            .ok_or(EnrollmentError::InvalidLifetime)?;
        self.pending.insert(
            challenge,
            Pending {
                binding,
                issued: now,
                expires,
            },
        );
        Ok(EnrollmentChallenge(challenge))
    }
    /// Call only from the trusted confirmation action; never deserialize "approved"
    /// from a request. Reacquire the kernel binding and trusted installation digest.
    /// Challenge is consumed even if the identity, wrap, or registration later fails.
    pub fn confirm_from_trusted_ui(
        &mut self,
        challenge: EnrollmentChallenge,
        current: ConnectorBinding,
        approved_install_sha256: [u8; 32],
        now: Instant,
        lifetime: Duration,
        registry: &mut PairingRegistry,
    ) -> Result<IssuedPairing, EnrollmentError> {
        let pending = self
            .pending
            .remove(&challenge.0)
            .ok_or(EnrollmentError::UnknownChallenge)?;
        if now < pending.issued || now >= pending.expires {
            return Err(EnrollmentError::Expired);
        }
        if current != pending.binding
            || current.executable_sha256 != approved_install_sha256
            || approved_install_sha256 == [0; 32]
        {
            return Err(EnrollmentError::IdentityChanged);
        }
        if lifetime.is_zero() || lifetime > MAX_PAIRING_LIFETIME {
            return Err(EnrollmentError::InvalidLifetime);
        }
        let id = *generate_key().map_err(|_| EnrollmentError::Randomness)?;
        let key = generate_key().map_err(|_| EnrollmentError::Randomness)?;
        // Complete fallible protection before exposing an active in-memory grant.
        let protected_key =
            WrappedPairingKey::protect(&key, &id).map_err(|_| EnrollmentError::KeyProtection)?;
        registry
            .approve_with_key_from_trusted_host(id, current, now, lifetime, key)
            .map_err(|_| EnrollmentError::RegistryRejected)?;
        Ok(IssuedPairing { id, protected_key })
    }
    /// Caller must hold exclusive registry access until return. A failed publish
    /// revokes the in-memory record and drops its key before the lock is released.
    #[allow(clippy::too_many_arguments)]
    pub fn confirm_and_persist_from_trusted_ui(
        &mut self,
        challenge: EnrollmentChallenge,
        current: ConnectorBinding,
        approved_install_sha256: [u8; 32],
        now: Instant,
        lifetime: Duration,
        registry: &mut PairingRegistry,
        store: &crate::credential_store::CredentialStore,
    ) -> Result<IssuedPairing, EnrollmentError> {
        let issued = self.confirm_from_trusted_ui(
            challenge,
            current,
            approved_install_sha256,
            now,
            lifetime,
            registry,
        )?;
        Self::publish_or_revoke(issued, registry, |issued| {
            store.persist_new(&issued.id, &issued.protected_key)
        })
    }
    fn publish_or_revoke(
        issued: IssuedPairing,
        registry: &mut PairingRegistry,
        publish: impl FnOnce(&IssuedPairing) -> std::io::Result<()>,
    ) -> Result<IssuedPairing, EnrollmentError> {
        if publish(&issued).is_err() {
            // No fallback to a memory-only grant after a disk error.
            let _ = registry.revoke(&issued.id);
            return Err(EnrollmentError::Persistence);
        }
        Ok(issued)
    }

    pub fn cancel(&mut self, challenge: EnrollmentChallenge) -> bool {
        self.pending.remove(&challenge.0).is_some()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    fn binding() -> ConnectorBinding {
        ConnectorBinding {
            process_id: 12,
            process_created_at: 12345,
            user_sid: vec![1],
            session_id: 1,
            authentication_id: (2, 0),
            executable_sha256: [4; 32],
        }
    }
    #[test]
    fn trusted_confirmation_issues_once_and_protected_key_authenticates() {
        let mut host = EnrollmentHost::default();
        let mut registry = PairingRegistry::default();
        let now = Instant::now();
        let challenge = host.prepare(binding(), [4; 32], now).unwrap();
        let retry = challenge.clone();
        let issued = host
            .confirm_from_trusted_ui(
                challenge,
                binding(),
                [4; 32],
                now,
                Duration::from_secs(10),
                &mut registry,
            )
            .unwrap();
        assert!(matches!(
            host.confirm_from_trusted_ui(
                retry,
                binding(),
                [4; 32],
                now,
                Duration::from_secs(10),
                &mut registry
            ),
            Err(EnrollmentError::UnknownChallenge)
        ));
        let key = issued.protected_key.unprotect(&issued.id).unwrap();
        let nonce = [5; 32];
        let payload = b"synthetic";
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key.as_ref()).unwrap();
        mac.update(b"OwO/IME/pipe-request/v1\0");
        mac.update(&issued.id);
        mac.update(&nonce);
        mac.update(&(payload.len() as u64).to_le_bytes());
        mac.update(payload);
        let tag: [u8; 32] = mac.finalize().into_bytes().into();
        assert_eq!(
            registry.authorize_proven_request(&issued.id, &binding(), nonce, payload, &tag, now),
            Ok(())
        );
        registry.revoke(&issued.id).unwrap();
        assert!(registry.check(&issued.id, &binding(), now).is_err());
    }
    #[test]
    fn expired_cancelled_or_changed_consent_cannot_be_reused() {
        let now = Instant::now();
        let mut registry = PairingRegistry::default();
        let mut host = EnrollmentHost::default();
        for case in 0..4 {
            let c = host.prepare(binding(), [4; 32], now).unwrap();
            let replay = c.clone();
            let mut actual = binding();
            if case == 0 {
                assert!(host.cancel(c));
                assert!(!host.cancel(replay));
                continue;
            }
            if case == 1 {
                actual.session_id += 1;
            }
            let at = if case == 2 { now + CONSENT_TTL } else { now };
            let digest = if case == 3 { [9; 32] } else { [4; 32] };
            assert!(host
                .confirm_from_trusted_ui(
                    c,
                    actual,
                    digest,
                    at,
                    Duration::from_secs(10),
                    &mut registry
                )
                .is_err());
            assert!(matches!(
                host.confirm_from_trusted_ui(
                    replay,
                    binding(),
                    [4; 32],
                    now,
                    Duration::from_secs(10),
                    &mut registry
                ),
                Err(EnrollmentError::UnknownChallenge)
            ));
        }
    }
    #[test]
    fn untrusted_install_and_pending_flood_fail_closed() {
        let now = Instant::now();
        let mut host = EnrollmentHost::default();
        assert!(matches!(
            host.prepare(binding(), [9; 32], now),
            Err(EnrollmentError::UntrustedInstallation)
        ));
        for _ in 0..MAX_PENDING {
            host.prepare(binding(), [4; 32], now).unwrap();
        }
        assert!(matches!(
            host.prepare(binding(), [4; 32], now),
            Err(EnrollmentError::Capacity)
        ));
        assert!(host.prepare(binding(), [4; 32], now + CONSENT_TTL).is_ok());
    }
    #[test]
    fn credential_publish_failure_revokes_before_return_and_consumes_consent() {
        let mut host = EnrollmentHost::default();
        let mut registry = PairingRegistry::default();
        let now = Instant::now();
        let challenge = host.prepare(binding(), [4; 32], now).unwrap();
        let replay = challenge.clone();
        let issued = host
            .confirm_from_trusted_ui(
                challenge,
                binding(),
                [4; 32],
                now,
                Duration::from_secs(10),
                &mut registry,
            )
            .unwrap();
        let id = issued.id;
        assert!(registry.check(&id, &binding(), now).is_ok());
        assert!(matches!(
            EnrollmentHost::publish_or_revoke(issued, &mut registry, |_| Err(
                std::io::Error::other("synthetic disk failure")
            )),
            Err(EnrollmentError::Persistence)
        ));
        assert_eq!(
            registry.check(&id, &binding(), now),
            Err(crate::pairing::PairingError::Revoked)
        );
        assert!(matches!(
            host.confirm_from_trusted_ui(
                replay,
                binding(),
                [4; 32],
                now,
                Duration::from_secs(10),
                &mut registry
            ),
            Err(EnrollmentError::UnknownChallenge)
        ));
    }
    #[test]
    fn confirmed_enrollment_publishes_private_credential_and_revoke_removes_it() {
        let parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.work-logs/enrollment-store-tests");
        std::fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let random = *generate_key().unwrap();
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root = parent.join(suffix);
        let store = crate::credential_store::CredentialStore::create_new(&root).unwrap();
        let now = Instant::now();
        let mut registry = PairingRegistry::default();
        let mut host = EnrollmentHost::default();
        let challenge = host.prepare(binding(), [4; 32], now).unwrap();
        let issued = host
            .confirm_and_persist_from_trusted_ui(
                challenge,
                binding(),
                [4; 32],
                now,
                Duration::from_secs(10),
                &mut registry,
                &store,
            )
            .unwrap();
        assert!(registry.check(&issued.id, &binding(), now).is_ok());
        assert!(store.credential_path(&issued.id).unwrap().is_file());
        registry.revoke(&issued.id).unwrap();
        store.remove(&issued.id).unwrap();
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        drop(store);
        std::fs::remove_dir(root).unwrap();
    }
}
