//! 会话元数据 HTTP API（§12：从 lib.rs 机械外移的 session 元数据域·第一刀）。
//!
//! 路由面（`GET /sessions`、`POST /session`、`GET /session/{id}`、
//! `POST /session/{id}/rename|archive|pin|model`、`GET /session/{id}/children`）。
//! M4.2：`/model` 为新增的会话级模型路由入口；创建请求的 `model` 字段语义
//! 升级为真实路由（进 `model_override` 并落库），未指定保持自动。
//! 回合执行（turn/diff/revert/…）仍留 lib.rs，经本模块共享
//! `to_session_info`/`load_session`，语义不变。
//!
//! 独立编译约束（沿 notes_api 惯例）：不使用 `crate::`/`super::`；引用 server
//! 类型一律写全限定名 `owo_agent_server::AppState`（根私有项如
//! `acquire_session_lock` 经 crate 名全限定访问——本模块为其后代可见）。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use owo_agent_core::session::Session;
use owo_agent_protocol::{CreateSessionRequest, SessionInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use owo_agent_server::AppState;

/// 模块内 `poison` 副本（§12 惯例：共享根助手保持各域独立可用）。
fn poison<T>(_error: std::sync::PoisonError<T>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, "状态锁中毒".to_string())
}

pub(super) fn to_session_info(session: &Session) -> SessionInfo {
    SessionInfo {
        id: session.id.clone(),
        workspace: session.workspace.to_string_lossy().into_owned(),
        model: session.model.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        title: Some(session.display_title()),
        archived: session.archived,
        pinned: session.pinned,
        parent_id: session.parent_id.clone(),
        fork_point: session.fork_point,
        // 目标状态映射归 `session_goal_api` 所有（一个功能一个文件）。
        goal: owo_agent_server::session_goal_api::goal_to_wire(session),
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RevertRequest {
    /// 可选执行收据；省略时消费最近一张未撤销收据，旧会话回退兼容快照撤销。
    #[serde(default)]
    pub receipt_id: Option<String>,
}

pub(super) fn load_session(state: &AppState, id: &str) -> Result<Session, (StatusCode, String)> {
    if let Ok(sessions) = state.sessions.lock() {
        if let Some(session) = sessions.get(id) {
            return Ok(session.clone());
        }
    }
    state.store.load(id).map_err(|_error| {
        // 不拼接底层错误原文：`store.load` 的失败原因本身就是"会话不存在"，
        // 拼上去会得到「会话不存在：X（session error: 会话不存在：X）」这种自我复读，
        // 用户看不出多出来的那层究竟提供了什么新信息（实测报错就是这样）。
        (StatusCode::NOT_FOUND, format!("会话不存在：{id}"))
    })
}

/// `DELETE /session/{id}`——删除会话（存储 + 内存缓存 + 事件流广播失效）。
///
/// 先 `load_session` 确认存在：直接删不存在的会话会得到"删了 0 行却回报成功"，
/// 而 `SessionStore::remove` 的契约是"不存在 → Err"，两者必须一致。
pub(super) async fn delete_session(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session = load_session(&state, &id)?;
    state
        .store
        .remove(&id)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    // 内存缓存里可能仍持有它（运行期会话不落盘也在列表中），必须同步清掉，
    // 否则"删完还在列"。
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.remove(&id);
    }
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            &id,
            "session_delete",
            None,
            Some(true),
            format!("删除会话（工作区 {}）", session.workspace.display()),
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(json!({ "ok": true, "id": id })))
}

pub(super) async fn list_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SessionInfo>>, (StatusCode, String)> {
    let mut sessions = Vec::new();
    for session_id in state.store.list() {
        if let Ok(session) = state.store.load(&session_id) {
            sessions.push(to_session_info(&session));
        }
    }
    sessions.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
    });
    Ok(Json(sessions))
}

