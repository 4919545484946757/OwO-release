//! 输入法 / 桌宠 ↔ Agent 协调协议 v1（Agent V2 Phase 7）。
//!
//! 路由（bearer 保护面）：
//!
//! - `POST /agent-bridge/messages`                接收 intent.submit / task.pause / task.resume /
//!                                                task.cancel / task.open
//! - `GET  /agent-bridge/tasks/{id}/status`       生成 task.status.changed 与
//!                                                task.approval.required 消息
//! - `GET  /agent-bridge/preferences`             读取桌宠通知开关
//! - `POST /agent-bridge/preferences`             关闭/开启桌宠通知
//!
//! 约束（方案 §11）：
//! - 每条消息都带 `protocol_version`、`message_id`、`timestamp`、`source`、
//!   `correlation_id`、`payload`；版本不认识就明确拒绝，不做猜测。
//! - `message_id` 幂等：重复提交返回同一任务，不重复创建/重复执行动作。
//! - 来源决定权限：输入法只能提交意图与打开详情，桌宠可以暂停/继续/取消，
//!   审批决定（task.approval.decided）只允许在可信主界面完成，桥接一律拒绝。
//! - 输入法/桌宠只需 HTTP + bearer token；Agent 服务不可用时它们仍可正常输入，
//!   恢复后再重发同一 `message_id` 即可。
//! - 状态消息只带任务状态、待审批数量与可跳转路由，不携带未脱敏的内容/参数。

use sha2::{Digest, Sha256};

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use owo_agent_core::task_runtime::{CreateTaskRequest, ExecutionStrategy, TaskState};
use owo_agent_server::AppState;

pub const BRIDGE_PROTOCOL_VERSION: u32 = 1;
const MAX_REMEMBERED_MESSAGES: usize = 512;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({ "ok": false, "code": code, "error": message.into() })),
    )
}

/// 输入法 / 桌宠 / 桌面的来源标识。
fn valid_source(source: &str) -> bool {
    matches!(source, "ime" | "pet" | "desktop")
}

fn allowed(source: &str, kind: &str) -> bool {
    match kind {
        "intent.submit" | "task.open" => matches!(source, "ime" | "pet" | "desktop"),
        "task.pause" | "task.resume" | "task.cancel" => matches!(source, "pet" | "desktop"),
        _ => false,
    }
}

/// 版本化 IPC 消息（v1）。字段缺失/版本不符会被明确拒绝。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeMessage {
    pub protocol_version: u32,
    pub message_id: String,
    pub timestamp: String,
    pub source: String,
    pub correlation_id: String,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Default)]
pub struct AgentBridgeState {
    /// message_id → 响应（幂等重放；只保留最近若干条）。
    seen: Mutex<HashMap<String, Value>>,
    order: Mutex<Vec<String>>,
    pet_notifications: Mutex<bool>,
}

impl AgentBridgeState {
    pub fn new() -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            pet_notifications: Mutex::new(true),
        }
    }

    fn lookup(&self, message_id: &str) -> Option<Value> {
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(message_id)
            .cloned()
    }

    fn remember(&self, message_id: String, response: Value) {
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut order = self
            .order
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        seen.insert(message_id.clone(), response);
        order.push(message_id);
        while order.len() > MAX_REMEMBERED_MESSAGES {
            if let Some(expired) = order.first().cloned() {
                order.remove(0);
                seen.remove(&expired);
            }
        }
    }

    pub fn pet_notifications(&self) -> bool {
        *self
            .pet_notifications
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_pet_notifications(&self, enabled: bool) {
        *self
            .pet_notifications
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = enabled;
    }
}

fn task_error(error: owo_agent_core::task_runtime::TaskError) -> (StatusCode, Json<Value>) {
    let status = match error.code.as_str() {
        "TASK_NOT_FOUND" => StatusCode::NOT_FOUND,
        "INVALID_TRANSITION" | "IDEMPOTENCY_CONFLICT" => StatusCode::CONFLICT,
        "STORE_ERROR" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        Json(json!({ "ok": false, "code": error.code, "error": error.message })),
    )
}

fn validate(message: &BridgeMessage) -> Result<(), (StatusCode, Json<Value>)> {
    if message.protocol_version != BRIDGE_PROTOCOL_VERSION {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "UNSUPPORTED_PROTOCOL_VERSION",
            format!(
                "协议版本 {} 不受支持，本服务只接受 v{BRIDGE_PROTOCOL_VERSION}",
                message.protocol_version
            ),
        ));
    }
    if message.message_id.trim().is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
            "message_id 不能为空",
        ));
    }
    if message.correlation_id.trim().is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
            "correlation_id 不能为空",
        ));
    }
    if !valid_source(&message.source) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "INVALID_SOURCE",
            format!("未知来源：{}（可选 ime / pet / desktop）", message.source),
        ));
    }
    // timestamp 必须是 RFC3339：协议要求可排序时间，但不允许用它替代 sequence。
    if chrono::DateTime::parse_from_rfc3339(&message.timestamp).is_err() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
            format!("timestamp 不是 RFC3339：{}", message.timestamp),
        ));
    }
    Ok(())
}

