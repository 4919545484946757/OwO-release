//! `GET /symbols` 集成测试（G14 服务端面）。
//!
//! 走真实路由（bearer + tower oneshot），工作区里放真实源文件，不依赖模型凭据。
//! 覆盖：命中与字段形状、kind 过滤、子目录限定、截断如实上报、空 query 400、
//! 越界 path 400、缺 query 400。

use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_core::SessionStore;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

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

struct Fixture {
    state: Arc<owo_agent_server::AppState>,
    #[allow(dead_code)]
    temp: tempfile::TempDir,
}

fn write(path: &std::path::Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    write(
        &workspace.join("src/lib.rs"),
        "pub fn parse_config() {}\npub struct Parser;\n",
    );
    write(
        &workspace.join("src/app.py"),
        "class ParseResult:\n    def parse(self):\n        pass\n",
    );
    write(
        &workspace.join("target/generated.rs"),
        "pub fn parse_config_shadow() {}\n",
    );
    for index in 0..40 {
        write(
            &workspace.join(format!("many/mod_{index}.rs")),
            "pub fn bulk_helper() {}\n",
        );
    }

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
    Fixture { state, temp }
}

async fn get(state: &Arc<owo_agent_server::AppState>, path: &str) -> (u16, Value) {
    let app = owo_agent_server::build_router(Arc::clone(state));
    let request = axum::http::Request::builder()
        .method(axum::http::Method::GET)
        .uri(path)
        .header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        )
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(request).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn returns_matching_definitions_with_paths_and_kinds() {
    let fixture = fixture();
    let (status, body) = get(&fixture.state, "/symbols?query=parse_config").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["query"], serde_json::json!("parse_config"));
    assert_eq!(body["root"], serde_json::json!("."));
    let results = body["results"].as_array().unwrap();
    let labelled: Vec<String> = results
        .iter()
        .map(|row| {
            format!(
                "{}:{}",
                row["path"].as_str().unwrap(),
                row["name"].as_str().unwrap()
            )
        })
        .collect();
    assert!(
        labelled.contains(&"src/lib.rs:parse_config".to_string()),
        "{labelled:?}"
    );
    assert!(
        labelled.iter().all(|item| !item.contains("target/")),
        "噪声目录不得出现在结果里：{labelled:?}"
    );
    assert_eq!(body["truncated"], serde_json::json!(false));
    let first = &results[0];
    assert!(first["line"].as_u64().unwrap() >= 1);
    assert_eq!(first["kind"], serde_json::json!("function"));
}

#[tokio::test]
async fn supports_kind_path_and_truncation_reporting() {
    let fixture = fixture();
    let (status, structs) = get(&fixture.state, "/symbols?query=parse&kind=struct").await;
    assert_eq!(status, 200, "{structs}");
    let rows = structs["results"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{structs}");
    assert_eq!(rows[0]["name"], serde_json::json!("Parser"));

    let (status, scoped) = get(&fixture.state, "/symbols?query=parse&path=src").await;
    assert_eq!(status, 200, "{scoped}");
    assert_eq!(scoped["root"], serde_json::json!("src"));
    assert!(scoped["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["path"].as_str().unwrap().starts_with("src/")));

    let (status, limited) = get(&fixture.state, "/symbols?query=bulk_helper&max_results=3").await;
    assert_eq!(status, 200, "{limited}");
    assert_eq!(limited["results"].as_array().unwrap().len(), 3);
    assert_eq!(limited["results_truncated"], serde_json::json!(true));
    assert_eq!(limited["truncated"], serde_json::json!(true));
}

#[tokio::test]
async fn session_id_scopes_the_scan_to_that_session_workspace() {
    // 桌面多项目场景：Daemon 的工作区 ≠ 被打开的会话工作区。
    // 会话工作区里放独有符号，Daemon 工作区里放同名但不同文件的符号。
    let temp = tempfile::tempdir().unwrap();
    let daemon_workspace = temp.path().join("daemon-ws");
    let session_workspace = temp.path().join("project-ws");
    std::fs::create_dir_all(&daemon_workspace).unwrap();
    std::fs::create_dir_all(&session_workspace).unwrap();
    write(
        &daemon_workspace.join("daemon_only.rs"),
        "pub fn scoped_marker() {}\n",
    );
    write(
        &session_workspace.join("project.rs"),
        "pub fn scoped_marker() {}\n",
    );

    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&daemon_workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&daemon_workspace.join("index.db")).unwrap();
    let session = store.create(&session_workspace, "mock", None).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        daemon_workspace.join("traces"),
        temp.path().to_path_buf(),
        daemon_workspace.clone(),
    ));

    // 不带 session_id：扫 Daemon 工作区。
    let (status, body) = get(&state, "/symbols?query=scoped_marker").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["results"][0]["path"],
        serde_json::json!("daemon_only.rs"),
        "{body}"
    );
    assert_eq!(body["session_id"], serde_json::json!(null));

    // 带 session_id：扫会话工作区（桌面打开的项目）。
    let (status, scoped) = get(
        &state,
        &format!("/symbols?query=scoped_marker&session_id={}", session.id),
    )
    .await;
    assert_eq!(status, 200, "{scoped}");
    assert_eq!(
        scoped["results"][0]["path"],
        serde_json::json!("project.rs"),
        "{scoped}"
    );
    assert_eq!(scoped["session_id"], serde_json::json!(session.id));
    assert_eq!(scoped["root"], serde_json::json!("."));

    // 会话不存在 → 404（不得静默回退到 Daemon 工作区）。
    let (status, _) = get(&state, "/symbols?query=x&session_id=no-such-session").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn rejects_missing_query_and_escaping_path() {
    let fixture = fixture();
    let (status, _body) = get(&fixture.state, "/symbols").await;
    // 错误体是纯文本（(StatusCode, String) 处理器），不是 JSON：这里只断言状态码。
    assert_eq!(status, 400, "缺少 query 必须 400");

    let (status, _) = get(&fixture.state, "/symbols?query=x&path=../outside").await;
    assert_eq!(status, 400, "越界 path 必须拒绝");
}
