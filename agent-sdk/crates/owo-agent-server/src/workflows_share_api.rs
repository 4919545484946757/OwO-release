//! 工作流的社区共享面：导出 / 导入前评审 / 导入（含安全门禁）。
//!
//! 三步与既有 `/team/export|review|import`（技能包通道）同形，但载体是**工作流模板**：
//! - `POST /workflows/user/{id}/export`：导出为自包含包（base64 JSON + 摘要 + sha256）；
//! - `POST /workflows/import/review`：只评审不落盘，返回逐条 findings；
//! - `POST /workflows/import`：评审通过才落盘（`origin=community`）；有 `block` 级发现时
//!   返回 422 且**不写盘**。
//!
//! 包格式（`kind=owo-workflow`，schema_version=1）刻意用纯 JSON 而非 zip：工作流是配置
//! 而非资产，JSON 便于人审、diff 与手工修复；`sha256` 覆盖模板规范化 JSON，导入端必校验。
//!
//! 模块纪律：不引用 `crate::`/`super::`，全限定 `owo_agent_server::AppState`。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use owo_agent_core::user_workflow::{review_workflow_template, workflow_review_blocked};
use owo_agent_core::workswarm::TeamTemplateRegistry;
use owo_agent_core::CasStore;
use owo_agent_protocol::TeamTemplate;
use owo_agent_server::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// 包格式版本（新增字段向后兼容；不兼容变更时提升）。
const PACKAGE_SCHEMA_VERSION: u32 = 1;
/// 包大小上限（base64 之前）——工作流是配置，正常几 KB。
const MAX_PACKAGE_BYTES: usize = 1_000_000;

#[derive(Debug, Deserialize)]
struct PackageBody {
    package_b64: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PackageManifest {
    schema_version: u32,
    kind: String,
    template_id: String,
    name: String,
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    role_count: usize,
    /// 导出时间（不参与校验）。
    exported_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkflowPackage {
    schema_version: u32,
    kind: String,
    manifest: PackageManifest,
    template: TeamTemplate,
    /// 模板规范化 JSON 的 sha256（导入端必校验）。
    sha256: String,
}

/// 治理动作写审计（社区包的导出/导入/被拒都属安全相关动作）。
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

fn is_user_visible(template: &TeamTemplate) -> bool {
    matches!(template.origin.as_deref(), Some("user") | Some("community"))
}

/// 模板的规范化 JSON（字段顺序稳定 = serde 结构体顺序），sha256 覆盖它。
fn canonical_template_json(template: &TeamTemplate) -> String {
    serde_json::to_string(template).unwrap_or_default()
}

fn template_sha256(template: &TeamTemplate) -> String {
    CasStore::hash_of(canonical_template_json(template).as_bytes())
}

fn decode_package(package_b64: &str) -> Result<WorkflowPackage, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(package_b64.trim())
        .map_err(|decode_error| format!("包不是合法 base64：{decode_error}"))?;
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(format!(
            "包过大（{} 字节 > {}）：工作流包应只含配置",
            bytes.len(),
            MAX_PACKAGE_BYTES
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| "包不是 UTF-8 文本".to_string())?;
    let package: WorkflowPackage =
        serde_json::from_str(&text).map_err(|parse_error| format!("包结构不符：{parse_error}"))?;
    if package.kind != "owo-workflow" {
        return Err(format!("包 kind 不是 owo-workflow：{}", package.kind));
    }
    if package.schema_version != PACKAGE_SCHEMA_VERSION {
        return Err(format!(
            "包 schema_version 不兼容：{}（本机支持 {}）",
            package.schema_version, PACKAGE_SCHEMA_VERSION
        ));
    }
    let expected = template_sha256(&package.template);
    if !expected.eq_ignore_ascii_case(package.sha256.trim()) {
        return Err(format!(
            "校验失败：包内 sha256={}，按模板重算={expected}（包可能被改动；手工组包必须使用规范化 JSON——建议直接用 `POST /workflows/user/{{id}}/export` 生成）",
            package.sha256.trim()
        ));
    }
    Ok(package)
}

/// 与本地同名工作流的差异（没有本地同 id 模板时返回 `null`）。
///
/// 社区更新场景：导入前先看清"这次更新改了什么、权限面是否变大"。
fn local_diff(state: &AppState, incoming: &TeamTemplate) -> Value {
    let Some(local) = registry(state).get_template(&incoming.template_id) else {
        return json!(null);
    };
    let diff = owo_agent_core::workflow_diff::diff_templates(&local, incoming);
    json!({
        "local_origin": local.origin.clone().unwrap_or_else(|| "builtin".to_string()),
        "local_version": local.version,
        "incoming_version": incoming.version,
        "diff": diff,
        "summary": owo_agent_core::workflow_diff::summarize_diff(&diff),
    })
}

fn review_json(template: &TeamTemplate) -> Value {
    let findings = review_workflow_template(template);
    json!({
        "blocked": workflow_review_blocked(&findings),
        "findings": findings,
    })
}

/// `POST /workflows/user/{id}/export`：导出用户/社区工作流为社区包。
async fn export_workflow(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let registry = registry(&state);
    let template = registry
        .get_template(&id)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, format!("模板不存在：{id}")))?;
    if !is_user_visible(&template) {
        return Err(error(
            StatusCode::FORBIDDEN,
            format!("内置模板不可导出为社区包（{id}）：社区包只分享用户/社区来源的工作流"),
        ));
    }
    let manifest = PackageManifest {
        schema_version: PACKAGE_SCHEMA_VERSION,
        kind: "owo-workflow".to_string(),
        template_id: template.template_id.clone(),
        name: template.name.clone(),
        version: template
            .version
            .clone()
            .unwrap_or_else(|| "1.0.0".to_string()),
        description: template.description.clone(),
        tags: template.tags.clone(),
        role_count: template.roles.len(),
        exported_at: chrono::Utc::now().to_rfc3339(),
    };
    let package = WorkflowPackage {
        schema_version: PACKAGE_SCHEMA_VERSION,
        kind: "owo-workflow".to_string(),
        manifest,
        sha256: template_sha256(&template),
        template: template.clone(),
    };
    let canonical = serde_json::to_string(&package).map_err(|serialize_error| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("包序列化失败：{serialize_error}"),
        )
    })?;
    let package_b64 = base64::engine::general_purpose::STANDARD.encode(canonical.as_bytes());
    audit(
        &state,
        "workflow_export",
        Some(template.template_id.clone()),
        Some(true),
        format!(
            "导出社区包：sha256={}… roles={}",
            &package.sha256[..12.min(package.sha256.len())],
            template.roles.len()
        ),
    );
    Ok(Json(json!({
        "ok": true,
        "package_b64": package_b64,
        "sha256": package.sha256,
        "manifest": package.manifest,
        // 导出即预审：分享者先看见自己会发布什么能力面。
        "review": review_json(&template),
    })))
}

