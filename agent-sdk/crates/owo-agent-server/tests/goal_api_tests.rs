//! Lane D Part 1 契约测试：Goal/Plan 编排 HTTP API。
//!
//! 独立编译目标：`goal_api.rs` 不引用 crate::/super::，本文件用 #[path] 挂载。
//! 存储全部落在 tempfile 临时目录。

#[path = "../src/goal_api/mod.rs"]
mod goal_api;

use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::{fleet_hub, fleet_router_with_hub, AppState};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

/// 串行化修改 OPENAI_API_KEY 的测试（并行下 remove_var/set_var 会互相污染）。
static ENV_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

/// 无外部依赖的最小模型 Provider（任何模型调用即失败）。
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

async fn test_state() -> (Arc<AppState>, tempfile::TempDir) {
    test_state_with_provider(Arc::new(IdleProvider)).await
}

async fn test_state_with_provider(
    provider: Arc<dyn owo_agent_core::ModelProvider>,
) -> (Arc<AppState>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        provider,
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    (state, temp)
}

fn request(method: &str, path: &str, body: Option<&str>) -> axum::http::Request<axum::body::Body> {
    use axum::http::{header, Method, Request};
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path);
    if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

async fn call(app: &axum::Router, method: &str, path: &str, body: Option<&str>) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(request(method, path, body))
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// 创建 goal + 三并行+join 计划，返回 (app, goal_id)。
async fn setup_goal_with_plan(state: Arc<AppState>) -> (axum::Router, String) {
    let app = goal_api::router(state);
    let (status, created) = call(
        &app,
        "POST",
        "/goal",
        Some(r#"{"objective":"编排验收目标"}"#),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let goal_id = created["goal"]["id"].as_str().unwrap().to_string();
    (app, goal_id)
}

/// 轮询运行状态直到非 Running/Pending 或超时（404=尚未落盘，继续轮询）。
async fn wait_terminal(app: &axum::Router, goal_id: &str, timeout_ms: u64) -> (u16, Value) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        let (status, value) = call(app, "GET", &format!("/goal/{goal_id}/status"), None).await;
        if status != 404 {
            let goal_status = value
                .get("goal_status")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !matches!(
                goal_status,
                "Running" | "Pending" | "Planning" | "Verifying"
            ) {
                return (status, value);
            }
        }
        if std::time::Instant::now() > deadline {
            panic!("运行状态轮询超时：最后响应 {status} {value}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn create_goal_and_list() {
    let (state, _temp) = test_state().await;
    let app = goal_api::router(state);
    let (status, created) =
        call(&app, "POST", "/goal", Some(r#"{"objective":"创建与列表"}"#)).await;
    assert_eq!(status, 201);
    let goal_id = created["goal"]["id"].as_str().unwrap().to_string();
    let (status, list) = call(&app, "GET", "/goal", None).await;
    assert_eq!(status, 200);
    assert_eq!(list["count"].as_u64().unwrap(), 1);
    assert_eq!(list["goals"][0]["id"].as_str().unwrap(), goal_id);
}

#[tokio::test]
async fn create_goal_empty_objective_rejected() {
    let (state, _temp) = test_state().await;
    let app = goal_api::router(state);
    let (status, value) = call(&app, "POST", "/goal", Some(r#"{"objective":"  "}"#)).await;
    assert_eq!(status, 400);
    assert!(value["error"].as_str().unwrap().contains("objective"));
}

#[tokio::test]
async fn get_goal_not_found() {
    let (state, _temp) = test_state().await;
    let app = goal_api::router(state);
    let (status, value) = call(&app, "GET", "/goal/missing-goal", None).await;
    assert_eq!(status, 404);
    assert!(value["error"].as_str().unwrap().contains("不存在"));
}

#[tokio::test]
async fn plan_cycle_rejected_with_400() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state).await;
    let body = r#"{"steps":[
        {"id":"a","worker":"echo","deps":["b"]},
        {"id":"b","worker":"echo","deps":["a"]}
    ]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 400);
    assert!(value["error"].as_str().unwrap().contains("环"), "{value}");
}

#[tokio::test]
async fn plan_waves_preview_for_parallel_join() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state).await;
    let body = r#"{"steps":[
        {"id":"a","worker":"echo","input":{"text":"A"}},
        {"id":"b","worker":"echo","input":{"text":"B"}},
        {"id":"c","worker":"echo","input":{"text":"C"}},
        {"id":"join","worker":"echo","deps":["a","b","c"],"input":{"text":"ABC"},"verify":"ABC"}
    ]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "{value}");
    let waves = value["waves"].as_array().unwrap();
    assert_eq!(waves.len(), 2);
    assert_eq!(waves[0].as_array().unwrap().len(), 3, "前三步应同层并行");
    assert_eq!(waves[1].as_array().unwrap().len(), 1);
    assert_eq!(waves[1][0], "join");
    assert_eq!(value["valid"], true);
}

#[tokio::test]
async fn run_echo_sleep_plan_to_succeeded() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state).await;
    let body = r#"{"steps":[
        {"id":"a","worker":"echo","input":{"text":"A"}},
        {"id":"b","worker":"echo","input":{"text":"B"}},
        {"id":"join","worker":"sleep","deps":["a","b"],"input":{"ms":20},"verify":"slept"}
    ]}"#;
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201);
    let (status, run) = call(&app, "POST", &format!("/goal/{goal_id}/run"), Some("{}")).await;
    assert_eq!(status, 202, "{run}");
    let run_id = run["run_id"].as_str().unwrap().to_string();
    assert!(run_id.starts_with("run-"));

    let (status, value) = wait_terminal(&app, &goal_id, 10_000).await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(value["goal_status"], "Succeeded", "{value}");
    let records = value["records"].as_object().unwrap();
    for (step_id, record) in records {
        let step_status = record["status"].as_str().unwrap();
        assert_eq!(step_status, "Succeeded", "步骤 {step_id} 应成功：{record}");
    }
    assert!(value["steps_taken"].as_u64().unwrap() >= 3);
    // runs 列表一致。
    let (_, runs) = call(&app, "GET", &format!("/goal/{goal_id}/runs"), None).await;
    assert_eq!(runs["count"].as_u64().unwrap(), 1);
    assert_eq!(runs["runs"][0]["run_id"].as_str().unwrap(), run_id);
}

