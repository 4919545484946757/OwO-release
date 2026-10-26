//! Checkpoint-owned task observations; the journal and delivery cursor commit together.
use super::run_events::{append, decode, SELECT};
use super::*;
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRunEvent {
    pub source_seq: u64,
    pub cause_source_seq: Option<u64>,
    pub event: NewRunEvent,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEventOutbox {
    pub schema_version: u8,
    pub next_seq: u64,
    pub history_complete: bool,
    pub heads: BTreeMap<String, u64>,
    pub pending: Vec<TaskRunEvent>,
}
impl TaskEventOutbox {
    pub fn empty(history_complete: bool) -> Self {
        Self {
            schema_version: 1,
            next_seq: 1,
            history_complete,
            heads: BTreeMap::new(),
            pending: Vec::new(),
        }
    }
}
pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS run_task_delivery_cursor(
 run_id TEXT PRIMARY KEY, source_seq INTEGER NOT NULL CHECK(source_seq>=0)
);
";
fn invalid(message: &str) -> ProjectSpaceStoreError {
    ProjectSpaceStoreError::InvalidState(message.into())
}
pub fn task_source_key(run: &str, seq: u64) -> String {
    format!("task:{run}:{seq}")
}
fn cursor(conn: &Connection, run: &str) -> Result<u64> {
    Ok(conn
        .query_row(
            "SELECT source_seq FROM run_task_delivery_cursor WHERE run_id=?1",
            [run],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}
impl SqliteProjectSpaceStore {
    pub(super) fn task_event_cursor_sql(&self, run: &str) -> Result<u64> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        cursor(&conn, run)
    }
    pub(super) fn publish_task_events_sql(
        &self,
        run: &str,
        events: &[TaskRunEvent],
    ) -> Result<u64> {
        if events.len() > 128 {
            return Err(invalid("task delivery batch exceeds 128"));
        }
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut through = cursor(&tx, run)?;
        for source in events {
            if source.source_seq == 0
                || source.source_seq > i64::MAX as u64
                || source.event.run_id != run
                || source.event.source_key != task_source_key(run, source.source_seq)
                || !source.event.event_type.starts_with("task.")
                || source.event.causation_id.is_some()
            {
                return Err(invalid("invalid checkpoint task event identity"));
            }
            if source.source_seq > through.saturating_add(1) {
                return Err(invalid("task delivery source sequence has a gap"));
            }
            let mut event = source.event.clone();
            if let Some(cause) = source.cause_source_seq {
                if cause == 0 || cause >= source.source_seq {
                    return Err(invalid("invalid task source cause"));
                }
                let prior = tx
                    .query_row(
                        &format!("{SELECT} WHERE run_id=?1 AND source_key=?2"),
                        params![run, task_source_key(run, cause)],
                        decode,
                    )
                    .optional()?
                    .ok_or_else(|| invalid("task source cause has not been published"))?;
                let step = event
                    .data
                    .get("step_id")
                    .and_then(serde_json::Value::as_str);
                if step.is_none_or(str::is_empty)
                    || prior
                        .data
                        .get("step_id")
                        .and_then(serde_json::Value::as_str)
                        != step
                {
                    return Err(invalid("task source cause belongs to another step"));
                }
                event.causation_id = Some(prior.event_id);
            }
            if source.source_seq <= through {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id=?1 AND source_key=?2)",
                    params![run, event.source_key],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Err(invalid("task delivery cursor has no canonical event"));
                }
            }
            append(&tx, &event)?;
            through = through.max(source.source_seq);
        }
        if !events.is_empty() {
            tx.execute(
                "INSERT INTO run_task_delivery_cursor(run_id,source_seq) VALUES(?1,?2)
                ON CONFLICT(run_id) DO UPDATE SET source_seq=excluded.source_seq",
                params![run, through],
            )?;
        }
        tx.commit()?;
        Ok(through)
    }
}

#[cfg(test)]
#[path = "task_events_tests.rs"]
mod tests;
