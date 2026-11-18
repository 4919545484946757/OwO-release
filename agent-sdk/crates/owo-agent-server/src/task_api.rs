//! 统一 Task Runtime HTTP API（Agent V2 Phase 2/3）。
//!
//! 路由面（bearer 保护面，路径前缀 `/agent-tasks`，避免与 WorkSwarm 的
//! `/tasks/{id}/handoff` 语义冲突）：
//!
//! - `GET  /agent-tasks`                         列表（state/limit/offset）
//! - `POST /agent-tasks`                         创建（支持 idempotency_key）
//! - `GET  /agent-tasks/{id}`                    详情（任务 + 检查点 + 产物 + 待审批）
//! - `POST /agent-tasks/{id}/start`              开始执行（未注册后端时明确返回未分发）
//! - `POST /agent-tasks/{id}/pause`              暂停
//! - `POST /agent-tasks/{id}/resume`             恢复
//! - `POST /agent-tasks/{id}/cancel`             取消
//! - `GET  /agent-tasks/{id}/events`             事件（after/limit）
//! - `GET  /agent-tasks/{id}/events/stream`      事件 SSE（after 游标重连，终态后结束）
//! - `GET  /agent-tasks/{id}/artifacts`          产物
//! - `GET  /agent-tasks/{id}/approvals`          审批列表（pending_only）
//! - `POST /agent-tasks/{id}/approvals`          请求审批
//! - `POST /agent-tasks/{id}/approvals/{approval_id}` 审批决策
//! - `POST /agent-tasks/{id}/verify`             独立验证（只有通过才进入 succeeded）
//!
//! 独立编译约束（沿 computer_api 惯例）：不使用 `crate::`/`super::`；server 类型
//! 一律写全限定名 `owo_agent_server::AppState`。

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use owo_agent_core::task_runtime::{
    ApprovalDecision, CreateTaskRequest, ExecutionStrategy, TaskBudget, TaskError, TaskState,
    VerificationEvidence,
};
use owo_agent_server::AppState;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn api_error(error: TaskError) -> (StatusCode, Json<Value>) {
    let status = match error.code.as_str() {
        "TASK_NOT_FOUND" => StatusCode::NOT_FOUND,
        "INVALID_TRANSITION" => StatusCode::CONFLICT,
        "UNSUPPORTED" => StatusCode::NOT_IMPLEMENTED,
        "STORE_ERROR" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        Json(json!({ "ok": false, "code": error.code, "error": error.message })),
    )
}

fn bad_request(code: &str, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "code": code, "error": message.into() })),
    )
}

#[derive(Deserialize)]
struct CreateTaskBody {
    user_goal: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    execution_strategy: Option<String>,
    #[serde(default)]
    budget: Option<TaskBudget>,
    /// 执行策略档位。`workspace`（默认）允许工作区内写入、拒绝命令执行；
    /// `workspace-execute` 显式允许后台任务执行命令。不放宽 Policy 的判定。
    #[serde(default)]
    policy_profile: Option<String>,
    #[serde(default)]
    workspace_ref: Option<String>,
    #[serde(default)]
    session_refs: Vec<String>,
    /// 后端专属输入（Runtime 不解释）。computer_use 用它承载
    /// `target_app` / `goals` / `max_duration_ms` / `action_budget`。
    #[serde(default)]
    payload: Option<Value>,
    /// 可选幂等键：重复提交返回同一任务，不重复创建。
    #[serde(default)]
    idempotency_key: Option<String>,
}

async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTaskBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if let Some(key) = body.idempotency_key.as_deref().map(str::trim) {
        if !key.is_empty() {
            if let Some((task_id, _payload)) = state
                .tasks
                .store()
                .get_idempotency(key)
                .map_err(api_error)?
            {
                let task = state.tasks.get_task(&task_id).map_err(api_error)?;
                return Ok((
                    StatusCode::OK,
                    Json(json!({ "ok": true, "idempotent_replay": true, "task": task })),
                ));
            }
        }
    }
    let strategy = match body.execution_strategy.as_deref() {
        None => ExecutionStrategy::SingleAgent,
        Some(text) => ExecutionStrategy::parse(text)
            .ok_or_else(|| bad_request("INVALID_REQUEST", format!("未知执行策略：{text}")))?,
    };
    let mut request = CreateTaskRequest::new(body.user_goal);
    request.title = body.title;
    request.execution_strategy = strategy;
    if let Some(budget) = body.budget {
        request.budget = budget;
    }
    if let Some(policy) = body.policy_profile {
        request.policy_profile = policy;
    }
    request.workspace_ref = body.workspace_ref;
    request.session_refs = body.session_refs;
    request.payload = body.payload;
    let task = state.tasks.create_task(request).map_err(api_error)?;
    if let Some(key) = body.idempotency_key.as_deref().map(str::trim) {
        if !key.is_empty() {
            state
                .tasks
                .store()
                .put_idempotency(
                    key,
                    task.task_id.as_str(),
                    &json!({"task_id": task.task_id}),
                )
                .map_err(api_error)?;
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(json!({ "ok": true, "task": task })),
    ))
}