pub(super) async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<SessionInfo>, (StatusCode, String)> {
    let workspace = PathBuf::from(&request.workspace);
    if !workspace.is_dir() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("工作区不存在：{}", request.workspace),
        ));
    }
    // M4.2 会话级模型路由：显式请求的模型进入 `model_override`（真正参与请求体）；
    // 未显式指定（含 `"default"` 哨兵归一化）时按 Provider 解析链自动选择
    // （OPENAI_MODEL 热切换 → 启动配置 → 内置默认），`model` 字段仅为展示值。
    let explicit = request
        .model
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v != owo_agent_core::gateway::MODEL_DEFAULT_SENTINEL);
    let model = explicit.clone().unwrap_or_else(|| {
        std::env::var("OPENAI_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| owo_agent_core::gateway::DEFAULT_MODEL_ID.to_string())
    });
    let session = state
        .store
        .create(&workspace, &model, request.system_prompt.as_deref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .with_model_override(explicit);
    state
        .store
        .save(&session)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(poison)?
        .insert(session.id.clone(), session.clone());
    let title = session.display_title();
    let goal = owo_agent_server::session_goal_api::goal_to_wire(&session);
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(SessionInfo {
        id: session.id,
        workspace: request.workspace,
        model: session.model,
        created_at: session.created_at,
        updated_at: session.updated_at,
        title: Some(title),
        archived: session.archived,
        pinned: session.pinned,
        parent_id: session.parent_id,
        fork_point: session.fork_point,
        goal,
    }))
}

pub(super) async fn get_session(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session = load_session(&state, &id)?;
    let history = state.store.turn_history(&id);
    let history_error = history.as_ref().err().map(|_| "history/unavailable");
    let active_turn_id = state.turn_controls.lock().map_err(poison)?.active_id(&id);
    let activity = state.activities.lock().map_err(poison)?.get(&id).cloned();
    Ok(Json(json!({
        "turn_history": history.ok().flatten(),
        "turn_history_error": history_error,
        "active_turn_id": active_turn_id,
        "activity": activity,
        "id": session.id,
        "title": session.display_title(),
        "model": session.model,
        // M4.2：路由真相 = model_override（null = 按 Provider 解析链自动）。
        "model_override": session.model_override,
        "workspace": session.workspace.to_string_lossy(),
        "created_at": session.created_at,
        "updated_at": session.updated_at,
        "archived": session.archived,
        "pinned": session.pinned,
        "parent_id": session.parent_id,
        "fork_point": session.fork_point,
        "todos": session.todos,
        // 目标模式状态：客户端 `/resume` 据此恢复目标与已推进轮次。
        "goal": owo_agent_server::session_goal_api::goal_to_wire(&session),
        "messages": session.messages,
    })))
}

#[derive(Deserialize)]
pub(super) struct RenameRequest {
    title: String,
}

#[derive(Deserialize)]
pub(super) struct ArchiveRequest {
    archived: bool,
}

#[derive(Deserialize)]
pub(super) struct PinRequest {
    pinned: bool,
}

#[derive(Deserialize)]
pub(super) struct SetModelRequest {
    /// M4.2 会话级模型路由：非空字符串固定请求模型；`null`/空串/`"default"`
    /// 哨兵 = 清除覆盖，回退 Provider 解析链（哨兵值不落库、不进请求体）。
    model: Option<String>,
}

pub(super) async fn session_set_model(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<SetModelRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = load_session(&state, &id)?;
    session.set_model_override(request.model);
    state
        .store
        .save(&session)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(poison)?
        .insert(session.id.clone(), session.clone());
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(json!({
        "id": session.id,
        "model": session.model,
        "model_override": session.model_override,
    })))
}

pub(super) async fn session_rename(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<RenameRequest>,
) -> Result<Json<SessionInfo>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = load_session(&state, &id)?;
    session.rename(request.title);
    state
        .store
        .save(&session)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(poison)?
        .insert(session.id.clone(), session.clone());
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(to_session_info(&session)))
}

pub(super) async fn session_archive(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<ArchiveRequest>,
) -> Result<Json<SessionInfo>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = load_session(&state, &id)?;
    session.set_archived(request.archived);
    state
        .store
        .save(&session)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(poison)?
        .insert(session.id.clone(), session.clone());
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(to_session_info(&session)))
}

pub(super) async fn session_pin(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<PinRequest>,
) -> Result<Json<SessionInfo>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = load_session(&state, &id)?;
    session.set_pinned(request.pinned);
    state
        .store
        .save(&session)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(poison)?
        .insert(session.id.clone(), session.clone());
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(to_session_info(&session)))
}

