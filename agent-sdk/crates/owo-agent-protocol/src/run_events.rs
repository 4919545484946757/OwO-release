//! Persisted public run observations. Sequence identity is independent of progress snapshots.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunEvent {
    pub schema_version: u8,
    pub seq: u64,
    pub event_id: String,
    pub run_id: String,
    pub event_type: String,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Opaque existing Artifact/CAS references; no inline media or implicit read authority.
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    /// Public observations and canonical references; HTTP may hydrate bounded sanitized previews.
    /// Never private reasoning or credentials. Publication time and source time remain distinct.
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEventPage {
    pub run_id: String,
    pub events: Vec<RunEvent>,
    pub next_after: u64,
    pub high_watermark: u64,
    pub has_more: bool,
    /// False for a pre-journal run. Missing history must not be fabricated from its current state.
    pub history_complete: bool,
    /// Whether task observations began in the initial checkpoint; absent at lower-level journal reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_history_complete: Option<bool>,
}

/// Reverse keyset history. Events remain in ascending order inside each bounded page.
/// before=0 selects the latest committed page; subsequent reads use next_before exclusively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEventHistoryPage {
    pub run_id: String,
    pub events: Vec<RunEvent>,
    pub next_before: Option<u64>,
    pub high_watermark: u64,
    /// Whether older records exist, independent of currently pending live delivery.
    pub has_more: bool,
    pub history_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_history_complete: Option<bool>,
}

/// Bounded canonical handoff records, ascending within a reverse history page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffPage {
    pub handoffs: Vec<crate::HandoffRecord>,
    /// Actual first record identity; exclusive on the next older read.
    pub next_before: Option<String>,
    pub has_more: bool,
}
