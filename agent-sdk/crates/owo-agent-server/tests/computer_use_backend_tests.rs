//! Computer Use 执行后端的 HTTP 契约测试。
//!
//! 全程注入内存 Mock 桌面面：不打开真实窗口、不截图、不发送输入。验证的是
//! "统一 Task → 任务级审批 → 感知闭环 → 执行事实 → 独立验证"这条产品路径，
//! 以及失败/熔断时**不伪造成功**。

use std::sync::{Arc, Mutex};

use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::task_surface::{TaskSurface, WindowIdentity};
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use owo_agent_server::computer_use_backend::ComputerUseBackend;
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

fn ocr_line(text: &str, x: i64, y: i64, role: &str) -> Value {
    json!({
        "text": text,
        "x": x,
        "y": y,
        "width": 120,
        "height": 20,
        "role_hint": role,
    })
}

/// Mock 桌面面：OCR 版面与动作序列都由测试控制。
#[derive(Clone)]
struct MockDesktop {
    app: String,
    lines: Arc<Mutex<Vec<Value>>>,
    actions: Arc<Mutex<Vec<String>>>,
    /// 每次 OCR 的耗时；用于让闭环"停在半路"，测试暂停/恢复。
    ocr_delay: std::time::Duration,
}

impl MockDesktop {
    fn new(app: &str, lines: Vec<Value>) -> Self {
        Self {
            app: app.to_string(),
            lines: Arc::new(Mutex::new(lines)),
            actions: Arc::new(Mutex::new(Vec::new())),
            ocr_delay: std::time::Duration::ZERO,
        }
    }

    fn with_ocr_delay(mut self, delay: std::time::Duration) -> Self {
        self.ocr_delay = delay;
        self
    }

    fn push_line(&self, text: &str) {
        if let Ok(mut lines) = self.lines.lock() {
            let y = 10 + (lines.len() as i64) * 30;
            lines.push(ocr_line(text, 10, y, "text"));
        }
    }

    fn actions(&self) -> Vec<String> {
        self.actions
            .lock()
            .map(|all| all.clone())
            .unwrap_or_default()
    }
}

struct MockSurface {
    desktop: MockDesktop,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_failure_preserves_each_executed_action_receipt() {
    let (state, _temp) = test_state().await;
    let desktop = MockDesktop::new("notepad", vec![ocr_line("anchor", 10, 10, "input")]);
    install_mock_backend(&state, desktop.clone());
    let app = build_router(Arc::clone(&state));
    let id = create_computer_task(&app, &state, json!({
        "target_app": "notepad",
        "goals": [
            {"anchor_text":"anchor", "action":"type", "value":"first result", "verify_text":"first result"},
            {"anchor_text":"anchor", "action":"key", "value":"Escape", "verify_text":"missing result"},
            {"anchor_text":"anchor", "action":"type", "value":"must not execute", "verify_text":"must not execute"}
        ]
    })).await;
    let first = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(first["code"], "APPROVAL_REQUIRED", "{first}");
    let approved = json_call(
        &app,
        &state,
        "POST",
        &format!("/computer-use/task/{id}/approve"),
        Some("{}"),
    )
    .await;
    assert_eq!(approved["ok"], true, "{approved}");
    let start = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(start["dispatched"], true, "{start}");
    assert_eq!(wait_until_settled(&app, &state, &id).await, "failed");
    assert_eq!(desktop.actions(), vec!["type:first result", "key:Escape"]);
    let events = json_call(
        &app,
        &state,
        "GET",
        &format!("/agent-tasks/{id}/events?limit=200"),
        None,
    )
    .await;
    let all = events["events"].as_array().unwrap();
    let actions: Vec<_> = all
        .iter()
        .filter(|event| event["event_type"] == "task.computer.action.recorded")
        .collect();
    assert_eq!(
        actions.len(),
        4,
        "each executed action needs durable execution and observation facts: {events}"
    );
    for pair in actions.chunks_exact(2) {
        assert_eq!(pair[0]["payload"]["stage"], "execution");
        assert_eq!(pair[1]["payload"]["stage"], "observation");
        assert_eq!(
            pair[0]["payload"]["receipt"]["action_id"],
            pair[1]["payload"]["receipt"]["action_id"]
        );
        assert_eq!(pair[0]["payload"]["receipt"]["outcome"]["kind"], "executed");
        assert_eq!(
            pair[0]["payload"]["receipt"]["verification"]["kind"],
            "not_verified"
        );
    }
    assert_eq!(
        actions[1]["payload"]["receipt"]["verification"]["kind"],
        "verified"
    );
    assert_eq!(
        actions[3]["payload"]["receipt"]["verification"]["kind"],
        "not_verified"
    );
    assert!(actions[3]["payload"]["receipt"]["after_observation_ref"].is_string());
    let failure = all
        .iter()
        .find(|event| event["event_type"] == "task.backend.failed")
        .unwrap();
    assert!(actions[3]["sequence"].as_u64().unwrap() < failure["sequence"].as_u64().unwrap());
    let artifacts = json_call(
        &app,
        &state,
        "GET",
        &format!("/agent-tasks/{id}/artifacts"),
        None,
    )
    .await;
    assert_eq!(
        artifacts["artifacts"].as_array().unwrap().len(),
        2,
        "do not lose or duplicate action artifacts: {artifacts}"
    );
}

/// Simulate the foreground changing while an OCR request is in flight.
struct SwitchDuringOcrSurface {
    inner: MockSurface,
    switched: bool,
    title_only: bool,
    observations: usize,
    switch_after_observations: usize,
}

#[async_trait::async_trait]
impl TaskSurface for SwitchDuringOcrSurface {
    fn app(&self) -> String {
        if self.switched && !self.title_only {
            "other-app".into()
        } else {
            "notepad".into()
        }
    }

