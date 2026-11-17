//! playbook_api 契约测试：模块经 `#[path]` 独立编译。
//!
//! 覆盖：空目录、列表（含进度与非法条目不中断）、记录写入并推进、
//! 非法 outcome/未知步骤 400、路径逃逸拒绝、状态文件与普通 json 的区分。

#[path = "../src/playbook_api.rs"]
mod playbook_api;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

/// 最小 AppState（与 plugin_market_api_tests 同一模式）。
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

fn request(method: &str, path: &str, body: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path);
    if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        builder.body(Body::from(body.to_string())).unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    }
}

async fn call(
    router: &Router,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(request(method, path, body))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// 写一份最小剧本到 `<workspace>/playbooks/`。
fn write_playbook(workspace: &std::path::Path, id: &str, extra_step: &str) {
    let dir = workspace.join("playbooks");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{id}.json")),
        format!(
            r#"{{"id":"{id}","name":"{id} 剧本","steps":[
                {{"id":"a","title":"A"}},
                {{"id":"b","title":"B","depends_on":["a"]}}{extra_step}
            ]}}"#
        ),
    )
    .unwrap();
}

#[tokio::test]
async fn empty_workspace_lists_nothing() {
    let (state, _temp) = test_state();
    let router = playbook_api::router(state);
    let (status, body) = call(&router, "GET", "/playbooks", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 0);
    assert!(body["playbooks"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn list_reports_progress_and_skips_state_files() {
    let (state, _temp) = test_state();
    let workspace = state.workspace.clone();
    write_playbook(&workspace, "demo", "");
    write_playbook(
        &workspace,
        "broken",
        r#",{"id":"c","title":"C","depends_on":["missing"]}"#,
    );
    // 状态文件与普通文本不应出现在列表里（状态文件内容合法，避免把 demo 判成坏状态）。
    std::fs::write(
        workspace.join("playbooks/demo.state.json"),
        r#"{"playbook_id":"demo","states":{}}"#,
    )
    .unwrap();
    std::fs::write(workspace.join("playbooks/notes.txt"), "x").unwrap();

    let router = playbook_api::router(Arc::clone(&state));
    let (status, body) = call(&router, "GET", "/playbooks", None).await;
    assert_eq!(status, StatusCode::OK);
    let playbooks = body["playbooks"].as_array().unwrap();
    assert_eq!(playbooks.len(), 2, "{body}");
    let demo = playbooks
        .iter()
        .find(|item| item["id"] == "demo")
        .expect("demo 应在列表");
    assert_eq!(demo["ok"], true);
    // 列表下发工作区相对路径：客户端可以原样回传给 status/record。
    assert_eq!(demo["path"], "playbooks/demo.json", "{demo}");
    assert_eq!(demo["progress"]["total"], 2);
    assert_eq!(demo["next"], "a");
    let broken = playbooks
        .iter()
        .find(|item| item["id"] == "broken")
        .expect("broken 应在列表");
    assert_eq!(broken["ok"], false);
    assert!(!broken["issues"].as_array().unwrap().is_empty());
    // 正常剧本没有高风险命令时 warnings=0。
    assert_eq!(demo["warnings"], 0);
}

#[tokio::test]
async fn record_writes_state_and_advances() {
    let (state, _temp) = test_state();
    let workspace = state.workspace.clone();
    write_playbook(&workspace, "demo", "");
    let router = playbook_api::router(Arc::clone(&state));

    let (status, body) = call(
        &router,
        "POST",
        "/playbooks/record",
        Some(r#"{"file":"playbooks/demo.json","step":"a","outcome":"pass","note":"证据 A"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["attempts"], 0);
    assert_eq!(body["progress"]["passed"], 1);
    assert_eq!(body["next"], "b");

    // 状态文件真的落盘，并能在 status 里读回。
    assert!(workspace.join("playbooks/demo.state.json").exists());
    let (status, body) = call(
        &router,
        "GET",
        "/playbooks/status?file=playbooks%2Fdemo.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["steps"][0]["state"], "passed");
    assert_eq!(body["steps"][0]["note"], "证据 A");
    assert_eq!(body["next"], "b");

    // 失败累计 attempts；越界/非法输入被拒。
    let (status, body) = call(
        &router,
        "POST",
        "/playbooks/record",
        Some(r#"{"file":"playbooks/demo.json","step":"b","outcome":"fail","note":"boom"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["attempts"], 1);

    let (status, _) = call(
        &router,
        "POST",
        "/playbooks/record",
        Some(r#"{"file":"playbooks/demo.json","step":"b","outcome":"maybe"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &router,
        "POST",
        "/playbooks/record",
        Some(r#"{"file":"playbooks/demo.json","step":"nope","outcome":"pass"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &router,
        "GET",
        "/playbooks/status?file=..%2Foutside.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn archive_creates_snapshot_and_rejects_bad_input() {
    let (state, _temp) = test_state();
    let workspace = state.workspace.clone();
    write_playbook(&workspace, "demo", "");
    let router = playbook_api::router(state);

    // 还没有状态文件 → 400。
    let (status, body) = call(
        &router,
        "POST",
        "/playbooks/archive",
        Some(r#"{"file":"playbooks/demo.json"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // 记录一步后归档：默认标记 ui-<millis>。
    let (status, _) = call(
        &router,
        "POST",
        "/playbooks/record",
        Some(r#"{"file":"playbooks/demo.json","step":"a","outcome":"pass","note":"ok"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &router,
        "POST",
        "/playbooks/archive",
        Some(r#"{"file":"playbooks/demo.json"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let marker = body["marker"].as_str().unwrap();
    assert!(marker.starts_with("ui-"), "{body}");
    assert!(workspace
        .join("playbooks/demo.runs")
        .join(format!("{marker}.json"))
        .exists());

    // 显式标记可重复校验：同名 400、非法标记 400。
    let (status, _) = call(
        &router,
        "POST",
        "/playbooks/archive",
        Some(r#"{"file":"playbooks/demo.json","marker":"snap-1"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &router,
        "POST",
        "/playbooks/archive",
        Some(r#"{"file":"playbooks/demo.json","marker":"snap-1"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &router,
        "POST",
        "/playbooks/archive",
        Some(r#"{"file":"playbooks/demo.json","marker":"../escape"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 历史能读到刚存的快照。
    let (status, body) = call(
        &router,
        "GET",
        "/playbooks/history?file=playbooks%2Fdemo.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["count"].as_u64().unwrap() >= 2, "{body}");
}

#[tokio::test]
async fn status_exposes_failure_kind() {
    let (state, _temp) = test_state();
    let workspace = state.workspace.clone();
    write_playbook(&workspace, "demo", "");
    std::fs::write(
        workspace.join("playbooks/demo.state.json"),
        r#"{"playbook_id":"demo","states":{"a":{"state":"failed","reason":"exit 1","attempts":1,"kind":"exit_nonzero"}}}"#,
    )
    .unwrap();
    let router = playbook_api::router(state);
    let (status, body) = call(
        &router,
        "GET",
        "/playbooks/status?file=playbooks%2Fdemo.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["steps"][0]["failure_kind"], "exit_nonzero", "{body}");
    assert_eq!(body["steps"][0]["attempts"], 1, "{body}");
}

#[tokio::test]
async fn status_reports_step_warnings() {
    let (state, _temp) = test_state();
    let dir = state.workspace.join("playbooks");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("risky.json"),
        r#"{"id":"risky","name":"危险","steps":[
            {"id":"w","title":"清理","kind":"check","acceptance":"exit 0","command":"rm -rf build"}
        ]}"#,
    )
    .unwrap();
    let router = playbook_api::router(state);
    let (status, body) = call(
        &router,
        "GET",
        "/playbooks/status?file=playbooks%2Frisky.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let warnings = body["steps"][0]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{body}");
    assert!(warnings[0].as_str().unwrap().contains("rm -rf"), "{body}");
}

#[tokio::test]
async fn history_lists_archives_with_step_states() {
    let (state, _temp) = test_state();
    let workspace = state.workspace.clone();
    write_playbook(&workspace, "demo", "");
    let runs = workspace.join("playbooks/demo.runs");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::write(
        runs.join("run-1.json"),
        r#"{"playbook_id":"demo","states":{"a":{"state":"passed","evidence":"ok"}}}"#,
    )
    .unwrap();
    std::fs::write(
        runs.join("run-2.json"),
        r#"{"playbook_id":"demo","states":{"b":{"state":"failed","reason":"boom","attempts":1,"kind":"exit_nonzero"}}}"#,
    )
    .unwrap();

    let router = playbook_api::router(state);
    let (status, body) = call(
        &router,
        "GET",
        "/playbooks/history?file=playbooks%2Fdemo.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 2, "{body}");
    assert_eq!(body["runs"][0]["marker"], "run-2");
    assert_eq!(body["runs"][0]["steps"]["b"], "failed");
    assert_eq!(body["runs"][0]["step_kinds"]["b"], "exit_nonzero", "{body}");
    assert_eq!(body["runs"][1]["steps"]["a"], "passed");
    // 未知剧本 → 400
    let (status, _) = call(
        &router,
        "GET",
        "/playbooks/history?file=playbooks%2Fmissing.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn status_rejects_missing_and_non_playbook_files() {
    let (state, _temp) = test_state();
    let workspace = state.workspace.clone();
    let router = playbook_api::router(Arc::clone(&state));
    let (status, _) = call(
        &router,
        "GET",
        "/playbooks/status?file=playbooks%2Fmissing.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // 非 json 扩展名拒绝（先造一个文件）。
    std::fs::create_dir_all(workspace.join("playbooks")).unwrap();
    std::fs::write(workspace.join("playbooks/notes.txt"), "{}").unwrap();
    let (status, _) = call(
        &router,
        "GET",
        "/playbooks/status?file=playbooks%2Fnotes.txt",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
