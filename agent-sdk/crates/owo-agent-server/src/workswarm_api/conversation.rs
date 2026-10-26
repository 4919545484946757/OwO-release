//! Bounded, cursor-based replay of public Team worker conversations.
use super::*;
use axum::extract::{Path, Query, State};
use axum::{http::StatusCode, Json};
use owo_agent_server::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default, Deserialize)]
pub(super) struct ConversationQuery {
    #[serde(default)]
    cursors: Option<String>,
    handoff_before: Option<String>,
    session_after: Option<String>,
}

pub(super) async fn team_conversation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ConversationQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| dto::error_response(&e))?;
    coordinator
        .get_team_run(&id)
        .await
        .map_err(|e| dto::error_response(&e))?;
    let cursors: BTreeMap<String, u64> = query
        .cursors
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"invalid conversation cursors"})),
            )
        })?
        .unwrap_or_default();
    let prefix = format!("{id}-worker-");
    if query.session_after.as_ref().is_some_and(|sid| {
        !sid.starts_with(&prefix) || sid.len() > 384 || sid.chars().any(char::is_control)
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid session rotation cursor"})),
        ));
    }
    if cursors.iter().any(|(sid, seq)| {
        !sid.starts_with(&prefix)
            || sid.len() > 384
            || sid.chars().any(char::is_control)
            || *seq > i64::MAX as u64
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid or foreign conversation cursor"})),
        ));
    }
    let mut sessions = Vec::new();
    let mut next = cursors.clone();
    let mut remaining = 512usize;
    // Indexed Team range and high-water cursors skip caught-up/foreign sessions.
    // The extra ID detects another page without loading all Team/global sessions.
    let mut pending = state
        .store
        .pending_conversation_sessions(&id, &cursors, query.session_after.as_deref(), 17)
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":error.to_string()})),
            )
        })?;
    let mut more = pending.len() > 16;
    pending.truncate(16);
    let mut next_session_after = query.session_after.clone();
    for session_id in pending {
        if remaining == 0 {
            more = true;
            break;
        }
        next_session_after = Some(session_id.clone());
        let limit = remaining.min(128);
        let after = cursors.get(&session_id).copied().unwrap_or(0);
        let mut events = state
            .store
            .turn_events_after(&session_id, None, after, limit + 1)
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error":error.to_string()})),
                )
            })?;
        more |= events.len() > limit;
        events.truncate(limit);
        remaining -= events.len();
        if events.is_empty() {
            continue;
        }
        if let Some(last) = events.last() {
            next.insert(session_id.clone(), last.seq);
        }
        let role = events.first().map(|event| event.turn_id.clone());
        sessions.push(json!({
            "session_id": session_id,
            "role": role,
            "events": workswarm_metrics::sanitize_value(&json!(events)),
        }));
    }
    let handoffs = coordinator
        .handoffs_before(&id, query.handoff_before.as_deref(), 64)
        .await
        .map_err(|e| dto::error_response(&e))?;
    Ok(Json(json!({
        "team_id": id, "sessions": sessions, "next_cursors": next, "has_more": more,
        "next_session_after": next_session_after,
        "handoffs": workswarm_metrics::sanitize_value(&json!(handoffs.handoffs)),
        "next_handoff_before": handoffs.next_before, "handoffs_has_more": handoffs.has_more,
    })))
}
