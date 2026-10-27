//! Read-only input admission uses the same provider as the real turn.
use axum::{extract::State, http::StatusCode, Json};
use owo_agent_core::{gateway::ModelProvider, ChatMessage};
use owo_agent_protocol::{ContentPart, InputPreflightRequest};
use owo_agent_server::AppState;
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) fn message_has_image(message: &ChatMessage) -> bool {
    !message.images.is_empty()
        || message.input.as_ref().is_some_and(|input| {
            input
                .content_parts
                .iter()
                .any(|part| matches!(part, ContentPart::Image { .. }))
        })
}

pub(crate) async fn input_preflight(
    State(state): State<Arc<AppState>>,
    Json(request): Json<InputPreflightRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if request.session_id.is_none() && request.history_keep.is_some()
        || request.session_id.is_some() && request.model.is_some()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "input/ambiguous_preflight_context".into(),
        ));
    }
    let session = request
        .session_id
        .as_deref()
        .map(|id| crate::session_api::load_session(&state, id))
        .transpose()?;
    let keep = request
        .history_keep
        .unwrap_or_else(|| session.as_ref().map_or(0, |s| s.messages.len()));
    if session.as_ref().is_some_and(|s| keep > s.messages.len()) {
        return Err((StatusCode::BAD_REQUEST, "input/invalid_history_keep".into()));
    }
    let has_images = request.has_images
        || session
            .as_ref()
            .is_some_and(|s| s.messages.iter().take(keep).any(message_has_image));
    let provider: Arc<dyn ModelProvider> = match &request.model_connection {
        Some(connection) => owo_agent_core::gateway::custom_model_provider(connection),
        None => Ok(state.agent.provider()),
    }
    .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    let model = request
        .model_connection
        .as_ref()
        .map(|c| c.model.as_str())
        .or_else(|| session.as_ref().and_then(|s| s.model_override.as_deref()))
        .or(request.model.as_deref());
    provider
        .validate_image_input(model, has_images)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    Ok(Json(json!({"ok":true})))
}
