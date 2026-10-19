//! In-memory pairing lifecycle policy, not a credential verifier or enrollment endpoint.
//! Only a trusted host may create records after verifying consent and connector integrity.
//! Callers must supply kernel-derived identity and a verified executable digest, never JSON claims.
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

pub const MAX_PAIRING_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
pub const MAX_PAIRING_RECORDS: usize = 1024;
pub const MAX_REQUEST_NONCES: usize = 4096;

// No Debug/Serialize: identity metadata is not ordinary diagnostic output.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectorBinding {
    pub user_sid: Vec<u8>,
    pub session_id: u32,
    pub authentication_id: (u32, i32),
    pub executable_sha256: [u8; 32],
}

struct Record {
    binding: ConnectorBinding,
    issued: Instant,
    expires: Instant,
    revoked: bool,
    consumed_nonces: HashSet<[u8; 32]>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairingError {
    InvalidLifetime,
    InvalidBinding,
    DuplicateId,
    Capacity,
    Unknown,
    Revoked,
    Expired,
    IdentityMismatch,
    ClockRegression,
    InvalidNonce,
    Replay,
    NonceCapacity,
}

/// Process-local policy only. Restart discards all grants; nothing auto-enrolls.
#[derive(Default)]
pub struct PairingRegistry {
    records: HashMap<[u8; 32], Record>,
}
impl PairingRegistry {
    /// The ID is an identifier, not a bearer credential or proof of request possession.
    /// This method must never be exposed directly to an untrusted pipe request.
    pub fn approve_from_trusted_host(
        &mut self,
        id: [u8; 32],
        binding: ConnectorBinding,
        now: Instant,
        lifetime: Duration,
    ) -> Result<(), PairingError> {
        if lifetime.is_zero() || lifetime > MAX_PAIRING_LIFETIME {
            return Err(PairingError::InvalidLifetime);
        }
        if binding.user_sid.is_empty() || binding.executable_sha256 == [0; 32] || id == [0; 32] {
            return Err(PairingError::InvalidBinding);
        }
        if self.records.contains_key(&id) {
            return Err(PairingError::DuplicateId);
        }
        // Keep tombstones: old IDs must not resurrect after revocation/expiration.
        if self.records.len() >= MAX_PAIRING_RECORDS {
            return Err(PairingError::Capacity);
        }
        let expires = now
            .checked_add(lifetime)
            .ok_or(PairingError::InvalidLifetime)?;
        self.records.insert(
            id,
            Record {
                binding,
                issued: now,
                expires,
                revoked: false,
                consumed_nonces: HashSet::new(),
            },
        );
        Ok(())
    }

    /// Re-check immediately before each authorized operation, including on existing connections.
    /// Success still requires separate cryptographic request proof verification.
    pub fn check(
        &self,
        id: &[u8; 32],
        actual: &ConnectorBinding,
        now: Instant,
    ) -> Result<(), PairingError> {
        let record = self.records.get(id).ok_or(PairingError::Unknown)?;
        if record.revoked {
            return Err(PairingError::Revoked);
        }
        if now < record.issued {
            return Err(PairingError::ClockRegression);
        }
        if now >= record.expires {
            return Err(PairingError::Expired);
        }
        if record.binding != *actual {
            return Err(PairingError::IdentityMismatch);
        }
        Ok(())
    }

    /// Call only after verifying a cryptographic proof binding the complete request
    /// and this nonce. This function does not verify that proof. Consume before
    /// dispatch, including when the handler later fails; retries need a fresh proof.
    /// Keep this check and consumption under the same registry lock in the host.
    pub fn consume_verified_request(
        &mut self,
        id: &[u8; 32],
        actual: &ConnectorBinding,
        nonce: [u8; 32],
        now: Instant,
    ) -> Result<(), PairingError> {
        self.check(id, actual, now)?;
        if nonce == [0; 32] {
            return Err(PairingError::InvalidNonce);
        }
        let record = self.records.get_mut(id).ok_or(PairingError::Unknown)?;
        if record.consumed_nonces.contains(&nonce) {
            return Err(PairingError::Replay);
        }
        // Never evict an old nonce to make room: that would admit old proofs again.
        if record.consumed_nonces.len() >= MAX_REQUEST_NONCES {
            return Err(PairingError::NonceCapacity);
        }
        record.consumed_nonces.insert(nonce);
        Ok(())
    }

