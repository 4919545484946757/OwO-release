//! 用户自定义工作流 HTTP 面（团队模板的"用户/社区"通道）。
//!
//! 四条路由：
//! - `GET /workflows/user`——列出用户自定义与导入的社区模板（含逐条校验结果）；
//! - `POST /workflows/user`——创建/更新（**先校验再落盘**，非法配置 422 附全部原因）；
//! - `POST /workflows/user/validate`——只校验不落盘（UI 实时提示 / AI 生成后的自检）；
//! - `DELETE /workflows/user/{id}`——删除（内置模板 403，不存在 404）。
//!
//! 复用同一份 `TeamTemplateRegistry`（`data_root/workswarm/templates`）：用户模板与内置
//! 模板存在同一注册表，`find_match` 自动匹配对两者一视同仁；因此这里只做
//! origin/校验/保护内置的门禁，不另建存储。
//!
//! 模块纪律（team_template_catalog_api 先例）：不引用 `crate::`/`super::`，
//! AppState 全限定 `owo_agent_server::AppState`。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use owo_agent_core::user_workflow::{
    review_workflow_template, validate_workflow_template, workflow_review_blocked,
};
use owo_agent_core::workswarm::TeamTemplateRegistry;
use owo_agent_protocol::TeamTemplate;
use owo_agent_server::AppState;
use serde_json::{json, Value};
use std::sync::Arc;

/// 用户模板来源：用户自己在 UI/CLI 里创建。
const ORIGIN_USER: &str = "user";
/// 社区包导入。
const ORIGIN_COMMUNITY: &str = "community";

/// 工作流治理动作写审计（社区导入/导出/删除都属于安全相关动作）。
fn audit(
    state: &AppState,
    event: &str,
    tool: Option<String>,
    approved: Option<bool>,
    detail: impl Into<String>,
) {
    if let Ok(mut log) = state.agent.audit_log().lock() {
        log.record("workflows", event, tool, approved, detail);
    }
}

fn registry(state: &AppState) -> TeamTemplateRegistry {
    TeamTemplateRegistry::new(state.data_root.join("workswarm").join("templates"))
}

fn error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.into() })))
}

fn validation_error(errors: Vec<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "ok": false, "errors": errors })),
    )
}

/// 单条模板的运行次数（无运行记录 → 0）。
fn attention_run_count(
    stats: &std::collections::BTreeMap<String, crate::workflow_stats_api::TemplateStats>,
    template: &TeamTemplate,
) -> usize {
    stats
        .get(&template.template_id)
        .map(|entry| entry.runs)
        .unwrap_or(0)
}

fn is_user_visible(template: &TeamTemplate) -> bool {
    matches!(
        template.origin.as_deref(),
        Some(ORIGIN_USER) | Some(ORIGIN_COMMUNITY)
    )
}

fn template_json(template: &TeamTemplate) -> Value {
    let errors = validate_workflow_template(template);
    let findings = review_workflow_template(template);
    // 能力面评审随列表返回：UI 在建队前可据此做一次性知情确认
    // （执行/联网工具、MCP 依赖、未声明写范围、人工节点）。
    let warnings = findings
        .iter()
        .filter(|finding| finding.level == "warn")
        .count();
    json!({
        "template": template,
        "origin": template.origin.clone().unwrap_or_else(|| "builtin".to_string()),
        "valid": errors.is_empty(),
        "validation_errors": errors,
        "warnings": warnings,
        "blocked": workflow_review_blocked(&findings),
        "findings": findings,
        "health": owo_agent_core::workflow_health(template),
    })
}

/// `GET /workflows/user`：用户/社区模板列表（按创建时间倒序，含校验结果）。
async fn list_user_workflows(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut templates: Vec<TeamTemplate> = registry(&state)
        .list_templates()
        .into_iter()
        .filter(is_user_visible)
        .collect();
    templates.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    let imports = owo_agent_core::community_provenance::load_community_imports(&state.data_root);
    // 运行期告警随列表返回（列表页不必先打开统计就能看到"需要修复"）；
    // 聚合失败（协调器未就绪等）不阻断列表，降级为空告警。
    let run_stats = crate::workflow_stats_api::collect_template_stats(&state)
        .await
        .unwrap_or_default();
    let items: Vec<Value> = templates
        .iter()
        .map(|template| {
            let mut item = template_json(template);
            // 社区导入溯源（导入时间/sha256/当时的发现数）；用户自建为 null。
            if let Some(object) = item.as_object_mut() {
                object.insert(
                    "provenance".to_string(),
                    match imports.get(&template.template_id) {
                        Some(entry) => serde_json::to_value(entry).unwrap_or(Value::Null),
                        None => Value::Null,
                    },
                );
                let attention = run_stats
                    .get(&template.template_id)
                    .map(crate::workflow_stats_api::attention_for)
                    .unwrap_or_default();
                object.insert(
                    "runs".to_string(),
                    json!(attention_run_count(&run_stats, template)),
                );
                object.insert("attention".to_string(), json!(attention));
            }
            item
        })
        .collect();
    let invalid = items
        .iter()
        .filter(|item| item["valid"] == json!(false))
        .count();
    let needs_attention = items
        .iter()
        .filter(|item| {
            item["attention"]
                .as_array()
                .map(|reasons| !reasons.is_empty())
                .unwrap_or(false)
        })
        .count();
    Ok(Json(json!({
        "templates": items,
        "count": items.len(),
        "invalid": invalid,
        "needs_attention": needs_attention,
    })))
}

