//! ChangeSet 审批、接受与安全撤销 HTTP 路由（八期 · 二路）。
//!
//! 路由面（经 [`crate::workswarm_api::router`] 合并挂载）：
//! - `GET  /teams/{id}/change-sets`：团队 ChangeSet 列表 + 批准门控状态；
//! - `GET  /change-sets/{id}`：单个 ChangeSet；
//! - `POST /change-sets/{id}/accept`：接受（保留文件现状；accept 后允许批准该团队
//!   代码 Artifact 成为最终 approved head）；
//! - `POST /change-sets/{id}/reject`：拒绝（恢复该 ChangeSet 修改的文件）；
//! - `POST /change-sets/{id}/revert`：撤销（同恢复语义）。
//!
//! 契约要点：
//! - 请求体 `idempotency_key` 必填（缺失走 axum Json extractor 422）；同动作重放
//!   零副作用（`replayed: true` 返回现状）；已决定后跨动作 → 409；
//! - reject/revert 恢复前逐文件比对当前哈希：用户改过 → 409 + `conflicted`
//!   （不覆盖用户新内容）；恢复成功才落终态；
//! - 全部决定写审计（`change_set.created/accepted/rejected/reverted/conflict`）。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::Json;
use owo_agent_core::change_set::restore_change_set;
use owo_agent_core::change_set_store::{ChangeSetAction, ChangeSetStore, ChangeSetStoreError};
use owo_agent_core::workswarm::TeamCoordinator;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::error_response;
use crate::AppState;

/// 决定请求体（accept/reject/revert 共用）。
#[derive(Debug, Deserialize)]
pub(crate) struct ChangeSetDecisionRequest {
    idempotency_key: String,
    /// 备注（可选；写入决定记录与审计）。
    #[serde(default)]
    note: Option<String>,
}

fn store_error(error: ChangeSetStoreError) -> (StatusCode, Json<Value>) {
    match error {
        ChangeSetStoreError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "ChangeSet 不存在" })),
        ),
        ChangeSetStoreError::Conflict(message) => {
            (StatusCode::CONFLICT, Json(json!({ "error": message })))
        }
        ChangeSetStoreError::Storage(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": message })),
        ),
    }
}

fn json_error(status: StatusCode, message: String) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message })))
}

/// 写审计（低频操作：锁内联记录；审计失败不影响主流程）。
fn audit(
    coordinator: &Arc<TeamCoordinator>,
    team_id: &str,
    event: &str,
    success: bool,
    detail: String,
) {
    if let Some(log) = coordinator.audit_log() {
        if let Ok(mut audit) = log.lock() {
            audit.record(
                team_id,
                event,
                Some(format!("workswarm/{team_id}")),
                Some(success),
                detail,
            );
        }
    }
}

/// ChangeSet 所属工作区根：绑定根优先，未绑定回退全局工作区。
fn workspace_root(
    state: &Arc<AppState>,
    coordinator: &Arc<TeamCoordinator>,
    team_id: &str,
) -> PathBuf {
    super::project_workspace::load_binding(coordinator.run_dir(), team_id)
        .map(|binding| binding.scope().root)
        .unwrap_or_else(|| state.workspace.clone())
}

