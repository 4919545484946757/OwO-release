//! Host-prepared Worker payload receipts, in the existing run journal.
//! This boundary precedes model prompt compilation and proves no model delivery.
use super::run_events::append;
use super::*;
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Selected by the host that actually embedded the context, never inferred
/// from arbitrary JSON supplied by a user or custom Worker.
#[derive(Debug, Clone, Copy)]
pub enum PreparedInputEncoding {
    Structured,
    Serialized,
    NoContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInputTarget {
    pub worker_id: String,
    pub step_id: String,
    pub task_id: Option<String>,
    pub attempt_id: Option<String>,
    pub phase_epoch: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedInputSource {
    pub producer_step_id: Option<String>,
    pub producer_worker_id: Option<String>,
    pub producer_task_id: Option<String>,
    pub producer_attempt_id: Option<String>,
    pub artifact_id: String,
    pub artifact_version: Option<u64>,
    pub artifact_sha256: Option<String>,
    pub content_bytes: Option<u64>,
    pub preview_bytes: usize,
    pub preview_sha256: String,
    pub preview_truncated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedWorkerInput {
    pub run_id: String,
    pub invocation_id: String,
    pub target: WorkerInputTarget,
    pub node_id: Option<String>,
    pub input_sha256: String,
    pub input_bytes: u64,
    pub context_encoding: String,
    pub sources: Vec<PreparedInputSource>,
}

fn invalid(message: &str) -> ProjectSpaceStoreError {
    ProjectSpaceStoreError::InvalidState(message.into())
}
fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
// Stream serialization into the digest; never duplicate a potentially large input.
#[derive(Default)]
struct DigestWriter {
    digest: Sha256,
    bytes: u64,
}
impl std::io::Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.digest.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl PreparedWorkerInput {
    /// Capture only context actually present in the Value passed to Worker::run.
    /// Existing builtins may retain a plain text input: never attribute omitted context to it.
    pub fn capture(
        run: &str,
        invocation: &str,
        target: WorkerInputTarget,
        input: &Value,
        context_encoding: PreparedInputEncoding,
    ) -> Result<Self> {
        let mut digest = DigestWriter::default();
        serde_json::to_writer(&mut digest, input)?;
        let parsed = match context_encoding {
            PreparedInputEncoding::Serialized => Some(serde_json::from_str::<Value>(
                input
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("missing serialized host context"))?,
            )?),
            _ => None,
        };
        let (context, encoding) = match context_encoding {
            PreparedInputEncoding::Structured => (
                Some(
                    input
                        .get("team_prompt_context")
                        .filter(|v| v.is_object())
                        .ok_or_else(|| invalid("missing structured host context"))?,
                ),
                "structured_team_context",
            ),
            PreparedInputEncoding::Serialized => (parsed.as_ref(), "serialized_team_context"),
            PreparedInputEncoding::NoContext => (None, "no_team_context_in_payload"),
        };
        let mut sources = Vec::new();
        if let Some(value) = context.and_then(|c| c.get("upstream")) {
            let items = value
                .as_array()
                .ok_or_else(|| invalid("prepared upstream must be an array"))?;
            for item in items {
                let artifact_id = text(item, "artifact_id")
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| invalid("prepared source has no artifact identity"))?;
                let preview = item
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("prepared source has no exact preview"))?;
                sources.push(PreparedInputSource {
                    producer_step_id: text(item, "step_id"),
                    producer_worker_id: text(item, "worker"),
                    producer_task_id: text(item, "task_id"),
                    producer_attempt_id: text(item, "attempt_id"),
                    artifact_id,
                    artifact_version: item.get("version").and_then(Value::as_u64),
                    artifact_sha256: text(item, "sha256"),
                    content_bytes: item.get("content_bytes").and_then(Value::as_u64),
                    preview_bytes: preview.len(),
                    preview_sha256: format!("{:x}", Sha256::digest(preview.as_bytes())),
                    preview_truncated: item.get("truncated").and_then(Value::as_bool),
                });
            }
        }
        Ok(Self {
            run_id: run.into(),
            invocation_id: invocation.into(),
            target,
            node_id: context.and_then(|c| text(c, "node_id")),
            input_sha256: format!("{:x}", digest.digest.finalize()),
            input_bytes: digest.bytes,
            context_encoding: encoding.into(),
            sources,
        })
    }
    fn event(&self, suffix: &str, kind: &str, data: Value) -> NewRunEvent {
        let mut event = NewRunEvent::new(
            &self.run_id,
            format!("worker-input:{}:{suffix}", self.invocation_id),
            kind,
            data,
        );
        event.node_id = self.node_id.clone();
        event.task_id = self.target.task_id.clone();
        event.attempt = self.target.attempt_id.clone();
        event
    }
}

impl SqliteProjectSpaceStore {
    pub(super) fn append_worker_input_sql(
        &self,
        receipt: &PreparedWorkerInput,
    ) -> Result<RunEvent> {
        if receipt.invocation_id.is_empty()
            || receipt.target.worker_id.is_empty()
            || receipt.target.step_id.is_empty()
        {
            return Err(invalid("prepared input has incomplete host identity"));
        }
        let header = receipt.event(
            "prepared",
            "worker.input.prepared",
            json!({
                "invocation_id":receipt.invocation_id, "target":receipt.target,
                "input_sha256":receipt.input_sha256, "input_bytes":receipt.input_bytes,
                "input_representation":"serde_json_worker_value",
                "context_encoding":receipt.context_encoding, "source_count":receipt.sources.len(),
                "boundary":"before_worker_run", "model_delivery":"not_proven"
            }),
        );
        header.check_limits()?;
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prepared = append(&tx, &header)?;
        for (index, source) in receipt.sources.iter().enumerate() {
            let mut event = receipt.event(
                &format!("source:{index}"),
                "worker.input.source",
                json!({
                    "invocation_id":receipt.invocation_id, "target":receipt.target,
                    "source":source, "boundary":"before_worker_run", "model_delivery":"not_proven"
                }),
            );
            event.artifact_refs = vec![source.artifact_id.clone()];
            event.causation_id = Some(prepared.event_id.clone());
            append(&tx, &event)?;
        }
        tx.commit()?;
        Ok(prepared)
    }
}

#[cfg(test)]
#[path = "worker_inputs_tests.rs"]
mod tests;