/// `POST /workflows/user`：创建/更新用户工作流（校验通过才落盘）。
///
/// - 不允许覆盖内置模板（同 id 且 origin 缺失 → 409）；
/// - 缺省 origin 归一为 `user`；`community` 仅用于导入流程（导入路由会显式标注）；
/// - 已存在的用户模板可被覆盖（更新），返回 200。
async fn upsert_user_workflow(
    State(state): State<Arc<AppState>>,
    Json(mut template): Json<TeamTemplate>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let registry = registry(&state);
    template.template_id = template.template_id.trim().to_string();
    if template.created_at.trim().is_empty() {
        template.created_at = chrono::Utc::now().to_rfc3339();
    }
    match template.origin.as_deref() {
        None | Some(ORIGIN_USER) => template.origin = Some(ORIGIN_USER.to_string()),
        Some(ORIGIN_COMMUNITY) => {}
        Some(other) => {
            return Err(error(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("origin 只能是 user/community（收到 {other}）"),
            ))
        }
    }
    if let Some(existing) = registry.get_template(&template.template_id) {
        if !is_user_visible(&existing) {
            return Err(error(
                StatusCode::CONFLICT,
                format!(
                    "模板 id 与内置模板冲突：{}（换个 template_id，或先删除用户模板）",
                    template.template_id
                ),
            ));
        }
    }
    let errors = validate_workflow_template(&template);
    if !errors.is_empty() {
        return Err(validation_error(errors));
    }
    registry.save_template(&template).map_err(|write_error| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("模板落盘失败：{write_error}"),
        )
    })?;
    audit(
        &state,
        "workflow_save",
        Some(template.template_id.clone()),
        Some(true),
        format!(
            "保存用户工作流：{} 个角色（origin={}）",
            template.roles.len(),
            template
                .origin
                .clone()
                .unwrap_or_else(|| "user".to_string())
        ),
    );
    Ok(Json(json!({ "ok": true, "template": template })))
}

/// `POST /workflows/user/validate`：只校验（UI 实时提示 / AI 生成后自检）。
async fn validate_user_workflow(Json(template): Json<TeamTemplate>) -> Json<Value> {
    let errors = validate_workflow_template(&template);
    Json(json!({
        "ok": errors.is_empty(),
        "errors": errors,
    }))
}

/// `DELETE /workflows/user/{id}`：删除用户/社区模板（内置 403，不存在 404）。
async fn delete_user_workflow(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let registry = registry(&state);
    let Some(existing) = registry.get_template(&id) else {
        return Err(error(StatusCode::NOT_FOUND, format!("模板不存在：{id}")));
    };
    if !is_user_visible(&existing) {
        return Err(error(
            StatusCode::FORBIDDEN,
            format!("内置模板不可删除：{id}（如需停用请在团队创建时不要选择它）"),
        ));
    }
    registry.delete_template(&id).map_err(|delete_error| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("删除失败：{delete_error}"),
        )
    })?;
    // 删除时清掉导入溯源（再导入再记录，避免残留旧 sha256）。
    {
        let mut imports =
            owo_agent_core::community_provenance::load_community_imports(&state.data_root);
        if imports.remove(&id).is_some() {
            if let Err(write_error) = owo_agent_core::community_provenance::save_community_imports(
                &state.data_root,
                &imports,
            ) {
                tracing::warn!("社区导入溯源清理失败：{write_error}");
            }
        }
    }
    audit(
        &state,
        "workflow_delete",
        Some(id.clone()),
        Some(true),
        format!(
            "删除用户工作流（origin={}）",
            existing.origin.unwrap_or_default()
        ),
    );
    Ok(Json(json!({ "ok": true, "deleted": id })))
}

/// 用户自定义工作流路由（由 build_router 合并；与目录路由同形：自带 state）。
pub fn user_workflows_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/workflows/user",
            get(list_user_workflows).post(upsert_user_workflow),
        )
        .route("/workflows/user/validate", post(validate_user_workflow))
        .route(
            "/workflows/user/{id}",
            axum::routing::delete(delete_user_workflow),
        )
        .with_state(state)
}