pub(super) async fn children(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<SessionInfo>>, (StatusCode, String)> {
    let mut result = Vec::new();
    for session_id in state.store.list() {
        if let Ok(session) = state.store.load(&session_id) {
            if session.parent_id.as_deref() == Some(id.as_str()) {
                result.push(to_session_info(&session));
            }
        }
    }
    Ok(Json(result))
}

// ---------- §12 第二刀：附件 / 中止 / diff / 回滚 / fork / rewind / redo / 导出 ----------

pub(super) fn attachment_dir(workspace: &std::path::Path, session_id: &str) -> std::path::PathBuf {
    workspace.join(".owo-attachments").join(session_id)
}

pub(super) fn sanitize_attachment_name(name: &str) -> Option<String> {
    let file_name = std::path::Path::new(name).file_name()?.to_str()?;
    let cleaned: String = file_name
        .chars()
        .filter(|character| {
            !matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        })
        .collect();
    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() || trimmed.len() > 200 {
        None
    } else {
        Some(trimmed)
    }
}

#[derive(Deserialize)]
pub(super) struct AttachmentUploadRequest {
    name: String,
    #[serde(default)]
    mime: Option<String>,
    data_b64: String,
}

pub(super) async fn attachment_upload(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AttachmentUploadRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let session = load_session(&state, &id)?;
    let safe_name = sanitize_attachment_name(&request.name)
        .ok_or((StatusCode::BAD_REQUEST, "附件名非法".to_string()))?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&request.data_b64)
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                format!("附件 base64 解码失败：{error}"),
            )
        })?;
    if bytes.len() > 50 * 1024 * 1024 {
        return Err((StatusCode::BAD_REQUEST, "附件超过 50MB 上限".to_string()));
    }
    let dir = attachment_dir(&session.workspace, &id);
    std::fs::create_dir_all(&dir)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let path = dir.join(&safe_name);
    std::fs::write(&path, &bytes)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            &id,
            "attachment",
            Some(safe_name.clone()),
            Some(true),
            format!("上传附件 {}（{} 字节）", safe_name, bytes.len()),
        );
    }
    Ok(Json(json!({
        "id": safe_name,
        "name": request.name,
        "mime": request.mime,
        "size": bytes.len(),
        "path": path.to_string_lossy(),
    })))
}

pub(super) async fn attachments_list(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<Value>>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let session = load_session(&state, &id)?;
    let dir = attachment_dir(&session.workspace, &id);
    let mut attachments = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            attachments.push(json!({ "id": name, "name": name, "size": size }));
        }
    }
    attachments.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    Ok(Json(attachments))
}

pub(super) async fn abort_turn(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, String)> {
    load_session(&state, &id)?;
    if body.len() > 4096 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "turn/cancel_body_too_large".into(),
        ));
    }
    #[derive(Default, serde::Deserialize)]
    struct CancelRequest {
        turn_id: Option<String>,
    }
    let request: CancelRequest = if body.is_empty() {
        CancelRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "turn/invalid_cancel_request".to_string(),
            )
        })?
    };
    let expected = request
        .turn_id
        .as_deref()
        .map(|id| {
            owo_agent_protocol::canonical_turn_id(id)
                .ok_or_else(|| (StatusCode::BAD_REQUEST, "turn/invalid_id".to_string()))
        })
        .transpose()?;
    let already_recorded = if let Some(expected) = &expected {
        !state
            .store
            .turn_events_after(&id, Some(expected), 0, 1)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .is_empty()
    } else {
        false
    };
    let mut controls = state.turn_controls.lock().map_err(poison)?;
    if already_recorded && expected.as_deref() != controls.active_id(&id).as_deref() {
        return Ok(Json(
            json!({ "ok":true, "state":"already_finished", "turn_id":expected }),
        ));
    }
    let disposition = controls
        .cancel(&id, expected.as_deref())
        .map_err(|code| (StatusCode::SERVICE_UNAVAILABLE, code.to_string()))?;
    Ok(Json(json!({
        "ok":true,
        "turn_id":expected,
        "state":match disposition {
            owo_agent_server::turn_control::CancelDisposition::Requested => "cancellation_requested",
            owo_agent_server::turn_control::CancelDisposition::Queued => "cancellation_queued",
            owo_agent_server::turn_control::CancelDisposition::NoActiveTurn => "no_active_turn",
        }
    })))
}

