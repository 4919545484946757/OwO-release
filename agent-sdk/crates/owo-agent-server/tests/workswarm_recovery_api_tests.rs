//! WorkSwarm R2 恢复 HTTP 契约测试（第三路冻结接口）。
//!
//! 覆盖：
//! - POST /teams/{id}/steer `{"command":"retry","step_id":…,"note":…}`：
//!   缺 step_id → 400、未知步骤 → 404、已成功目标 → 409；
//! - 失败步骤局部重试全链路（replace 换内层 worker → retry → 循环自动重启 → 成功）；
//!   已成功步骤产物版本/CAS ref 不变；重复 retry → 409 且无新增副作用；
//! - 活跃运行中 retry → 409；
//! - 重启遗留：磁盘 Running + 无活动循环 → GET /teams/{id} 返回 `interrupted: true`，
//!   continue 显式恢复后归零（绝不静默重放）。
//!
//! 全部使用内置 echo/sleep/fail worker（不依赖模型凭据）。

use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::permissions::Policy;
use owo_agent_core::project_space_store::{ProjectSpaceStoreBackend, SqliteProjectSpaceStore};
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_protocol::TeamRunStatus;
use owo_agent_server::build_router;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

/// 无外部依赖的最小模型 Provider（本测试只用 echo/sleep/fail worker，模型调用即失败）。
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

/// 发请求并解析 JSON 响应体。
async fn call(
    state: &Arc<owo_agent_server::AppState>,
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (u16, Value) {
    let resp = app
        .clone()
        .oneshot(request(state, method, path, body))
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!(null))
    };
    (status, value)
}

/// 轮询 GET /teams/{id} 直到状态进入目标集合（超时返回最后的值）。
async fn poll_team_status(
    state: &Arc<owo_agent_server::AppState>,
    app: &axum::Router,
    team_id: &str,
    want: &[&str],
    timeout: std::time::Duration,
) -> Value {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = json!({});
    loop {
        let (status, body) = call(state, app, "GET", &format!("/teams/{team_id}"), None).await;
        assert!(
            status == 200,
            "GET /teams/{team_id} 应 200（实际 {status}：{body}）"
        );
        let st = body["team"]["status"].as_str().unwrap_or("").to_string();
        last = body;
        if want.contains(&st.as_str()) {
            return last;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("等待团队状态 {want:?} 超时，最后状态：{st}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn create_team(
    state: &Arc<owo_agent_server::AppState>,
    app: &axum::Router,
    roles: Value,
) -> String {
    let body = json!({ "objective": "R2 恢复契约测试", "mode": "team", "roles": roles });
    let (status, created) = call(state, app, "POST", "/teams", Some(&body.to_string())).await;
    assert_eq!(status, 202, "POST /teams 应 202：{created}");
    created["team_id"].as_str().unwrap().to_string()
}

fn steer_body(command: &str, extra: Value) -> String {
    let mut obj = json!({ "command": command, "note": "R2 测试" });
    if let (Some(a), Some(b)) = (obj.as_object_mut(), extra.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
    obj.to_string()
}

// ---------------------------------------------------------------------------
// 1. 契约面：400 / 404 / 409
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retry_endpoint_contract_400_404_409() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let roles = json!([
        { "role": "runner", "assignee": "agent", "worker": "echo", "verify": "non_empty" }
    ]);
    let team_id = create_team(&state, &app, roles).await;
    let detail = poll_team_status(
        &state,
        &app,
        &team_id,
        &["succeeded", "failed"],
        std::time::Duration::from_secs(20),
    )
    .await;
    assert_eq!(detail["team"]["status"].as_str().unwrap(), "succeeded");

    // 缺 step_id → 400（冻结契约字段校验）。
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body("retry", json!({}))),
    )
    .await;
    assert_eq!(status, 400, "缺 step_id 应 400：{res}");
    assert!(
        res["error"]
            .as_str()
            .unwrap_or_default()
            .contains("step_id"),
        "错误信息应指向 step_id：{res}"
    );

    // 未知步骤 → 404。
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body("retry", json!({ "step_id": "s-ghost" }))),
    )
    .await;
    assert_eq!(status, 404, "未知步骤应 404：{res}");

    // 已成功目标（重复发送同一 retry）→ 409，明确无副作用语义。
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body("retry", json!({ "step_id": "s-runner" }))),
    )
    .await;
    assert_eq!(status, 409, "已成功目标应 409：{res}");
    assert!(
        res["error"]
            .as_str()
            .unwrap_or_default()
            .contains("不产生额外副作用"),
        "错误信息应说明幂等语义：{res}"
    );
}