#[tokio::test]
async fn fail_step_triggers_replan_and_fails_goal() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state).await;
    // join 验证恒失败 → 每次 replan 后仍失败，replan 次数用尽 → Failed。
    let body = r#"{"steps":[
        {"id":"a","worker":"echo","input":{"text":"A"}},
        {"id":"b","worker":"echo","input":{"text":"B"}},
        {"id":"join","worker":"echo","deps":["a","b"],"input":{"text":"X"},"verify":"NEVER-MATCH"}
    ]}"#;
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201);
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/run"), Some("{}")).await;
    assert_eq!(status, 202);
    let (status, value) = wait_terminal(&app, &goal_id, 10_000).await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(value["goal_status"], "Failed", "{value}");
    assert!(
        value["replan_count"].as_u64().unwrap() >= 1,
        "验证失败应触发 replan：{value}"
    );
    // 审计含 replan 记录。
    let (_, audit) = call(&app, "GET", &format!("/goal/{goal_id}/audit"), None).await;
    let text = serde_json::to_string(&audit).unwrap();
    assert!(text.contains("replan"), "审计应含 replan：{text}");
}

#[tokio::test]
async fn abort_marks_goal_aborted() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state).await;
    let body = r#"{"steps":[
        {"id":"slow","worker":"sleep","input":{"ms":5000}}
    ]}"#;
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201);
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/run"), Some("{}")).await;
    assert_eq!(status, 202);
    // 立即 abort。
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/abort"), Some("{}")).await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(value["aborted"].as_u64().unwrap(), 1);
    let (status, value) = wait_terminal(&app, &goal_id, 5_000).await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(value["goal_status"], "Aborted", "{value}");
}

#[tokio::test]
async fn run_persists_recovery_state_consistent() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state).await;
    let body = r#"{"steps":[
        {"id":"a","worker":"echo","input":{"text":"A"}},
        {"id":"b","worker":"echo","deps":["a"],"input":{"text":"B"},"verify":"B"}
    ]}"#;
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201);
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/run"), Some("{}")).await;
    assert_eq!(status, 202);
    let (status, first) = wait_terminal(&app, &goal_id, 10_000).await;
    assert_eq!(first["goal_status"], "Succeeded");
    // 二次读取状态一致（幂等）。
    let (_, second) = call(&app, "GET", &format!("/goal/{goal_id}/status"), None).await;
    assert_eq!(first["goal_status"], second["goal_status"]);
    assert_eq!(first["records"], second["records"]);
    assert_eq!(status, 200);
}

#[tokio::test]
async fn unknown_goal_404_on_all_subresources() {
    let (state, _temp) = test_state().await;
    let app = goal_api::router(state);
    for (method, path, body) in [
        (
            "POST",
            "/goal/none/plan",
            Some(r#"{"steps":[{"id":"a","worker":"echo"}]}"#),
        ),
        ("POST", "/goal/none/run", Some("{}")),
        ("GET", "/goal/none/status", None),
        ("POST", "/goal/none/abort", Some("{}")),
        ("GET", "/goal/none/audit", None),
        ("GET", "/goal/none/runs", None),
    ] {
        let (status, value) = call(&app, method, path, body).await;
        assert_eq!(status, 404, "{method} {path} 应 404：{value}");
    }
}

#[tokio::test]
async fn audit_tail_records_write_operations() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state).await;
    let (_, audit) = call(&app, "GET", &format!("/goal/{goal_id}/audit"), None).await;
    let entries = audit["audit"].as_array().unwrap();
    assert!(!entries.is_empty());
    let text = serde_json::to_string(&audit).unwrap();
    assert!(text.contains("goal.create"), "审计应含创建记录");
}

#[tokio::test]
async fn plan_missing_404() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state).await;
    let (status, value) = call(&app, "GET", &format!("/goal/{goal_id}/plan"), None).await;
    assert_eq!(status, 404, "{value}");
}

// ==================== R5：agent worker / status 增强 ====================

/// 创建 goal + 计划（steps 由调用方提供），返回 (app, goal_id)。
async fn setup_goal_with_steps(state: Arc<AppState>, steps: &str) -> (axum::Router, String) {
    let (app, goal_id) = setup_goal_with_plan(state).await;
    let body = format!(r#"{{"steps":{steps}}}"#);
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(&body)).await;
    assert_eq!(status, 201, "计划创建应成功：{value}");
    (app, goal_id)
}

fn agent_step_json(prompt: &str, read_only: bool) -> String {
    format!(
        r#"[{{"id":"a1","worker":"agent","input":{{"prompt":"{}","read_only":{}}},"verify":{{"kind":"nonempty"}}}}]"#,
        prompt, read_only
    )
}

#[tokio::test]
async fn r5_agent_worker_without_key_fails_readably() {
    // 保证无凭据：保存并移除 OPENAI_API_KEY
    let previous = std::env::var("OPENAI_API_KEY").ok();
    let _guard = ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    std::env::remove_var("OPENAI_API_KEY");
    // 用生产同款 DeferredProvider：无凭据时调用点返回可读的 not_configured 指引
    // （IdleProvider 会在凭据校验之前被调用，无法覆盖本契约）。
    let (state, _temp) =
        test_state_with_provider(Arc::new(owo_agent_core::gateway::DeferredProvider::new())).await;
    let steps = agent_step_json("做一个总结", true);
    let (app, goal_id) = setup_goal_with_steps(state.clone(), &steps).await;
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(r#"{"config":{}}"#),
    )
    .await;
    assert_eq!(status, 202, "{value}");
    let (_status, terminal) = wait_terminal(&app, &goal_id, 8000).await;
    let steps_value = terminal["steps"].as_array().unwrap();
    let a1 = steps_value.iter().find(|s| s["step_id"] == "a1").unwrap();
    let error = a1["error"].as_str().unwrap_or("");
    assert!(
        error.contains("OPENAI_API_KEY"),
        "无凭据时错误应可读且含 KEY 提示：{a1}"
    );
    // 恢复环境
    if let Some(key) = previous {
        std::env::set_var("OPENAI_API_KEY", key);
    }
}

