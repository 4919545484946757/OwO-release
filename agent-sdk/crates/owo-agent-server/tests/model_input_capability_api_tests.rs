//! Isolated Settings HTTP persistence, validation and native provider admission.
use owo_agent_core::{
    gateway::{ChatMessage, ModelOutput, ModelProvider},
    permissions::Policy,
    sqlite_store::SqliteSessionStore,
    tools::{ToolRegistry, ToolSpec},
    Agent,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
struct Idle;
#[async_trait::async_trait]
impl ModelProvider for Idle {
    async fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ModelOutput, String> {
        Err("fixture: no model request expected".into())
    }
}
#[tokio::test]
async fn settings_image_declaration_roundtrip_and_invalid_update_is_atomic() {
    if std::env::var_os("OWO_INPUT_SETTINGS_TEST_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "settings_image_declaration_roundtrip_and_invalid_update_is_atomic",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("OWO_INPUT_SETTINGS_TEST_CHILD", "1")
            .env("OWO_MODEL_INPUT_CAPABILITIES", "[]")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let agent = Agent::new(
        Arc::new(Idle),
        ToolRegistry::new(),
        Policy::new(dir.path()),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&dir.path().join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        dir.path().join("traces"),
        dir.path().to_owned(),
        dir.path().to_owned(),
    ));
    let token = state.auth_token.token().to_string();
    let router = owo_agent_server::build_router(state);
    let request = |method: &str, body: Value| {
        axum::http::Request::builder()
            .method(method)
            .uri("/settings")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    };
    let binding = json!({"model":"fixture","base_url":"http://127.0.0.1:4999/v1","api_format":"openai","image_input":false});
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            json!({"model_input_capabilities":[binding.clone()]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = router
        .clone()
        .oneshot(request("GET", Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["model_input_capabilities"][0]["image_input"], false);
    assert_eq!(
        value["runtime"]["model_input_capabilities"][0]["image_input"],
        false
    );
    assert!(value["runtime"]["input_capability_config_error"].is_null());
    assert_eq!(value["model_input_capabilities"][0].get("verified"), None);
    let before = std::fs::read(dir.path().join("settings.json")).unwrap();
    let mut bad = binding.clone();
    bad["base_url"] = json!("https://user:secret@example.com/v1");
    let response = router
        .clone()
        .oneshot(request("POST", json!({"model_input_capabilities":[bad]})))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).unwrap(),
        before
    );
    assert_eq!(
        owo_agent_core::gateway::input_capabilities::input_capability_bindings_from_env()
            .unwrap()
            .len(),
        1
    );
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            json!({"model_input_capabilities":[binding.clone(),binding]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let response = router
        .oneshot(request("POST", json!({"model_input_capabilities":[]})))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        owo_agent_core::gateway::input_capabilities::input_capability_bindings_from_env()
            .unwrap()
            .is_empty()
    );
}