// ---------------------------------------------------------------------------
// 2. 失败步骤局部重试全链路（replace 换 worker → retry → 循环重启 → 成功）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retry_failed_step_full_loop_via_http() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // planner（echo）→ builder（fail 注入失败）。
    let roles = json!([
        { "role": "planner", "assignee": "agent", "worker": "echo", "verify": "non_empty" },
        { "role": "builder", "assignee": "agent", "worker": "fail", "depends_on": ["planner"],
          "extra_input": { "text": "builder 注入失败" }, "verify": "non_empty" }
    ]);
    let team_id = create_team(&state, &app, roles).await;
    let detail = poll_team_status(
        &state,
        &app,
        &team_id,
        &["failed"],
        std::time::Duration::from_secs(20),
    )
    .await;
    let builder_task = detail["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["task_id"] == "s-builder")
        .unwrap();
    assert_eq!(builder_task["status"], "Failed", "{detail}");
    assert_eq!(builder_task["attempts"], 1);

    // 项目空间事实：planner 产物 v1 + CAS ref（后续必须保持不变）。
    let project_id = detail["team"]["project_space_id"].as_str().unwrap();
    let (_, arts) = call(
        &state,
        &app,
        "GET",
        &format!("/projects/{project_id}/artifacts"),
        None,
    )
    .await;
    let plan_v1 = arts["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "plan")
        .cloned()
        .expect("planner 产物应存在");
    assert_eq!(plan_v1["version"], 1);

    // replace：builder 内层 worker fail → echo（运行循环已退出，改盘立即生效）。
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body(
            "replace",
            json!({ "role": "builder", "new_worker": "echo" }),
        )),
    )
    .await;
    assert_eq!(status, 200, "replace 应 200：{res}");

    // retry 失败步骤 → 200 + 运行循环自动重启。
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body(
            "retry",
            json!({ "step_id": "s-builder", "note": "修复输入后重试" }),
        )),
    )
    .await;
    assert_eq!(status, 200, "retry 应 200：{res}");
    assert_eq!(res["status"], "Created", "{res}");

    // 循环重启后跑完。
    let detail = poll_team_status(
        &state,
        &app,
        &team_id,
        &["succeeded"],
        std::time::Duration::from_secs(20),
    )
    .await;
    let tasks = detail["tasks"].as_array().unwrap();
    let builder_task = tasks.iter().find(|t| t["task_id"] == "s-builder").unwrap();
    assert_eq!(builder_task["status"], "Succeeded");
    assert_eq!(
        builder_task["attempts"], 1,
        "retry 重置计数后本窗口恰好执行一次（成功 1）：{detail}"
    );
    let planner_task = tasks.iter().find(|t| t["task_id"] == "s-planner").unwrap();
    assert_eq!(
        planner_task["attempts"], 1,
        "已成功步骤执行次数不得增加：{detail}"
    );

    // 产物：2 个；planner 产物版本/CAS ref 不变。
    let (_, arts) = call(
        &state,
        &app,
        "GET",
        &format!("/projects/{project_id}/artifacts"),
        None,
    )
    .await;
    let list = arts["artifacts"].as_array().unwrap();
    assert_eq!(list.len(), 2, "retry 不得产生 planner 新版本产物：{arts}");
    let plan_after = list.iter().find(|a| a["kind"] == "plan").unwrap();
    assert_eq!(plan_after["version"], 1);
    assert_eq!(
        plan_after["content_ref"], plan_v1["content_ref"],
        "CAS ref 必须保持不变"
    );

    // 决策：恰好 1 条（retry 留痕）。
    let (_, space) = call(
        &state,
        &app,
        "GET",
        &format!("/projects/{project_id}"),
        None,
    )
    .await;
    assert_eq!(
        space["project"]["decisions"].as_array().unwrap().len(),
        1,
        "应恰好有一条 retry 决策：{space}"
    );

    // 重复发送同一 retry → 409，且无任何新增副作用。
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body(
            "retry",
            json!({ "step_id": "s-builder", "note": "修复输入后重试" }),
        )),
    )
    .await;
    assert_eq!(status, 409, "重复 retry 应 409：{res}");
    let (_, arts2) = call(
        &state,
        &app,
        "GET",
        &format!("/projects/{project_id}/artifacts"),
        None,
    )
    .await;
    assert_eq!(
        arts2["artifacts"].as_array().unwrap().len(),
        2,
        "重复 retry 不得新增产物"
    );
    let (_, space2) = call(
        &state,
        &app,
        "GET",
        &format!("/projects/{project_id}"),
        None,
    )
    .await;
    assert_eq!(
        space2["project"]["decisions"].as_array().unwrap().len(),
        1,
        "重复 retry 不得新增决策"
    );
}

