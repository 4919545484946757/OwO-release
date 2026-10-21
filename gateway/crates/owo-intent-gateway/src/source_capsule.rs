//! Read-only provenance issued only after authenticated pipe dispatch.
//! Neither a semantic-intent assertion nor permission to invoke tools.
use crate::{http::BridgeError, pairing::ConnectorBinding, protocol::AgentIpcRequest};
use hmac::{Hmac, Mac};
use owo_agent_protocol::*;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

// No public constructor, Debug or serialization. JSON cannot supply this evidence.
pub struct VerifiedOrigin {
    binding: ConnectorBinding,
    raw: Vec<u8>,
}
impl VerifiedOrigin {
    pub(crate) fn authenticated(binding: ConnectorBinding, raw: Vec<u8>) -> Self {
        Self { binding, raw }
    }
    pub(crate) fn matches(&self, raw: &[u8]) -> bool {
        self.raw == raw
    }
}
struct Receipt {
    _raw: Vec<u8>,
    expires: Instant,
}
pub(crate) struct SourceSigner {
    key: Zeroizing<[u8; 32]>,
    receipts: Mutex<HashMap<String, Receipt>>,
}
fn invalid() -> BridgeError {
    BridgeError::Protocol("expression origin unavailable".into())
}
impl SourceSigner {
    pub(crate) fn from_env() -> Result<Self, BridgeError> {
        let value =
            Zeroizing::new(std::env::var("OWO_EXPRESSION_ORIGIN_KEY").map_err(|_| invalid())?);
        if !canonical_lower_hex(&value, 64) {
            return Err(invalid());
        }
        let mut key = Zeroizing::new([0; 32]);
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).map_err(|_| invalid())?;
        }
        Ok(Self {
            key,
            receipts: Mutex::new(HashMap::new()),
        })
    }
    pub(crate) fn prune(&self) {
        if let Ok(mut receipts) = self.receipts.lock() {
            let now = Instant::now();
            receipts.retain(|_, r| now < r.expires);
        }
    }
    pub(crate) fn issue(
        &self,
        evidence: &VerifiedOrigin,
        request: &AgentIpcRequest,
        prompt: &str,
    ) -> Result<ExpressionOriginCapsule, BridgeError> {
        // Bind the decoded request to the exact kernel-authenticated bytes too.
        let original = AgentIpcRequest::decode(&evidence.raw).map_err(|_| invalid())?;
        if original != *request || evidence.raw.len() > 256 * 1024 {
            return Err(invalid());
        }
        let now = Instant::now();
        let mut receipts = self.receipts.lock().map_err(|_| invalid())?;
        receipts.retain(|_, r| now < r.expires);
        if receipts.len() >= 64 {
            return Err(invalid());
        }
        let nonce =
            uuid::Uuid::new_v4().simple().to_string() + &uuid::Uuid::new_v4().simple().to_string();
        let payload = ExpressionOriginPayload {
            ic_version: 1,
            capsule_id: uuid::Uuid::new_v4().simple().to_string(),
            issuer: "owo-intent-gateway".into(),
            audience: "owo-daemon-expression".into(),
            binding: ExpressionOriginBinding {
                session_id: request.session_id.clone(),
                request_id: request.request_id.clone(),
                task_revision: request.task_revision,
            },
            origin: ExpressionOriginClaims {
                source: "ime".into(),
                context_id: request.application.context_id.clone(),
                connector_pid: evidence.binding.process_id,
                process_created_at_hex: format!("{:016x}", evidence.binding.process_created_at),
                executable_sha256: evidence
                    .binding
                    .executable_sha256
                    .iter()
                    .map(|v| format!("{v:02x}"))
                    .collect(),
                selection_sha256: request.selection.as_ref().map(|v| v.sha256.clone()),
                application_metadata: "connector_claim".into(),
            },
            origin_request_sha256: format!("{:x}", Sha256::digest(&evidence.raw)),
            prompt_sha256: format!("{:x}", Sha256::digest(prompt.as_bytes())),
            freshness: ExpressionOriginFreshness {
                issued_at_ms: u64::try_from(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| invalid())?
                        .as_millis(),
                )
                .map_err(|_| invalid())?,
                ttl_ms: EXPRESSION_ORIGIN_MAX_TTL_MS,
            },
            nonce,
            scope: ExpressionOnlyScope {
                mode: "expression_only".into(),
            },
        };
        let bytes = payload.canonical_bytes().map_err(|_| invalid())?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_ref()).map_err(|_| invalid())?;
        mac.update(EXPRESSION_ORIGIN_PURPOSE);
        mac.update(&bytes);
        let signature = ExpressionOriginSignature {
            alg: "HMAC-SHA256".into(),
            key_id: EXPRESSION_ORIGIN_KEY_ID.into(),
            value: mac
                .finalize()
                .into_bytes()
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect(),
        };
        receipts.insert(
            payload.capsule_id.clone(),
            Receipt {
                _raw: evidence.raw.clone(),
                expires: now + Duration::from_millis(EXPRESSION_ORIGIN_MAX_TTL_MS),
            },
        );
        Ok(ExpressionOriginCapsule { payload, signature })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signer() -> SourceSigner {
        SourceSigner {
            key: Zeroizing::new([7; 32]),
            receipts: Mutex::new(HashMap::new()),
        }
    }
    fn evidence() -> (VerifiedOrigin, AgentIpcRequest) {
        let raw =
            include_bytes!("../../../../contracts/agent-ipc/fixtures/request.submit.valid.json")
                .to_vec();
        let request = AgentIpcRequest::decode(&raw).unwrap();
        let binding = ConnectorBinding {
            process_id: 1234,
            process_created_at: 0x01dc123456789abc,
            user_sid: vec![1],
            session_id: 2,
            authentication_id: (3, 4),
            executable_sha256: [5; 32],
        };
        (VerifiedOrigin::authenticated(binding, raw), request)
    }
    #[test]
    fn signs_kernel_evidence_and_exact_request_without_rebinding() {
        let signer = signer();
        let (origin, request) = evidence();
        let capsule = signer.issue(&origin, &request, "synthetic").unwrap();
        assert_eq!(capsule.payload.origin.connector_pid, 1234);
        assert_eq!(
            capsule.payload.origin.process_created_at_hex,
            "01dc123456789abc"
        );
        assert_eq!(capsule.payload.origin.executable_sha256, "05".repeat(32));
        assert_eq!(
            capsule.payload.origin.application_metadata,
            "connector_claim"
        );
        assert_eq!(capsule.payload.binding.request_id, request.request_id);
        assert_eq!(
            capsule.payload.origin_request_sha256,
            format!("{:x}", Sha256::digest(&origin.raw))
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(&[7; 32]).unwrap();
        mac.update(EXPRESSION_ORIGIN_PURPOSE);
        mac.update(&capsule.payload.canonical_bytes().unwrap());
        let tag: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&capsule.signature.value[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        mac.verify_slice(&tag).unwrap();
        let mut changed = request.clone();
        changed.application.context_id = "another-context".into();
        assert!(signer.issue(&origin, &changed, "synthetic").is_err());
        assert!(!origin.matches(b"another JSON frame"));
        let second = signer.issue(&origin, &request, "synthetic").unwrap();
        assert_ne!(capsule.payload.nonce, second.payload.nonce);
        assert_eq!(
            signer
                .receipts
                .lock()
                .unwrap()
                .get(&capsule.payload.capsule_id)
                .unwrap()
                ._raw,
            origin.raw
        );
    }
    #[test]
    fn bounded_raw_receipts_fail_closed_and_expired_text_is_reaped() {
        let signer = signer();
        let (origin, request) = evidence();
        for _ in 0..64 {
            signer.issue(&origin, &request, "synthetic").unwrap();
        }
        assert!(signer.issue(&origin, &request, "synthetic").is_err());
        for receipt in signer.receipts.lock().unwrap().values_mut() {
            receipt.expires = Instant::now();
        }
        signer.prune();
        assert!(signer.receipts.lock().unwrap().is_empty());
        signer.issue(&origin, &request, "synthetic").unwrap();
    }
}
