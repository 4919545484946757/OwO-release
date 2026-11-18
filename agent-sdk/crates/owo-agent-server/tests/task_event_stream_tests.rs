//! 统一 Task Runtime 事件流（Agent V2 Phase 5）契约测试。
//!
//! 覆盖：
//! 1. 终态任务的事件流按序号逐帧发送 `task-event`，最后发送一次 `done` 并结束；
//! 2. `after` 游标重连只补发游标之后的事件（断线重连不重复、不漏）；
//! 3. 不存在的任务返回 404，而不是一条永不结束的空流。

use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::{Agent, ChatMessage, ModelOutput, ToolSpec};
use owo_agent_server::build_router;
use std::sync::Arc;
use tower::ServiceExt;

/// 模型不可用：任务必然如实失败，用来得到一个确定的终态。
struct IdleProvider;

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        Err("IdleProvider 不应被调用".to_string())
    }
}

async fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
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
        .header(header::HOST, "127.0.0.1:4096")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

/// 创建一个会如实失败的任务，并等到终态（终态之后事件流才会发送 done）。
async fn failed_task(app: &axum::Router, state: &Arc<owo_agent_server::AppState>) -> String {
    let created = app
        .clone()
        .oneshot(request(
            state,
            "POST",
            "/agent-tasks",
            Some(r#"{"user_goal":"事件流契约任务","execution_strategy":"single_agent"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(created.status().as_u16(), 201);
    let body = axum::body::to_bytes(created.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = parsed["task"]["task_id"]
        .as_str()
        .expect("任务应返回 task_id")
        .to_string();

    let started = app
        .clone()
        .oneshot(request(
            state,
            "POST",
            &format!("/agent-tasks/{task_id}/start"),
            Some("{}"),
        ))
        .await
        .unwrap();
    assert_eq!(started.status().as_u16(), 200);

    for _ in 0..400 {
        let detail = app
            .clone()
            .oneshot(request(
                state,
                "GET",
                &format!("/agent-tasks/{task_id}"),
                None,
            ))
            .await
            .unwrap();
        let body = axum::body::to_bytes(detail.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let state_text = parsed["task"]["state"].as_str().unwrap_or_default();
        if state_text == "failed" {
            return task_id;
        }
        assert_ne!(state_text, "succeeded", "模型不可用时不得伪装成功");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("任务未在预期时间内进入终态");
}

async fn listed_events(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    task_id: &str,
) -> Vec<serde_json::Value> {
    let response = app
        .clone()
        .oneshot(request(
            state,
            "GET",
            &format!("/agent-tasks/{task_id}/events?limit=200"),
            None,
        ))
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    parsed["events"]
        .as_array()
        .expect("事件响应应包含 events 数组")
        .clone()
}

fn sse_ids(body: &str) -> Vec<u64> {
    body.lines()
        .filter_map(|line| line.strip_prefix("id: "))
        .filter_map(|value| value.trim().parse::<u64>().ok())
        .collect()
}

#[tokio::test]
async fn terminal_task_stream_emits_ordered_events_then_done() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let task_id = failed_task(&app, &state).await;

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        app.clone().oneshot(request(
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}/events/stream?after=0"),
            None,
        )),
    )
    .await
    .expect("事件流必须在终态后结束")
    .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/event-stream"),
        "事件流必须声明 SSE 类型，实际：{content_type}"
    );

    let body = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024),
    )
    .await
    .expect("读取事件流不得挂起")
    .unwrap();
    let text = String::from_utf8_lossy(&body).to_string();

    assert!(text.contains("event: task-event"), "缺少任务事件帧：{text}");
    assert!(
        text.contains("event: done"),
        "终态后必须发送 done 帧：{text}"
    );

    let ids = sse_ids(&text);
    let listed = listed_events(&app, &state, &task_id).await;
    assert_eq!(
        ids.len(),
        listed.len(),
        "SSE 帧数必须与事件日志一致：ids={ids:?}"
    );
    assert!(
        ids.windows(2).all(|pair| pair[0] < pair[1]),
        "事件帧必须严格升序：{ids:?}"
    );
    assert_eq!(
        ids.last().copied(),
        listed.last().and_then(|event| event["sequence"].as_u64()),
        "最后一帧必须是最后一条事件"
    );
}

#[tokio::test]
async fn after_cursor_resumes_without_replaying_older_events() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let task_id = failed_task(&app, &state).await;

    let listed = listed_events(&app, &state, &task_id).await;
    assert!(
        listed.len() >= 3,
        "本用例需要多条事件才能验证游标，实际 {}",
        listed.len()
    );
    let last = listed.last().unwrap()["sequence"].as_u64().unwrap();
    let after = last - 1;

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        app.clone().oneshot(request(
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}/events/stream?after={after}"),
            None,
        )),
    )
    .await
    .expect("事件流必须在终态后结束")
    .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body).to_string();

    assert_eq!(
        sse_ids(&text),
        vec![last],
        "after 游标只应补发最后一条事件：{text}"
    );
    assert!(text.contains("event: done"));
}

#[tokio::test]
async fn missing_task_stream_is_not_found() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        app.clone().oneshot(request(
            &state,
            "GET",
            "/agent-tasks/no-such-task/events/stream",
            None,
        )),
    )
    .await
    .expect("404 必须立即返回，不得挂起")
    .unwrap();
    assert_eq!(response.status().as_u16(), 404);
}
