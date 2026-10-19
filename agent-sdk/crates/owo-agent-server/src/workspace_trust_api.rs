//! 工作区信任 HTTP API（审查 N1）。
//!
//! 路由面：
//! - `GET /workspace/trust`：返回当前信任状态、配置指纹、危险配置清单；
//! - `POST /workspace/trust` `{ "trusted": true|false }`：信任/撤销信任。
//!
//! 信任是**用户显式动作**（桌面端弹窗/CLI），不是工作区自报；记录写在数据根
//! `trust.json`（与 `grant_store`/`approval.json` 同一数据面），仓库文件无法自授权。
//! 信任后即时加载工作区 hooks/权限/白名单（与启动同口径）；插件 MCP 子进程
//! 只在核心启动时拉起，需要重启核心才生效。
//!
//! 独立编译约束（沿 notes_api/subagent_api 惯例）：不使用 `crate::`/`super::`；
//! 引用 server 类型一律写全限定名 `owo_agent_server::AppState`。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

fn status_json(state: &AppState, status: &owo_agent_core::trust::TrustStatus, ok: bool) -> Value {
    json!({
        "ok": ok,
        "workspace": status.workspace,
        "trusted": status.trusted,
        "changed": status.changed,
        "gated": status.gated,
        "gated_at_start": state.workspace_trust_gated,
        "fingerprint": status.fingerprint,
        "recorded_fingerprint": status.recorded_fingerprint,
        "reasons": status.reasons,
        "note": if status.trusted {
            "已信任：hooks/权限/白名单即时生效；插件 MCP 需重启核心后加载"
        } else if status.gated {
            "未信任：已跳过工作区 hooks / 插件 MCP / 权限放宽 / 白名单（受限模式）"
        } else {
            "未信任，但该工作区没有会自动执行的配置（无需确认）"
        },
    })
}

pub(super) async fn workspace_trust_get(State(state): State<Arc<AppState>>) -> Json<Value> {
    let status = state.trust_store.status(&state.workspace);
    Json(status_json(&state, &status, true))
}

#[derive(Deserialize)]
pub(super) struct WorkspaceTrustRequest {
    trusted: bool,
}

pub(super) async fn workspace_trust_set(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkspaceTrustRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = if request.trusted {
        state.trust_store.trust(&state.workspace)
    } else {
        state.trust_store.revoke(&state.workspace)
    }
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;

    apply_workspace_config_after_trust_change(&state, request.trusted);
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "workspace_trust",
            if request.trusted { "grant" } else { "revoke" },
            None,
            Some(request.trusted),
            format!(
                "工作区信任{}（指纹 {}；危险配置：{}）",
                if request.trusted {
                    "已授予"
                } else {
                    "已撤销"
                },
                &status.fingerprint[..status.fingerprint.len().min(16)],
                if status.reasons.is_empty() {
                    "无".to_string()
                } else {
                    status.reasons.join("；")
                }
            ),
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Settings);
    Ok(Json(status_json(&state, &status, true)))
}

/// 信任状态变化后的即时生效：信任 → 加载工作区 hooks/权限/白名单；撤销 → 清空 hooks。
///
/// 与 `AppState::new` 的启动装载同口径；插件 MCP 进程只在启动时创建，不做热连接
/// （避免"点一下信任"隐式拉起任意子进程）。
fn apply_workspace_config_after_trust_change(state: &AppState, trusted: bool) {
    if !trusted {
        state
            .agent
            .set_hooks(owo_agent_core::hooks::HookManager::from_configs(&[]));
        return;
    }
    let settings = owo_agent_core::Settings::load(&state.workspace);
    state
        .agent
        .set_hooks(owo_agent_core::hooks::HookManager::from_configs(
            &settings.hooks,
        ));
    state
        .agent
        .apply_policy_settings(settings.read_only, &settings.deny_commands);
    if !settings.read_only {
        if let Some(profile) = settings
            .permission_profile
            .as_deref()
            .and_then(owo_agent_core::PermissionProfile::parse)
        {
            state.agent.set_permission_profile(profile);
        }
    }
    match settings.permission_spec.clone() {
        Some(spec) => state.agent.policy().set_spec(spec),
        None => state.agent.policy().clear_spec(),
    }
}