fn outcome(status: StatusCode, body: Value) -> (StatusCode, Json<Value>) {
    (status, Json(body))
}

async fn handle_message(
    State(state): State<Arc<AppState>>,
    Json(message): Json<BridgeMessage>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate(&message)?;
    if message.kind == "task.approval.decided" || message.kind == "task.approval.required" {
        return Err(error(
            StatusCode::FORBIDDEN,
            "APPROVAL_REQUIRES_MAIN_UI",
            "审批决定只能在可信的 OwO 主界面完成，输入法/桌宠不能静默授权",
        ));
    }
    if !allowed(&message.source, &message.kind) {
        return Err(error(
            StatusCode::FORBIDDEN,
            "SOURCE_NOT_ALLOWED",
            format!("来源 {} 不能发送 {}", message.source, message.kind),
        ));
    }
    if let Some(cached) = (message.kind != "intent.submit")
        .then(|| state.bridge.lookup(&message.message_id))
        .flatten()
    {
        let mut replay = cached.clone();
        replay["duplicate"] = Value::Bool(true);
        return Ok(outcome(StatusCode::OK, replay));
    }

    let (status, response) = match message.kind.as_str() {
        "intent.submit" => submit_intent(&state, &message).await?,
        "task.pause" => task_action(&state, &message, "pause").await?,
        "task.resume" => task_action(&state, &message, "resume").await?,
        "task.cancel" => task_action(&state, &message, "cancel").await?,
        "task.open" => open_task(&message)?,
        other => {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "INVALID_REQUEST",
                format!("未知消息类型：{other}"),
            ))
        }
    };
    if status.is_success() && message.kind != "intent.submit" {
        state
            .bridge
            .remember(message.message_id.clone(), response.clone());
    }
    Ok(outcome(status, response))
}

async fn submit_intent(
    state: &Arc<AppState>,
    message: &BridgeMessage,
) -> ApiResult<(StatusCode, Value)> {
    let goal = message
        .payload
        .get("user_goal")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "EMPTY_GOALS",
                "intent.submit 必须提供非空 payload.user_goal",
            )
        })?;
    let strategy = match message
        .payload
        .get("execution_strategy")
        .and_then(Value::as_str)
    {
        None => ExecutionStrategy::SingleAgent,
        Some(text) => ExecutionStrategy::parse(text).ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "INVALID_REQUEST",
                format!("未知执行策略：{text}"),
            )
        })?,
    };
    let mut request = CreateTaskRequest::new(goal);
    request.execution_strategy = strategy;
    request.title = message
        .payload
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    // 输入法/桌宠只能提交默认（只读倾向）档位；执行命令需要主界面显式创建任务。
    request.policy_profile = "workspace".to_string();
    // Source and kind scope prevent collisions with other bridge operations.
    // Store only a digest, rather than another copy of potentially sensitive input.
    let key = format!(
        "bridge-intent:v1:{}:{}",
        message.source,
        format_args!("{:x}", Sha256::digest(message.message_id.as_bytes()))
    );
    let encoded = serde_json::to_vec(message)
        .map_err(|err| error(StatusCode::BAD_REQUEST, "INVALID_REQUEST", err.to_string()))?;
    let fingerprint = json!({"bridge_message_sha256": format!("{:x}", Sha256::digest(encoded))});
    let (task, duplicate) = state
        .tasks
        .create_task_idempotent(request, &key, &fingerprint)
        .map_err(task_error)?;
    Ok((
        if duplicate {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        json!({
            "ok": true,
            "duplicate": duplicate,
            "kind": "intent.accepted",
            "protocol_version": BRIDGE_PROTOCOL_VERSION,
            "correlation_id": message.correlation_id,
            "message_id": message.message_id,
            "task_id": task.task_id,
            "state": task.state.as_str(),
            "open_route": format!("/agent-tasks/{}", task.task_id),
        }),
    ))
}

async fn task_action(
    state: &Arc<AppState>,
    message: &BridgeMessage,
    action: &str,
) -> ApiResult<(StatusCode, Value)> {
    let task_id = message
        .payload
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "INVALID_REQUEST",
                format!("{} 必须提供 payload.task_id", message.kind),
            )
        })?;
    let reason = message
        .payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or(match action {
            "pause" => "来源请求暂停",
            "cancel" => "来源请求取消",
            _ => "来源请求继续",
        })
        .to_string();
    let task = match action {
        "pause" => state.tasks.pause_task_with_backend(task_id, &reason).await,
        "resume" => state.tasks.resume_task_with_backend(task_id).await,
        _ => state.tasks.cancel_task_with_backend(task_id, &reason).await,
    }
    .map_err(task_error)?;
    Ok((
        StatusCode::OK,
        json!({
            "ok": true,
            "kind": "task.action.applied",
            "protocol_version": BRIDGE_PROTOCOL_VERSION,
            "correlation_id": message.correlation_id,
            "message_id": message.message_id,
            "task_id": task.task_id,
            "state": task.state.as_str(),
            "action": action,
        }),
    ))
}

