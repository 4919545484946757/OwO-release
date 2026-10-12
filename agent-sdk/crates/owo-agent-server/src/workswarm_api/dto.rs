use axum::http::StatusCode;
use axum::Json;
use owo_agent_core::workswarm::{TeamCoordinator, WorkSwarmError};
use serde_json::{json, Value};
use std::sync::Arc;
// ---------------------------------------------------------------------------
// 响应辅助
// ---------------------------------------------------------------------------

pub(crate) fn error_response(e: &WorkSwarmError) -> (StatusCode, Json<Value>) {
    let (code, msg) = match e {
        WorkSwarmError::Validation(m) => (StatusCode::BAD_REQUEST, m.clone()),
        WorkSwarmError::Conflict(m) | WorkSwarmError::DeliveryPending(m) => {
            (StatusCode::CONFLICT, m.clone())
        }
        WorkSwarmError::DeliveryRepair { reason, .. } => (StatusCode::CONFLICT, reason.clone()),
        WorkSwarmError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
        // R2：状态文件损坏 → 明确 500 失败；原文件已保留、未被覆盖（消息中注明），
        // 不返回可重试语义——需人工修复后才能继续。
        WorkSwarmError::CorruptState(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.clone()),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    };
    (code, Json(json!({ "error": msg })))
}

pub(crate) fn task_view(state: &owo_agent_core::goal::GoalRunState) -> Vec<Value> {
    state
        .plan
        .steps
        .iter()
        .map(|s| {
            let rec = state.records.get(&s.id);
            json!({
                "task_id": s.id,
                "worker": s.worker,
                "role": s.worker.strip_prefix("m-").unwrap_or(&s.worker),
                "depends_on": s.depends_on,
                "status": format!("{:?}", rec.map(|r| r.status).unwrap_or(
                    owo_agent_core::plan::StepStatus::Pending
                )),
                "attempts": rec.map(|r| r.attempts).unwrap_or(0),
                "error": rec.and_then(|r| r.error.clone()),
            })
        })
        .collect()
}

/// 团队审计尾迹（最近 20 条；S0 可见性：关键动作可回看）。
pub(crate) fn audit_tail(coordinator: &Arc<TeamCoordinator>, team_id: &str) -> Vec<Value> {
    let Some(log) = coordinator.audit_log() else {
        return Vec::new();
    };
    let Ok(entries) = log.lock() else {
        return Vec::new();
    };
    entries
        .entries
        .iter()
        .filter(|e| e.session_id == team_id)
        .rev()
        .take(20)
        .map(|e| json!({ "ts": e.ts, "event": e.event, "tool": e.tool, "detail": e.detail }))
        .collect()
}

// ---------------------------------------------------------------------------
// 路由
// ---------------------------------------------------------------------------
