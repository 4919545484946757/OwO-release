//! Desktop-owned credential references. Secret values exist only in process environment.
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use owo_agent_server::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialBindingRequest {
    credential_ref: String,
    model: String,
    base_url: String,
    #[serde(default)]
    api_format: String,
    #[serde(default)]
    api_key: Option<String>,
}
pub(crate) async fn register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<CredentialBindingRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let fail = |status, code: &str| (status, Json(json!({"error": code})));
    let secret = crate::auth_token::desktop_pairing_secret()
        .ok_or_else(|| fail(StatusCode::FORBIDDEN, "credentials/desktop_required"))?;
    let instance = crate::auth_token::desktop_instance_id()
        .ok_or_else(|| fail(StatusCode::FORBIDDEN, "credentials/desktop_required"))?;
    if !crate::auth_token::verify_desktop_pairing(
        headers.get(crate::auth_token::DESKTOP_PAIRING_HEADER),
        &secret,
    ) || !crate::auth_token::verify_desktop_instance(
        headers.get(crate::auth_token::DESKTOP_INSTANCE_HEADER),
        &instance,
    ) {
        return Err(fail(StatusCode::FORBIDDEN, "credentials/desktop_required"));
    }
    let (name, binding_name) =
        owo_agent_protocol::custom_credential_env_names(&input.credential_ref)
            .ok_or_else(|| fail(StatusCode::BAD_REQUEST, "credentials/invalid_reference"))?;
    if let Some(key) = input.api_key.as_deref() {
        if key.is_empty()
            || key.len() > 8192
            || key.chars().any(char::is_control)
            || input.model.is_empty()
            || input.model.len() > 200
        {
            return Err(fail(StatusCode::BAD_REQUEST, "credentials/invalid_binding"));
        }
        let connection = owo_agent_protocol::CustomModelConnection {
            model: input.model.clone(),
            base_url: input.base_url.clone(),
            api_format: input.api_format.clone(),
            use_full_url: false,
            api_key: Some(key.to_owned()),
            api_key_env: None,
            temperature: None,
            timeout_secs: None,
        };
        // Validate through the same gateway boundary used for execution; no model request is made.
        owo_agent_core::gateway::custom_model_provider(&connection)
            .map_err(|_| fail(StatusCode::BAD_REQUEST, "credentials/invalid_binding"))?;
        let base = reqwest::Url::parse(&input.base_url)
            .map_err(|_| fail(StatusCode::BAD_REQUEST, "credentials/invalid_binding"))?;
        let format = if input.api_format.is_empty() {
            "openai"
        } else {
            input.api_format.as_str()
        };
        let binding = json!({"model": input.model.trim(), "base_url": base.to_string(), "api_format": format});
        std::env::set_var(binding_name, binding.to_string());
        std::env::set_var(name, key);
    } else {
        std::env::remove_var(name);
        std::env::remove_var(binding_name);
    }
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "desktop",
            "model_credential",
            None,
            Some(true),
            json!({
                "credential_ref": input.credential_ref, "configured": input.api_key.is_some()
            })
            .to_string(),
        );
    }
    crate::flush_audit(&state);
    Ok(Json(
        json!({"ok": true, "configured": input.api_key.is_some()}),
    ))
}
