//! `GET /sharedcore/state`：SharedCore 桌面看板的只读数据面。

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    use owo_agent_core::agent::Agent;
    use owo_agent_core::gateway::ModelProvider;
    use owo_agent_core::permissions::Policy;
    use owo_agent_core::sqlite_store::SqliteSessionStore;
    use owo_agent_core::tools::ToolRegistry;

    struct IdleProvider;
    #[async_trait::async_trait]
    impl ModelProvider for IdleProvider {
        async fn complete(
            &self,
            _messages: &[owo_agent_core::ChatMessage],
            _tools: &[owo_agent_core::ToolSpec],
        ) -> Result<owo_agent_core::ModelOutput, String> {
            Err("IdleProvider 不应被调用".to_string())
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    (state, temp)
}

async fn get_state(state: Arc<owo_agent_server::AppState>) -> Value {
    let request = Request::builder()
        .method("GET")
        .uri("/sharedcore/state")
        .header(header::HOST, "127.0.0.1:4096")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        )
        .body(Body::empty())
        .unwrap();
    let response = owo_agent_server::build_router(Arc::clone(&state))
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn state_reports_absent_before_init() {
    let (state, _temp) = test_state();
    let body = get_state(state).await;
    assert_eq!(body["exists"], false);
    assert_eq!(body["dir"], ".sharedcore");
}

#[tokio::test]
async fn state_reports_plan_and_events() {
    let (state, _temp) = test_state();
    let dir = state.workspace.join(".sharedcore");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.json"),
        r#"{"name":"demo","goal":"goal","core_version":2,"context_revision":7,"baseline_commit":"abc"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("tasks.json"),
        serde_json::json!({"tasks": {
            "T-001": {"id":"T-001","title":"契约","status":"done","depends_on":[],"write_scope":["contracts/"]},
            "T-002": {"id":"T-002","title":"后端","status":"ready","depends_on":["T-001"],"write_scope":["src/backend/"]},
            "T-003": {"id":"T-003","title":"前端","status":"ready","depends_on":["T-001"],"write_scope":["src/frontend/"]},
            "T-004": {"id":"T-004","title":"文档","status":"ready","depends_on":["T-002"],"write_scope":["docs/"]}
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join("contracts.json"),
        r#"{"contracts":{"auth-api":{"version":2,"status":"accepted"}}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("events.jsonl"),
        "{\"type\":\"task.completed\"}\n{\"type\":\"contract.updated\"}\n",
    )
    .unwrap();

    let body = get_state(state).await;
    assert_eq!(body["exists"], true);
    assert_eq!(body["project"]["name"], "demo");
    assert_eq!(
        body["plan"]["batches"][0],
        serde_json::json!(["T-002", "T-003"])
    );
    assert_eq!(
        body["plan"]["blocked"]["T-004"],
        serde_json::json!(["T-002"])
    );
    assert_eq!(body["contracts"]["auth-api"]["version"], 2);
    assert_eq!(body["events"].as_array().unwrap().len(), 2);
}
