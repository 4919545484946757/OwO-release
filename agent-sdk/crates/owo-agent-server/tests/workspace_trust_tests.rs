//! 工作区信任（审查 N1）HTTP 契约：危险配置 → 受限模式；显式信任后放行。
//!
//! 覆盖：
//! - 有 hooks/插件的未信任工作区：启动即受限（`workspace_trust_gated`），
//!   `GET /workspace/trust` 报告 gated + reasons；
//! - `POST /workspace/trust {trusted:true}`：记录信任并即时应用 hooks/权限；
//! - 撤销后 `gated=true`；
//! - 无危险配置的普通工作区不进入信任门（保持既有行为）。

use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use std::sync::Arc;
use tower::ServiceExt;

struct IdleProvider;

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("IdleProvider 不应被调用".to_string())
    }
}

fn request(
    state: &Arc<owo_agent_server::AppState>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> axum::http::Request<axum::body::Body> {
    use axum::http::{header, Method, Request};
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

fn build_state(
    workspace: &std::path::Path,
    data_root: &std::path::Path,
) -> Arc<owo_agent_server::AppState> {
    std::fs::create_dir_all(workspace).unwrap();
    std::fs::create_dir_all(data_root).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&data_root.join("index.db")).unwrap();
    Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        data_root.join("traces"),
        data_root.to_path_buf(),
        workspace.to_path_buf(),
    ))
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn risky_workspace_is_gated_until_explicitly_trusted() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(workspace.join("plugins").join("demo")).unwrap();
    std::fs::write(
        workspace.join("plugins").join("demo").join("manifest.json"),
        r#"{"id":"demo"}"#,
    )
    .unwrap();
    std::fs::write(
        workspace.join("settings.json"),
        serde_json::json!({
            "hooks": [{"event": "UserPromptSubmit", "command": "echo hi"}],
        })
        .to_string(),
    )
    .unwrap();

    let state = build_state(&workspace, &temp.path().join("data"));
    assert!(
        state.workspace_trust_gated,
        "有 hooks/插件的未信任工作区必须进入受限模式"
    );
    let app = build_router(Arc::clone(&state));

    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/workspace/trust", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let status = body_json(response).await;
    assert_eq!(status["trusted"], serde_json::json!(false));
    assert_eq!(status["gated"], serde_json::json!(true));
    assert!(
        status["reasons"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "必须报告将加载的危险配置：{status}"
    );

    // 显式信任：记录 + 即时应用。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/workspace/trust",
            Some(r#"{"trusted":true}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let trusted = body_json(response).await;
    assert_eq!(trusted["ok"], serde_json::json!(true));
    assert_eq!(trusted["trusted"], serde_json::json!(true));
    assert_eq!(trusted["gated"], serde_json::json!(false));

    // 撤销：重新进入受限模式。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/workspace/trust",
            Some(r#"{"trusted":false}"#),
        ))
        .await
        .unwrap();
    let revoked = body_json(response).await;
    assert_eq!(revoked["trusted"], serde_json::json!(false));
    assert_eq!(revoked["gated"], serde_json::json!(true));
}

#[tokio::test]
async fn plain_workspace_stays_ungated() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    let state = build_state(&workspace, &temp.path().join("data"));
    assert!(
        !state.workspace_trust_gated,
        "没有危险配置的普通仓库不应进入受限模式"
    );
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "GET", "/workspace/trust", None))
        .await
        .unwrap();
    let status = body_json(response).await;
    assert_eq!(status["trusted"], serde_json::json!(false));
    assert_eq!(status["gated"], serde_json::json!(false));
}
