//! WorkSwarm 执行后端的 HTTP 契约测试。
//!
//! 全程使用内置 echo / sleep Worker：不调用模型、不碰网络。验证的是
//! "统一 Task → 团队运行 → 步骤事件 → 执行事实"这条路径，以及暂停/恢复
//! 是否真的作用在团队运行上。

use std::sync::Arc;

use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use serde_json::{json, Value};
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
    if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

async fn json_call(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Value {
    let response = app
        .clone()
        .oneshot(request(state, method, path, body))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn create_work_swarm_task(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    payload: Value,
) -> String {
    let body = json!({
        "user_goal": "用团队完成一次说明文档的组稿",
        "execution_strategy": "work_swarm",
        "payload": payload,
    });
    let created = json_call(app, state, "POST", "/agent-tasks", Some(&body.to_string())).await;
    created["task"]["task_id"]
        .as_str()
        .unwrap_or_else(|| panic!("创建任务应返回 task_id：{created}"))
        .to_string()
}

/// 等到任务离开所有"仍在推进"的状态（verifying 是等待独立验证的稳定态）。
async fn wait_until_settled(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    task_id: &str,
) -> String {
    let mut last = String::new();
    for _ in 0..300 {
        let detail = json_call(app, state, "GET", &format!("/agent-tasks/{task_id}"), None).await;
        last = detail["task"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if !matches!(
            last.as_str(),
            "draft" | "queued" | "planning" | "running" | "repairing" | "cancel_requested"
        ) {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    last
}

async fn event_types(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    task_id: &str,
) -> Vec<String> {
    events(app, state, task_id)
        .await
        .iter()
        .filter_map(|event| event["event_type"].as_str().map(str::to_string))
        .collect()
}

async fn events(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    task_id: &str,
) -> Vec<Value> {
    let body = json_call(
        app,
        state,
        "GET",
        &format!("/agent-tasks/{task_id}/events?limit=500"),
        None,
    )
    .await;
    body["events"].as_array().cloned().unwrap_or_default()
}

async fn team_status(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    team_id: &str,
) -> String {
    let body = json_call(app, state, "GET", &format!("/teams/{team_id}"), None).await;
    body["team"]["status"].as_str().unwrap_or("").to_string()
}

async fn wait_team_status(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    team_id: &str,
    wanted: &str,
) -> String {
    let mut last = String::new();
    for _ in 0..100 {
        last = team_status(app, state, team_id).await;
        if last == wanted {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    last
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_swarm_task_runs_a_team_through_unified_runtime() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let task_id = create_work_swarm_task(
        &app,
        &state,
        json!({
            "mode": "team",
            "strategy": "team",
            "roles": [
                {"role": "planner", "assignee": "agent", "worker": "echo",
                 "handoff_contract": "产出方案大纲", "verify": "non_empty"},
                {"role": "builder", "assignee": "agent", "worker": "echo",
                 "depends_on": ["planner"], "handoff_contract": "产出草稿",
                 "verify": "non_empty"}
            ]
        }),
    )
    .await;

    let started = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(started["dispatched"], true, "应分发团队运行：{started}");

    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert!(
        matches!(final_state.as_str(), "succeeded" | "verifying"),
        "团队跑完后应进入验证或成功，而不是 {final_state}：{}",
        json_call(
            &app,
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}"),
            None
        )
        .await
    );

    let all = events(&app, &state, &task_id).await;
    let types: Vec<String> = all
        .iter()
        .filter_map(|event| event["event_type"].as_str().map(str::to_string))
        .collect();
    for expected in [
        "task.team.created",
        "task.step.started",
        "task.step.completed",
        "task.backend.completed",
    ] {
        assert!(
            types.contains(&expected.to_string()),
            "缺少事件 {expected}：{types:?}"
        );
    }

    // 团队运行本身成功，且团队 id 来自后端登记的事件（前端据此跳到团队视图）。
    let team_id = all
        .iter()
        .find(|event| event["event_type"] == "task.team.created")
        .and_then(|event| event["payload"]["team_id"].as_str())
        .expect("task.team.created 应带 team_id")
        .to_string();
    assert_eq!(
        team_status(&app, &state, &team_id).await,
        "succeeded",
        "echo 接力应成功"
    );

    // 团队产物登记到统一任务下（前端只查 /agent-tasks 也能看到交付物）。
    let artifacts = json_call(
        &app,
        &state,
        "GET",
        &format!("/agent-tasks/{task_id}/artifacts"),
        None,
    )
    .await;
    let artifacts = artifacts["artifacts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !artifacts.is_empty(),
        "团队步骤产物应登记到统一任务：{artifacts:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_stops_the_team_and_resume_continues_the_same_run() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // sleep Worker 让团队停在执行中，暂停请求才能落在阶段运行期间。
    let task_id = create_work_swarm_task(
        &app,
        &state,
        json!({
            "mode": "team",
            "strategy": "team",
            "roles": [
                {"role": "worker-1", "assignee": "agent", "worker": "sleep",
                 "verify": "non_empty", "extra_input": {"ms": 2500}}
            ]
        }),
    )
    .await;
    let started = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(started["dispatched"], true, "应分发团队运行：{started}");
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;

    let paused = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/pause"),
        Some(r#"{"reason":"测试暂停"}"#),
    )
    .await;
    assert_eq!(paused["task"]["state"], "paused", "暂停失败：{paused}");
    let types = event_types(&app, &state, &task_id).await;
    assert!(
        types.contains(&"task.backend.pause_requested".to_string()),
        "暂停必须真的通知后端：{types:?}"
    );

    // 暂停 = 团队运行被取消（不是只改徽标），团队确实停下。
    let team_id = events(&app, &state, &task_id)
        .await
        .iter()
        .find(|event| event["event_type"] == "task.team.created")
        .and_then(|event| event["payload"]["team_id"].as_str())
        .expect("应登记团队运行")
        .to_string();
    assert_eq!(
        wait_team_status(&app, &state, &team_id, "cancelled").await,
        "cancelled",
        "暂停后团队运行应停止"
    );
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let still = json_call(
        &app,
        &state,
        "GET",
        &format!("/agent-tasks/{task_id}"),
        None,
    )
    .await;
    assert_eq!(
        still["task"]["state"], "paused",
        "后端迟到的取消结果不能把暂停翻成失败：{still}"
    );

    // 恢复 = continue 同一次运行（已完成步骤不重跑），而不是新建一次团队。
    let resumed = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/resume"),
        Some("{}"),
    )
    .await;
    assert_eq!(resumed["task"]["state"], "running", "恢复失败：{resumed}");
    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert!(
        matches!(final_state.as_str(), "succeeded" | "verifying"),
        "恢复后应继续执行并完成，实际 {final_state}：{}",
        json_call(
            &app,
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}"),
            None
        )
        .await
    );
    assert_eq!(
        team_status(&app, &state, &team_id).await,
        "succeeded",
        "恢复后同一次团队运行应完成"
    );
}