    pub fn revoke(&mut self, id: &[u8; 32]) -> Result<(), PairingError> {
        self.records
            .get_mut(id)
            .ok_or(PairingError::Unknown)?
            .revoked = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> ConnectorBinding {
        ConnectorBinding {
            user_sid: vec![1, 2, 3],
            session_id: 7,
            authentication_id: (9, 0),
            executable_sha256: [4; 32],
        }
    }
    #[test]
    fn default_deny_and_expiry_boundary() {
        let mut store = PairingRegistry::default();
        let now = Instant::now();
        let id = [1; 32];
        let b = binding();
        assert_eq!(store.check(&id, &b, now), Err(PairingError::Unknown));
        store
            .approve_from_trusted_host(id, b.clone(), now, Duration::from_secs(10))
            .unwrap();
        assert_eq!(store.check(&id, &b, now + Duration::from_secs(9)), Ok(()));
        assert_eq!(
            store.check(&id, &b, now + Duration::from_secs(10)),
            Err(PairingError::Expired)
        );
        assert_eq!(
            store.check(&id, &b, now - Duration::from_secs(1)),
            Err(PairingError::ClockRegression)
        );
        assert_eq!(
            store.approve_from_trusted_host(
                id,
                b,
                now + Duration::from_secs(11),
                Duration::from_secs(10)
            ),
            Err(PairingError::DuplicateId)
        );
    }
    #[test]
    fn all_identity_components_are_bound() {
        let mut store = PairingRegistry::default();
        let now = Instant::now();
        store
            .approve_from_trusted_host([1; 32], binding(), now, MAX_PAIRING_LIFETIME)
            .unwrap();
        for field in 0..5 {
            let mut wrong = binding();
            match field {
                0 => wrong.user_sid.push(4),
                1 => wrong.session_id += 1,
                2 => wrong.authentication_id.0 += 1,
                3 => wrong.authentication_id.1 += 1,
                _ => wrong.executable_sha256[0] ^= 1,
            }
            assert_eq!(
                store.check(&[1; 32], &wrong, now),
                Err(PairingError::IdentityMismatch)
            );
        }
        assert_eq!(store.check(&[1; 32], &binding(), now), Ok(()));
    }
    #[test]
    fn revocation_is_immediate_idempotent_and_cannot_be_overwritten() {
        let mut store = PairingRegistry::default();
        let now = Instant::now();
        store
            .approve_from_trusted_host([1; 32], binding(), now, MAX_PAIRING_LIFETIME)
            .unwrap();
        assert_eq!(store.check(&[1; 32], &binding(), now), Ok(()));
        for _ in 0..2 {
            assert_eq!(store.revoke(&[1; 32]), Ok(()));
        }
        assert_eq!(
            store.check(&[1; 32], &binding(), now),
            Err(PairingError::Revoked)
        );
        assert_eq!(
            store.approve_from_trusted_host([1; 32], binding(), now, MAX_PAIRING_LIFETIME),
            Err(PairingError::DuplicateId)
        );
        assert_eq!(store.revoke(&[2; 32]), Err(PairingError::Unknown));
    }
    #[test]
    fn invalid_enrollment_does_not_create_grants_and_capacity_preserves_tombstones() {
        let mut store = PairingRegistry::default();
        let now = Instant::now();
        for ttl in [
            Duration::ZERO,
            MAX_PAIRING_LIFETIME + Duration::from_secs(1),
        ] {
            assert_eq!(
                store.approve_from_trusted_host([1; 32], binding(), now, ttl),
                Err(PairingError::InvalidLifetime)
            );
        }
        let mut invalid = binding();
        invalid.executable_sha256 = [0; 32];
        assert_eq!(
            store.approve_from_trusted_host([1; 32], invalid, now, MAX_PAIRING_LIFETIME),
            Err(PairingError::InvalidBinding)
        );
        assert_eq!(
            store.check(&[1; 32], &binding(), now),
            Err(PairingError::Unknown)
        );
        for n in 1..=MAX_PAIRING_RECORDS {
            let mut id = [0; 32];
            id[..8].copy_from_slice(&(n as u64).to_le_bytes());
            store
                .approve_from_trusted_host(id, binding(), now, MAX_PAIRING_LIFETIME)
                .unwrap();
            store.revoke(&id).unwrap();
        }
        assert_eq!(
            store.approve_from_trusted_host([9; 32], binding(), now, MAX_PAIRING_LIFETIME),
            Err(PairingError::Capacity)
        );
    }
    #[test]
    fn request_replay_is_consumed_once_and_rechecked_after_revoke_or_expiry() {
        let mut store = PairingRegistry::default();
        let now = Instant::now();
        let id = [1; 32];
        let b = binding();
        store
            .approve_from_trusted_host(id, b.clone(), now, Duration::from_secs(10))
            .unwrap();
        let mut wrong = b.clone();
        wrong.session_id += 1;
        assert_eq!(
            store.consume_verified_request(&id, &wrong, [2; 32], now),
            Err(PairingError::IdentityMismatch)
        );
        assert_eq!(
            store.consume_verified_request(&id, &b, [0; 32], now),
            Err(PairingError::InvalidNonce)
        );
        assert_eq!(
            store.consume_verified_request(&id, &b, [2; 32], now),
            Ok(())
        );
        assert_eq!(
            store.consume_verified_request(&id, &b, [2; 32], now),
            Err(PairingError::Replay)
        );
        assert_eq!(
            store.consume_verified_request(&id, &b, [3; 32], now + Duration::from_secs(10)),
            Err(PairingError::Expired)
        );
        store.revoke(&id).unwrap();
        assert_eq!(
            store.consume_verified_request(&id, &b, [4; 32], now),
            Err(PairingError::Revoked)
        );
    }
    #[test]
    fn nonce_capacity_does_not_evict_old_proofs_or_affect_other_pairings() {
        let mut store = PairingRegistry::default();
        let now = Instant::now();
        for id in [[1; 32], [2; 32]] {
            store
                .approve_from_trusted_host(id, binding(), now, MAX_PAIRING_LIFETIME)
                .unwrap();
        }
        let mut first = [0; 32];
        first[..8].copy_from_slice(&1u64.to_le_bytes());
        for n in 1..=MAX_REQUEST_NONCES {
            let mut nonce = [0; 32];
            nonce[..8].copy_from_slice(&(n as u64).to_le_bytes());
            assert_eq!(
                store.consume_verified_request(&[1; 32], &binding(), nonce, now),
                Ok(())
            );
        }
        assert_eq!(
            store.consume_verified_request(&[1; 32], &binding(), [9; 32], now),
            Err(PairingError::NonceCapacity)
        );
        assert_eq!(
            store.consume_verified_request(&[1; 32], &binding(), first, now),
            Err(PairingError::Replay)
        );
        assert_eq!(
            store.consume_verified_request(&[2; 32], &binding(), first, now),
            Ok(())
        );
    }
}
