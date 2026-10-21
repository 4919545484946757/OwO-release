//! In-memory pairing lifecycle and exact-frame proof verification; no enrollment endpoint.
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
    pub process_id: u32,
    pub process_created_at: u64,
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
    proof_key: Option<zeroize::Zeroizing<[u8; 32]>>,
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
    InvalidProof,
    InvalidEnvelope,
    InvalidPayload,
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
        if binding.process_id == 0
            || binding.process_created_at == 0
            || binding.user_sid.is_empty()
            || binding.executable_sha256 == [0; 32]
            || id == [0; 32]
        {
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
                proof_key: None,
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
    fn consume_verified_request(
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

    /// Trusted enrollment only. Never reuse the model API key or HTTP bearer here.
    /// Legacy lifecycle-only records have no key and cannot authorize a request.
    pub fn approve_with_key_from_trusted_host(
        &mut self,
        id: [u8; 32],
        binding: ConnectorBinding,
        now: Instant,
        lifetime: Duration,
        key: zeroize::Zeroizing<[u8; 32]>,
    ) -> Result<(), PairingError> {
        if *key == [0; 32] {
            return Err(PairingError::InvalidProof);
        }
        self.approve_from_trusted_host(id, binding, now, lifetime)?;
        self.records
            .get_mut(&id)
            .ok_or(PairingError::Unknown)?
            .proof_key = Some(key);
        Ok(())
    }

    /// Verify the exact framed payload BEFORE parsing it or dispatching business logic.
    /// Proof bytes cover domain, pairing ID, nonce, little-endian length and raw payload.
    /// A host must enforce one parser's semantics (including duplicate-key rejection).
    /// This is a local transport proof, not the later canonical signed Intent Capsule.
    #[allow(clippy::too_many_arguments)] // Keep proof bytes and kernel identity explicit at this boundary.
    pub fn authorize_proven_request(
        &mut self,
        id: &[u8; 32],
        actual: &ConnectorBinding,
        nonce: [u8; 32],
        payload: &[u8],
        tag: &[u8; 32],
        now: Instant,
    ) -> Result<(), PairingError> {
        use hmac::{Hmac, Mac};
        self.check(id, actual, now)?;
        if payload.is_empty() || payload.len() > crate::protocol::MAXIMUM_AGENT_PAYLOAD_BYTES {
            return Err(PairingError::InvalidPayload);
        }
        let record = self.records.get(id).ok_or(PairingError::Unknown)?;
        let key = record
            .proof_key
            .as_ref()
            .ok_or(PairingError::InvalidProof)?;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key.as_ref())
            .map_err(|_| PairingError::InvalidProof)?;
        mac.update(b"OwO/IME/pipe-request/v1\0");
        mac.update(id);
        mac.update(&nonce);
        mac.update(&(payload.len() as u64).to_le_bytes());
        mac.update(payload);
        mac.verify_slice(tag)
            .map_err(|_| PairingError::InvalidProof)?;
        self.consume_verified_request(id, actual, nonce, now)
    }

    /// Authenticate exact envelope bytes before strict DTO decoding. No legacy fallback.
    /// Caller holds the registry lock through verification/consumption; dispatch follows.
    pub fn authorize_frame(
        &mut self,
        actual: &ConnectorBinding,
        bytes: &[u8],
        now: Instant,
    ) -> Result<crate::protocol::AgentIpcRequest, PairingError> {
        let frame = crate::proof_frame::ProofFrame::decode(bytes)?;
        self.authorize_proven_request(
            frame.pairing_id,
            actual,
            *frame.nonce,
            frame.payload,
            frame.tag,
            now,
        )?;
        crate::protocol::AgentIpcRequest::decode(frame.payload)
            .map_err(|_| PairingError::InvalidPayload)
    }

    pub fn revoke(&mut self, id: &[u8; 32]) -> Result<(), PairingError> {
        let record = self.records.get_mut(id).ok_or(PairingError::Unknown)?;
        record.revoked = true;
        record.proof_key = None; // Zeroizing drops the revoked key immediately.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> ConnectorBinding {
        ConnectorBinding {
            process_id: 12,
            process_created_at: 12345,
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
        for field in 0..7 {
            let mut wrong = binding();
            match field {
                0 => wrong.user_sid.push(4),
                1 => wrong.session_id += 1,
                2 => wrong.authentication_id.0 += 1,
                3 => wrong.authentication_id.1 += 1,
                4 => wrong.executable_sha256[0] ^= 1,
                5 => wrong.process_id += 1,
                _ => wrong.process_created_at += 1,
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
        for field in 0..2 {
            let mut invalid = binding();
            if field == 0 {
                invalid.process_id = 0;
            } else {
                invalid.process_created_at = 0;
            }
            assert_eq!(
                store.approve_from_trusted_host([1; 32], invalid, now, MAX_PAIRING_LIFETIME),
                Err(PairingError::InvalidBinding)
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
    fn tag(key: &[u8], id: &[u8; 32], nonce: &[u8; 32], payload: &[u8]) -> [u8; 32] {
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
        mac.update(b"OwO/IME/pipe-request/v1\0");
        mac.update(id);
        mac.update(nonce);
        mac.update(&(payload.len() as u64).to_le_bytes());
        mac.update(payload);
        mac.finalize().into_bytes().into()
    }
    #[test]
    fn proof_binds_entire_frame_and_consumes_only_after_verification() {
        let mut store = PairingRegistry::default();
        let now = Instant::now();
        let id = [1; 32];
        let nonce = [2; 32];
        let key = [3; 32];
        let payload = b"synthetic text";
        store
            .approve_with_key_from_trusted_host(
                id,
                binding(),
                now,
                MAX_PAIRING_LIFETIME,
                zeroize::Zeroizing::new(key),
            )
            .unwrap();
        let valid = tag(&key, &id, &nonce, payload);
        assert_eq!(
            valid,
            [
                150, 72, 218, 47, 139, 151, 240, 49, 153, 89, 238, 98, 178, 176, 189, 40, 237, 177,
                224, 35, 2, 70, 130, 73, 213, 18, 58, 230, 60, 94, 84, 88
            ]
        );
        for bad in [
            b"synthetic texT".as_slice(),
            b"synthetic text extra".as_slice(),
        ] {
            assert_eq!(
                store.authorize_proven_request(&id, &binding(), nonce, bad, &valid, now),
                Err(PairingError::InvalidProof)
            );
        }
        assert_eq!(
            store.authorize_proven_request(&id, &binding(), [4; 32], payload, &valid, now),
            Err(PairingError::InvalidProof)
        );
        let wrong_key = tag(&[5; 32], &id, &nonce, payload);
        assert_eq!(
            store.authorize_proven_request(&id, &binding(), nonce, payload, &wrong_key, now),
            Err(PairingError::InvalidProof)
        );
        assert_eq!(
            store.authorize_proven_request(&id, &binding(), nonce, payload, &valid, now),
            Ok(())
        );
        assert_eq!(
            store.authorize_proven_request(&id, &binding(), nonce, payload, &valid, now),
            Err(PairingError::Replay)
        );
        store.revoke(&id).unwrap();
        assert_eq!(
            store.authorize_proven_request(
                &id,
                &binding(),
                [6; 32],
                payload,
                &tag(&key, &id, &[6; 32], payload),
                now
            ),
            Err(PairingError::Revoked)
        );
    }
    #[test]
    fn keyless_records_and_cross_pairing_proofs_fail_closed() {
        let mut store = PairingRegistry::default();
        let now = Instant::now();
        let key = [3; 32];
        let nonce = [2; 32];
        store
            .approve_from_trusted_host([1; 32], binding(), now, MAX_PAIRING_LIFETIME)
            .unwrap();
        let valid = tag(&key, &[1; 32], &nonce, b"synthetic");
        assert_eq!(
            store.authorize_proven_request(&[1; 32], &binding(), nonce, b"synthetic", &valid, now),
            Err(PairingError::InvalidProof)
        );
        store
            .approve_with_key_from_trusted_host(
                [4; 32],
                binding(),
                now,
                MAX_PAIRING_LIFETIME,
                zeroize::Zeroizing::new(key),
            )
            .unwrap();
        assert_eq!(
            store.authorize_proven_request(&[4; 32], &binding(), nonce, b"synthetic", &valid, now),
            Err(PairingError::InvalidProof)
        );
        assert_eq!(
            store.authorize_proven_request(&[4; 32], &binding(), nonce, b"", &valid, now),
            Err(PairingError::InvalidPayload)
        );
    }
}