#[tokio::test]
async fn r5_agent_step_missing_prompt_rejected_400() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[{"id":"a1","worker":"agent","input":{"read_only":true}}]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 400, "agent 步骤缺 prompt 应 400：{value}");
    assert!(value["error"].as_str().unwrap_or("").contains("prompt"));
}

#[tokio::test]
async fn r5_echo_sleep_fail_regression() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_steps(
        state.clone(),
        r#"[{"id":"e1","worker":"echo","input":{"text":"hello"}},
            {"id":"s1","worker":"sleep","input":{"ms":20},"deps":["e1"]},
            {"id":"f1","worker":"fail","input":{"text":"boom"},"deps":["s1"]}]"#,
    )
    .await;
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(r#"{"config":{}}"#),
    )
    .await;
    assert_eq!(status, 202, "{value}");
    let (_status, terminal) = wait_terminal(&app, &goal_id, 8000).await;
    assert_eq!(terminal["goal_status"], "Failed");
    let steps = terminal["steps"].as_array().unwrap();
    let e1 = steps.iter().find(|s| s["step_id"] == "e1").unwrap();
    assert_eq!(e1["output"], "hello", "{e1}");
    let f1 = steps.iter().find(|s| s["step_id"] == "f1").unwrap();
    assert!(f1["error"].as_str().unwrap_or("").contains("boom"));
}

#[tokio::test]
async fn r5_status_includes_worker_name_and_truncated_output() {
    let (state, _temp) = test_state().await;
    let long_text = "长文本".repeat(1000); // 3000 字符
    let steps = format!(
        r#"[{{"id":"e1","worker":"echo","input":{{"text":"{}"}}}}]"#,
        long_text
    );
    let (app, goal_id) = setup_goal_with_steps(state.clone(), &steps).await;
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(r#"{"config":{}}"#),
    )
    .await;
    assert_eq!(status, 202, "{value}");
    let (_status, terminal) = wait_terminal(&app, &goal_id, 8000).await;
    let steps_value = terminal["steps"].as_array().unwrap();
    let e1 = steps_value.iter().find(|s| s["step_id"] == "e1").unwrap();
    assert_eq!(e1["worker"], "echo", "status 应含 worker 名：{e1}");
    assert_eq!(e1["output_truncated"], true, "超过 2000 字符应截断");
    let output = e1["output"].as_str().unwrap();
    assert!(output.chars().count() <= 2000, "截断后不超过 2000");
}

#[tokio::test]
async fn r5_goal_plan_requires_agent_worker_validation_skips_other_workers() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    // 非 agent worker 无需 prompt 校验
    let body = r#"{"steps":[{"id":"e1","worker":"echo","input":{}}]}"#;
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "echo 步骤无需 prompt");
}

#[tokio::test]
async fn r5_agent_worker_online_skipped_without_key() {
    // 真实联网用例：仅当 OPENAI_API_KEY 且 OWO_AGENT_LIVE_TEST=1 才执行。
    // （scoped 门禁环境用 IdleProvider，真实模型调用必然失败；联调时显式开启双开关。）
    let has_key = std::env::var("OPENAI_API_KEY")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let live = std::env::var("OWO_AGENT_LIVE_TEST")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !(has_key && live) {
        return;
    }
    // 与 remove_var KEY 的测试串行（共享进程级环境变量）。
    let _guard = ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    // 与生产 Worker 使用同一模型解析路径；default 由 Provider 解析当前端点模型。
    let model = goal_api::agent_worker::AgentWorker::resolve_model(&serde_json::json!({}));
    let (state, _temp) = test_state().await;
    let steps = format!(
        r#"[{{"id":"a1","worker":"agent","input":{{"prompt":"回复 ok 即可","read_only":true,"model":"{model}"}}}}]"#
    );
    let (app, goal_id) = setup_goal_with_steps(state.clone(), &steps).await;
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(r#"{"config":{}}"#),
    )
    .await;
    assert_eq!(status, 202, "{value}");
    let (_status, terminal) = wait_terminal(&app, &goal_id, 60000).await;
    assert_eq!(terminal["goal_status"], "Succeeded");
}

#[tokio::test]
async fn r5_agent_worker_resolve_model_default() {
    // 无任务级模型时保留 Provider 解析权，避免覆盖运行时端点模型。
    let input = serde_json::json!({ "prompt": "x" });
    let ok = goal_api::agent_worker::validate_agent_input(&input).is_ok();
    assert!(ok, "含 prompt 的 agent input 应通过预校验");
    let configured_model = std::env::var("OWO_AGENT_MODEL").ok();
    assert_eq!(
        goal_api::agent_worker::AgentWorker::resolve_model(&input),
        owo_agent_core::gateway::resolve_subagent_model(None, configured_model.as_deref())
    );
    let input_model = serde_json::json!({ "prompt": "x", "model": "my-model" });
    assert_eq!(
        goal_api::agent_worker::AgentWorker::resolve_model(&input_model),
        "my-model"
    );
}