async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let state_filter = match query.get("state").map(String::as_str) {
        None | Some("") => None,
        Some(text) => Some(
            TaskState::parse(text)
                .ok_or_else(|| bad_request("INVALID_REQUEST", format!("未知状态：{text}")))?,
        ),
    };
    let limit = query
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50);
    let offset = query
        .get("offset")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let tasks = state
        .tasks
        .list_tasks(state_filter, limit, offset)
        .map_err(api_error)?;
    Ok(Json(
        json!({ "ok": true, "count": tasks.len(), "tasks": tasks }),
    ))
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let task = state.tasks.get_task(&id).map_err(api_error)?;
    let checkpoint = state.tasks.latest_checkpoint(&id).map_err(api_error)?;
    let artifacts = state.tasks.artifacts(&id).map_err(api_error)?;
    let approvals = state.tasks.approvals(&id, false).map_err(api_error)?;
    Ok(Json(json!({
        "ok": true,
        "task": task,
        "latest_checkpoint": checkpoint,
        "artifacts": artifacts,
        "approvals": approvals,
    })))
}

async fn start_task(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    // 状态先进入 running，再交给分发层。即使没有注册后端，也要走
    // `dispatch_start`：它负责写入 `task.backend.unsupported` 事件，并返回明确的
    // UNSUPPORTED，调用方据此显示"未分发"，而不是伪装成执行成功。
    let (_task, _registered) = state.tasks.start_task(&id).map_err(api_error)?;
    match state.tasks.dispatch_start(&id).await {
        Ok(run_id) => Ok(Json(json!({
            "ok": true,
            "task": state.tasks.get_task(&id).map_err(api_error)?,
            "dispatched": true,
            "backend_run_id": run_id.0,
        }))),
        Err(error) => Ok(Json(json!({
            "ok": true,
            "task": state.tasks.get_task(&id).map_err(api_error)?,
            "dispatched": false,
            "code": error.code,
            "message": error.message,
        }))),
    }
}

#[derive(Deserialize, Default)]
struct ReasonBody {
    #[serde(default)]
    reason: Option<String>,
}

async fn pause_task(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ReasonBody>,
) -> ApiResult<Json<Value>> {
    let reason = body.reason.unwrap_or_else(|| "用户暂停".to_string());
    // 暂停要真正通知执行后端：只改状态会让后台回合/闭环继续跑。
    let task = state
        .tasks
        .pause_task_with_backend(&id, &reason)
        .await
        .map_err(api_error)?;
    Ok(Json(json!({ "ok": true, "task": task })))
}

#[derive(Deserialize)]
struct ResumeTaskBody {
    /// The exact current definition returned by GET /workflow/{name}.
    approved_workflow_revision: Option<Value>,
}

async fn resume_task(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<ResumeTaskBody>>,
) -> ApiResult<Json<Value>> {
    // 恢复 = 恢复状态 + 让后端重新开始执行，不是只把徽标改回 running。
    let task = state
        .tasks
        .resume_task_with_revision(
            &id,
            body.and_then(|Json(body)| body.approved_workflow_revision),
        )
        .await
        .map_err(api_error)?;
    Ok(Json(json!({ "ok": true, "task": task })))
}

async fn cancel_task(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ReasonBody>,
) -> ApiResult<Json<Value>> {
    let reason = body.reason.unwrap_or_else(|| "用户取消".to_string());
    // 先通知后端停止（支持取消的后端会中止真实回合），再收口任务状态。
    // 后端迟到的结果由运行时丢弃，不会把已取消的任务翻成成功。
    let task = state
        .tasks
        .cancel_task_with_backend(&id, &reason)
        .await
        .map_err(api_error)?;
    Ok(Json(json!({ "ok": true, "task": task })))
}

async fn list_events(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let after = query
        .get("after")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let limit = query
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(200);
    let events = state.tasks.events(&id, after, limit).map_err(api_error)?;
    Ok(Json(json!({ "ok": true, "events": events })))
}

/// 任务事件 SSE 的流状态：已消费到的事件序号、待发送帧与是否已收口。
struct TaskEventStreamState {
    state: Arc<AppState>,
    task_id: String,
    cursor: u64,
    queue: VecDeque<Event>,
    done: bool,
}

