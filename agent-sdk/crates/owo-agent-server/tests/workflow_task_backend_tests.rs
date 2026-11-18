//! Workflow 执行后端的 HTTP 契约测试。
//!
//! 全程使用临时工作区里的 `.owflow` 与真实文件后端：不调用模型、不碰网络、
//! 不操作桌面。验证的是"统一 Task → .owflow 运行 → 步骤事实 → 持久化结果"，
//! 以及暂停/恢复是否真的作用在运行上（恢复 = 新建尝试并复用持久步骤，不重放已完成写入）。

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

fn flow(steps: Value) -> Value {
    json!({
        "id": "smoke", "name": "smoke-flow", "version": 1,
        "triggers": [{"id": "t1", "kind": {"kind": "manual"}}],
        "permissions": [{"scope": "fs.write", "mode": "allow"}],
        "preconditions": [],
        "rollback_points": [],
        "max_steps": 100, "subflow_depth_limit": 5,
        "steps": steps
    })
}

fn write_flow(workspace: &std::path::Path, name: &str, definition: &Value) {
    std::fs::write(
        workspace.join(format!("{name}.owflow")),
        serde_json::to_string_pretty(definition).unwrap(),
    )
    .unwrap();
}

async fn create_workflow_task(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    payload: Value,
) -> String {
    let body = json!({
        "user_goal": "执行一次工作流",
        "execution_strategy": "workflow",
        "payload": payload,
    });
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
    for _ in 0..200 {
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

/// 等某个事件出现（监控器按轮询间隔回报，事件可能比运行状态晚一个节拍）。
async fn wait_for_event(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    task_id: &str,
    event_type: &str,
) -> bool {
    for _ in 0..50 {
        if event_types(&events(app, state, task_id).await).contains(&event_type.to_string()) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

async fn run_state(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    run_id: &str,
) -> Value {
    let snapshot = json_call(app, state, "GET", &format!("/workflow/run/{run_id}"), None).await;
    snapshot["state"].clone()
}

async fn wait_run_state(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    run_id: &str,
    wanted: &str,
) -> Value {
    let mut last = Value::Null;
    for _ in 0..100 {
        last = run_state(app, state, run_id).await;
        if last.as_str() == Some(wanted) {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    last
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_task_runs_a_flow_and_stops_at_verifying() {
    let (state, _temp) = test_state().await;
    write_flow(
        &state.workspace,
        "write-smoke",
        &flow(json!([
            {"kind": "act", "id": "w1", "scope": "fs.write",
             "spec": {"action": "write_file", "target": "note.txt", "value": "hello"}}
        ])),
    );
    let app = build_router(Arc::clone(&state));

    let task_id = create_workflow_task(
        &app,
        &state,
        json!({"name": "write-smoke", "backend": "real"}),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(started["dispatched"], true, "应分发工作流运行：{started}");
    let run_id = started["backend_run_id"]
        .as_str()
        .expect("分发应返回运行 id")
        .to_string();
    assert!(
        run_id.starts_with("wf-"),
        "运行 id 应来自工作流运行器：{run_id}"
    );

    // 执行完成 ≠ 成功：工作流引擎的 Succeeded 是自报，任务停在 verifying。
    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert_eq!(
        final_state,
        "verifying",
        "工作流完成后应等待独立验证，而不是直接成功：{}",
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
        "task.workflow.created",
        "task.step.completed",
        "task.workflow.completed",
        "task.backend.completed",
    ] {
        assert!(
            types.contains(&expected.to_string()),
            "缺少事件 {expected}：{types:?}"
        );
    }
    let step = all
        .iter()
        .find(|event| event["event_type"] == "task.step.completed")
        .expect("应记录步骤事件");
    assert_eq!(step["payload"]["step_id"], "w1");
    assert_eq!(step["payload"]["ok"], true);
    let completed = all
        .iter()
        .find(|event| event["event_type"] == "task.backend.completed")
        .expect("应提交执行事实");
    assert_eq!(
        completed["payload"]["completion_status"], "candidate",
        "真实运行提交候选完成，不自行判定成功：{completed}"
    );

    assert_eq!(
        completed["payload"]["tool_calls"], 1,
        "one actual file backend call must be reported; assert/control-flow are not tool calls"
    );

    // 真实后端确实执行了动作：文件落在工作区里。
    let written = std::fs::read_to_string(state.workspace.join("note.txt")).unwrap();
    assert_eq!(written, "hello");
    assert_eq!(
        run_state(&app, &state, &run_id).await.as_str(),
        Some("succeeded"),
        "工作流运行本身应成功"
    );

    // 产物登记到统一任务下（前端只查 /agent-tasks 也能看到交付物）。
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
            .any(|artifact| artifact["kind"] == "workflow_outcome"),
        "运行结果应登记为产物：{artifacts:?}"
    );

    // 独立验证通过后才进入 succeeded（验证是任务层的动作，不是后端自报）。
    let verified = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/verify"),
        Some(
            &json!({
                "execution": {"passed": true, "detail": "工作流步骤全部成功"},
                "goal": {"passed": true, "detail": "note.txt 内容为 hello"},
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
async fn failing_flow_fails_the_task_without_faking_success() {
    let (state, _temp) = test_state().await;
    write_flow(
        &state.workspace,
        "failing-smoke",
        &flow(json!([
            {"kind": "assert", "id": "a1", "expr": "false", "timeout_ms": 100}
        ])),
    );
    let app = build_router(Arc::clone(&state));

    let task_id = create_workflow_task(
        &app,
        &state,
        json!({"name": "failing-smoke", "backend": "real"}),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(started["dispatched"], true, "应分发工作流运行：{started}");

    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert_eq!(final_state, "failed", "断言失败应让任务失败");

    let all = events(&app, &state, &task_id).await;
    let types = event_types(&all);
    for expected in ["task.workflow.failed", "task.backend.failed"] {
        assert!(
            types.contains(&expected.to_string()),
            "缺少事件 {expected}：{types:?}"
        );
    }
    let failed = all
        .iter()
        .find(|event| event["event_type"] == "task.backend.failed")
        .expect("应记录后端失败");
    assert_eq!(failed["payload"]["code"], "WORKFLOW_RUN_FAILED");
    assert!(
        !types.contains(&"task.succeeded".to_string()),
        "失败不得伪装成功：{types:?}"
    );
    let step = all
        .iter()
        .find(|event| event["event_type"] == "task.step.completed")
        .expect("失败步骤也应记录");
    assert_eq!(step["payload"]["ok"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_aborts_the_run_and_resume_starts_a_new_one() {
    let (state, _temp) = test_state().await;
    write_flow(
        &state.workspace,
        "approval-smoke",
        &flow(json!([
            {"kind": "act", "id": "w1", "scope": "fs.write",
             "spec": {"action": "write_file", "target": "a.txt", "value": "pre"}},
            {"kind": "human_approve", "id": "h1", "prompt": "是否继续？"},
            {"kind": "notify", "id": "n1", "message": "done"}
        ])),
    );
    let app = build_router(Arc::clone(&state));

    // 人审超时 700ms：暂停后运行会在超时点看到取消标志并中止。
    let task_id = create_workflow_task(
        &app,
        &state,
        json!({
            "name": "approval-smoke",
            "backend": "real",
            "approval_timeout_ms": 700
        }),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(started["dispatched"], true, "应分发工作流运行：{started}");
    let first_run = started["backend_run_id"].as_str().unwrap().to_string();
    assert_eq!(
        wait_run_state(&app, &state, &first_run, "waiting_approval")
            .await
            .as_str(),
        Some("waiting_approval"),
        "运行应停在人审节点"
    );
    assert!(
        wait_for_event(&app, &state, &task_id, "task.backend.awaiting_human").await,
        "人审等待应通知统一任务：{:?}",
        event_types(&events(&app, &state, &task_id).await)
    );

    // 暂停 = 运行协作式中止（不是只改徽标）。
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
    assert_eq!(
        wait_run_state(&app, &state, &first_run, "aborted")
            .await
            .as_str(),
        Some("aborted"),
        "暂停后工作流运行应停止"
    );

    // 恢复在新运行中重建控制流，复用已完成步骤的持久结果。
    let resumed = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/resume"),
        Some("{}"),
    )
    .await;
    assert_eq!(resumed["task"]["state"], "running", "恢复失败：{resumed}");
    let all = events(&app, &state, &task_id).await;
    let types = event_types(&all);
    assert!(
        types.contains(&"task.workflow.recovery.loaded".to_string()),
        "必须有持久步骤加载事实，不依赖旧执行器内存句柄：{types:?}"
    );
    let restored = all
        .iter()
        .find(|event| event["event_type"] == "task.workflow.recovery.loaded")
        .unwrap();
    assert_eq!(restored["payload"]["completed_invocations"], 1);
    assert_eq!(restored["payload"]["completed_effects"], 1);
    let created: Vec<&Value> = all
        .iter()
        .filter(|event| event["event_type"] == "task.workflow.created")
        .collect();
    assert_eq!(created.len(), 2, "恢复应新建一次运行：{created:?}");
    let second_run = created[1]["payload"]["run_id"]
        .as_str()
        .expect("新建运行应带 run_id")
        .to_string();
    assert_ne!(second_run, first_run, "恢复不能沿用已中止的运行");

    // 新运行同样停在人审：批准后应走完并进入待验证。
    assert_eq!(
        wait_run_state(&app, &state, &second_run, "waiting_approval")
            .await
            .as_str(),
        Some("waiting_approval")
    );
    let approved = json_call(
        &app,
        &state,
        "POST",
        &format!("/workflow/run/{second_run}/approval"),
        Some(r#"{"decision":"approve"}"#),
    )
    .await;
    assert_eq!(approved["decision"], "approve", "批准失败：{approved}");
    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert_eq!(final_state, "verifying", "恢复后的运行应完成并等待验证");
    assert_eq!(
        run_state(&app, &state, &second_run).await.as_str(),
        Some("succeeded")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_flow_name_fails_the_task_instead_of_leaving_it_running() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let task_id = create_workflow_task(&app, &state, json!({"backend": "real"})).await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(
        started["dispatched"], false,
        "缺少工作流名不得分发：{started}"
    );
    assert_eq!(started["code"], "INVALID_REQUEST");
    assert_eq!(
        started["task"]["state"], "failed",
        "输入不成立应把任务收口为失败，而不是停在 running：{started}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_flow_file_fails_the_task() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let task_id = create_workflow_task(
        &app,
        &state,
        json!({"name": "not-there", "backend": "real"}),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(
        started["dispatched"], false,
        "不存在的工作流不得分发：{started}"
    );
    assert_eq!(started["code"], "INVALID_REQUEST");
    assert_eq!(started["task"]["state"], "failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_resume_does_not_replay_a_completed_file_write() {
    let (state, _temp) = test_state().await;
    write_flow(
        &state.workspace,
        "recovery-smoke",
        &flow(json!([
            {"kind": "act", "id": "w1", "scope": "fs.write",
             "spec": {"action": "write_file", "target": "a.txt", "value": "pre"}},
            {"kind": "human_approve", "id": "h1", "prompt": "是否继续？"},
            {"kind": "notify", "id": "n1", "message": "done"}
        ])),
    );
    let app = build_router(Arc::clone(&state));

    // 人审超时 700ms：暂停后运行会在超时点看到取消标志并中止。
    let task_id = create_workflow_task(
        &app,
        &state,
        json!({
            "name": "recovery-smoke",
            "backend": "real",
            "approval_timeout_ms": 700
        }),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    assert_eq!(started["dispatched"], true, "应分发工作流运行：{started}");
    let first_run = started["backend_run_id"].as_str().unwrap().to_string();
    assert_eq!(
        wait_run_state(&app, &state, &first_run, "waiting_approval")
            .await
            .as_str(),
        Some("waiting_approval"),
        "运行应停在人审节点"
    );
    assert!(
        wait_for_event(&app, &state, &task_id, "task.backend.awaiting_human").await,
        "人审等待应通知统一任务：{:?}",
        event_types(&events(&app, &state, &task_id).await)
    );

    // 暂停 = 运行协作式中止（不是只改徽标）。
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
    assert_eq!(
        wait_run_state(&app, &state, &first_run, "aborted")
            .await
            .as_str(),
        Some("aborted"),
        "暂停后工作流运行应停止"
    );

    std::fs::write(state.workspace.join("a.txt"), "host-preserved").unwrap();
    // 恢复在新运行中重建控制流，复用已完成步骤的持久结果。
    let resumed = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/resume"),
        Some("{}"),
    )
    .await;
    assert_eq!(resumed["task"]["state"], "running", "恢复失败：{resumed}");
    let all = events(&app, &state, &task_id).await;
    let types = event_types(&all);
    assert!(
        types.contains(&"task.workflow.recovery.loaded".to_string()),
        "必须有持久步骤加载事实，不依赖旧执行器内存句柄：{types:?}"
    );
    let restored = all
        .iter()
        .find(|event| event["event_type"] == "task.workflow.recovery.loaded")
        .unwrap();
    assert_eq!(restored["payload"]["completed_invocations"], 1);
    assert_eq!(restored["payload"]["completed_effects"], 1);
    let created: Vec<&Value> = all
        .iter()
        .filter(|event| event["event_type"] == "task.workflow.created")
        .collect();
    assert_eq!(created.len(), 2, "恢复应新建一次运行：{created:?}");
    let second_run = created[1]["payload"]["run_id"]
        .as_str()
        .expect("新建运行应带 run_id")
        .to_string();
    assert_ne!(second_run, first_run, "恢复不能沿用已中止的运行");

    // 新运行同样停在人审：批准后应走完并进入待验证。
    assert_eq!(
        wait_run_state(&app, &state, &second_run, "waiting_approval")
            .await
            .as_str(),
        Some("waiting_approval")
    );
    let approved = json_call(
        &app,
        &state,
        "POST",
        &format!("/workflow/run/{second_run}/approval"),
        Some(r#"{"decision":"approve"}"#),
    )
    .await;
    assert_eq!(approved["decision"], "approve", "批准失败：{approved}");
    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert_eq!(
        std::fs::read_to_string(state.workspace.join("a.txt")).unwrap(),
        "host-preserved",
        "completed write must not run again on resume"
    );
    assert_eq!(final_state, "verifying", "恢复后的运行应完成并等待验证");
    let completed_events = events(&app, &state, &task_id).await;
    let completed = completed_events
        .iter()
        .find(|event| event["event_type"] == "task.backend.completed")
        .unwrap();
    assert_eq!(
        completed["payload"]["tool_calls"], 1,
        "only the new notify call belongs to this attempt"
    );
    assert_eq!(
        completed["payload"]["completion_record"]["metrics"]["reused_tool_calls"], 1,
        "the prior confirmed write is reused, not counted again"
    );

    assert_eq!(
        run_state(&app, &state, &second_run).await.as_str(),
        Some("succeeded")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_flow_requires_reconciliation_without_new_write() {
    let (state, _temp) = test_state().await;
    let definition = flow(json!([
        {"kind":"act","id":"write","scope":"fs.write","spec":{"action":"append_file","target":"protected.txt","value":"original"}},
        {"kind":"human_approve","id":"wait","prompt":"continue"},
        {"kind":"act","id":"final","scope":"fs.write","spec":{"action":"write_file","target":"result.txt","value":"old result"}}
    ]));
    write_flow(&state.workspace, "changed-recovery", &definition);
    let app = build_router(Arc::clone(&state));
    let task_id = create_workflow_task(
        &app,
        &state,
        json!({"name":"changed-recovery","backend":"real"}),
    )
    .await;
    let started = start_task(&app, &state, &task_id).await;
    let run = started["backend_run_id"].as_str().unwrap();
    assert_eq!(
        wait_run_state(&app, &state, run, "waiting_approval").await,
        "waiting_approval"
    );
    let paused = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/pause"),
        Some("{}"),
    )
    .await;
    assert_eq!(paused["task"]["state"], "paused");
    assert_eq!(
        wait_run_state(&app, &state, run, "aborted").await,
        "aborted"
    );
    let mut changed = definition.clone();
    changed["steps"][0]["spec"]["value"] = json!("must-not-overwrite");
    write_flow(&state.workspace, "changed-recovery", &changed);
    let resumed = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/resume"),
        Some("{}"),
    )
    .await;
    assert_eq!(resumed["code"], "WORKFLOW_RECOVERY_REQUIRED", "{resumed}");
    let detail = json_call(
        &app,
        &state,
        "GET",
        &format!("/agent-tasks/{task_id}"),
        None,
    )
    .await;
    assert_eq!(detail["task"]["state"], "needs_user_input");
    assert_eq!(
        std::fs::read_to_string(state.workspace.join("protected.txt")).unwrap(),
        "original"
    );
    assert_eq!(
        events(&app, &state, &task_id)
            .await
            .iter()
            .filter(|e| e["event_type"] == "task.workflow.created")
            .count(),
        1
    );
    let approved_bad = json_call(&app, &state, "GET", "/workflow/changed-recovery", None).await;
    let bad_body = json!({"approved_workflow_revision": approved_bad["definition"]}).to_string();
    let rejected = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/resume"),
        Some(&bad_body),
    )
    .await;
    assert_eq!(rejected["code"], "WORKFLOW_RECOVERY_REQUIRED");
    assert!(!state.workspace.join("result.txt").exists());

    let mut revised = definition;
    revised["steps"][2]["spec"]["value"] = json!("revised result");
    write_flow(&state.workspace, "changed-recovery", &revised);
    let reviewed = json_call(&app, &state, "GET", "/workflow/changed-recovery", None).await;
    let body = json!({"approved_workflow_revision": reviewed["definition"]}).to_string();
    let resumed = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/resume"),
        Some(&body),
    )
    .await;
    assert_eq!(resumed["task"]["state"], "running", "{resumed}");
    let all = events(&app, &state, &task_id).await;
    let runs: Vec<_> = all
        .iter()
        .filter(|e| e["event_type"] == "task.workflow.created")
        .collect();
    assert_eq!(runs.len(), 2);
    let revised_run = runs[1]["payload"]["run_id"].as_str().unwrap();
    assert_eq!(
        wait_run_state(&app, &state, revised_run, "waiting_approval").await,
        "waiting_approval"
    );
    assert!(!state.workspace.join("result.txt").exists());
    let approved = json_call(
        &app,
        &state,
        "POST",
        &format!("/workflow/run/{revised_run}/approval"),
        Some(r#"{"decision":"approve"}"#),
    )
    .await;
    assert_eq!(approved["decision"], "approve");
    assert_eq!(
        wait_until_settled(&app, &state, &task_id).await,
        "verifying"
    );
    assert_eq!(
        std::fs::read_to_string(state.workspace.join("protected.txt")).unwrap(),
        "original"
    );
    assert_eq!(
        std::fs::read_to_string(state.workspace.join("result.txt")).unwrap(),
        "revised result"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_git_head_blocks_resume_before_next_effect() {
    let (state, temp) = test_state().await;
    assert!(state.workspace.starts_with(temp.path()));
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&state.workspace)
            .args([
                "-c",
                "user.name=OwO test",
                "-c",
                "user.email=owo@test.local",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q", "--template="]);
    let tree = git(&["write-tree"]);
    let first = git(&["commit-tree", &tree, "-m", "Synthetic before"]);
    git(&["update-ref", "HEAD", &first]);

    let definition = flow(json!([
        {"kind":"act","id":"before","scope":"fs.write","spec":{"action":"append_file","target":"progress.txt","value":"A"}},
        {"kind":"human_approve","id":"wait","prompt":"continue"},
        {"kind":"act","id":"after","scope":"fs.write","spec":{"action":"append_file","target":"progress.txt","value":"B"}}
    ]));
    write_flow(&state.workspace, "git-recovery", &definition);
    let app = build_router(Arc::clone(&state));
    let task = create_workflow_task(
        &app,
        &state,
        json!({"name":"git-recovery","backend":"real"}),
    )
    .await;
    let started = start_task(&app, &state, &task).await;
    let run = started["backend_run_id"].as_str().unwrap();
    assert_eq!(
        wait_run_state(&app, &state, run, "waiting_approval").await,
        "waiting_approval"
    );
    assert!(events(&app, &state, &task)
        .await
        .iter()
        .any(|e| e["payload"]["workspace_revision"] == first));
    assert_eq!(
        json_call(
            &app,
            &state,
            "POST",
            &format!("/agent-tasks/{task}/pause"),
            Some("{}"),
        )
        .await["task"]["state"],
        "paused"
    );
    assert_eq!(
        wait_run_state(&app, &state, run, "aborted").await,
        "aborted"
    );

    let next = git(&["commit-tree", &tree, "-p", &first, "-m", "Synthetic after"]);
    assert_ne!(first, next);
    git(&["update-ref", "HEAD", &next, &first]);
    let resumed = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task}/resume"),
        Some("{}"),
    )
    .await;
    assert_eq!(resumed["code"], "WORKFLOW_RECOVERY_REQUIRED", "{resumed}");
    assert_eq!(
        json_call(&app, &state, "GET", &format!("/agent-tasks/{task}"), None).await["task"]
            ["state"],
        "needs_user_input"
    );
    assert_eq!(
        std::fs::read_to_string(state.workspace.join("progress.txt")).unwrap(),
        "A"
    );
    assert_eq!(
        events(&app, &state, &task)
            .await
            .iter()
            .filter(|e| e["event_type"] == "task.workflow.created")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_run_or_orphan_effect_claim_stops_before_write() {
    let (state, _temp) = test_state().await;
    write_flow(
        &state.workspace,
        "orphan-recovery",
        &flow(json!([
            {"kind":"act","id":"write","scope":"fs.write","spec":{"action":"write_file","target":"forbidden.txt","value":"NO"}}
        ])),
    );
    let app = build_router(Arc::clone(&state));
    for legacy in [true, false] {
        let task_id = create_workflow_task(
            &app,
            &state,
            json!({"name":"orphan-recovery","backend":"real"}),
        )
        .await;
        if legacy {
            state
                .tasks
                .store()
                .append_event(
                    &task_id,
                    "task.workflow.created",
                    1,
                    json!({"run_id":"legacy"}),
                    None,
                )
                .unwrap();
        } else {
            state
                .tasks
                .store()
                .put_idempotency(
                    &format!("workflow-effect:{task_id}:0"),
                    &task_id,
                    &json!({"sequence":0}),
                )
                .unwrap();
        }
        let _ = start_task(&app, &state, &task_id).await;
        assert_eq!(
            wait_until_settled(&app, &state, &task_id).await,
            "needs_user_input"
        );
        let all = events(&app, &state, &task_id).await;
        assert!(all
            .iter()
            .any(|e| e["event_type"] == "task.user_input.required"
                && e["payload"]["code"] == "WORKFLOW_RECOVERY_REQUIRED"));
        assert!(!state.workspace.join("forbidden.txt").exists());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_effect_budget_survives_resume() {
    let (state, _temp) = test_state().await;
    write_flow(
        &state.workspace,
        "budget-recovery",
        &flow(json!([
            {"kind":"act","id":"first","scope":"fs.write","spec":{"action":"write_file","target":"first.txt","value":"ONCE"}},
            {"kind":"human_approve","id":"wait","prompt":"continue"},
            {"kind":"act","id":"second","scope":"fs.write","spec":{"action":"write_file","target":"second.txt","value":"NO"}}
        ])),
    );
    let app = build_router(Arc::clone(&state));
    let body = json!({"user_goal":"Bounded workflow recovery","execution_strategy":"workflow",
        "budget":{"max_attempts":8,"max_repairs":0,"max_tool_calls":1,"max_model_tokens":0,"max_duration_ms":0},
        "payload":{"name":"budget-recovery","backend":"real"}});
    let created = json_call(
        &app,
        &state,
        "POST",
        "/agent-tasks",
        Some(&body.to_string()),
    )
    .await;
    let task_id = created["task"]["task_id"].as_str().unwrap();
    let started = start_task(&app, &state, task_id).await;
    let run = started["backend_run_id"].as_str().unwrap();
    assert_eq!(
        wait_run_state(&app, &state, run, "waiting_approval").await,
        "waiting_approval"
    );
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/pause"),
        Some("{}"),
    )
    .await;
    assert_eq!(
        wait_run_state(&app, &state, run, "aborted").await,
        "aborted"
    );
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/resume"),
        Some("{}"),
    )
    .await;
    let created_runs = events(&app, &state, task_id).await;
    let resumed_run = created_runs
        .iter()
        .rev()
        .find(|e| e["event_type"] == "task.workflow.created")
        .unwrap()["payload"]["run_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        wait_run_state(&app, &state, resumed_run, "waiting_approval").await,
        "waiting_approval"
    );
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/workflow/run/{resumed_run}/approval"),
        Some(r#"{"decision":"approve"}"#),
    )
    .await;
    assert_eq!(
        wait_until_settled(&app, &state, task_id).await,
        "needs_user_input"
    );
    assert_eq!(
        std::fs::read_to_string(state.workspace.join("first.txt")).unwrap(),
        "ONCE"
    );
    assert!(!state.workspace.join("second.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_journal_and_completion_omit_full_inputs_and_file_samples() {
    let (state, _temp) = test_state().await;
    let private_name = "SYNTHETIC_PRIVATE_FILENAME_7251.txt";
    let private_value = "SYNTHETIC_PRIVATE_FILE_VALUE_8441";
    let private_scope = "SYNTHETIC_PRIVATE_SCOPE_VALUE_7462";
    std::fs::write(state.workspace.join(private_name), "fixture-only").unwrap();
    write_flow(
        &state.workspace,
        "minimal-journal",
        &flow(json!([
            {"kind":"sense","id":"listing","spec":{"target":"files"}},
            {"kind":"act","id":"write","scope":"fs.write","spec":{"action":"write_file","target":"result.txt","value":private_value}},
            {"kind":"assert","id":"present","expr":"exists(listing)"}
        ])),
    );
    let app = build_router(Arc::clone(&state));
    let task_id = create_workflow_task(
        &app,
        &state,
        json!({"name":"minimal-journal","backend":"real","ctx":{"private_extra":private_scope}}),
    )
    .await;
    assert_eq!(start_task(&app, &state, &task_id).await["dispatched"], true);
    assert_eq!(
        wait_until_settled(&app, &state, &task_id).await,
        "verifying"
    );
    assert_eq!(
        std::fs::read_to_string(state.workspace.join("result.txt")).unwrap(),
        private_value
    );
    let all = events(&app, &state, &task_id).await;
    let serialized = serde_json::to_string(&all).unwrap();
    for private in [private_name, private_value, private_scope] {
        assert!(
            !serialized.contains(private),
            "task events copied complete input/sampled file names"
        );
    }
    let latest = all
        .iter()
        .rev()
        .find(|event| event["event_type"] == "task.workflow.journal")
        .unwrap();
    assert!(latest["payload"]["scope"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert_eq!(latest["payload"]["state"]["version"], 2);
    assert!(latest["payload"]["state"].get("definition").is_none());
    assert_eq!(latest["payload"]["state"]["reserved_tool_calls"], 2);
    let completed = all
        .iter()
        .find(|event| event["event_type"] == "task.backend.completed")
        .unwrap();
    assert_eq!(
        completed["payload"]["tool_calls"], 2,
        "sense and write are separate backend calls"
    );
    assert_eq!(
        completed["payload"]["completion_record"]["metrics"]["reserved_task_tool_calls"],
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_read_budget_survives_resume_and_blocks_the_next_write() {
    let (state, _temp) = test_state().await;
    write_flow(
        &state.workspace,
        "read-budget",
        &flow(json!([
            {"kind":"sense","id":"listing","spec":{"target":"files"}},
            {"kind":"human_approve","id":"wait","prompt":"continue"},
            {"kind":"act","id":"write","scope":"fs.write","spec":{"action":"write_file","target":"must-not-exist.txt","value":"NO"}}
        ])),
    );
    let app = build_router(Arc::clone(&state));
    let body = json!({"user_goal":"Read budget must not reset","execution_strategy":"workflow",
        "budget":{"max_attempts":8,"max_repairs":0,"max_tool_calls":1,"max_model_tokens":0,"max_duration_ms":0},
        "payload":{"name":"read-budget","backend":"real"}});
    let created = json_call(
        &app,
        &state,
        "POST",
        "/agent-tasks",
        Some(&body.to_string()),
    )
    .await;
    let task_id = created["task"]["task_id"].as_str().unwrap();
    let started = start_task(&app, &state, task_id).await;
    let run = started["backend_run_id"].as_str().unwrap();
    assert_eq!(
        wait_run_state(&app, &state, run, "waiting_approval").await,
        "waiting_approval"
    );
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/pause"),
        Some("{}"),
    )
    .await;
    assert_eq!(
        wait_run_state(&app, &state, run, "aborted").await,
        "aborted"
    );
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/resume"),
        Some("{}"),
    )
    .await;
    let all = events(&app, &state, task_id).await;
    let resumed_run = all
        .iter()
        .rev()
        .find(|event| event["event_type"] == "task.workflow.created")
        .unwrap()["payload"]["run_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        wait_run_state(&app, &state, resumed_run, "waiting_approval").await,
        "waiting_approval"
    );
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/workflow/run/{resumed_run}/approval"),
        Some(r#"{"decision":"approve"}"#),
    )
    .await;
    assert_eq!(
        wait_until_settled(&app, &state, task_id).await,
        "needs_user_input"
    );
    assert!(!state.workspace.join("must-not-exist.txt").exists());
    assert!(
        state
            .tasks
            .store()
            .get_idempotency(&format!("workflow-effect:{task_id}:2"))
            .unwrap()
            .is_none(),
        "budget denial must precede write intent claim"
    );
    let all = events(&app, &state, task_id).await;
    let metrics = all
        .iter()
        .rev()
        .find(|event| event["event_type"] == "task.workflow.metrics")
        .unwrap();
    assert_eq!(metrics["payload"]["metrics"]["tool_calls"], 0);
    assert_eq!(metrics["payload"]["metrics"]["reused_tool_calls"], 1);
    assert_eq!(metrics["payload"]["metrics"]["reserved_task_tool_calls"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_list_object_is_not_boolean_true_and_chooses_the_right_branch() {
    let (state, _temp) = test_state().await;
    write_flow(
        &state.workspace,
        "strict-equality",
        &flow(json!([
            {"kind":"sense","id":"listing","spec":{"target":"files"}},
            {"kind":"cond","id":"choice","expr":"listing == true",
             "then":[{"kind":"act","id":"wrong","scope":"fs.write","spec":{"action":"write_file","target":"branch.txt","value":"WRONG"}}],
             "otherwise":[{"kind":"act","id":"right","scope":"fs.write","spec":{"action":"write_file","target":"branch.txt","value":"RIGHT"}}]}
        ])),
    );
    let app = build_router(Arc::clone(&state));
    let task_id = create_workflow_task(
        &app,
        &state,
        json!({"name":"strict-equality","backend":"real"}),
    )
    .await;
    assert_eq!(start_task(&app, &state, &task_id).await["dispatched"], true);
    assert_eq!(
        wait_until_settled(&app, &state, &task_id).await,
        "verifying"
    );
    assert_eq!(
        std::fs::read_to_string(state.workspace.join("branch.txt")).unwrap(),
        "RIGHT"
    );
    let all = events(&app, &state, &task_id).await;
    assert!(all
        .iter()
        .any(|e| e["event_type"] == "task.step.completed" && e["payload"]["step_id"] == "right"));
    assert!(!all
        .iter()
        .any(|e| e["event_type"] == "task.step.completed" && e["payload"]["step_id"] == "wrong"));
    let completed = all
        .iter()
        .find(|e| e["event_type"] == "task.backend.completed")
        .unwrap();
    assert_eq!(completed["payload"]["tool_calls"], 2);
    assert_eq!(
        completed["payload"]["completion_record"]["metrics"]["reserved_task_tool_calls"],
        2
    );
}
