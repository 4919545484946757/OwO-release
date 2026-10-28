//! 内置团队模板目录 HTTP 面（六期 · 第三路）。
//!
//! 两条路由：
//! - `GET /teams/templates/catalog`——内置模板候选目录（含 `installed` 标志、
//!   角色/DAG/预算/完成条件/工具范围预览）。目录只展示候选：未安装模板不进入
//!   `TeamTemplateRegistry`，自动匹配（`find_match`）不会命中——「自动模式只
//!   匹配已安装模板」由注册表机制保证。
//! - `POST /teams/templates/catalog/{id}/install`——用户手动安装（幂等、非破坏：
//!   已存在同名模板时返回现状不覆盖，避免冲掉用户定制；安装只写注册表的
//!   模板修订，不触碰任何权限/设置面——不自动扩大文件、命令或
//!   网络权限）。
//!
//! 模块纪律（usage.rs 先例）：不引用 `crate::`/`super::`，AppState 全限定
//! `owo_agent_server::AppState`——可被测试以 `#[path] mod` 独立编译；正式路由
//! 合并由第四路在 build_router 接线（server lib.rs 归第四路）。
//!
//! 注意：目录/安装刻意**不初始化** WorkSwarm 协调器（轻量 `TeamTemplateRegistry`
//! 直读同目录 `data_root/workswarm`；注册表无缓存，读写一致），避免目录浏览
//! 产生 space.db/CAS 等重副作用。

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use owo_agent_core::builtin_team_templates::{self, BuiltinTemplateDescriptor};
use owo_agent_core::workswarm::TeamTemplateRegistry;
use owo_agent_server::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// The coordinator and catalog share this registry root. Legacy templates/templates/*.json
/// are imported once into templates/workflow-templates.sqlite3, retaining the original files.
fn registry(state: &AppState) -> TeamTemplateRegistry {
    TeamTemplateRegistry::for_workswarm_root(&state.data_root.join("workswarm"))
}

fn error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.into() })))
}

fn descriptor_json(d: &BuiltinTemplateDescriptor, installed: bool) -> Value {
    json!({
        "template": d.template,
        "artifact_kinds": d.artifact_kinds,
        "budget_calls_per_role": d.budget_calls_per_role,
        "budget": d.budget,
        "completion_criteria": d.completion_criteria,
        "tool_scope": d.tool_scope,
        "auto_match_keywords": builtin_team_templates::auto_match_keywords(&d.template.applicability),
        "installed": installed,
    })
}

/// GET /teams/templates/catalog：内置模板候选目录（只展示，不自动启用）。
async fn catalog(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let registry = registry(&state);
    let entries: Vec<Value> = builtin_team_templates::catalog()
        .iter()
        .map(|d| {
            let record = registry
                .get_revision(&d.template.template_id)
                .map_err(owo_agent_server::workflow_write_service::store_error)?;
            let installed = record.as_ref().is_some_and(|r| r.template.is_some());
            let mut entry = descriptor_json(d, installed);
            entry["revision"] = json!(record.as_ref().map(|r| r.revision).unwrap_or(0));
            entry["content_hash"] = json!(record.and_then(|r| r.content_hash));
            Ok(entry)
        })
        .collect::<Result<Vec<_>, (StatusCode, Json<Value>)>>()?;
    let installed_count = entries
        .iter()
        .filter(|e| e["installed"] == Value::Bool(true))
        .count();
    Ok(Json(json!({
        "catalog": entries,
        "installed_count": installed_count,
        "total": entries.len(),
        "note": "目录只展示候选：安装后才能参与自动匹配与显式 template_id 建队；安装幂等且不自动扩大文件/命令/网络权限",
    })))
}

/// POST /teams/templates/catalog/{id}/install：安装内置模板（幂等、非破坏）。
///
/// - 首次安装：保存模板修订 → `{installed: true}`（此后该模板可
///   参与自动匹配与显式 template_id 建队）。
/// - 重复安装：已存在同名模板（可能是用户定制版）→ 不覆盖，返回现状
///   `{installed: false, already_installed: true, template: 现状}`。
/// - 未知 id → 404。
/// - 安装不触碰任何权限/设置面（只写模板注册表）。
#[derive(Default, Deserialize)]
struct InstallQuery {
    expected_revision: Option<u64>,
}

async fn install(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<InstallQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let Some(descriptor) = builtin_team_templates::descriptor(&id) else {
        return Err(error(
            StatusCode::NOT_FOUND,
            format!(
                "未知内置模板「{id}」；候选目录见 GET /teams/templates/catalog（{}）",
                builtin_team_templates::CATALOG_IDS.join("、")
            ),
        ));
    };
    let registry = registry(&state);
    let (saved, installed) = registry
        .install_if_absent(&descriptor.template, query.expected_revision)
        .map_err(owo_agent_server::workflow_write_service::store_error)?;
    Ok(Json(json!({
        "installed": installed, "already_installed": !installed,
        "template": saved.template, "revision": saved.revision, "content_hash": saved.content_hash,
        "auto_match": "已可参与自动匹配与显式 template_id 建队；安装不扩大权限",
        "budget_hint": descriptor.budget,
    })))
}

/// 模板目录路由（第四路接线：`team_template_catalog_api::team_template_catalog_router(state)`）。
pub fn team_template_catalog_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/teams/templates/catalog", get(catalog))
        .route("/teams/templates/catalog/{id}/install", post(install))
        .with_state(state)
}