/// 任务事件 SSE：`after` 是调用方最后收到的序号（不含），断线重连只需带上它。
///
/// 事件来自同一个持久化 Task Store，这里按 300 ms 轮询增量（事件低频，无需为
/// 每个任务额外维护广播通道）；任务进入终态且事件发完后发送一次 `done` 并结束。
async fn stream_task_events(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    if state.tasks.get_task(&id).is_err() {
        return Err((StatusCode::NOT_FOUND, format!("任务 {id} 不存在")));
    }
    let after = query
        .get("after")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let initial = TaskEventStreamState {
        state,
        task_id: id,
        cursor: after,
        queue: VecDeque::new(),
        done: false,
    };
    let stream = futures_util::stream::unfold(initial, |mut stream| async move {
        loop {
            if let Some(event) = stream.queue.pop_front() {
                return Some((Ok(event), stream));
            }
            if stream.done {
                return None;
            }
            match stream
                .state
                .tasks
                .events(&stream.task_id, stream.cursor, 200)
            {
                Ok(events) if !events.is_empty() => {
                    for event in events {
                        stream.cursor = event.sequence;
                        let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
                        stream.queue.push_back(
                            Event::default()
                                .id(stream.cursor.to_string())
                                .event("task-event")
                                .data(data),
                        );
                    }
                }
                Ok(_) => {
                    let terminal = stream
                        .state
                        .tasks
                        .get_task(&stream.task_id)
                        .map(|task| task.state.is_terminal())
                        .unwrap_or(true);
                    if terminal {
                        let task_id = stream.task_id.clone();
                        stream.queue.push_back(
                            Event::default()
                                .event("done")
                                .data(json!({"task_id": task_id}).to_string()),
                        );
                        stream.done = true;
                    } else {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                }
                Err(_) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let artifacts = state.tasks.artifacts(&id).map_err(api_error)?;
    Ok(Json(json!({ "ok": true, "artifacts": artifacts })))
}

async fn list_approvals(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let pending_only = matches!(
        query.get("pending_only").map(String::as_str),
        Some("1" | "true")
    );
    let approvals = state
        .tasks
        .approvals(&id, pending_only)
        .map_err(api_error)?;
    Ok(Json(json!({ "ok": true, "approvals": approvals })))
}

#[derive(Deserialize)]
struct RequestApprovalBody {
    action: String,
    #[serde(default)]
    scope: Value,
    #[serde(default)]
    expires_at: Option<String>,
}

async fn request_approval(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<RequestApprovalBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if body.action.trim().is_empty() {
        return Err(bad_request("INVALID_REQUEST", "审批必须说明动作"));
    }
    let approval = state
        .tasks
        .request_approval(&id, body.action.trim(), body.scope, body.expires_at)
        .map_err(api_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "ok": true, "approval": approval })),
    ))
}

#[derive(Deserialize)]
struct ApprovalDecisionBody {
    decision: String,
    #[serde(default)]
    decided_by: Option<String>,
}

async fn decide_approval_with_body(
    State(state): State<Arc<AppState>>,
    AxumPath((id, approval_id)): AxumPath<(String, String)>,
    Json(body): Json<ApprovalDecisionBody>,
) -> ApiResult<Json<Value>> {
    let decision = ApprovalDecision::parse(&body.decision)
        .ok_or_else(|| bad_request("INVALID_REQUEST", "decision 必须是 approved 或 rejected"))?;
    let approval = state
        .tasks
        .decide_approval(&id, &approval_id, decision, body.decided_by.as_deref())
        .map_err(api_error)?;
    Ok(Json(json!({
        "ok": true,
        "approval": approval,
        "task": state.tasks.get_task(&id).map_err(api_error)?,
    })))
}

async fn verify_task(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(evidence): Json<VerificationEvidence>,
) -> ApiResult<Json<Value>> {
    let report = state.tasks.verify_task(&id, evidence).map_err(api_error)?;
    Ok(Json(json!({
        "ok": true,
        "report": report,
        "task": state.tasks.get_task(&id).map_err(api_error)?,
    })))
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/agent-tasks", get(list_tasks).post(create_task))
        .route("/agent-tasks/{id}", get(get_task))
        .route("/agent-tasks/{id}/start", post(start_task))
        .route("/agent-tasks/{id}/pause", post(pause_task))
        .route("/agent-tasks/{id}/resume", post(resume_task))
        .route("/agent-tasks/{id}/cancel", post(cancel_task))
        .route("/agent-tasks/{id}/events", get(list_events))
        .route("/agent-tasks/{id}/events/stream", get(stream_task_events))
        .route("/agent-tasks/{id}/artifacts", get(list_artifacts))
        .route(
            "/agent-tasks/{id}/approvals",
            get(list_approvals).post(request_approval),
        )
        .route(
            "/agent-tasks/{id}/approvals/{approval_id}",
            post(decide_approval_with_body),
        )
        .route("/agent-tasks/{id}/verify", post(verify_task))
        .with_state(state)
}