#[tokio::test]
async fn r5_status_steps_shape_complete() {
    // status.steps 每个条目应含 worker/status/attempts/output/output_truncated/error 字段。
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_steps(
        state.clone(),
        r#"[{"id":"e1","worker":"echo","input":{"text":"shape"}}]"#,
    )
    .await;
    let (status, _) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(r#"{"config":{}}"#),
    )
    .await;
    assert_eq!(status, 202);
    let (_status, terminal) = wait_terminal(&app, &goal_id, 8000).await;
    let steps = terminal["steps"].as_array().unwrap();
    assert!(!steps.is_empty());
    let s = &steps[0];
    assert_eq!(s["worker"], "echo");
    assert!(s["status"].is_string());
    assert!(s["attempts"].is_number());
    assert!(s["output"].is_string());
    assert!(s["output_truncated"].is_boolean());
    assert!(
        s["error"].is_null() || s["error"].is_string(),
        "error 字段应存在：{s}"
    );
}

// ==================== P1：WorkerPool 运行模式（子进程执行） ====================

/// 子进程协议入口：父进程用 `--exact owo_worker_child_entry --nocapture --quiet`
/// 配合 `OWO_WORKER_CHILD=1` 拉起本测试二进制作为受控 worker。
/// 行为：`{"ms":N}` → 睡 N 毫秒返回 "slept Nms"；`{"sleep_ms":N}` → 睡 N 毫秒；
/// `{"crash":true}` → 直接退出（崩溃）；其余回显 "out-{text}"。
#[test]
fn owo_worker_child_entry() {
    if std::env::var("OWO_WORKER_CHILD").is_err() {
        return; // 父进程测试模式下直接返回
    }
    // 写 pid 文件（测试用：验证子进程被终止/无孤儿）。
    if let Ok(path) = std::env::var("OWO_WORKER_PID_FILE") {
        let _ = std::fs::write(&path, std::process::id().to_string());
    }
    owo_agent_core::worker_pool::child::run_child_protocol(|input: &Value| {
        if input.get("crash").and_then(Value::as_bool).unwrap_or(false) {
            std::process::exit(42); // 任务中崩溃（自愈/熔断演示）
        }
        if let Some(ms) = input.get("ms").and_then(Value::as_u64) {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            return Ok(format!("slept {ms}ms"));
        }
        if let Some(ms) = input.get("sleep_ms").and_then(Value::as_u64) {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        let text = input
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(format!("out-{text}"))
    });
}

/// 受控 worker 配置 JSON（命令 = 当前测试二进制，符合"命令仅限当前可执行文件"约束）。
fn pool_worker_json(name: &str, env_extra: &[(&str, &str)]) -> serde_json::Value {
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let mut env = serde_json::Map::new();
    env.insert(
        "OWO_WORKER_CHILD".to_string(),
        serde_json::Value::String("1".to_string()),
    );
    for (k, v) in env_extra {
        env.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    serde_json::json!({
        "name": name,
        "command": exe,
        "cwd": cwd,
        "args": ["--exact", "owo_worker_child_entry", "--nocapture", "--quiet"],
        "env": env,
    })
}

/// 等待子进程 pid 文件出现（spawn 后由子进程写入）。
async fn wait_pid_file(path: &std::path::Path, timeout_ms: u64) -> u32 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(pid) = raw.trim().parse::<u32>() {
                return pid;
            }
        }
        if std::time::Instant::now() > deadline {
            panic!("子进程 pid 文件未在 {timeout_ms}ms 内出现：{path:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// 用 tasklist 检查进程是否存活（Windows）。
async fn pid_alive(pid: u32) -> bool {
    let filter = format!("PID eq {pid}");
    let out = tokio::process::Command::new("tasklist")
        .args(["/FI", &filter])
        .output()
        .await;
    match out {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            text.contains(&pid.to_string())
        }
        Err(_) => false,
    }
}

/// P1 契约：worker_pool 模式下 echo/sleep 步骤经子进程执行，任务成功且审计含生命周期事件。
/// 受控子进程 worker 会与 HTTP handler 在同一 runtime 交错；生产 axum 是多线程
/// runtime，单线程测试 runtime 会被阻塞路径饿死（实测挂起），这里与生产对齐。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p1_worker_pool_mode_runs_steps_in_subprocess() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[
        {"id":"e1","worker":"echo","input":{"text":"A"}},
        {"id":"s1","worker":"sleep","deps":["e1"],"input":{"ms":20},"verify":"slept"}
    ]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "{value}");
    let run_body = serde_json::json!({
        "execution": {
            "mode": "worker_pool",
            "workers": [pool_worker_json("echo", &[]), pool_worker_json("sleep", &[])],
        }
    });
    let (status, run) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&run_body.to_string()),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let (status, terminal) = wait_terminal(&app, &goal_id, 15_000).await;
    assert_eq!(status, 200, "{terminal}");
    assert_eq!(terminal["goal_status"], "Succeeded", "{terminal}");
    // 子进程输出（协议 out-{text}）与进程内 echo（原文本 A）不同，证明走了子进程。
    let steps = terminal["steps"].as_array().unwrap();
    let e1 = steps.iter().find(|s| s["step_id"] == "e1").unwrap();
    assert_eq!(e1["output"], "out-A", "echo 步骤应经子进程执行：{e1}");
    // 审计链路含 worker 生命周期事件（started/stopped 同源写入）。
    // worker.stopped 由子进程回收路径异步写入，终态后可能略晚于 Succeeded；
    // 单次读取会在并行测试负载下抖动，这里在有界时间内轮询。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut text;
    loop {
        let (_, audit) = call(&app, "GET", &format!("/goal/{goal_id}/audit"), None).await;
        text = serde_json::to_string(&audit).unwrap();
        if text.contains("worker.started") && text.contains("worker.stopped") {
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        text.contains("worker.started"),
        "审计应含 worker 启动事件：{text}"
    );
    assert!(
        text.contains("worker.stopped"),
        "审计应含 worker 停止事件：{text}"
    );
}

