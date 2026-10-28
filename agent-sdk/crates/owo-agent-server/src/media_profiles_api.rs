//! Authenticated host configuration, separate from untrusted workspace settings.
//! GET never contacts a provider; saving configuration is not service verification.
use crate::AppState;
use axum::{extract::State, http::StatusCode, Json};
use owo_agent_core::gateway::{
    media_config::MediaService,
    media_profiles::{
        check_profile_tcp, checked_status, MediaCheckRequest, MediaConnectionCheck,
        MediaProfileCatalog, MediaProfileStore,
    },
};
use serde_json::{json, Value};
use std::sync::Arc;

type ApiError = (StatusCode, String);
fn api_error(error: String) -> ApiError {
    let code = if error.starts_with("media_profiles/conflict")
        || error.starts_with("media_profiles/check_busy")
    {
        StatusCode::CONFLICT
    } else if error.starts_with("media_profiles/storage_error") {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::BAD_REQUEST
    };
    (code, error)
}
fn view(catalog: MediaProfileCatalog, checks: Vec<MediaConnectionCheck>) -> Value {
    let mut services = serde_json::Map::new();
    for service in [MediaService::ImageGeneration, MediaService::TextToSpeech] {
        services.insert(
            service.name().into(),
            checked_status(&catalog, service, &checks),
        );
    }
    json!({"catalog":catalog,"services":services,"supported_services":["image_generation","text_to_speech"],"verification_supported":false,"connection_test_supported":true,"checks":checks})
}
pub(crate) async fn get(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let store = MediaProfileStore::new(&state.data_root);
    let (catalog, checks) = tokio::task::spawn_blocking(move || {
        let catalog = store.load()?;
        let checks = store.checks(catalog.revision)?;
        Ok::<_, String>((catalog, checks))
    })
    .await
    .map_err(|_| api_error("media_profiles/storage_error: 配置读取任务失败".into()))?
    .map_err(api_error)?;
    Ok(Json(view(catalog, checks)))
}
pub(crate) async fn put(
    State(state): State<Arc<AppState>>,
    Json(proposed): Json<MediaProfileCatalog>,
) -> Result<Json<Value>, ApiError> {
    let store = MediaProfileStore::new(&state.data_root);
    let result = tokio::task::spawn_blocking(move || store.save(proposed))
        .await
        .map_err(|_| api_error("media_profiles/storage_error: 配置保存任务失败".into()))?;
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "media_profiles",
            "save",
            None,
            Some(result.is_ok()),
            match &result {
                Ok(catalog) => format!(
                    "revision={} profiles={}",
                    catalog.revision,
                    catalog.profiles.len()
                ),
                Err(_) => "save rejected; no credentials logged".into(),
            },
        );
    }
    Ok(Json(view(result.map_err(api_error)?, Vec::new())))
}

/// Explicit connection observation only. Never loads credentials or calls a model.
pub(crate) async fn check(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MediaCheckRequest>,
) -> Result<Json<MediaConnectionCheck>, ApiError> {
    let result = check_profile_tcp(MediaProfileStore::new(&state.data_root), request).await;
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "media_profiles",
            "tcp_check",
            None,
            Some(result.as_ref().is_ok_and(|c| c.connected)),
            match &result {
                Ok(check) => format!("revision={} code={}", check.catalog_revision, check.code),
                Err(_) => "check rejected; endpoint and credentials omitted".into(),
            },
        );
    }
    result.map(Json).map_err(api_error)
}
