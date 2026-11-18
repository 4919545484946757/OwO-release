//! CloudRemote 执行后端的 HTTP 契约测试。
//!
//! 未配置 `OWO_CLOUD_BASE_URL` 时云端队列走本地模拟传输：命令在隔离副本里执行，
//! 不联网。验证的是"统一 Task → 云端任务 → 云端执行事实"这条路径，以及暂停/恢复
//! 是否真的作用在云端任务上（恢复不会伪装断点续跑）。

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
    // 契约测试必须走本地模拟传输，不能把命令发到真实云端。
    std::env::remove_var("OWO_CLOUD_BASE_URL");
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

async fn create_cloud_task(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    payload: Value,
    policy_profile: Option<&str>,
) -> String {
    let mut body = json!({
        "user_goal": "在云端执行一次验证",
        "execution_strategy": "cloud_remote",
        "payload": payload,
    });
    if let Some(profile) = policy_profile {
        body["policy_profile"] = json!(profile);
    }
    let created = json_call(app, state, "POST", "/agent-tasks", Some(&body.to_string())).await;
    created["task"]["task_id"]
        .as_str()
        .unwrap_or_else(|| panic!("创建任务应返回 task_id：{created}"))
        .to_string()
}

async fn start_task(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    task_id: &str,
) -> Value {
    json_call(
        app,
        state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await
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

async fn task_state(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    task_id: &str,
) -> String {
    json_call(app, state, "GET", &format!("/agent-tasks/{task_id}"), None).await["task"]["state"]
        .as_str()
        .unwrap_or_default()
        .to_string()
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

fn event_types(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter_map(|event| event["event_type"].as_str().map(str::to_string))
        .collect()
}

async fn cloud_state(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    cloud_task_id: &str,
) -> String {
    json_call(
        app,
        state,
        "GET",
        &format!("/cloud/tasks/{cloud_task_id}"),
        None,
    )
    .await["state"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloud_task_runs_in_mock_transport_and_stops_at_verifying() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let task_id = create_cloud_task(
        &app,
        &state,
        json!({
            "name": "cloud-smoke",
            "commands": ["echo hello > note.txt"]
        }),
        Some("workspace-execute"),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(started["dispatched"], true, "应提交云端任务：{started}");
    let cloud_task_id = started["backend_run_id"]
        .as_str()
        .expect("应返回云端任务 id")
        .to_string();
    assert!(
        cloud_task_id.starts_with("cloud-"),
        "任务应提交到云端队列：{cloud_task_id}"
    );

    // 执行完成 ≠ 成功：云端退出码是远端自报，任务停在 verifying 等独立验证。
    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert_eq!(
        final_state,
        "verifying",
        "云端任务完成后应等待独立验证：{}",
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
    let types = event_types(&all);
    for expected in [
        "task.cloud.submitted",
        "task.cloud.completed",
        "task.backend.completed",
    ] {
        assert!(
            types.contains(&expected.to_string()),
            "缺少事件 {expected}：{types:?}"
        );
    }
    let submitted = all
        .iter()
        .find(|event| event["event_type"] == "task.cloud.submitted")
        .expect("应记录云端提交");
    assert_eq!(submitted["payload"]["transport"], "mock");
    assert_eq!(submitted["payload"]["commands"], 1);
    let completed = all
        .iter()
        .find(|event| event["event_type"] == "task.backend.completed")
        .expect("应提交执行事实");
    assert_eq!(
        completed["payload"]["completion_status"], "candidate",
        "云端结果只提交候选完成：{completed}"
    );

    // 云端任务本身成功，diff 可经既有接口取回（本后端不自动 apply）。
    assert_eq!(cloud_state(&app, &state, &cloud_task_id).await, "Succeeded");
    let result = json_call(
        &app,
        &state,
        "GET",
        &format!("/cloud/tasks/{cloud_task_id}/result"),
        None,
    )
    .await;
    let diff_summary = result["diff_summary"].as_str().unwrap_or_default();
    assert!(
        diff_summary.contains("note.txt"),
        "结果 diff 应包含写入的文件：{result}"
    );

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
        artifacts
            .iter()
            .any(|artifact| artifact["kind"] == "cloud_diff"),
        "云端 diff 应登记为产物：{artifacts:?}"
    );

    let verified = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/verify"),
        Some(
            &json!({
                "execution": {"passed": true, "detail": "云端退出码 0"},
                "goal": {"passed": true, "detail": "diff 已取回并检查"},
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(
        verified["task"]["state"], "succeeded",
        "验证通过后应成功：{verified}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failing_cloud_command_fails_the_task_without_faking_success() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let task_id = create_cloud_task(
        &app,
        &state,
        json!({"commands": ["exit /b 3"]}),
        Some("workspace-execute"),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(started["dispatched"], true, "应提交云端任务：{started}");

    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert_eq!(final_state, "failed", "非零退出码应让任务失败");

    let all = events(&app, &state, &task_id).await;
    let types = event_types(&all);
    assert!(
        types.contains(&"task.cloud.failed".to_string()),
        "缺少 task.cloud.failed：{types:?}"
    );
    let failed = all
        .iter()
        .find(|event| event["event_type"] == "task.backend.failed")
        .expect("应记录后端失败");
    assert_eq!(failed["payload"]["code"], "CLOUD_TASK_FAILED");
    assert!(
        !types.contains(&"task.succeeded".to_string()),
        "失败不得伪装成功：{types:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commands_require_workspace_execute_profile() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // 缺省 policy_profile=workspace：只允许工作区写入，不得借云端任务执行命令。
    let task_id = create_cloud_task(
        &app,
        &state,
        json!({"commands": ["echo should-not-run"]}),
        None,
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(
        started["dispatched"], false,
        "未授权命令不得分发：{started}"
    );
    assert_eq!(started["code"], "INVALID_REQUEST");
    assert_eq!(started["task"]["state"], "failed");
    let types = event_types(&events(&app, &state, &task_id).await);
    assert!(
        !types.contains(&"task.cloud.submitted".to_string()),
        "未授权时不得提交云端任务：{types:?}"
    );

    // 空命令同样是输入不成立，不提交空任务。
    let empty_id = create_cloud_task(
        &app,
        &state,
        json!({"commands": []}),
        Some("workspace-execute"),
    )
    .await;
    let empty = start_task(&app, &state, &empty_id).await;
    assert_eq!(empty["dispatched"], false, "空命令不得分发：{empty}");
    assert_eq!(empty["code"], "INVALID_REQUEST");
    assert_eq!(empty["task"]["state"], "failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_cancels_the_cloud_run_and_resume_submits_a_new_one() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // ping 约 3 秒：暂停落在云端任务仍执行期间，恢复时取消尚未被 runner 观察到。
    let task_id = create_cloud_task(
        &app,
        &state,
        json!({
            "commands": ["ping -n 4 127.0.0.1"],
            "timeout_secs": 30
        }),
        Some("workspace-execute"),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(started["dispatched"], true, "应提交云端任务：{started}");
    let first_cloud_id = started["backend_run_id"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    // 暂停 = 投递云端取消：远端没有"冻结"语义，任务停在 paused。
    let paused = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/pause"),
        Some(r#"{"reason":"测试暂停"}"#),
    )
    .await;
    assert_eq!(paused["task"]["state"], "paused", "暂停失败：{paused}");
    assert!(
        event_types(&events(&app, &state, &task_id).await)
            .contains(&"task.backend.pause_requested".to_string()),
        "暂停必须真的通知后端"
    );
    // 本地模拟传输里正在执行的命令不可中断：运行会跑完，但任务保持 paused，
    // 不会被迟到的云端结果翻成 verifying/succeeded。
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    assert_eq!(
        task_state(&app, &state, &task_id).await,
        "paused",
        "云端迟到的结果不能把暂停翻成完成"
    );

    // 恢复：暂停已投递取消，不能接一个正在被取消的云端任务，只能重新提交；
    // 后端必须如实回报"不支持断点续跑"，而不是伪装成继续执行。
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
    assert_eq!(
        final_state,
        "verifying",
        "恢复后应重新执行并进入待验证：{}",
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
    let types = event_types(&all);
    assert!(
        types.contains(&"task.backend.resume_unsupported".to_string()),
        "恢复必须如实回报不支持续跑：{types:?}"
    );
    let submitted: Vec<String> = all
        .iter()
        .filter(|event| event["event_type"] == "task.cloud.submitted")
        .filter_map(|event| {
            event["payload"]["cloud_task_id"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    assert_eq!(submitted.len(), 2, "恢复应重新提交一次云端任务：{types:?}");
    assert_ne!(submitted[0], submitted[1], "两次提交应是不同的云端任务");
    assert_eq!(submitted[0], first_cloud_id);
    // 旧运行被取消，新运行成功——旧监控器不能把新尝试收口成失败。
    assert_eq!(
        cloud_state(&app, &state, &submitted[0]).await,
        "Canceled",
        "暂停应取消第一次云端任务"
    );
    assert_eq!(
        cloud_state(&app, &state, &submitted[1]).await,
        "Succeeded",
        "恢复后的云端任务应成功"
    );
    assert!(
        !types.contains(&"task.cloud.failed".to_string()),
        "旧运行被取消不得记为失败：{types:?}"
    );
    let detail = json_call(
        &app,
        &state,
        "GET",
        &format!("/agent-tasks/{task_id}"),
        None,
    )
    .await;
    assert_eq!(detail["task"]["current_attempt"], 2, "恢复应进入第二次尝试");
}