/// GET /teams/{id}/change-sets：团队 ChangeSet 列表 + 批准门控状态。
pub(crate) async fn list_team_change_sets(
    State(state): State<Arc<AppState>>,
    AxumPath(team_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let store = ChangeSetStore::new(coordinator.run_dir());
    let records = store.list_for_team(&team_id).map_err(store_error)?;
    let block_reason = ChangeSetStore::approval_block_reason(&records);
    Ok(Json(json!({
        "team_id": team_id,
        "approval_blocked": block_reason.is_some(),
        "approval_block_reason": block_reason,
        "change_sets": records,
    })))
}

/// GET /change-sets/{id}：单个 ChangeSet。
pub(crate) async fn get_change_set(
    State(state): State<Arc<AppState>>,
    AxumPath(change_set_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let store = ChangeSetStore::new(coordinator.run_dir());
    let change_set = store
        .find(&change_set_id)
        .map_err(store_error)?
        .ok_or_else(|| {
            json_error(
                StatusCode::NOT_FOUND,
                format!("ChangeSet {change_set_id} 不存在"),
            )
        })?;
    Ok(Json(json!({ "change_set": change_set })))
}

/// POST /change-sets/{id}/accept：接受（保留文件现状，无文件操作）。
pub(crate) async fn accept_change_set(
    State(state): State<Arc<AppState>>,
    AxumPath(change_set_id): AxumPath<String>,
    Json(request): Json<ChangeSetDecisionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    decide(state, change_set_id, ChangeSetAction::Accept, request).await
}

/// POST /change-sets/{id}/reject：拒绝并恢复该 ChangeSet 修改的文件。
pub(crate) async fn reject_change_set(
    State(state): State<Arc<AppState>>,
    AxumPath(change_set_id): AxumPath<String>,
    Json(request): Json<ChangeSetDecisionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    decide(state, change_set_id, ChangeSetAction::Reject, request).await
}

/// POST /change-sets/{id}/revert：撤销并恢复该 ChangeSet 修改的文件。
pub(crate) async fn revert_change_set(
    State(state): State<Arc<AppState>>,
    AxumPath(change_set_id): AxumPath<String>,
    Json(request): Json<ChangeSetDecisionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    decide(state, change_set_id, ChangeSetAction::Revert, request).await
}

/// ChangeSet 人工接受后，若团队先前停在交付门等待人工处理，则重跑唯一 DeliveryGate。
async fn finalize_waiting_delivery(
    coordinator: &Arc<TeamCoordinator>,
    state: &Arc<AppState>,
    team_id: &str,
    workspace_root: &Path,
) {
    let Ok(team) = coordinator.get_team_run(team_id).await else {
        return;
    };
    if team.status != owo_agent_protocol::TeamRunStatus::AwaitingHuman {
        return;
    }
    let Ok(run_state) = coordinator.load_run_state(team_id) else {
        return;
    };
    if !run_state
        .goal
        .error
        .as_deref()
        .is_some_and(|error| error.starts_with("delivery_pending:changeset:"))
    {
        return;
    }
    let _delivery_lease = super::acquire_workspace_delivery_lease(workspace_root).await;
    if let Err(error) = coordinator.finalize_success(team_id).await {
        if matches!(
            &error,
            owo_agent_core::workswarm::WorkSwarmError::DeliveryRepair { .. }
        ) {
            let coordinator = Arc::clone(coordinator);
            let state = Arc::clone(state);
            let id = team_id.to_string();
            tokio::spawn(async move {
                for _ in 0..100 {
                    if !coordinator.is_loop_alive(&id) {
                        super::run_team_loop(state, coordinator, id).await;
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                tracing::warn!(team_id=%id,"返工已登记，原循环尚未退出，请从工作台继续");
            });
            return;
        }
        if !matches!(
            error,
            owo_agent_core::workswarm::WorkSwarmError::DeliveryPending(_)
        ) {
            tracing::error!(team_id, %error, "ChangeSet 已决定，但交付门重试失败");
        }
    }
}

/// 决定编排（accept/reject/revert 共用）：
/// 1) 幂等重放先判（不做任何文件操作、零副作用）；
/// 2) reject/revert 先恢复文件（冲突 → conflicted + 409，不覆盖用户内容）；
/// 3) `apply_decision` 落状态（内部二次校验终态/幂等，写决定记录）。
async fn decide(
    state: Arc<AppState>,
    change_set_id: String,
    action: ChangeSetAction,
    request: ChangeSetDecisionRequest,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let store = ChangeSetStore::new(coordinator.run_dir());
    let existing = store
        .find(&change_set_id)
        .map_err(store_error)?
        .ok_or_else(|| {
            json_error(
                StatusCode::NOT_FOUND,
                format!("ChangeSet {change_set_id} 不存在"),
            )
        })?;

    // 1) 幂等重放：同动作 → 零副作用返回现状（reject/revert 也不做文件操作）。
    if let Some(decision) = &existing.decision {
        if decision.action == action.as_str() {
            let root = workspace_root(&state, &coordinator, &existing.team_id);
            finalize_waiting_delivery(&coordinator, &state, &existing.team_id, &root).await;
            return Ok(Json(json!({
                "replayed": true,
                "change_set": existing,
            })));
        }
        return Err(json_error(
            StatusCode::CONFLICT,
            format!(
                "ChangeSet {change_set_id} 已 {}，不能再 {}",
                decision.action,
                action.as_str()
            ),
        ));
    }
    if !matches!(
        existing.status,
        owo_agent_protocol::ChangeSetStatus::PendingReview
            | owo_agent_protocol::ChangeSetStatus::Conflicted
    ) {
        return Err(json_error(
            StatusCode::CONFLICT,
            format!(
                "ChangeSet {change_set_id} 状态为 {}，不接受 {}",
                existing.status_label(),
                action.as_str()
            ),
        ));
    }

    // 2) reject/revert：先恢复文件（冲突 → conflicted + 409，不覆盖用户内容）。
    if !matches!(action, ChangeSetAction::Accept) {
        let root = workspace_root(&state, &coordinator, &existing.team_id);
        let _workspace_lease = super::acquire_workspace_delivery_lease(&root).await;
        let report = restore_change_set(&root, &existing, coordinator.cas()).await;
        drop(_workspace_lease);
        if !report.conflicts.is_empty() {
            let updated = store
                .mark_conflicted(&change_set_id, &report.conflicts)
                .map_err(store_error)?;
            audit(
                &coordinator,
                &updated.team_id,
                "change_set.conflict",
                false,
                format!(
                    "{} {} 检测到用户改动，未覆盖：{}",
                    action.as_str(),
                    change_set_id,
                    report.conflicts.join(", ")
                ),
            );
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!(
                        "文件已被用户修改，未覆盖（请人工处理后再试）：{}",
                        report.conflicts.join(", ")
                    ),
                    "conflicts": report.conflicts,
                    "change_set": updated,
                })),
            ));
        }
    }

    // 3) 落决定（内部二次校验；并发窗口内的跨动作竞争 → 409）。
    let outcome = store
        .apply_decision(
            &change_set_id,
            action,
            &request.idempotency_key,
            request.note.as_deref(),
        )
        .map_err(store_error)?;
    audit(
        &coordinator,
        &outcome.change_set.team_id,
        &format!("change_set.{}", action.as_str()),
        true,
        format!(
            "{} {}（步骤 {}，文件：{}）{}",
            action.as_str(),
            change_set_id,
            outcome.change_set.step_id,
            outcome.change_set.changed_files.join(", "),
            if outcome.replayed {
                "·幂等重放"
            } else {
                ""
            }
        ),
    );
    let root = workspace_root(&state, &coordinator, &outcome.change_set.team_id);
    finalize_waiting_delivery(&coordinator, &state, &outcome.change_set.team_id, &root).await;
    Ok(Json(json!({
        "replayed": outcome.replayed,
        "change_set": outcome.change_set,
    })))
}
