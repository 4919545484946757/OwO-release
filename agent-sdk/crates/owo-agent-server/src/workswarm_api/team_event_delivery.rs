//! At-least-once durable delivery, exactly-once journal materialization by canonical source identity.
use owo_agent_core::workswarm::{TeamCoordinator, WorkSwarmError};
use owo_agent_core::SessionStore;
use owo_agent_protocol::RunEventPage;
use serde_json::json;

/// A failed append or acknowledgement leaves the source index pending for restart/read recovery.
pub(super) fn relay(
    coordinator: &TeamCoordinator,
    store: &dyn SessionStore,
    run: &str,
    limit: usize,
) -> Result<bool, WorkSwarmError> {
    let mut pending = store
        .pending_team_turn_events(run, limit + 1)
        .map_err(|e| WorkSwarmError::Run(format!("Team delivery read failed: {e}")))?;
    let has_more = pending.len() > limit;
    pending.truncate(limit);
    for delivery in pending {
        coordinator.publish_team_turn_delivery(&delivery)?;
        store
            .acknowledge_team_turn_event(run, delivery.delivery_id)
            .map_err(|e| {
                WorkSwarmError::Run(format!("Team delivery acknowledgement failed: {e}"))
            })?;
    }
    Ok(has_more)
}

/// Public payloads are resolved from the original session store, never copied into a second transcript.
pub(super) fn page(
    coordinator: &TeamCoordinator,
    store: &dyn SessionStore,
    run: &str,
    after: u64,
    limit: usize,
) -> Result<RunEventPage, WorkSwarmError> {
    if after > 0 {
        coordinator.run_events_after(run, after, 1)?;
    }
    let (task_pending, task_history) = coordinator.relay_task_events(run, 64)?;
    let pending = relay(coordinator, store, run, 64)?;
    let mut page = coordinator.run_events_after(run, after, limit)?;
    page.has_more |= pending || task_pending;
    page.task_history_complete = task_history;
    hydrate(store, run, &mut page.events)?;
    Ok(page)
}

pub(super) fn history_page(
    coordinator: &TeamCoordinator,
    store: &dyn SessionStore,
    run: &str,
    before: u64,
    limit: usize,
) -> Result<owo_agent_protocol::RunEventHistoryPage, WorkSwarmError> {
    // Validate the requested history cursor before any pending source is published.
    if before > 0 {
        coordinator.run_events_before(run, before, 1)?;
    }
    let (_, task_history) = coordinator.relay_task_events(run, 64)?;
    relay(coordinator, store, run, 64)?;
    let mut page = coordinator.run_events_before(run, before, limit)?;
    page.task_history_complete = task_history;
    hydrate(store, run, &mut page.events)?;
    Ok(page)
}

fn hydrate(
    store: &dyn SessionStore,
    run: &str,
    records: &mut [owo_agent_protocol::RunEvent],
) -> Result<(), WorkSwarmError> {
    for record in records {
        let Some(reference) = record.data.get("record_ref") else {
            continue;
        };
        let session = reference
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WorkSwarmError::Run("invalid canonical Team event reference".into()))?;
        let seq = reference
            .get("seq")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| WorkSwarmError::Run("invalid canonical Team event sequence".into()))?;
        if !session.starts_with(&format!("{run}-worker-")) {
            return Err(WorkSwarmError::Run(
                "foreign canonical Team event reference".into(),
            ));
        }
        let raw = store
            .turn_event_json_bounded(session, seq, 8192)
            .map_err(|e| WorkSwarmError::Run(format!("Team source read failed: {e}")))?;
        if let Some(raw) = raw {
            let value: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|_| WorkSwarmError::Run("corrupt canonical Team event payload".into()))?;
            if !matches!(
                value.get("type").and_then(|v| v.as_str()),
                Some(
                    "progress"
                        | "token_delta"
                        | "final"
                        | "tool_use"
                        | "tool_result"
                        | "plan_update"
                )
            ) {
                return Err(WorkSwarmError::Run(
                    "source is not a public Team event".into(),
                ));
            }
            let public = super::workswarm_metrics::sanitize_value(&value);
            record.data["payload_complete"] = json!(public == value);
            record.data["payload_preview"] = public;
        } else {
            record.data["payload_complete"] = json!(false);
            record.data["payload_unavailable"] = json!(true);
        }
    }
    Ok(())
}