fn open_task(message: &BridgeMessage) -> ApiResult<(StatusCode, Value)> {
    let task_id = message
        .payload
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "INVALID_REQUEST",
                "task.open 必须提供 payload.task_id",
            )
        })?;
    Ok((
        StatusCode::OK,
        json!({
            "ok": true,
            "kind": "task.open",
            "protocol_version": BRIDGE_PROTOCOL_VERSION,
            "correlation_id": message.correlation_id,
            "message_id": message.message_id,
            "task_id": task_id,
            // 审批/详情只打开主界面，不在悬浮窗口里就地授权。
            "open_route": format!("/agent-tasks/{task_id}"),
            "requires_main_ui": true,
        }),
    ))
}

fn notice(kind: &str, task_id: &str, payload: Value) -> BridgeMessage {
    BridgeMessage {
        protocol_version: BRIDGE_PROTOCOL_VERSION,
        message_id: format!("srv-{}", uuid::Uuid::new_v4()),
        timestamp: chrono::Utc::now().to_rfc3339(),
        source: "owo-server".to_string(),
        correlation_id: task_id.to_string(),
        kind: kind.to_string(),
        payload,
    }
}

async fn task_status(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let after = query
        .get("after")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let task = state.tasks.get_task(&id).map_err(task_error)?;
    let approvals = state.tasks.approvals(&id, true).map_err(task_error)?;
    let events = state.tasks.events(&id, after, 200).map_err(task_error)?;
    let last_sequence = events.last().map(|event| event.sequence).unwrap_or(after);
    let terminal = task.state.is_terminal();
    let needs_attention = matches!(
        task.state,
        TaskState::AwaitingApproval
            | TaskState::NeedsUserInput
            | TaskState::Interrupted
            | TaskState::Paused
    );
    let notifications = state.bridge.pet_notifications();

    let mut messages = Vec::new();
    if notifications {
        messages.push(notice(
            "task.status.changed",
            &id,
            json!({
                "task_id": id,
                "state": task.state.as_str(),
                "terminal": terminal,
                "needs_attention": needs_attention,
                "pending_approvals": approvals.len(),
                "last_sequence": last_sequence,
                "updated_at": task.updated_at,
                "open_route": format!("/agent-tasks/{id}"),
            }),
        ));
        for approval in &approvals {
            messages.push(notice(
                "task.approval.required",
                &id,
                json!({
                    "task_id": id,
                    "approval_id": approval.approval_id,
                    "action": approval.action,
                    "requested_at": approval.requested_at,
                    // 悬浮窗口只负责跳转，审批卡与参数摘要只在主界面展示。
                    "open_route": format!("/agent-tasks/{id}"),
                    "requires_main_ui": true,
                }),
            ));
        }
    }

    Ok(Json(json!({
        "ok": true,
        "protocol_version": BRIDGE_PROTOCOL_VERSION,
        "notifications_enabled": notifications,
        "task_id": id,
        "state": task.state.as_str(),
        "last_sequence": last_sequence,
        "messages": messages,
    })))
}

async fn get_preferences(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "protocol_version": BRIDGE_PROTOCOL_VERSION,
        "pet_notifications": state.bridge.pet_notifications(),
    }))
}

#[derive(Deserialize)]
struct PreferencesBody {
    pet_notifications: bool,
}

async fn set_preferences(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PreferencesBody>,
) -> Json<Value> {
    state.bridge.set_pet_notifications(body.pet_notifications);
    Json(json!({
        "ok": true,
        "protocol_version": BRIDGE_PROTOCOL_VERSION,
        "pet_notifications": state.bridge.pet_notifications(),
    }))
}

/// 桥接开关：`OWO_AGENT_BRIDGE_ENABLED=false` 时不挂载任何 `/agent-bridge/*` 路由
/// （外部入口整体下线；主界面任务与审批不受影响）。
pub fn bridge_enabled() -> bool {
    match std::env::var("OWO_AGENT_BRIDGE_ENABLED") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    if !bridge_enabled() {
        return Router::new().with_state(state);
    }
    Router::new()
        .route("/agent-bridge/messages", post(handle_message))
        .route("/agent-bridge/tasks/{id}/status", get(task_status))
        .route(
            "/agent-bridge/preferences",
            get(get_preferences).post(set_preferences),
        )
        .with_state(state)
}
