//! Purpose-bound origin capsule. This proves a trusted gateway signed bytes,
//! not that natural-language intent or application metadata is trustworthy.
//! Canonical encoding is serde JSON in the declared field order, with ASCII-only
//! strings, integers <= 2^53-1, explicit null selection hash, and no floats/maps.
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionOriginCapsule {
    pub payload: ExpressionOriginPayload,
    pub signature: ExpressionOriginSignature,
}
impl std::fmt::Debug for ExpressionOriginCapsule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExpressionOriginCapsule([redacted])")
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionOriginPayload {
    pub ic_version: u32,
    pub capsule_id: String,
    pub issuer: String,
    pub audience: String,
    pub binding: ExpressionOriginBinding,
    pub origin: ExpressionOriginClaims,
    pub origin_request_sha256: String,
    pub prompt_sha256: String,
    pub freshness: ExpressionOriginFreshness,
    pub nonce: String,
    pub scope: ExpressionOnlyScope,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionOriginBinding {
    pub session_id: String,
    pub request_id: String,
    pub task_revision: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionOriginClaims {
    pub source: String,
    pub context_id: String,
    pub connector_pid: u32,
    pub process_created_at_hex: String,
    pub executable_sha256: String,
    #[serde(default)]
    pub selection_sha256: Option<String>,
    pub application_metadata: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionOriginFreshness {
    pub issued_at_ms: u64,
    pub ttl_ms: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionOnlyScope {
    pub mode: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionOriginSignature {
    pub alg: String,
    pub key_id: String,
    pub value: String,
}
// Admission proves source bytes, never tool authorization or task success.
// Keep each audience, scope, key id and MAC purpose distinct from expression.
pub const IME_TASK_ORIGIN_AUDIENCE: &str = "owo-daemon-ime-task";
pub const IME_TASK_ORIGIN_SCOPE: &str = "agent_task";
pub const IME_TASK_ORIGIN_KEY_ID: &str = "gateway-ime-task-v1";
pub const IME_TASK_ORIGIN_PURPOSE: &[u8] = b"OwO/ime-task-origin/v1\0";

pub const EXPRESSION_ORIGIN_PURPOSE: &[u8] = b"OwO/expression-origin/v1\0";
pub const EXPRESSION_ORIGIN_KEY_ID: &str = "gateway-expression-v1";
pub const EXPRESSION_ORIGIN_MAX_TTL_MS: u64 = 120_000;
pub fn canonical_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && value.bytes().any(|b| b != b'0')
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}
impl ExpressionOriginPayload {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, &'static str> {
        self.canonical_scoped_bytes("owo-daemon-expression", "expression_only")
    }

    /// Task admission has a separate canonical scope and MAC domain.
    /// The resulting bytes still require signature and freshness verification.
    pub fn canonical_task_bytes(&self) -> Result<Vec<u8>, &'static str> {
        self.canonical_scoped_bytes(IME_TASK_ORIGIN_AUDIENCE, IME_TASK_ORIGIN_SCOPE)
            .map_err(|error| match error {
                "expression/origin_scope" => "ime/task_origin_scope",
                _ => "ime/task_origin_invalid",
            })
    }

    fn canonical_scoped_bytes(&self, audience: &str, scope: &str) -> Result<Vec<u8>, &'static str> {
        const MAX_SAFE_INTEGER: u64 = (1u64 << 53) - 1;
        if self.ic_version != 1
            || self.issuer != "owo-intent-gateway"
            || self.audience != audience
            || self.scope.mode != scope
            || self.origin.source != "ime"
            || self.origin.application_metadata != "connector_claim"
        {
            return Err("expression/origin_scope");
        }
        if !canonical_lower_hex(&self.capsule_id, 32)
            || !canonical_lower_hex(&self.binding.session_id, 32)
            || !identifier(&self.binding.request_id)
            || self.binding.task_revision > MAX_SAFE_INTEGER
            || !identifier(&self.origin.context_id)
            || self.origin.connector_pid == 0
            || !canonical_lower_hex(&self.origin.process_created_at_hex, 16)
            || !canonical_lower_hex(&self.origin.executable_sha256, 64)
            || self
                .origin
                .selection_sha256
                .as_ref()
                .is_some_and(|v| !canonical_lower_hex(v, 64))
            || !canonical_lower_hex(&self.origin_request_sha256, 64)
            || !canonical_lower_hex(&self.prompt_sha256, 64)
            || !canonical_lower_hex(&self.nonce, 64)
            || self.freshness.issued_at_ms > MAX_SAFE_INTEGER
            || self.freshness.ttl_ms == 0
            || self.freshness.ttl_ms > EXPRESSION_ORIGIN_MAX_TTL_MS
        {
            return Err("expression/origin_invalid");
        }
        serde_json::to_vec(self).map_err(|_| "expression/origin_invalid")
    }
}

#[cfg(test)]
mod task_origin_tests {
    use super::*;
    fn golden() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../../contracts/expression-origin/synthetic-golden.json"
        ))
        .unwrap()
    }
    fn expression() -> ExpressionOriginPayload {
        serde_json::from_value(golden()["capsule"]["payload"].clone()).unwrap()
    }
    fn task() -> ExpressionOriginPayload {
        let mut payload = expression();
        payload.audience = IME_TASK_ORIGIN_AUDIENCE.into();
        payload.scope.mode = IME_TASK_ORIGIN_SCOPE.into();
        payload
    }

    #[test]
    fn expression_golden_bytes_remain_identical() {
        assert_eq!(
            expression().canonical_bytes().unwrap(),
            golden()["canonical_payload"].as_str().unwrap().as_bytes()
        );
        assert!(expression().canonical_task_bytes().is_err());
    }

    #[test]
    fn canonical_task_scope_cannot_cross_into_expression() {
        let payload = task();
        let bytes = payload.canonical_task_bytes().unwrap();
        assert_eq!(bytes, serde_json::to_vec(&payload).unwrap());
        assert!(payload.canonical_bytes().is_err());
        let mut changed = payload.clone();
        changed.audience = "owo-daemon-expression".into();
        assert!(changed.canonical_task_bytes().is_err());
        assert!(changed.canonical_bytes().is_err());
        changed = payload;
        changed.scope.mode = "expression_only".into();
        assert!(changed.canonical_task_bytes().is_err());
        assert!(changed.canonical_bytes().is_err());
    }

    #[test]
    fn task_scope_retains_all_common_canonical_checks() {
        let invalid: [fn(&mut ExpressionOriginPayload); 9] = [
            |p| p.nonce = "0".repeat(64),
            |p| p.freshness.ttl_ms = 0,
            |p| p.freshness.ttl_ms = EXPRESSION_ORIGIN_MAX_TTL_MS + 1,
            |p| p.origin.connector_pid = 0,
            |p| p.origin.application_metadata = "verified-application".into(),
            |p| p.origin.context_id = "invalid/context".into(),
            |p| p.binding.task_revision = 1 << 53,
            |p| p.prompt_sha256 = "abc".into(),
            |p| p.origin.selection_sha256 = Some("A".repeat(64)),
        ];
        for change in invalid {
            let mut payload = task();
            change(&mut payload);
            assert!(payload.canonical_task_bytes().is_err());
        }
    }

    #[test]
    fn task_signature_identifiers_are_domain_separated() {
        assert_ne!(IME_TASK_ORIGIN_KEY_ID, EXPRESSION_ORIGIN_KEY_ID);
        assert_ne!(IME_TASK_ORIGIN_PURPOSE, EXPRESSION_ORIGIN_PURPOSE);
        assert_ne!(IME_TASK_ORIGIN_AUDIENCE, "owo-daemon-expression");
        assert_ne!(IME_TASK_ORIGIN_SCOPE, "expression_only");
    }
}