/// `POST /workflows/import/review`：只评审不落盘（导入前知情）。
async fn review_import(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PackageBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let package = decode_package(&body.package_b64)
        .map_err(|reason| error(StatusCode::UNPROCESSABLE_ENTITY, reason))?;
    let diff_vs_local = local_diff(&state, &package.template);
    Ok(Json(json!({
        "ok": true,
        "manifest": package.manifest,
        "template": package.template,
        "sha256": package.sha256,
        "review": review_json(&package.template),
        "diff_vs_local": diff_vs_local,
    })))
}

/// `GET /workflows/provenance`：导出"我的社区来源清单"（可在团队内分享核对）。
///
/// 只含身份字段（template_id + sha256 + 导入时间），**不含配置正文**。
async fn export_provenance(State(state): State<Arc<AppState>>) -> Json<Value> {
    let imports = owo_agent_core::community_provenance::load_community_imports(&state.data_root);
    let manifest = owo_agent_core::community_provenance::SourcesManifest::from_imports(&imports);
    Json(json!({
        "ok": true,
        "count": manifest.sources.len(),
        "manifest": manifest,
    }))
}

/// `POST /workflows/import`：评审通过才落盘（origin=community）。
async fn import_workflow(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PackageBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let package = decode_package(&body.package_b64)
        .map_err(|reason| error(StatusCode::UNPROCESSABLE_ENTITY, reason))?;
    let review = review_json(&package.template);
    let diff_vs_local = local_diff(&state, &package.template);
    if review["blocked"] == json!(true) {
        audit(
            &state,
            "workflow_import_blocked",
            Some(package.template.template_id.clone()),
            Some(false),
            format!(
                "社区包导入被门禁拒绝：sha256={}… findings={}",
                &package.sha256[..12.min(package.sha256.len())],
                review["findings"].as_array().map(Vec::len).unwrap_or(0)
            ),
        );
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "ok": false,
                "blocked": true,
                "findings": review["findings"],
                "manifest": package.manifest,
            })),
        ));
    }
    let registry = registry(&state);
    if let Some(existing) = registry.get_template(&package.template.template_id) {
        if !is_user_visible(&existing) {
            return Err(error(
                StatusCode::CONFLICT,
                format!(
                    "模板 id 与内置模板冲突：{}（请让分享者换 id，或先删除本地用户模板）",
                    package.template.template_id
                ),
            ));
        }
    }
    let mut template = package.template.clone();
    template.origin = Some("community".to_string());
    if template.created_at.trim().is_empty() {
        template.created_at = chrono::Utc::now().to_rfc3339();
    }
    registry.save_template(&template).map_err(|write_error| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("模板落盘失败：{write_error}"),
        )
    })?;
    // 导入溯源：记录 sha256 与当时的发现数（放在 templates/ 之外，避免被模板扫描）。
    {
        let mut imports =
            owo_agent_core::community_provenance::load_community_imports(&state.data_root);
        imports.insert(
            template.template_id.clone(),
            owo_agent_core::community_provenance::CommunityProvenance {
                sha256: package.sha256.clone(),
                imported_at: chrono::Utc::now().to_rfc3339(),
                warnings: review["findings"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter(|item| item["level"] == json!("warn"))
                            .count()
                    })
                    .unwrap_or(0),
                findings: review["findings"].as_array().map(Vec::len).unwrap_or(0),
            },
        );
        if let Err(write_error) =
            owo_agent_core::community_provenance::save_community_imports(&state.data_root, &imports)
        {
            tracing::warn!("社区导入溯源写入失败：{write_error}");
        }
    }
    audit(
        &state,
        "workflow_import",
        Some(template.template_id.clone()),
        Some(true),
        format!(
            "导入社区包 → origin=community：sha256={}… warnings={}",
            &package.sha256[..12.min(package.sha256.len())],
            review["findings"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| item["level"] == json!("warn"))
                        .count()
                })
                .unwrap_or(0)
        ),
    );
    Ok(Json(json!({
        "ok": true,
        "template": template,
        "manifest": package.manifest,
        "review": review,
        "diff_vs_local": diff_vs_local,
    })))
}

/// 工作流共享路由（由 build_router 合并）。
pub fn workflows_share_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/workflows/provenance",
            axum::routing::get(export_provenance),
        )
        .route(
            "/workflows/user/{id}/export",
            post(export_workflow).layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route(
            "/workflows/import/review",
            post(review_import).layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route(
            "/workflows/import",
            post(import_workflow).layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .with_state(state)
}