/// P1 契约：预算超时中止子进程并 kill，任务落到已定义 Failed 态（不出现 Unknown）。
#[tokio::test]
async fn p1_worker_pool_budget_timeout_terminates_subprocess() {
    let (state, temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[
        {"id":"slow","worker":"slow","input":{"text":"x","sleep_ms":20000}}
    ]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "{value}");
    let pid_file = temp.path().join("slow.pid");
    let mut worker = pool_worker_json(
        "slow",
        &[("OWO_WORKER_PID_FILE", pid_file.to_str().unwrap())],
    );
    worker["budget"] = serde_json::json!({ "max_duration_secs": 1 });
    let run_body = serde_json::json!({
        "execution": { "mode": "worker_pool", "workers": [worker] },
        "allow_replan": false,
    });
    let (status, run) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&run_body.to_string()),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let pid = wait_pid_file(&pid_file, 8_000).await;
    let (status, terminal) = wait_terminal(&app, &goal_id, 15_000).await;
    assert_eq!(status, 200, "{terminal}");
    assert_eq!(
        terminal["goal_status"], "Failed",
        "预算超时应落到已定义 Failed 态：{terminal}"
    );
    let steps = terminal["steps"].as_array().unwrap();
    let slow = steps.iter().find(|s| s["step_id"] == "slow").unwrap();
    assert_eq!(slow["status"], "Failed", "{slow}");
    let error = slow["error"].as_str().unwrap_or("");
    assert!(error.contains("预算"), "步骤错误应含预算中止信息：{slow}");
    // 子进程必须已被 kill（无孤儿）。
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!pid_alive(pid).await, "预算中止必须终止子进程（pid={pid}）");
    // 审计链路：worker.budget_aborted 事件。
    let (_, audit) = call(&app, "GET", &format!("/goal/{goal_id}/audit"), None).await;
    let text = serde_json::to_string(&audit).unwrap();
    assert!(
        text.contains("worker.budget_aborted"),
        "审计应含预算中止事件：{text}"
    );
}

/// P1 契约：abort 取消传播到子进程，任务落到已定义 Aborted 态且子进程被回收（无孤儿）。
#[tokio::test]
async fn p1_worker_pool_cancel_terminates_subprocess() {
    let (state, temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[
        {"id":"slow","worker":"slow","input":{"text":"x","sleep_ms":20000}}
    ]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "{value}");
    let pid_file = temp.path().join("slow.pid");
    let worker = pool_worker_json(
        "slow",
        &[("OWO_WORKER_PID_FILE", pid_file.to_str().unwrap())],
    );
    let run_body = serde_json::json!({
        "execution": { "mode": "worker_pool", "workers": [worker] },
    });
    let (status, run) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&run_body.to_string()),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let pid = wait_pid_file(&pid_file, 8_000).await;
    // 任务已提交（子进程阻塞 sleep 中）→ abort → 取消传播到子进程。
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/abort"), Some("{}")).await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(value["aborted"].as_u64().unwrap(), 1);
    let (status, terminal) = wait_terminal(&app, &goal_id, 10_000).await;
    assert_eq!(status, 200, "{terminal}");
    assert_eq!(
        terminal["goal_status"], "Aborted",
        "取消应落到已定义 Aborted 态：{terminal}"
    );
    let steps = terminal["steps"].as_array().unwrap();
    let slow = steps.iter().find(|s| s["step_id"] == "slow").unwrap();
    assert_eq!(slow["status"], "Aborted", "{slow}");
    // 运行结束 shutdown 必须回收子进程（无孤儿）。
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(!pid_alive(pid).await, "取消后子进程必须被终止（pid={pid}）");
    // 审计链路：取消/停止事件。
    let (_, audit) = call(&app, "GET", &format!("/goal/{goal_id}/audit"), None).await;
    let text = serde_json::to_string(&audit).unwrap();
    assert!(
        text.contains("worker.cancelled") || text.contains("worker.stopped"),
        "审计应含取消/停止事件：{text}"
    );
}

/// P1 契约：显式 process 模式保持进程内语义（echo 输出原文本，非子进程 out-A）。
#[tokio::test]
async fn p1_execution_process_mode_keeps_in_process_semantics() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[
        {"id":"e1","worker":"echo","input":{"text":"A"}},
        {"id":"s1","worker":"sleep","deps":["e1"],"input":{"ms":20},"verify":"slept"}
    ]}"#;
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201);
    // 显式 process 模式：进程内语义（echo 输出 = 原文本 A，非子进程 out-A）。
    let run_body = r#"{"execution":{"mode":"process"}}"#;
    let (status, run) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(run_body),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let (status, terminal) = wait_terminal(&app, &goal_id, 10_000).await;
    assert_eq!(status, 200, "{terminal}");
    assert_eq!(terminal["goal_status"], "Succeeded", "{terminal}");
    let steps = terminal["steps"].as_array().unwrap();
    let e1 = steps.iter().find(|s| s["step_id"] == "e1").unwrap();
    assert_eq!(
        e1["output"], "A",
        "process 模式 echo 应进程内输出原文本：{e1}"
    );
}