// ---------------------------------------------------------------------------
// 3. 活跃运行中 retry → 409
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retry_during_active_run_returns_409() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let roles = json!([
        { "role": "runner", "assignee": "agent", "worker": "sleep",
          "extra_input": { "ms": 3000 }, "verify": "non_empty" }
    ]);
    let team_id = create_team(&state, &app, roles).await;

    // 等运行进入活动窗口。
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body("retry", json!({ "step_id": "s-runner" }))),
    )
    .await;
    assert_eq!(status, 409, "运行中 retry 应 409：{res}");

    // cancel 收尾（清理现场；不留给并行测试脏状态）。
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body("cancel", json!({}))),
    )
    .await;
    assert_eq!(status, 200, "cancel 应 200：{res}");
    let _ = poll_team_status(
        &state,
        &app,
        &team_id,
        &["cancelled"],
        std::time::Duration::from_secs(20),
    )
    .await;
}

// ---------------------------------------------------------------------------
// 4. 重启遗留：磁盘 Running 无活动循环 → interrupted；continue 显式恢复
// ---------------------------------------------------------------------------

#[tokio::test]
async fn interrupted_leftover_is_surfaced_then_continue_recovers() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // 短跑成功（保证退出后无活动循环/活动阶段）。
    let roles = json!([
        { "role": "runner", "assignee": "agent", "worker": "sleep",
          "extra_input": { "ms": 150 }, "verify": "non_empty" }
    ]);
    let team_id = create_team(&state, &app, roles).await;
    let detail = poll_team_status(
        &state,
        &app,
        &team_id,
        &["succeeded"],
        std::time::Duration::from_secs(20),
    )
    .await;
    let project_id = detail["team"]["project_space_id"]
        .as_str()
        .unwrap()
        .to_string();
    let completed_seq = detail["progress"]["seq"]
        .as_u64()
        .expect("完成快照应携带 progress.seq");
    let (_, arts0) = call(
        &state,
        &app,
        "GET",
        &format!("/projects/{project_id}/artifacts"),
        None,
    )
    .await;
    assert_eq!(arts0["artifacts"].as_array().unwrap().len(), 1);

    // ——模拟进程重启遗留：直接经存储层把磁盘状态改回 Running
    //   （真实场景：运行循环随进程消亡，磁盘停留在 Running）。
    let ws_dir = temp.path().join("workswarm");
    let store = SqliteProjectSpaceStore::open(&ws_dir.join("space.db")).unwrap();
    let mut team = store.get_team_run(&team_id).await.unwrap().clone();
    assert!(team.status.is_terminal());
    team.status = TeamRunStatus::Running;
    store.save_team_run(&team).await.unwrap();
    drop(store);

    // GET /teams/{id}：请求时中断识别生效 → interrupted = true（状态保持 Running 展示）。
    let (status, body) = call(&state, &app, "GET", &format!("/teams/{team_id}"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["interrupted"], true,
        "重启遗留 Running 应被识别为中断：{body}"
    );
    assert_eq!(
        body["team"]["status"].as_str().unwrap(),
        "running",
        "磁盘状态保持 Running 展示（由 interrupted 标记区分中断）：{body}"
    );
    assert_eq!(
        body["progress"]["supervisor"]["state"], "blocked",
        "中断遗留必须被监督器标记为 blocked，而不是 idle：{body}"
    );
    assert!(
        body["progress"]["seq"].as_u64().unwrap_or(0) > completed_seq,
        "监督状态变化必须推进 progress.seq，否则 SSE 客户端看不到新快照：{body}"
    );
    assert_eq!(
        body["progress"]["supervisor"]["interrupted"], true,
        "监督器快照应携带中断标记：{body}"
    );
    assert!(
        body["progress"]["supervisor"]["recommendation"]
            .as_str()
            .unwrap_or_default()
            .contains("恢复"),
        "监督建议应指向恢复入口：{body}"
    );

    // 列表与事件快照同样暴露标记。
    let (_, list) = call(&state, &app, "GET", "/teams", None).await;
    let entry = list["teams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["team_id"].as_str() == Some(team_id.as_str()))
        .unwrap();
    assert_eq!(entry["interrupted"], true, "{list}");

    // ——显式 continue 恢复（循环重启）。
    let (status, res) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{team_id}/steer"),
        Some(&steer_body("continue", json!({}))),
    )
    .await;
    assert_eq!(status, 200, "continue 应 200：{res}");
    assert_eq!(res["interrupted"], false, "恢复后标记清除：{res}");

    let detail = poll_team_status(
        &state,
        &app,
        &team_id,
        &["succeeded"],
        std::time::Duration::from_secs(20),
    )
    .await;
    assert_eq!(detail["interrupted"], false, "{detail}");

    // continue 语义 = 只重置未完成步骤、绝不重跑已完成步骤（禁止静默重复执行写操作）：
    // runner 步骤早已成功，恢复后不得再次执行 → 产物仍是 v1。
    let (_, arts) = call(
        &state,
        &app,
        "GET",
        &format!("/projects/{project_id}/artifacts"),
        None,
    )
    .await;
    let list = arts["artifacts"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{arts}");
    assert_eq!(
        list[0]["version"], 1,
        "continue 恢复不得重跑已完成步骤（产物保持 v1）：{arts}"
    );
}
