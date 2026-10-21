//! Expression-only origin capsule. This proves a trusted gateway signed bytes,
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
        const MAX_SAFE_INTEGER: u64 = (1u64 << 53) - 1;
        if self.ic_version != 1
            || self.issuer != "owo-intent-gateway"
            || self.audience != "owo-daemon-expression"
            || self.scope.mode != "expression_only"
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
