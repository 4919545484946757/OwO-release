//! Candidate authenticated transport payload. Opt-in only; no legacy auto-detection.
//! Existing production JSON pipe mode is unchanged until trusted enrollment is wired.
use crate::{pairing::PairingError, protocol::MAXIMUM_AGENT_PAYLOAD_BYTES};

const MAGIC: &[u8; 8] = b"OwOPRF01";
pub const PROOF_HEADER_BYTES: usize = 8 + 32 + 32 + 32 + 4;
pub const MAX_PROVEN_PAYLOAD_BYTES: usize = MAXIMUM_AGENT_PAYLOAD_BYTES - PROOF_HEADER_BYTES;

// No Debug: the borrowed payload may contain private source text.
pub struct ProofFrame<'a> {
    pub pairing_id: &'a [u8; 32],
    pub nonce: &'a [u8; 32],
    pub tag: &'a [u8; 32],
    pub payload: &'a [u8],
}
impl<'a> ProofFrame<'a> {
    /// Layout: magic[8], id[32], nonce[32], HMAC[32], payload_len u32 LE, raw JSON.
    /// The outer existing u32 frame length includes this entire envelope.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, PairingError> {
        if bytes.len() <= PROOF_HEADER_BYTES || bytes.len() > MAXIMUM_AGENT_PAYLOAD_BYTES {
            return Err(PairingError::InvalidEnvelope);
        }
        if &bytes[..8] != MAGIC {
            return Err(PairingError::InvalidEnvelope);
        }
        let length = u32::from_le_bytes(
            bytes[104..108]
                .try_into()
                .map_err(|_| PairingError::InvalidEnvelope)?,
        ) as usize;
        if length == 0
            || length > MAX_PROVEN_PAYLOAD_BYTES
            || length != bytes.len() - PROOF_HEADER_BYTES
        {
            return Err(PairingError::InvalidEnvelope);
        }
        let pairing_id = bytes[8..40]
            .try_into()
            .map_err(|_| PairingError::InvalidEnvelope)?;
        let nonce = bytes[40..72]
            .try_into()
            .map_err(|_| PairingError::InvalidEnvelope)?;
        let tag = bytes[72..104]
            .try_into()
            .map_err(|_| PairingError::InvalidEnvelope)?;
        if pairing_id == &[0; 32] || nonce == &[0; 32] {
            return Err(PairingError::InvalidEnvelope);
        }
        Ok(Self {
            pairing_id,
            nonce,
            tag,
            payload: &bytes[PROOF_HEADER_BYTES..],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::{ConnectorBinding, PairingRegistry, MAX_PAIRING_LIFETIME};
    use hmac::{Hmac, Mac};
    use std::time::Instant;
    fn binding() -> ConnectorBinding {
        ConnectorBinding {
            process_id: 12,
            process_created_at: 12345,
            user_sid: vec![1],
            session_id: 1,
            authentication_id: (1, 0),
            executable_sha256: [1; 32],
        }
    }
    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&[3; 32]).unwrap();
        mac.update(b"OwO/IME/pipe-request/v1\0");
        mac.update(&[1; 32]);
        mac.update(&[2; 32]);
        mac.update(&(payload.len() as u64).to_le_bytes());
        mac.update(payload);
        let mut bytes = b"OwOPRF01".to_vec();
        bytes.extend_from_slice(&[1; 32]);
        bytes.extend_from_slice(&[2; 32]);
        bytes.extend_from_slice(&mac.finalize().into_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
    fn store(now: Instant) -> PairingRegistry {
        let mut store = PairingRegistry::default();
        store
            .approve_with_key_from_trusted_host(
                [1; 32],
                binding(),
                now,
                MAX_PAIRING_LIFETIME,
                zeroize::Zeroizing::new([3; 32]),
            )
            .unwrap();
        store
    }
    #[test]
    fn authenticated_envelope_decodes_real_request_once() {
        let payload =
            include_bytes!("../../../../contracts/agent-ipc/fixtures/request.submit.valid.json");
        let bytes = frame(payload);
        let now = Instant::now();
        let mut store = store(now);
        assert!(store.authorize_frame(&binding(), &bytes, now).is_ok());
        assert!(matches!(
            store.authorize_frame(&binding(), &bytes, now),
            Err(PairingError::Replay)
        ));
    }
    #[test]
    fn malformed_envelopes_never_authorize_or_downgrade() {
        let valid = frame(b"{}");
        for cut in 0..valid.len() {
            assert!(ProofFrame::decode(&valid[..cut]).is_err());
        }
        for offset in [0, 104, 105, 106, 107] {
            let mut changed = valid.clone();
            changed[offset] ^= 1;
            assert!(ProofFrame::decode(&changed).is_err());
        }
        let mut trailing = valid.clone();
        trailing.push(0);
        assert!(ProofFrame::decode(&trailing).is_err());
        assert!(ProofFrame::decode(br#"{"schema_version":3}"#).is_err());
        let large = frame(&vec![0; MAX_PROVEN_PAYLOAD_BYTES + 1]);
        assert!(ProofFrame::decode(&large).is_err());
        let boundary = frame(&vec![0; MAX_PROVEN_PAYLOAD_BYTES]);
        assert!(ProofFrame::decode(&boundary).is_ok());
    }
    #[test]
    fn bad_proofs_do_not_consume_but_authenticated_invalid_json_does() {
        let now = Instant::now();
        let mut store = store(now);
        let bytes = frame(br#"{"a":1,"a":2}"#);
        let mut changed = bytes.clone();
        changed[72] ^= 1;
        assert!(matches!(
            store.authorize_frame(&binding(), &changed, now),
            Err(PairingError::InvalidProof)
        ));
        assert!(matches!(
            store.authorize_frame(&binding(), &bytes, now),
            Err(PairingError::InvalidPayload)
        ));
        assert!(matches!(
            store.authorize_frame(&binding(), &bytes, now),
            Err(PairingError::Replay)
        ));
    }
}
