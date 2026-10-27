//! Task observations derive from committed checkpoints, not coalesced notification timing.
use super::*;
use crate::project_space_store::{task_source_key, NewRunEvent, TaskEventOutbox, TaskRunEvent};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TaskView {
    step_id: String,
    task_id: String,
    worker: String,
    node_id: Option<String>,
    assignee: Option<String>,
    waiting_reason: Option<String>,
    status: String,
    attempt: Option<String>,
    attempts: u32,
    phase_epoch: Option<u64>,
    depends_on: Vec<String>,
    depends_on_complete: bool,
    waiting_for: Vec<String>,
    waiting_for_complete: bool,
    error_preview: Option<String>,
    error_complete: bool,
    skip_reason: Option<String>,
    skip_reason_complete: bool,
}
fn bounded_ids(ids: &[String]) -> (Vec<String>, bool) {
    let mut result = Vec::new();
    let mut bytes: usize = 0;
    let mut complete = true;
    for id in ids {
        let cost = serde_json::to_string(id).map_or(usize::MAX, |s| s.len() + 1);
        if result.len() >= 64 || id.len() > 256 || bytes.saturating_add(cost) > 2048 {
            complete = false;
            continue;
        }
        bytes += cost;
        result.push(id.clone());
    }
    (result, complete)
}
fn preview(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|s| s.chars().take(400).collect())
}
fn views(state: &GoalRunState, roles: &[RoleSpec]) -> WorkSwarmResult<BTreeMap<String, TaskView>> {
    let mut result = BTreeMap::new();
    for step in &state.plan.steps {
        let Some(record) = state.records.get(&step.id) else {
            continue;
        };
        let role = roles
            .iter()
            .find(|r| format!("m-{}", r.role) == step.worker);
        let node = role.and_then(|r| r.node_id.clone());
        let task_id = step
            .input
            .get("assigned_task_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .unwrap_or(&step.id)
            .to_owned();
        let status = serde_json::to_value(record.status)?
            .as_str()
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        let error_preview = preview(&record.error);
        let (depends_on, depends_on_complete) = bounded_ids(&step.depends_on);
        let waits = step
            .depends_on
            .iter()
            .filter(|d| {
                state
                    .records
                    .get(*d)
                    .is_none_or(|r| r.status != StepStatus::Succeeded)
            })
            .cloned()
            .collect::<Vec<_>>();
        let waiting_reason = if matches!(record.status, StepStatus::Pending | StepStatus::Ready) {
            if !waits.is_empty() {
                Some("dependencies".to_string())
            } else if role.is_some_and(|r| r.assignee == "human") {
                Some("human_input".to_string())
            } else {
                Some("scheduler".to_string())
            }
        } else {
            None
        };
        let skip_reason = preview(&record.skip_reason);
        let (waiting_for, waiting_for_complete) = bounded_ids(&waits);
        result.insert(
            step.id.clone(),
            TaskView {
                step_id: step.id.clone(),
                task_id,
                worker: step.worker.clone(),
                node_id: node,
                assignee: role.map(|r| r.assignee.clone()),
                waiting_reason,
                status,
                attempt: record.attempt_id.clone(),
                attempts: record.attempts,
                phase_epoch: record.phase_epoch,
                depends_on,
                depends_on_complete,
                waiting_for,
                waiting_for_complete,
                error_complete: error_preview == record.error,
                error_preview,
                skip_reason_complete: skip_reason == record.skip_reason,
                skip_reason,
            },
        );
    }
    Ok(result)
}
fn validate_outbox(out: &TaskEventOutbox, cursor: u64) -> WorkSwarmResult<()> {
    if out.schema_version != 1
        || out.next_seq == 0
        || out.next_seq > i64::MAX as u64
        || cursor >= out.next_seq
        || out
            .heads
            .values()
            .any(|seq| *seq == 0 || *seq >= out.next_seq)
    {
        return Err(WorkSwarmError::CorruptState(
            "task checkpoint delivery identity is inconsistent".into(),
        ));
    }
    let mut expected = cursor + 1;
    for entry in out.pending.iter().filter(|e| e.source_seq > cursor) {
        if entry.source_seq != expected {
            return Err(WorkSwarmError::CorruptState(
                "task checkpoint pending sequence is incomplete".into(),
            ));
        }
        expected += 1;
    }
    if expected != out.next_seq {
        return Err(WorkSwarmError::CorruptState(
            "task checkpoint lost pending observations".into(),
        ));
    }
    Ok(())
}
pub(super) fn next_outbox(
    previous: Option<&GoalRunState>,
    state: &GoalRunState,
    roles: &[RoleSpec],
    cursor: u64,
    complete: bool,
) -> WorkSwarmResult<TaskEventOutbox> {
    let mut out = previous
        .and_then(|s| s.team_task_events.clone())
        .unwrap_or_else(|| TaskEventOutbox::empty(complete));
    validate_outbox(&out, cursor)?;
    out.pending.retain(|entry| entry.source_seq > cursor);
    let old = previous
        .map(|s| views(s, roles))
        .transpose()?
        .unwrap_or_default();
    let new = views(state, roles)?;
    let timestamp = now_ts();
    for id in old
        .keys()
        .chain(new.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let before = old.get(id);
        let after = new.get(id);
        if before == after {
            continue;
        }
        let view = after.or(before).expect("key belongs to a task");
        let seq = out.next_seq;
        out.next_seq = seq
            .checked_add(1)
            .filter(|next| *next <= i64::MAX as u64)
            .ok_or_else(|| WorkSwarmError::Run("task source sequence exhausted".into()))?;
        let kind = match (before, after) {
            (_, None) => "task.removed".into(),
            (None, _) => "task.created".into(),
            (Some(old), Some(next)) if old.status != next.status => format!("task.{}", next.status),
            (Some(old), Some(next)) if old.attempt != next.attempt => "task.attempt_changed".into(),
            _ => "task.updated".into(),
        };
        let mut event = NewRunEvent::new(
            &state.run_id,
            task_source_key(&state.run_id, seq),
            &kind,
            json!({"step_id":id,"source_seq":seq,"source_timestamp":timestamp,
                "task_history_complete":out.history_complete,
                "previous_status":before.map(|v|&v.status),"previous_task_id":before.map(|v|&v.task_id),"task":view,"removed":after.is_none()}),
        );
        event.node_id = view.node_id.clone();
        event.task_id = Some(view.task_id.clone());
        event.attempt = view.attempt.clone();
        event.check_limits()?;
        out.pending.push(TaskRunEvent {
            source_seq: seq,
            cause_source_seq: out.heads.insert(id.clone(), seq),
            event,
        });
    }
    Ok(out)
}
impl TeamCoordinator {
    /// Relay a bounded checkpoint prefix. Cursor and journal publication share one SQL transaction.
    pub fn relay_task_events(
        &self,
        run: &str,
        limit: usize,
    ) -> WorkSwarmResult<(bool, Option<bool>)> {
        if !(1..=128).contains(&limit) {
            return Err(WorkSwarmError::Validation(
                "invalid task delivery limit".into(),
            ));
        }
        let (cursor, projected) = self.task_delivery_snapshot(run, || {
            #[derive(Deserialize)]
            struct Projection {
                team_task_events: Option<TaskEventOutbox>,
            }
            let path = self.run_dir.join(format!("{run}.json"));
            let raw = match std::fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(WorkSwarmError::Io(error.to_string())),
            };
            let projected: Projection = serde_json::from_str(&raw)
                .map_err(|e| WorkSwarmError::CorruptState(e.to_string()))?;
            Ok(projected.team_task_events)
        })?;
        let Some(outbox) = projected else {
            if cursor != 0 {
                return Err(WorkSwarmError::CorruptState(
                    "task delivery metadata disappeared from checkpoint".into(),
                ));
            }
            return Ok((false, Some(false)));
        };
        validate_outbox(&outbox, cursor)?;
        let entries = outbox
            .pending
            .iter()
            .filter(|e| e.source_seq > cursor)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        let published = self.store.publish_task_events(run, &entries)?;
        Ok((
            outbox.next_seq - 1 > published,
            Some(outbox.history_complete),
        ))
    }
    // A reader may overlap checkpoint replacement and a successful publisher. Retry a
    // moving SQL cursor instead of diagnosing a valid older file as corrupt. The
    // publisher still checks every source identity in its serialized SQL transaction.
    fn task_delivery_snapshot(
        &self,
        run: &str,
        mut read: impl FnMut() -> WorkSwarmResult<Option<TaskEventOutbox>>,
    ) -> WorkSwarmResult<(u64, Option<TaskEventOutbox>)> {
        for _ in 0..4 {
            let before = self.store.task_event_cursor(run)?;
            let projected = read()?;
            if self.store.task_event_cursor(run)? == before {
                return Ok((before, projected));
            }
        }
        Err(WorkSwarmError::Run(
            "task delivery advanced during snapshot read; retry the request".into(),
        ))
    }
    pub(super) fn persist_observed_state(&self, state: &GoalRunState) -> WorkSwarmResult<()> {
        let path = self.run_dir.join(format!("{}.json", state.run_id));
        let previous = if path.exists() {
            Some(self.load_goal_state(&state.run_id)?)
        } else {
            None
        };
        let meta_path = self.run_dir.join(format!("{}-meta.json", state.run_id));
        let roles = if meta_path.exists() {
            self.load_run_meta(&state.run_id)?.roles
        } else {
            Vec::new()
        };
        let cursor = self.store.task_event_cursor(&state.run_id)?;
        let outbox = next_outbox(previous.as_ref(), state, &roles, cursor, false)?;
        state
            .persist_team_checkpoint(&self.run_dir, &outbox)
            .map_err(WorkSwarmError::Run)?;
        // A failed projection leaves the events in the committed source checkpoint.
        if let Err(error) = self.store.publish_task_events(
            &state.run_id,
            &outbox
                .pending
                .iter()
                .filter(|e| e.source_seq > cursor)
                .take(128)
                .cloned()
                .collect::<Vec<_>>(),
        ) {
            self.record_runtime_event(
                &state.run_id,
                "team.task_delivery.pending",
                error.to_string(),
            );
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "coord_task_events_tests.rs"]
mod tests;
