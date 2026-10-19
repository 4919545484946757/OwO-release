//! 会话级目标模式 HTTP API（CLI `/goal` 的持久化落点）。
//!
//! 路由面：`POST /session/{id}/goal`，请求体 `{"action": "...", "objective": "..."}`：
//! - `set`（默认，`objective` 必填）：设定/替换目标（轮次与完成标记复位）；
//! - `advance`：推进一轮；`done`：标记完成；`clear`：清除目标。
//!
//! 状态落在会话上并持久化（core `SessionGoal` + SQLite migration v10），
//! 因此重开进程 `/resume` 后目标与进度不丢。路由只做状态迁移，不驱动回合：
//! 推进与完成判定仍由调用方（CLI 目标循环 + 宿主完成裁决）发起。
//!
//! 独立编译约束（沿 session_api 惯例）：不使用 `crate::`/`super::`；引用 server
//! 类型一律写全限定名 `owo_agent_server::AppState`（根私有项如
//! `acquire_session_lock` 经 crate 名全限定访问——本模块为其后代可见）。

use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use owo_agent_core::Session;
use owo_agent_protocol::SessionGoalV1;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

/// 模块内 `poison` 副本（session_api 惯例：共享根助手保持各域独立可用）。
fn poison<T>(_error: std::sync::PoisonError<T>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, "状态锁中毒".to_string())
}

/// 请求体：`action` 缺省 `set`（与 `/goal <目标>` 同观感）。
#[derive(Debug, Default, Deserialize)]
pub(super) struct SessionGoalRequest {
    /// `set`（默认）| `advance` | `done` | `clear`。
    #[serde(default)]
    pub action: Option<String>,
    /// `action=set` 时的目标描述。
    #[serde(default)]
    pub objective: Option<String>,
}

/// core 会话目标 → 线格式 DTO；`SessionInfo.goal` 与本路由响应共用同一映射。
pub(super) fn goal_to_wire(session: &Session) -> Option<SessionGoalV1> {
    session.goal.as_ref().map(|goal| SessionGoalV1 {
        objective: goal.objective.clone(),
        iterations: goal.iterations,
        done: goal.done,
        updated_at: goal.updated_at.clone(),
        origin: goal.origin.clone(),
    })
}

pub(super) async fn session_goal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<SessionGoalRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = owo_agent_server::session_api::load_session(&state, &id)?;
    let action = request
        .action
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("set")
        .to_ascii_lowercase();
    let objective = request
        .objective
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    match action.as_str() {
        "set" => {
            if objective.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "action=set 需要非空 objective".to_string(),
                ));
            }
            session.set_goal(objective);
        }
        "advance" => {
            if session.advance_goal().is_none() {
                return Err((StatusCode::NOT_FOUND, "会话未设定目标".to_string()));
            }
        }
        "done" | "complete" => {
            if session.complete_goal().is_none() {
                return Err((StatusCode::NOT_FOUND, "会话未设定目标".to_string()));
            }
        }
        "clear" => session.clear_goal(),
        other => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("未知 action：{other}（可用 set/advance/done/clear）"),
            ))
        }
    }
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
        "session_id": id,
        "action": action,
        "goal": goal_to_wire(&session),
    })))
}