/// P1 契约：worker_pool 非法输入必须返回明确 400（不默认开启、不静默降级）。
#[tokio::test]
async fn p1_execution_invalid_inputs_400() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[{"id":"e1","worker":"echo","input":{"text":"A"}}]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "{value}");
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let ok_worker = serde_json::json!({
        "name": "echo",
        "command": exe,
        "cwd": cwd,
        "args": ["--exact", "owo_worker_child_entry", "--nocapture", "--quiet"],
        "env": { "OWO_WORKER_CHILD": "1" },
    });
    // 1) worker_pool 缺 workers → 400
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(r#"{"execution":{"mode":"worker_pool"}}"#),
    )
    .await;
    assert_eq!(status, 400, "{value}");
    assert!(value["error"].as_str().unwrap_or("").contains("workers"));
    // 2) 命令非当前可执行文件 → 400
    let mut bad_cmd = ok_worker.clone();
    bad_cmd["command"] = serde_json::json!("C:/Windows/System32/cmd.exe");
    let body = serde_json::json!({ "execution": { "mode": "worker_pool", "workers": [bad_cmd] } });
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(status, 400, "{value}");
    assert!(value["error"]
        .as_str()
        .unwrap_or("")
        .contains("当前服务可执行文件"));
    // 3) env 白名单含凭据类键（OPENAI_API_KEY）→ 400
    let mut bad_env = ok_worker.clone();
    bad_env["env"] = serde_json::json!({ "OWO_WORKER_CHILD": "1", "OPENAI_API_KEY": "sk-test" });
    let body = serde_json::json!({ "execution": { "mode": "worker_pool", "workers": [bad_env] } });
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(status, 400, "{value}");
    assert!(value["error"]
        .as_str()
        .unwrap_or("")
        .contains("OPENAI_API_KEY"));
    // 4) cwd 不存在 → 400
    let mut bad_cwd = ok_worker.clone();
    bad_cwd["cwd"] = serde_json::json!("T:/nonexistent-dir-owo-p1");
    let body = serde_json::json!({ "execution": { "mode": "worker_pool", "workers": [bad_cwd] } });
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(status, 400, "{value}");
    assert!(value["error"].as_str().unwrap_or("").contains("工作目录"));
    // 5) 计划步骤引用未在池中提供的 worker → 400
    let body = r#"{"steps":[{"id":"e1","worker":"missing-worker","input":{"text":"A"}}]}"#;
    let (status, _) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201);
    let body =
        serde_json::json!({ "execution": { "mode": "worker_pool", "workers": [ok_worker] } });
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(status, 400, "{value}");
    assert!(value["error"]
        .as_str()
        .unwrap_or("")
        .contains("未在 execution.workers 中提供"));
    // 6) 未知 mode → 4xx（serde 严格枚举，反序列化失败由 axum 映射为 422）
    let (status, _) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(r#"{"execution":{"mode":"docker"}}"#),
    )
    .await;
    assert!(
        (400..500).contains(&status),
        "未知 mode 应返回明确 4xx，实际 {status}"
    );
}

// ==================== A2：显式执行目标绑定（第四路接线契约） ====================

/// A2 契约：in_process 显式绑定按 step id 键控命中，进程内语义不变。
#[tokio::test]
async fn a2_in_process_explicit_target_runs_registry_worker() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[
        {"id":"e1","worker":"echo","input":{"text":"A"},"verify":"A"}
    ]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "{value}");
    // 绑定键用步骤 id「e1」（select_binding 的 step-id 精确匹配路径）。
    let run_body = r#"{"execution":{"targets":[{"worker":"e1","target":"in_process"}]}}"#;
    let (status, run) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(run_body),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let (status, terminal) = wait_terminal(&app, &goal_id, 10_000).await;
    assert_eq!(status, 200, "{terminal}");
    assert_eq!(terminal["goal_status"], "Succeeded", "{terminal}");
    let steps = terminal["steps"].as_array().unwrap();
    let e1 = steps.iter().find(|s| s["step_id"] == "e1").unwrap();
    assert_eq!(
        e1["output"], "A",
        "in_process 绑定应进程内执行原 echo：{e1}"
    );
    assert_eq!(e1["error"], Value::Null, "{e1}");
}

/// A2 契约：local_process 显式绑定经受控子进程执行（out- 协议前缀证明通道正确）。
/// 同 `p1_worker_pool_mode_runs_steps_in_subprocess`：子进程 worker + HTTP handler
/// 需要多线程 runtime，避免单线程测试 runtime 被阻塞路径饿死。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a2_local_process_explicit_target_completes_step() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[
        {"id":"e1","worker":"echo","input":{"text":"A"}},
        {"id":"s1","worker":"sleep","deps":["e1"],"input":{"ms":20},"verify":"slept"}
    ]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "{value}");
    let run_body = serde_json::json!({
        "execution": {
            "mode": "worker_pool",
            "workers": [pool_worker_json("echo", &[]), pool_worker_json("sleep", &[])],
            "targets": [
                { "worker": "echo", "target": "local_process" },
                { "worker": "s1", "target": "local_process" }
            ]
        }
    });
    let (status, run) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&run_body.to_string()),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let (status, terminal) = wait_terminal(&app, &goal_id, 15_000).await;
    assert_eq!(status, 200, "{terminal}");
    assert_eq!(terminal["goal_status"], "Succeeded", "{terminal}");
    let steps = terminal["steps"].as_array().unwrap();
    let e1 = steps.iter().find(|s| s["step_id"] == "e1").unwrap();
    assert_eq!(
        e1["output"], "out-A",
        "local_process 绑定必须走子进程协议：{e1}"
    );
    let s1 = steps.iter().find(|s| s["step_id"] == "s1").unwrap();
    assert!(
        s1["output"].as_str().unwrap_or("").starts_with("slept"),
        "sleep 步骤应经子进程返回 slept 输出：{s1}"
    );
}