pub(super) async fn diff(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<owo_agent_protocol::FileDiff>>, (StatusCode, String)> {
    let session = load_session(&state, &id)?;
    Ok(Json(session.diff()))
}

pub(super) async fn revert(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    request: Option<Json<RevertRequest>>,
) -> axum::response::Response {
    let _session_guard = match owo_agent_server::acquire_session_lock(&state, &id).await {
        Ok(guard) => guard,
        Err((status, message)) => return session_revert_error(status, message),
    };
    let mut session = match load_session(&state, &id) {
        Ok(session) => session,
        Err((status, message)) => return session_revert_error(status, message),
    };
    let receipt_id = request.and_then(|Json(body)| body.receipt_id);
    let restored = match session.revert_receipt(receipt_id.as_deref()).await {
        Ok(restored) => restored,
        Err(owo_agent_core::AgentError::RevertConflict { paths }) => {
            let code = owo_agent_server::error_codes::code("storage", "revert_conflict", false);
            return owo_agent_server::api_error_response(
                &code,
                format!(
                    "撤销冲突，以下文件在 Agent 写入后再次变化：{}",
                    paths.join("、")
                ),
            )
            .into_response();
        }
        Err(error) => {
            let code = owo_agent_server::error_codes::code("internal", "unexpected", true);
            return owo_agent_server::api_error_response(&code, format!("回滚失败：{error}"))
                .into_response();
        }
    };
    if let Err(error) = state.store.save(&session) {
        let code = owo_agent_server::error_codes::code("internal", "unexpected", true);
        return owo_agent_server::api_error_response(&code, error.to_string()).into_response();
    }
    match state.sessions.lock() {
        Ok(mut sessions) => {
            sessions.insert(session.id.clone(), session);
        }
        Err(_) => {
            return session_revert_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "会话状态锁中毒".to_string(),
            );
        }
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Json(json!({
        "ok": true,
        "restored": restored,
        "receipt_id": receipt_id,
    }))
    .into_response()
}

fn session_revert_error(status: StatusCode, message: String) -> axum::response::Response {
    let (domain, reason, retryable) = match status {
        StatusCode::NOT_FOUND => ("storage", "not_found", false),
        StatusCode::CONFLICT => ("validation", "conflict", false),
        _ => ("internal", "unexpected", true),
    };
    let code = owo_agent_server::error_codes::code(domain, reason, retryable);
    owo_agent_server::api_error_response(&code, message).into_response()
}

pub(super) async fn fork_session(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<owo_agent_protocol::ForkRequest>,
) -> Result<Json<SessionInfo>, (StatusCode, String)> {
    let session = load_session(&state, &id)?;
    let child = session.fork(request.message_index);
    state
        .store
        .save(&child)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(poison)?
        .insert(child.id.clone(), child.clone());
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(to_session_info(&child)))
}

pub(super) async fn rewind_session(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<owo_agent_protocol::RewindRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = load_session(&state, &id)?;
    if request.keep < session.messages.len() {
        // 取优合并（远端 engine）：只回滚被截断段落的写操作（revert_from），
        // 更早回合的快照保留给 /diff 与后续 /revert；必须在 rewind 之前执行
        // （rewind 会清掉被截断段落的快照归属）。
        session.revert_from(request.keep).await.map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("回滚失败：{error}"),
            )
        })?;
    }
    let removed = session.rewind(request.keep);
    state
        .store
        .save(&session)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(poison)?
        .insert(session.id.clone(), session);
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(json!({ "ok": true, "removed": removed.len() })))
}

pub(super) async fn redo_session(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = load_session(&state, &id)?;
    let restored = session.redo().map(|tail| tail.len()).unwrap_or(0);
    state
        .store
        .save(&session)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(poison)?
        .insert(session.id.clone(), session);
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(json!({ "ok": true, "restored": restored })))
}

pub(super) async fn export_session(
    State(state): State<Arc<AppState>>,
    AxumPath((id, format)): AxumPath<(String, String)>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let session = load_session(&state, &id)?;
    let (body, content_type) = match format.as_str() {
        "md" | "markdown" => (
            owo_agent_core::export_markdown(&session),
            "text/markdown; charset=utf-8",
        ),
        "html" => (
            owo_agent_core::export_html(&session),
            "text/html; charset=utf-8",
        ),
        _ => return Err((StatusCode::BAD_REQUEST, "格式仅支持 md / html".to_string())),
    };
    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, content_type)],
        body,
    )
        .into_response())
}

// ---------- 上下文诊断（§12：context_snapshot/session_context 自 lib.rs 机械外移） ----------

/// v0.4：情景快照（感知面 + 剪贴板 + UIA 刷新后返回 snapshot）。
pub(super) async fn context_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<owo_agent_core::perception::SituationSnapshot>, (StatusCode, String)> {
    let mut perception = state.perception.lock().map_err(poison)?;
    let _ = perception.refresh_from_platform();
    let sequence = owo_agent_core::clipboard_sequence();
    perception.refresh_clipboard(sequence);
    let _ = perception.refresh_from_uia(2, 64);
    Ok(Json(perception.snapshot()))
}