    fn window_identity(&self) -> Option<WindowIdentity> {
        Some(WindowIdentity {
            app: self.app(),
            pid: if self.switched && !self.title_only {
                43
            } else {
                42
            },
            hwnd: if self.switched && !self.title_only {
                8
            } else {
                7
            },
            title: if self.switched && self.title_only {
                "other document"
            } else {
                "test window"
            }
            .into(),
            foreground: true,
            observed_at: "2026-11-04T00:00:00Z".into(),
        })
    }

    async fn ocr(&mut self) -> Result<Value, String> {
        let observed = self.inner.ocr().await?;
        self.observations += 1;
        self.switched = self.observations >= self.switch_after_observations;
        Ok(observed)
    }
    async fn click(&mut self, x: i32, y: i32) -> Result<(), String> {
        self.inner.click(x, y).await
    }
    async fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.inner.type_text(text).await
    }
    async fn key(&mut self, key: &str) -> Result<(), String> {
        self.inner.key(key).await
    }
    async fn launch(&mut self, target: &str) -> Result<(), String> {
        self.inner.launch(target).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreground_switch_during_ocr_parks_both_tasks_for_confirmation() {
    assert_foreground_switch_parks_task(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_document_switch_in_same_window_blocks_input() {
    assert_foreground_switch_parks_task(true).await;
}

async fn assert_foreground_switch_parks_task(title_only: bool) {
    let (state, _temp) = test_state().await;
    let desktop = MockDesktop::new("notepad", vec![ocr_line("anchor", 10, 10, "button")]);
    let factory_desktop = desktop.clone();
    let backend = ComputerUseBackend::new(
        Arc::downgrade(&state.tasks),
        Arc::clone(&state.computer_tasks),
        state.agent.audit_log(),
    )
    .with_surface_factory(Arc::new(move || {
        Ok(Box::new(SwitchDuringOcrSurface {
            inner: MockSurface {
                desktop: factory_desktop.clone(),
            },
            switched: false,
            title_only,
            observations: 0,
            switch_after_observations: 1,
        }) as Box<dyn TaskSurface>)
    }));
    state.tasks.register_backend(Arc::new(backend));
    let app = build_router(Arc::clone(&state));
    let id = create_computer_task(&app, &state, json!({
        "target_app": "notepad",
        "target_window_title": title_only.then_some("test window"),
        "goals": [{"anchor_text":"anchor", "action":"click", "value":"", "verify_text":"anchor"}]
    })).await;
    let pending = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(pending["code"], "APPROVAL_REQUIRED", "{pending}");
    let approved = json_call(
        &app,
        &state,
        "POST",
        &format!("/computer-use/task/{id}/approve"),
        Some("{}"),
    )
    .await;
    assert_eq!(approved["state"], "Approved", "{approved}");
    let started = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(started["dispatched"], true, "{started}");
    let settled = wait_until_settled(&app, &state, &id).await;
    assert_eq!(
        settled, "needs_user_input",
        "a window switch must be recoverable"
    );
    assert!(
        desktop.actions().is_empty(),
        "never inject into the changed window"
    );
    assert_eq!(
        state.computer_tasks.get(&id).unwrap().state,
        owo_agent_core::computer_task::TaskState::Paused
    );
    let response = json_call(
        &app,
        &state,
        "GET",
        &format!("/agent-tasks/{id}/events?limit=200"),
        None,
    )
    .await;
    let all = response["events"].as_array().expect("task events array");
    assert!(all
        .iter()
        .any(|event| event["event_type"] == "task.user_input.required"
            && event["payload"]["code"] == "STALE_WINDOW_IDENTITY"));
}

#[async_trait::async_trait]
impl TaskSurface for MockSurface {
    fn app(&self) -> String {
        self.desktop.app.clone()
    }

    fn window_identity(&self) -> Option<WindowIdentity> {
        None
    }

    async fn ocr(&mut self) -> Result<Value, String> {
        if !self.desktop.ocr_delay.is_zero() {
            tokio::time::sleep(self.desktop.ocr_delay).await;
        }
        let lines = self
            .desktop
            .lines
            .lock()
            .map_err(|_| "Mock 版面锁中毒".to_string())?
            .clone();
        Ok(json!({ "lines": lines }))
    }

    async fn click(&mut self, x: i32, y: i32) -> Result<(), String> {
        if let Ok(mut actions) = self.desktop.actions.lock() {
            actions.push(format!("click:{x},{y}"));
        }
        // 点击"发送"后界面出现结果文本，供动作后验证命中。
        self.desktop.push_line("已发送");
        Ok(())
    }

    async fn type_text(&mut self, text: &str) -> Result<(), String> {
        if let Ok(mut actions) = self.desktop.actions.lock() {
            actions.push(format!("type:{text}"));
        }
        self.desktop.push_line(text);
        Ok(())
    }

    async fn key(&mut self, key: &str) -> Result<(), String> {
        if let Ok(mut actions) = self.desktop.actions.lock() {
            actions.push(format!("key:{key}"));
        }
        Ok(())
    }

    async fn launch(&mut self, target: &str) -> Result<(), String> {
        if let Ok(mut actions) = self.desktop.actions.lock() {
            actions.push(format!("launch:{target}"));
        }
        Ok(())
    }
}

/// 用 Mock 桌面面替换 `AppState::new` 注册的 Computer Use 后端（同一注册表）。
fn install_mock_backend(state: &Arc<owo_agent_server::AppState>, desktop: MockDesktop) {
    let factory_desktop = desktop.clone();
    let backend = ComputerUseBackend::new(
        Arc::downgrade(&state.tasks),
        Arc::clone(&state.computer_tasks),
        state.agent.audit_log(),
    )
    .with_surface_factory(Arc::new(move || {
        let surface = MockSurface {
            desktop: factory_desktop.clone(),
        };
        Ok(Box::new(surface) as Box<dyn TaskSurface>)
    }));
    state.tasks.register_backend(Arc::new(backend));
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

async fn create_computer_task(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    payload: Value,
) -> String {
    let body = json!({
        "user_goal": "在记事本里输入问候并发送",
        "execution_strategy": "computer_use",
        "payload": payload,
    });
    let created = json_call(app, state, "POST", "/agent-tasks", Some(&body.to_string())).await;
    created["task"]["task_id"]
        .as_str()
        .unwrap_or_else(|| panic!("创建任务应返回 task_id：{created}"))
        .to_string()
}

async fn wait_until_settled(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    task_id: &str,
) -> String {
    let mut last = String::new();
    for _ in 0..100 {
        let detail = json_call(app, state, "GET", &format!("/agent-tasks/{task_id}"), None).await;
        last = detail["task"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if !matches!(last.as_str(), "running" | "verifying" | "cancel_requested") {
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
    let events = json_call(
        app,
        state,
        "GET",
        &format!("/agent-tasks/{task_id}/events?limit=200"),
        None,
    )
    .await;
    events["events"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|event| event["event_type"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_use_task_waits_for_approval_then_runs_closed_loop() {
    let (state, _temp) = test_state().await;
    let desktop = MockDesktop::new(
        "notepad",
        vec![
            ocr_line("请输入消息", 10, 10, "input"),
            ocr_line("发送", 10, 40, "button"),
        ],
    );
    install_mock_backend(&state, desktop.clone());
    let app = build_router(Arc::clone(&state));

    let task_id = create_computer_task(
        &app,
        &state,
        json!({
            "target_app": "notepad",
            "goals": [
                {"anchor_text": "请输入消息", "action": "type", "value": "你好", "verify_text": "你好"},
                {"anchor_text": "发送", "action": "click", "value": "", "verify_text": "已发送"}
            ]
        }),
    )
    .await;

    // 1) 第一次分发只创建待批准的 ComputerTask，不碰桌面。
    let started = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(started["dispatched"], false, "未批准前不得分发：{started}");
    assert_eq!(started["code"], "APPROVAL_REQUIRED");
    assert_eq!(started["task"]["state"], "needs_user_input");
    assert!(
        desktop.actions().is_empty(),
        "未批准前不得向桌面发送任何输入：{:?}",
        desktop.actions()
    );
    let types = event_types(&app, &state, &task_id).await;
    assert!(types.contains(&"task.user_input.required".to_string()));

    // 2) 审批走既有的 computer-use 接口（任务 id 与统一任务相同）。
    let approved = json_call(
        &app,
        &state,
        "POST",
        &format!("/computer-use/task/{task_id}/approve"),
        Some("{}"),
    )
    .await;
    assert_eq!(approved["state"], "Approved", "批准失败：{approved}");

    // 3) 再次 start 才真的执行闭环。
    let rerun = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(rerun["dispatched"], true, "批准后应分发：{rerun}");

    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert_eq!(
        final_state,
        "succeeded",
        "每步都有动作后验证时任务应通过独立验证：{}",
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
        desktop.actions(),
        vec!["type:你好".to_string(), "click:70,50".to_string()]
    );

    let types = event_types(&app, &state, &task_id).await;
    for expected in [
        "task.computer.started",
        "task.computer.completed",
        "task.backend.completed",
        "task.verification.completed",
        "task.succeeded",
    ] {
        assert!(
            types.contains(&expected.to_string()),
            "缺少事件 {expected}：{types:?}"
        );
    }

    // 4) 产物是每个动作的观察引用；ComputerTask 也应进入 Completed。
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
    assert_eq!(artifacts.len(), 2, "每个动作都应该有产物：{artifacts:?}");
    assert!(artifacts
        .iter()
        .all(|artifact| artifact["kind"] == "desktop_action"));
    let tasks = json_call(&app, &state, "GET", "/computer-use/tasks", None).await;
    let computer_task = tasks["tasks"]
        .as_array()
        .and_then(|all| all.iter().find(|task| task["id"] == task_id.as_str()))
        .cloned()
        .expect("computer-use 任务应出现在注册表里");
    assert_eq!(computer_task["state"], "Completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_use_task_without_goals_fails_without_touching_desktop() {
    let (state, _temp) = test_state().await;
    let desktop = MockDesktop::new("notepad", vec![ocr_line("确定", 10, 10, "button")]);
    install_mock_backend(&state, desktop.clone());
    let app = build_router(Arc::clone(&state));

    let task_id =
        create_computer_task(&app, &state, json!({"target_app": "notepad", "goals": []})).await;
    let started = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(started["dispatched"], false, "空目标不得分发：{started}");
    assert_eq!(started["code"], "EMPTY_GOALS");
    assert_eq!(started["task"]["state"], "failed");
    assert!(desktop.actions().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_notepad_save_as_must_end_the_action_plan() {
    let (state, _temp) = test_state().await;
    let desktop = MockDesktop::new("notepad", vec![ocr_line("anchor", 10, 10, "button")]);
    install_mock_backend(&state, desktop.clone());
    let app = build_router(Arc::clone(&state));
    let shortcut = json!({"anchor_text":"anchor", "action":"key", "value":"Shift+Control+S"});
    for (goals, expected) in [
        (
            json!([shortcut.clone(), {"anchor_text":"anchor", "action":"type", "value":"x"}]),
            "INVALID_REQUEST",
        ),
        (json!([shortcut]), "APPROVAL_REQUIRED"),
    ] {
        let task_id = create_computer_task(
            &app,
            &state,
            json!({
                "target_app": "notepad", "target_window_title": "test window", "goals": goals
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
        assert_eq!(started["code"], expected, "{started}");
        assert_eq!(started["dispatched"], false);
    }
    assert!(desktop.actions().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_use_sensitive_ui_fuses_and_requires_human_takeover() {
    let (state, _temp) = test_state().await;
    // 界面出现敏感元素：闭环应在触碰任何内容前熔断。
    let desktop = MockDesktop::new(
        "notepad",
        vec![
            ocr_line("请输入密码", 10, 10, "input"),
            ocr_line("确定", 10, 40, "button"),
        ],
    );
    install_mock_backend(&state, desktop.clone());
    let app = build_router(Arc::clone(&state));

    let task_id = create_computer_task(
        &app,
        &state,
        json!({
            "target_app": "notepad",
            "goals": [
                {"anchor_text": "确定", "action": "click", "value": "", "verify_text": "完成"}
            ]
        }),
    )
    .await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/computer-use/task/{task_id}/approve"),
        Some("{}"),
    )
    .await;
    let rerun = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(rerun["dispatched"], true, "批准后应分发：{rerun}");

    let final_state = wait_until_settled(&app, &state, &task_id).await;
    assert_eq!(
        final_state,
        "failed",
        "熔断后不得伪装成功：{}",
        json_call(
            &app,
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}"),
            None
        )
        .await
    );
    assert!(
        desktop.actions().is_empty(),
        "敏感界面下不得执行任何动作：{:?}",
        desktop.actions()
    );
    let events = json_call(
        &app,
        &state,
        "GET",
        &format!("/agent-tasks/{task_id}/events?limit=200"),
        None,
    )
    .await;
    let failed = events["events"]
        .as_array()
        .and_then(|all| {
            all.iter()
                .find(|event| event["event_type"] == "task.backend.failed")
        })
        .cloned()
        .expect("应记录后端失败事件");
    assert_eq!(failed["payload"]["code"], "HUMAN_TAKEOVER_REQUIRED");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_stops_the_loop_and_resume_dispatches_again() {
    let (state, _temp) = test_state().await;
    // 每次 OCR 250ms：暂停请求会落在两步之间，闭环必须在下一次门禁前停下。
    let desktop = MockDesktop::new(
        "notepad",
        vec![
            ocr_line("请输入消息", 10, 10, "input"),
            ocr_line("发送", 10, 40, "button"),
        ],
    )
    .with_ocr_delay(std::time::Duration::from_millis(250));
    install_mock_backend(&state, desktop.clone());
    let app = build_router(Arc::clone(&state));

    let task_id = create_computer_task(
        &app,
        &state,
        json!({
            "target_app": "notepad",
            "goals": [
                {"anchor_text": "请输入消息", "action": "type", "value": "你好", "verify_text": "你好"}
            ]
        }),
    )
    .await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/start"),
        Some("{}"),
    )
    .await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/computer-use/task/{task_id}/approve"),
        Some("{}"),
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
    assert_eq!(started["dispatched"], true, "批准后应分发：{started}");

    // 暂停：状态立刻是 paused，后端在下一个门禁点停下。
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let paused = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{task_id}/pause"),
        Some(r#"{"reason":"测试暂停"}"#),
    )
    .await;
    assert_eq!(paused["task"]["state"], "paused");
    let actions_when_paused = desktop.actions().len();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert_eq!(
        desktop.actions().len(),
        actions_when_paused,
        "暂停后闭环不得继续执行动作：{:?}",
        desktop.actions()
    );
    assert_eq!(
        json_call(
            &app,
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}"),
            None
        )
        .await["task"]["state"],
        "paused",
        "后端迟到的中止结果不能把暂停翻成失败"
    );

    // 恢复：必须重新分发并最终走完闭环。
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
        "succeeded",
        "恢复后应重新执行并完成：{}",
        json_call(
            &app,
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}"),
            None
        )
        .await
    );
    assert!(
        desktop.actions().len() > actions_when_paused,
        "恢复后应真的重新执行：{:?}",
        desktop.actions()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_does_not_replay_a_verified_text_side_effect() {
    let (state, _temp) = test_state().await;
    let desktop = MockDesktop::new("notepad", vec![ocr_line("anchor", 10, 10, "input")]);
    let shared = desktop.clone();
    let creations = std::sync::atomic::AtomicUsize::new(0);
    let backend = ComputerUseBackend::new(
        Arc::downgrade(&state.tasks),
        Arc::clone(&state.computer_tasks),
        state.agent.audit_log(),
    )
    .with_surface_factory(Arc::new(move || {
        let inner = MockSurface {
            desktop: shared.clone(),
        };
        if creations.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            Ok(Box::new(SwitchDuringOcrSurface {
                inner,
                switched: false,
                title_only: false,
                observations: 0,
                switch_after_observations: 2,
            }) as Box<dyn TaskSurface>)
        } else {
            Ok(Box::new(inner) as Box<dyn TaskSurface>)
        }
    }));
    state.tasks.register_backend(Arc::new(backend));
    let app = build_router(Arc::clone(&state));
    let id = create_computer_task(&app, &state, json!({"target_app":"notepad", "goals":[
        {"anchor_text":"anchor","action":"type","value":"WRITE_ONCE","verify_text":"WRITE_ONCE"},
        {"anchor_text":"anchor","action":"type","value":"REMAINING","verify_text":"REMAINING"}
    ]})).await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/start"),
        Some("{}"),
    )
    .await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/computer-use/task/{id}/approve"),
        Some("{}"),
    )
    .await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/start"),
        Some("{}"),
    )
    .await;
    assert_eq!(
        wait_until_settled(&app, &state, &id).await,
        "needs_user_input"
    );
    assert_eq!(desktop.actions(), vec!["type:WRITE_ONCE"]);
    let checkpoint = state
        .tasks
        .latest_checkpoint(&id)
        .unwrap()
        .expect("confirmed side effect must have a durable checkpoint");
    assert_eq!(checkpoint.metadata["confirmed_goal_indices"], json!([0]));

    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/resume"),
        Some("{}"),
    )
    .await;
    assert_eq!(wait_until_settled(&app, &state, &id).await, "succeeded");
    assert_eq!(
        desktop.actions(),
        vec!["type:WRITE_ONCE", "type:REMAINING"],
        "confirmed text must not be injected twice after resume"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orphan_intent_and_changed_plan_stop_without_input() {
    let (state, _temp) = test_state().await;
    let desktop = MockDesktop::new("notepad", vec![ocr_line("anchor", 10, 10, "input")])
        .with_ocr_delay(std::time::Duration::from_millis(750));
    install_mock_backend(&state, desktop.clone());
    let app = build_router(Arc::clone(&state));
    let id = create_computer_task(&app, &state, json!({"target_app":"notepad", "goals":[
        {"anchor_text":"anchor","action":"type","value":"NEVER_REPLAY","verify_text":"NEVER_REPLAY"}
    ]})).await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/start"),
        Some("{}"),
    )
    .await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/computer-use/task/{id}/approve"),
        Some("{}"),
    )
    .await;
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/start"),
        Some("{}"),
    )
    .await;
    let mut plan_ref = None;
    for _ in 0..50 {
        let events = state.tasks.events(&id, 0, 100).unwrap();
        plan_ref = events
            .iter()
            .find(|e| e.event_type == "task.computer.started")
            .and_then(|e| e.payload["plan_ref"].as_str())
            .map(str::to_string);
        if plan_ref.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let plan_ref = plan_ref.expect("started event contains the actual immutable plan fingerprint");
    let action_id = format!("{id}:1:{}", plan_ref.trim_start_matches("sha256:"));
    let intent =
        json!({"plan_ref":plan_ref,"goal_index":0,"action_id":action_id,"journal_version":1});
    // Inject the exact durable cut point: claim committed, readable intent event absent.
    // This also races the running adapter after its initial load, testing atomic exclusion.
    state
        .tasks
        .store()
        .put_idempotency(&format!("computer-action:{action_id}"), &id, &intent)
        .unwrap();
    assert_eq!(
        wait_until_settled(&app, &state, &id).await,
        "needs_user_input"
    );
    assert!(
        desktop.actions().is_empty(),
        "a failed unique intent claim must prevent input"
    );
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/resume"),
        Some("{}"),
    )
    .await;
    assert_eq!(
        wait_until_settled(&app, &state, &id).await,
        "needs_user_input"
    );
    assert!(
        desktop.actions().is_empty(),
        "orphan durable intent cannot be replayed after resume"
    );
    state
        .tasks
        .store()
        .append_event(
            &id,
            "task.computer.action.intended",
            3,
            json!({"plan_ref":"different-plan","goal_index":0,"action_id":action_id}),
            None,
        )
        .unwrap();
    let _ = json_call(
        &app,
        &state,
        "POST",
        &format!("/agent-tasks/{id}/resume"),
        Some("{}"),
    )
    .await;
    assert_eq!(
        wait_until_settled(&app, &state, &id).await,
        "needs_user_input"
    );
    assert!(desktop.actions().is_empty());
    let events = state.tasks.events(&id, 0, 200).unwrap();
    assert!(events
        .iter()
        .any(|e| e.event_type == "task.user_input.required"
            && e.payload["code"] == "ACTION_RECOVERY_REQUIRED"));
    assert!(events
        .iter()
        .any(|e| e.payload.to_string().contains("计划指纹")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_tool_budget_counts_input_and_cannot_be_disabled_by_surface_budget() {
    let (state, _temp) = test_state().await;
    let desktop = MockDesktop::new("notepad", vec![ocr_line("anchor", 10, 10, "input")]);
    install_mock_backend(&state, desktop.clone());
    let app = build_router(Arc::clone(&state));
    for (values, expected_state) in [
        (vec!["AT_LIMIT"], "succeeded"),
        (vec!["FIRST_ONLY", "DENIED"], "failed"),
    ] {
        let goals: Vec<_> = values.iter().map(|value| json!({"anchor_text":"anchor","action":"type","value":value,"verify_text":value})).collect();
        let budget = owo_agent_core::task_runtime::TaskBudget {
            max_tool_calls: 1,
            ..Default::default()
        };
        let body = json!({"user_goal":"bounded native input fixture","execution_strategy":"computer_use", "budget":budget,
            "payload":{"target_app":"notepad","action_budget":0,"goals":goals}});
        let created = json_call(
            &app,
            &state,
            "POST",
            "/agent-tasks",
            Some(&body.to_string()),
        )
        .await;
        let id = created["task"]["task_id"].as_str().unwrap().to_string();
        let before = desktop.actions().len();
        let _ = json_call(
            &app,
            &state,
            "POST",
            &format!("/agent-tasks/{id}/start"),
            Some("{}"),
        )
        .await;
        let _ = json_call(
            &app,
            &state,
            "POST",
            &format!("/computer-use/task/{id}/approve"),
            Some("{}"),
        )
        .await;
        let _ = json_call(
            &app,
            &state,
            "POST",
            &format!("/agent-tasks/{id}/start"),
            Some("{}"),
        )
        .await;
        assert_eq!(wait_until_settled(&app, &state, &id).await, expected_state);
        assert_eq!(
            &desktop.actions()[before..],
            &[format!("type:{}", values[0])]
        );
        if values.len() > 1 {
            assert!(state
                .tasks
                .events(&id, 0, 200)
                .unwrap()
                .iter()
                .any(|e| e.event_type == "task.backend.failed"
                    && e.payload["code"] == "BUDGET_EXCEEDED"));
        }
    }
}