/// A2 契约：fleet_node 显式绑定把任务提交到真实控制面，已注册节点按
/// 节点协议领取并回传后，Goal 步骤落定 Succeeded；correlation ID 全程贯通。
#[tokio::test]
async fn a2_fleet_node_registered_node_completes_step_via_real_protocol() {
    let (state, _temp) = test_state().await;
    let hub = fleet_hub(&state.data_root);
    let app = goal_api::router(state.clone()).merge(fleet_router_with_hub(hub.clone()));
    // 创建 goal + 计划：worker 名与节点 ID 一致（控制面按 task.worker == node_id 匹配领取）。
    let (status, created) = call(
        &app,
        "POST",
        "/goal",
        Some(r#"{"objective":"A2 fleet 显式目标"}"#),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let goal_id = created["goal"]["id"].as_str().unwrap().to_string();
    let plan_body = r#"{"steps":[
        {"id":"step1","worker":"capw","input":{"text":"F"},"verify":"done-F"}
    ]}"#;
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/plan"),
        Some(plan_body),
    )
    .await;
    assert_eq!(status, 201, "{value}");
    // 注册真实节点（node_id=capw），拿到 lease_token/epoch。
    let card = owo_agent_core::capability::CapabilityCard::new("capw");
    let register_body = serde_json::json!({
        "node_id": "capw",
        "card": serde_json::to_value(card).unwrap(),
    });
    let (status, reg) = call(
        &app,
        "POST",
        "/fleet/nodes/register",
        Some(&register_body.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{reg}");
    let token = reg["lease_token"].as_str().unwrap().to_string();
    let epoch = reg["lease_epoch"].as_u64().unwrap();
    // 启动带显式 fleet_node 绑定的运行。
    let run_body = serde_json::json!({
        "execution": {
            "targets": [{
                "worker": "capw",
                "target": "fleet_node",
                "node_id": "capw",
                "capabilities": ["demo"],
                "budget": { "max_duration_secs": 15 }
            }]
        }
    });
    let (status, run) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&run_body.to_string()),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    // 节点侧轮询可领取任务 → correlation ID 必须是 HTTP 层派生值 <goal>/<run>/<键>。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    // 延迟初始化：循环内要么赋值后 break，要么 deadline 断言 panic。
    let task_id;
    loop {
        let (_, tasks) = call(&app, "GET", "/fleet/nodes/capw/tasks", None).await;
        let list = tasks["tasks"].as_array().cloned().unwrap_or_default();
        if let Some(t) = list.iter().find(|t| t["claimable"] == true) {
            task_id = t["task_id"].as_str().unwrap().to_string();
            let corr = t["correlation_id"].as_str().unwrap_or("");
            assert!(
                corr.starts_with(&format!("{goal_id}/")),
                "correlation_id 应为 <goal_id>/<run_id>/<worker> 派生值：{corr}"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "任务未在时限内出现于节点可领取队列：{tasks}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // 领取 + 回传成功结果（output 以 JSON 字符串载荷回传）。
    let claim_body = serde_json::json!({
        "node_id": "capw", "lease_token": token, "epoch": epoch,
    });
    let (status, claimed) = call(
        &app,
        "POST",
        &format!("/fleet/tasks/{task_id}/claim"),
        Some(&claim_body.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{claimed}");
    let result_body = serde_json::json!({
        "node_id": "capw", "lease_token": token, "epoch": epoch,
        "ok": true,
        "output": "done-F",
        "output_cas": null,
        "evidence": [],
        "error": null,
    });
    let (status, reported) = call(
        &app,
        "POST",
        &format!("/fleet/tasks/{task_id}/result"),
        Some(&result_body.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{reported}");
    // Goal 落定成功：显式远端通道完成任务且不产生跨目标改派痕迹。
    let (status, terminal) = wait_terminal(&app, &goal_id, 20_000).await;
    assert_eq!(status, 200, "{terminal}");
    assert_eq!(terminal["goal_status"], "Succeeded", "{terminal}");
    let steps = terminal["steps"].as_array().unwrap();
    let step1 = steps.iter().find(|s| s["step_id"] == "step1").unwrap();
    assert_eq!(step1["status"], "Succeeded", "{step1}");
    // 三类目标输出语义一致：fleet 成功步骤的输出 = 节点回传的 output 值。
    assert_eq!(
        step1["output"], "done-F",
        "远端成功输出应贯通到步骤结果：{step1}"
    );
    assert_eq!(step1["error"], Value::Null, "{step1}");
}

/// A2 契约：不可路由的显式目标必须返回明确结果且不静默改派。
/// 分两层验证：
/// ① 幽灵节点（控制面未注册）→ 启动即 400，运行根本不创建；
/// ② 已注册但静默的节点 → 绑定预算到期后派发超时，步骤/Goal 落到
///    已定义 Failed 态，全程无任何本机 fallback 输出。
#[tokio::test]
async fn a2_unroutable_fleet_target_rejects_without_fallback() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let body = r#"{"steps":[
        {"id":"e1","worker":"echo","input":{"text":"A"}}
    ]}"#;
    let (status, value) = call(&app, "POST", &format!("/goal/{goal_id}/plan"), Some(body)).await;
    assert_eq!(status, 201, "{value}");
    // 进程模式（本来没有 transport）：即使挂着显式远端目标也不允许本机 echo 执行成功。
    let run_body = r#"{"execution":{"targets":[{"worker":"e1","target":"fleet_node","node_id":"ghost-node"}]}}"#;
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(run_body),
    )
    .await;
    assert_eq!(
        status, 400,
        "未注册节点的显式目标必须在准入时明确拒绝：{value}"
    );
    assert!(
        value["error"].as_str().unwrap_or("").contains("不可路由"),
        "错误信息应说明目标不可路由：{value}"
    );
    assert!(
        value["error"].as_str().unwrap_or("").contains("ghost-node"),
        "{value}"
    );
    // 400 拒绝路径不得产生运行记录（无状态文件 → 状态接口 404）。
    let (status, _) = call(&app, "GET", &format!("/goal/{goal_id}/status"), None).await;
    assert_eq!(status, 404, "被拒的运行不应留下任何状态");
}

/// A2 契约：已注册但静默的节点——任务提交真实控制面后无人领取，
/// 绑定预算到期触发派发超时，步骤落到已定义 Failed 且无本机改派输出。
#[tokio::test]
async fn a2_silent_registered_node_times_out_to_defined_failure() {
    let (state, _temp) = test_state().await;
    let hub = fleet_hub(&state.data_root);
    let app = goal_api::router(state.clone()).merge(fleet_router_with_hub(hub.clone()));
    let (status, created) = call(
        &app,
        "POST",
        "/goal",
        Some(r#"{"objective":"A2 静默节点超时"}"#),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let goal_id = created["goal"]["id"].as_str().unwrap().to_string();
    let plan_body = r#"{"steps":[{"id":"s1","worker":"silence","input":{"text":"x"}}]}"#;
    let (status, _) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/plan"),
        Some(plan_body),
    )
    .await;
    assert_eq!(status, 201);
    // 注册节点但故意不领取任何任务。
    let register_body = serde_json::json!({
        "node_id": "silence",
        "card": serde_json::to_value(owo_agent_core::capability::CapabilityCard::new("silence")).unwrap(),
    });
    let (status, reg) = call(
        &app,
        "POST",
        "/fleet/nodes/register",
        Some(&register_body.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{reg}");
    // 2 秒派发预算：超时先 cancel 再报错（防孤儿任务挂起）。
    let run_body = serde_json::json!({
        "execution": {
            "targets": [{
                "worker": "silence",
                "target": "fleet_node",
                "node_id": "silence",
                "budget": { "max_duration_secs": 2 }
            }]
        },
        "allow_replan": false,
    });
    let (status, run) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/run"),
        Some(&run_body.to_string()),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let (status, terminal) = wait_terminal(&app, &goal_id, 15_000).await;
    assert_eq!(status, 200, "{terminal}");
    assert_eq!(
        terminal["goal_status"], "Failed",
        "静默节点超时必须落到已定义 Failed 态：{terminal}"
    );
    let steps = terminal["steps"].as_array().unwrap();
    let s1 = steps.iter().find(|s| s["step_id"] == "s1").unwrap();
    assert_eq!(s1["status"], "Failed", "{s1}");
    let error = s1["error"].as_str().unwrap_or("");
    assert!(
        error.contains("等待超时"),
        "步骤错误应说明派发等待超时：{s1}"
    );
    assert!(
        s1["output"].as_str().unwrap_or("").is_empty(),
        "超时路径绝不允许产生输出（不改派）：{s1}"
    );
}

/// A2 契约：非法/矛盾的目标绑定配置返回明确的 400/422，不启动运行。
#[tokio::test]
async fn a2_target_binding_validation_matrix() {
    let (state, _temp) = test_state().await;
    let (app, goal_id) = setup_goal_with_plan(state.clone()).await;
    let base_plan = r#"{"steps":[
        {"id":"e1","worker":"echo","input":{"text":"A"}},
        {"id":"ag","worker":"agent","input":{"prompt":"hi"}}
    ]}"#;
    let (status, value) = call(
        &app,
        "POST",
        &format!("/goal/{goal_id}/plan"),
        Some(base_plan),
    )
    .await;
    assert_eq!(status, 201, "{value}");

    async fn run_call(app: &axum::Router, goal_id: &str, execution: Value) -> (u16, Value) {
        let body = serde_json::json!({ "execution": execution });
        call(
            app,
            "POST",
            &format!("/goal/{goal_id}/run"),
            Some(&body.to_string()),
        )
        .await
    }

    // 1) fleet_node 缺 node_id → 400
    let (status, v) = run_call(
        &app,
        &goal_id,
        serde_json::json!({ "targets": [{ "worker": "e1", "target": "fleet_node" }] }),
    )
    .await;
    assert_eq!(status, 400, "{v}");
    assert!(v["error"].as_str().unwrap_or("").contains("node_id"), "{v}");
    // 2) 同一键重复绑定 → 400
    let (status, v) = run_call(
        &app,
        &goal_id,
        serde_json::json!({ "targets": [
            { "worker": "e1", "target": "in_process" },
            { "worker": "e1", "target": "in_process" }
        ] }),
    )
    .await;
    assert_eq!(status, 400, "{v}");
    assert!(v["error"].as_str().unwrap_or("").contains("重复"), "{v}");
    // 3) 键未命中任何步骤 → 400
    let (status, v) = run_call(
        &app,
        &goal_id,
        serde_json::json!({ "targets": [{ "worker": "no-such-worker", "target": "in_process" }] }),
    )
    .await;
    assert_eq!(status, 400, "{v}");
    assert!(v["error"].as_str().unwrap_or("").contains("未命中"), "{v}");
    // 4) agent 步骤绑定为非 in_process → 400（凭据不离开本进程）
    for t in ["fleet_node", "local_process"] {
        let mut target = serde_json::json!({ "worker": "ag", "target": t });
        if t == "fleet_node" {
            target["node_id"] = serde_json::json!("n1");
        }
        let (status, v) =
            run_call(&app, &goal_id, serde_json::json!({ "targets": [target] })).await;
        assert_eq!(status, 400, "agent→{t} 必须 400：{v}");
        assert!(v["error"].as_str().unwrap_or("").contains("agent"), "{v}");
    }
    // 5) local_process 与 execution.workers 矛盾（无受控配置）→ 400
    let (status, v) = run_call(
        &app,
        &goal_id,
        serde_json::json!({ "mode": "process", "targets": [{ "worker": "e1", "target": "local_process" }] }),
    )
    .await;
    assert_eq!(status, 400, "{v}");
    assert!(v["error"].as_str().unwrap_or("").contains("矛盾"), "{v}");
    // 6) worker_pool 模式下把内置 worker 绑为 in_process → 400
    //    （pool 必须同时覆盖计划非 agent 引用，先通过 validate_execution 的计划校验，
    //      才能命中 targets 的 in_process/内置矛盾检查）
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let mk_worker = |name: &str| {
        serde_json::json!({
            "name": name, "command": exe, "cwd": cwd,
            "args": ["--exact", "owo_worker_child_entry", "--nocapture", "--quiet"],
            "env": { "OWO_WORKER_CHILD": "1" },
        })
    };
    let (status, v) = run_call(
        &app,
        &goal_id,
        serde_json::json!({
            "mode": "worker_pool",
            "workers": [mk_worker("echo")],
            "targets": [{ "worker": "e1", "target": "in_process" }]
        }),
    )
    .await;
    assert_eq!(status, 400, "{v}");
    assert!(v["error"].as_str().unwrap_or("").contains("矛盾"), "{v}");
    // 7) 未知 target 字面量 → 422（serde 严格枚举）
    let (status, _) = run_call(
        &app,
        &goal_id,
        serde_json::json!({ "targets": [{ "worker": "e1", "target": "docker_swarm" }] }),
    )
    .await;
    assert!(
        (400..500).contains(&status),
        "未知 target 字面量应 4xx，实际 {status}"
    );
}