/// 会话上下文诊断：消息数/估算 token/预算占用/规则注入/最近压缩摘要。
pub(super) async fn session_context(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session = load_session(&state, &id)?;
    let messages: Vec<owo_agent_core::ChatMessage> = session.messages.clone();
    let estimated = owo_agent_core::estimate_tokens(&messages);
    // 与回合注入同源：层级规则（根 → 进程 cwd）——诊断必须反映实际注入内容，
    // 且列出每一份来源文件（层级发现后"哪几份规则生效"要可解释）。
    let cwd = std::env::current_dir().unwrap_or_else(|_| session.workspace.clone());
    let rule_files =
        owo_agent_core::context::collect_instruction_files(&session.workspace, Some(&cwd));
    let rules = owo_agent_core::context::render_instruction_files(&rule_files);
    let config = state.agent.config();
    let mut last_compaction: Option<String> = None;
    // 反向找最近的压缩摘要（system 消息以"历史摘要"开头）。
    for message in messages.iter().rev() {
        if message.role == "system"
            && message
                .content
                .as_deref()
                .is_some_and(|content| content.starts_with("历史摘要"))
        {
            last_compaction = message.content.clone();
            break;
        }
    }
    Ok(Json(json!({
        "session_id": id,
        "messages": messages.len(),
        "estimated_tokens": estimated,
        "token_budget": config.token_budget,
        "compaction_enabled": config.compaction_enabled,
        "over_budget": estimated > config.token_budget,
        "rules_injected": !rules.is_empty(),
        "rules_chars": rules.chars().count(),
        "rules_files": rule_files
            .iter()
            .map(|file| json!({
                "label": file.label,
                "scope": file.scope,
                "chars": file.content.chars().count(),
                "truncated": file.truncated,
            }))
            .collect::<Vec<_>>(),
        "last_compaction": last_compaction,
    })))
}

/// `POST /session/{id}/compact`：显式压缩会话历史（对齐 Codex `/compact`）。
/// 不依赖 token 预算；历史不足或模型未产出摘要时 `compacted=false`。
pub(super) async fn compact_session(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = load_session(&state, &id)?;
    let tokens_before = owo_agent_core::estimate_tokens(&session.messages);
    let messages_before = session.messages.len();
    let summary = state
        .agent
        .compact_session(&mut session)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let compacted = summary.is_some();
    if compacted {
        state
            .store
            .save(&session)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        state
            .sessions
            .lock()
            .map_err(poison)?
            .insert(session.id.clone(), session.clone());
        owo_agent_server::event_stream::hub()
            .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    }
    Ok(Json(json!({
        "session_id": id,
        "compacted": compacted,
        "summary": summary,
        "tokens_before": tokens_before,
        "tokens_after": owo_agent_core::estimate_tokens(&session.messages),
        "messages_before": messages_before,
        "messages_after": session.messages.len(),
    })))
}

/// ask_user 应答路由（取优合并自远端 engine）：把用户答案送回挂起中的回合。
/// 与审批响应同策略：request_id（question_id）为 uuid v4 且服务仅监听本机，
/// 会话归属只做软校验；提问不存在/已回答一律 404（幂等拒绝重复提交）。
pub(super) async fn respond_question(
    State(state): State<Arc<AppState>>,
    AxumPath((session_id, question_id)): AxumPath<(String, String)>,
    Json(response): Json<owo_agent_core::question::QuestionAnswer>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let sender = state
        .pending_questions
        .lock()
        .map_err(poison)?
        .remove(&question_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("提问不存在或已回答：{question_id}"),
            )
        })?;
    let registered_session = state
        .pending_question_sessions
        .lock()
        .map_err(poison)?
        .remove(&question_id);
    if registered_session.as_deref() != Some(session_id.as_str()) {
        // 软校验：跨会话响应不硬拒绝（与审批一致），但记审计便于溯源。
        if let Ok(mut audit) = state.agent.audit_log().lock() {
            audit.record(
                &session_id,
                "question_session_mismatch",
                None,
                Some(false),
                format!(
                    "提问归属 {} 但由 {session_id} 应答",
                    registered_session.as_deref().unwrap_or("unknown")
                ),
            );
        }
    }
    let _ = sender.send(owo_agent_core::question::QuestionAnswer {
        question_id,
        answer: response.answer,
    });
    Ok(Json(json!({ "ok": true })))
}
