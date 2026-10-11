use owo_agent_core::{
    permissions::Policy, session::SessionStore, sqlite_store::SqliteSessionStore,
    tools::ToolRegistry, Agent,
};
use owo_agent_protocol::{CompletionStatusV1, CustomModelConnection, SseEvent};
use std::sync::Arc;
use tower::ServiceExt;
struct Idle;
#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for Idle {
    async fn complete(
        &self,
        _: &[owo_agent_core::ChatMessage],
        _: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("no model call expected".into())
    }
}
fn stats() -> SseEvent {
    serde_json::from_value(serde_json::json!({"type":"turn_stats","steps":1,"duration_ms":25,"completion_status":"response_complete","model_calls":[{"request_id":"r","provider":"fixture","model":"actual","usage":null,"latency_ms":25}]})).unwrap()
}
#[test]
fn durable_terminal_and_latest_failure_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("index.db");
    let store = SqliteSessionStore::open(&file).unwrap();
    let session = store.create(dir.path(), "fixture", None).unwrap();
    assert!(store.turn_history(&session.id).unwrap().is_none());
    store
        .append_turn_event(&session.id, "old", &stats())
        .unwrap();
    drop(store);
    let store = SqliteSessionStore::open(&file).unwrap();
    let h = store.turn_history(&session.id).unwrap().unwrap();
    assert_eq!(h.last_turn_id, "old");
    assert_eq!(h.last_stats.as_ref().unwrap().turn_id, "old");
    store
        .append_turn_event(
            &session.id,
            "new",
            &SseEvent::TokenDelta {
                delta: "partial".into(),
            },
        )
        .unwrap();
    let h = store.turn_history(&session.id).unwrap().unwrap();
    assert_eq!(h.last_turn_id, "new");
    assert_eq!(h.last_terminal.unwrap().turn_id, "old");
    store
        .append_turn_event(
            &session.id,
            "new",
            &SseEvent::TurnFailed {
                message: "stopped".into(),
                completion_status: CompletionStatusV1::Aborted,
                completion_record: None,
            },
        )
        .unwrap();
    drop(store);
    let store = SqliteSessionStore::open(&file).unwrap();
    let h = store.turn_history(&session.id).unwrap().unwrap();
    assert_eq!(h.last_terminal.unwrap().turn_id, "new");
    assert_eq!(h.last_stats.unwrap().turn_id, "old");
    assert_eq!(h.latest_seq, 3);
}
#[tokio::test]
async fn session_http_returns_durable_host_facts() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteSessionStore::open(&dir.path().join("index.db")).unwrap();
    let session = store.create(dir.path(), "fixture", None).unwrap();
    store
        .append_turn_event(&session.id, "turn", &stats())
        .unwrap();
    let agent = Agent::new(
        Arc::new(Idle),
        ToolRegistry::new(),
        Policy::new(dir.path()),
        Default::default(),
    );
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        dir.path().join("traces"),
        dir.path().to_owned(),
        dir.path().to_owned(),
    ));
    let req = axum::http::Request::builder()
        .uri(format!("/session/{}", session.id))
        .header(
            "authorization",
            format!("Bearer {}", state.auth_token.token()),
        )
        .body(axum::body::Body::empty())
        .unwrap();
    let response = owo_agent_server::build_router(state)
        .oneshot(req)
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let data: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(data["turn_history"]["last_terminal"]["turn_id"], "turn");
    assert!(data["active_turn_id"].is_null());
    assert!(data["turn_history_error"].is_null());
}
#[test]
fn secure_credential_references_are_endpoint_bound_and_revocable() {
    let reference = uuid::Uuid::new_v4().simple().to_string();
    let (name, binding) = owo_agent_protocol::custom_credential_env_names(&reference).unwrap();
    struct Cleanup(String, String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::env::remove_var(&self.0);
            std::env::remove_var(&self.1);
        }
    }
    let _cleanup = Cleanup(name.clone(), binding.clone());
    std::env::set_var(&name, "synthetic-local-test-value");
    std::env::set_var(&binding,serde_json::json!({"model":"fixture","base_url":"http://127.0.0.1:4999/v1","api_format":"openai"}).to_string());
    let mut connection = CustomModelConnection {
        model: "fixture".into(),
        base_url: "http://127.0.0.1:4999/v1".into(),
        api_format: "openai".into(),
        use_full_url: false,
        api_key: None,
        api_key_env: Some(name.clone()),
        temperature: None,
        timeout_secs: None,
    };
    assert!(owo_agent_core::gateway::custom_model_provider(&connection).is_ok());
    connection.base_url = "http://127.0.0.1:5999/v1".into();
    assert!(owo_agent_core::gateway::custom_model_provider(&connection)
        .err()
        .unwrap()
        .contains("binding_mismatch"));
    connection.base_url = "http://127.0.0.1:4999/v1".into();
    connection.api_key = Some("ambiguous".into());
    assert!(owo_agent_core::gateway::custom_model_provider(&connection)
        .err()
        .unwrap()
        .contains("ambiguous"));
    connection.api_key = None;
    std::env::remove_var(&name);
    assert!(owo_agent_core::gateway::custom_model_provider(&connection)
        .err()
        .unwrap()
        .contains("not_configured"));
    connection.api_key_env = Some("OPENAI_API_KEY".into());
    assert!(owo_agent_core::gateway::custom_model_provider(&connection)
        .err()
        .unwrap()
        .contains("invalid_credential_reference"));
}
#[tokio::test]
async fn bearer_alone_cannot_register_desktop_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteSessionStore::open(&dir.path().join("index.db")).unwrap();
    let agent = Agent::new(
        Arc::new(Idle),
        ToolRegistry::new(),
        Policy::new(dir.path()),
        Default::default(),
    );
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        dir.path().join("traces"),
        dir.path().to_owned(),
        dir.path().to_owned(),
    ));
    let req=axum::http::Request::builder().method("POST").uri("/desktop/model-credentials").header("authorization",format!("Bearer {}",state.auth_token.token())).header("content-type","application/json").body(axum::body::Body::from(serde_json::json!({"credential_ref":uuid::Uuid::new_v4().simple().to_string(),"model":"fixture","base_url":"http://127.0.0.1:4999/v1","api_key":"synthetic-test-only"}).to_string())).unwrap();
    let response = owo_agent_server::build_router(state)
        .oneshot(req)
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("synthetic-test-only"));
}
