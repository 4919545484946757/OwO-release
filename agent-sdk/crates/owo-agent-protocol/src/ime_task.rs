//! Trusted input-method intent admission into the unified task runtime.
//! A receipt is a queued task identity, never a tool grant or success claim.
use crate::ExpressionOriginCapsule;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImeTaskRequest {
    pub prompt: String,
    pub origin_capsule: ExpressionOriginCapsule,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImeTaskReceipt {
    pub task_id: String,
    pub state: String,
    pub origin_verified: bool,
    pub duplicate: bool,
    pub open_route: String,
    pub requires_main_ui: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImeTaskStatus {
    pub task_id: String,
    pub state: String,
    pub pending_approvals: usize,
    pub last_sequence: u64,
    pub open_route: String,
    pub requires_main_ui: bool,
}
