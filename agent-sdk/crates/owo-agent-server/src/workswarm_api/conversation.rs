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
    let mut sessions = Vec::new();
    let mut more = false;
    let mut next = cursors.clone();
    let mut remaining = 512usize;
    // Only sessions with a host-generated team-specific prefix can be returned.
    for session_id in state
        .store
        .list()
        .into_iter()
        .filter(|sid| sid.starts_with(&prefix))
    {
        if remaining == 0 {
            more = true;
            break;
        }
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
        .list_handoffs(&id)
        .await
        .map_err(|e| dto::error_response(&e))?;
    Ok(Json(json!({
        "team_id": id, "sessions": sessions, "next_cursors": next, "has_more": more,
        "handoffs": workswarm_metrics::sanitize_value(&json!(handoffs)),
    })))
}
