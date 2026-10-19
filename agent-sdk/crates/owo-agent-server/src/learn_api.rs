//! 流程学习 HTTP API（§12：从 lib.rs 机械外移的 learn 域）。
//!
//! 路由面（`/learn/start|record|pause|resume|stop|clear|status|execute|packages|
//! packages/{name}|sink|execute-package|export/{name}|import` 与 `POST /skill/verify`）
//! 与 /openapi.json 登记保持不变，零行为变化。
//! 录制→沉淀→执行→健康度→导入导出全链路语义原样保留。
//!
//! 独立编译约束（沿 notes_api 惯例）：不使用 `crate::`/`super::`；引用 server
//! 类型一律写全限定名 `owo_agent_server::AppState`。

use axum::body::Bytes;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use owo_agent_core::learn::{LearnState, RecordedAction, Sensitivity};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use owo_agent_server::AppState;

/// 模块内 `poison` 副本（§12 惯例：共享根助手保持各域独立可用）。
fn poison<T>(_error: std::sync::PoisonError<T>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, "状态锁中毒".to_string())
}

#[derive(Deserialize)]
pub(super) struct LearnRecordRequest {
    action: RecordedAction,
}

pub(super) async fn learn_start(
    State(state): State<Arc<AppState>>,
) -> Result<Json<LearnState>, (StatusCode, String)> {
    let mut pipeline = state.pipeline.lock().map_err(poison)?;
    pipeline.recorder.start();
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Learn);
    Ok(Json(pipeline.recorder.state()))
}

pub(super) async fn learn_record(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LearnRecordRequest>,
) -> Result<Json<LearnState>, (StatusCode, String)> {
    let mut pipeline = state.pipeline.lock().map_err(poison)?;
    pipeline
        .recorder
        .record(request.action)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Learn);
    Ok(Json(pipeline.recorder.state()))
}

pub(super) async fn learn_pause(
    State(state): State<Arc<AppState>>,
) -> Result<Json<LearnState>, (StatusCode, String)> {
    let mut pipeline = state.pipeline.lock().map_err(poison)?;
    pipeline.recorder.pause();
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Learn);
    Ok(Json(pipeline.recorder.state()))
}

pub(super) async fn learn_resume(
    State(state): State<Arc<AppState>>,
) -> Result<Json<LearnState>, (StatusCode, String)> {
    let mut pipeline = state.pipeline.lock().map_err(poison)?;
    pipeline.recorder.resume();
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Learn);
    Ok(Json(pipeline.recorder.state()))
}

pub(super) async fn learn_stop(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut pipeline = state.pipeline.lock().map_err(poison)?;
    let samples = pipeline.stop_recording().len();
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Learn);
    Ok(Json(json!({
        "state": pipeline.recorder.state(),
        "samples": samples,
    })))
}

pub(super) async fn learn_clear(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut pipeline = state.pipeline.lock().map_err(poison)?;
    pipeline.recorder.clear();
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Learn);
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn learn_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let pipeline = state.pipeline.lock().map_err(poison)?;
    Ok(Json(json!({
        "state": pipeline.recorder.state(),
        "samples": pipeline.recorder.samples(),
        "sensitive_break": pipeline.recorder.sensitive_break(),
    })))
}

#[derive(Deserialize)]
pub(super) struct ExecuteRequest {
    graph: owo_agent_core::ActionGraph,
    #[serde(default)]
    variables: HashMap<String, String>,
    #[serde(default)]
    max_steps: Option<usize>,
    /// 首次执行必须显式确认（服务端强制审批）。
    #[serde(default)]
    confirm: bool,
}

/// 执行流程技能包动作图（Windows：UI Automation + SendInput，敏感面熔断）。
pub(super) async fn learn_execute(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<owo_agent_core::ExecReport>, (StatusCode, String)> {
    if !request.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            "首次执行必须确认（confirm: true）".to_string(),
        ));
    }
    let source = ui_action_source(state.elements.clone())?;
    let report = owo_agent_core::execute_graph(
        source.as_ref(),
        &request.graph,
        &request.variables,
        request.max_steps.unwrap_or(20),
    );
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        for step in &report.steps {
            audit.record(
                "learn-execute",
                "exec",
                Some(step.node_id.clone()),
                Some(step.status == "ok"),
                step.detail.clone(),
            );
        }
    }
    Ok(Json(report))
}

fn parse_sensitivity(value: &str) -> Result<Sensitivity, String> {
    match value {
        "low" => Ok(Sensitivity::Low),
        "medium" => Ok(Sensitivity::Medium),
        "high" => Ok(Sensitivity::High),
        "none" => Ok(Sensitivity::None),
        other => Err(format!("未知敏感度：{other}（low/medium/high/none）")),
    }
}

/// 流程技能包列表（用户学习产物）。
pub(super) async fn learn_packages(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Value>>, (StatusCode, String)> {
    let pipeline = state.pipeline.lock().map_err(poison)?;
    let mut packages = Vec::new();
    for name in pipeline
        .store
        .list()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
    {
        if let Ok(package) = pipeline.store.load(&name) {
            packages.push(json!({
                "name": package.manifest.name,
                "target_apps": package.manifest.target_apps,
                "variables": package.manifest.variables,
                "sensitivity": package.manifest.sensitivity,
                "version": package.manifest.version,
                "health": pipeline.store.health_state(&name),
            }));
        }
    }
    Ok(Json(packages))
}

pub(super) async fn learn_package_detail(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let pipeline = state.pipeline.lock().map_err(poison)?;
    let package = pipeline.store.load(&name).map_err(|error| {
        // 存储层抛的是 io 文案（"系统找不到指定的路径。 (os error 3)"），
        // 直接透出既没有"什么不存在"也没有"该怎么办"。统一成人话。
        (
            StatusCode::NOT_FOUND,
            format!("技能包不存在或不可读：{name}（{error}）"),
        )
    })?;
    Ok(Json(json!({
        "name": package.manifest.name,
        "target_apps": package.manifest.target_apps,
        "variables": package.manifest.variables,
        "sensitivity": package.manifest.sensitivity,
        "version": package.manifest.version,
        "skill_md": package.skill_md,
        "graph": package.graph,
    })))
}

pub(super) async fn learn_package_delete(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let pipeline = state.pipeline.lock().map_err(poison)?;
    pipeline
        .store
        .delete(&name)
        .map_err(|error| (StatusCode::NOT_FOUND, error))?;
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "learn",
            "delete-package",
            Some(name.clone()),
            Some(true),
            format!("删除流程技能包：{name}"),
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Packages);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(super) struct SinkRequest {
    name: String,
    target_apps: Vec<String>,
    sensitivity: String,
    description: String,
}

/// 结束录制并沉淀为流程技能包。
pub(super) async fn learn_sink(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SinkRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let sensitivity = parse_sensitivity(&request.sensitivity)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    let mut pipeline = state.pipeline.lock().map_err(poison)?;
    let package = pipeline
        .sink_skill(
            &request.name,
            request.target_apps,
            sensitivity,
            &request.description,
        )
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Packages);
    Ok(Json(json!({
        "ok": true,
        "name": package.manifest.name,
        "variables": package.manifest.variables,
    })))
}

#[derive(Deserialize)]
pub(super) struct ExecutePackageRequest {
    name: String,
    #[serde(default)]
    variables: HashMap<String, String>,
    #[serde(default)]
    max_steps: Option<usize>,
    /// 首次执行必须显式确认（服务端强制审批）。
    #[serde(default)]
    confirm: bool,
    /// 高敏感（High）技能包需二次确认。
    #[serde(default)]
    high_risk_ack: bool,
}

/// 从流程技能包加载动作图并执行（首次执行需在 UI 确认，步审计入库）。
pub(super) async fn learn_execute_package(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ExecutePackageRequest>,
) -> Result<Json<owo_agent_core::ExecReport>, (StatusCode, String)> {
    if !request.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            "首次执行必须确认（confirm: true）".to_string(),
        ));
    }
    let package = {
        let pipeline = state.pipeline.lock().map_err(poison)?;
        pipeline
            .store
            .execution_gate(&request.name, false)
            .map_err(|error| (StatusCode::CONFLICT, error))?;
        pipeline
            .store
            .load(&request.name)
            .map_err(|error| (StatusCode::NOT_FOUND, error))?
    };
    if package.manifest.sensitivity == Sensitivity::High && !request.high_risk_ack {
        return Err((
            StatusCode::BAD_REQUEST,
            "高敏感技能包需二次确认（high_risk_ack: true）".to_string(),
        ));
    }
    let source = ui_action_source(state.elements.clone())?;
    let report = owo_agent_core::execute_graph(
        source.as_ref(),
        &package.graph,
        &request.variables,
        request.max_steps.unwrap_or(20),
    );
    {
        let pipeline = state.pipeline.lock().map_err(poison)?;
        let failed = report.steps.iter().find(|step| step.status != "ok");
        let _ = pipeline.store.record_execution(
            &request.name,
            report.ok,
            failed
                .map(|step| step.node_id.as_str())
                .unwrap_or("completed"),
            failed.map(|step| step.detail.as_str()).unwrap_or(""),
        );
    }
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        if package.manifest.sensitivity == Sensitivity::High {
            audit.record(
                "learn-execute-package",
                "high_risk_ack",
                Some(request.name.clone()),
                Some(true),
                "高敏感技能包二次确认",
            );
        }
        audit.record(
            "learn-execute-package",
            "approval",
            Some(request.name.clone()),
            Some(true),
            "首次执行已确认",
        );
        for step in &report.steps {
            audit.record(
                "learn-execute-package",
                "exec",
                Some(step.node_id.clone()),
                Some(step.status == "ok"),
                step.detail.clone(),
            );
        }
    }
    Ok(Json(report))
}

/// 根据运行环境选择执行器源：模拟面用 SimUiActionSource（虚拟窗口），
/// 真实桌面用 WindowsUiaSource。
fn ui_action_source(
    elements: std::sync::Arc<std::sync::Mutex<owo_agent_core::ElementRegistry>>,
) -> Result<Box<dyn owo_agent_core::UiActionSource>, (StatusCode, String)> {
    if std::env::var("OWO_SIM_QQ_URL")
        .map(|value| !value.is_empty())
        .unwrap_or(false)
    {
        owo_agent_core::computer_use::SimUiActionSource::new()
            .map(|source| Box::new(source) as Box<dyn owo_agent_core::UiActionSource>)
            .map_err(|error| (StatusCode::BAD_REQUEST, error))
    } else {
        owo_agent_core::WindowsUiaSource::new_with_registry(Some(elements))
            .map(|source| Box::new(source) as Box<dyn owo_agent_core::UiActionSource>)
            .map_err(|error| (StatusCode::BAD_REQUEST, error))
    }
}

/// 导出流程技能包为 `.owskill`（ZIP）。
pub(super) async fn learn_export(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> Result<Response, (StatusCode, String)> {
    let package = {
        let pipeline = state.pipeline.lock().map_err(poison)?;
        pipeline.store.load(&name).map_err(|error| {
            (
                StatusCode::NOT_FOUND,
                format!("技能包不存在或不可读：{name}（{error}）"),
            )
        })?
    };
    let bytes = owo_agent_core::export_flow_skill_package(&package)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    let disposition = format!("attachment; filename=\"{name}.owskill\"");
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/zip".to_string(),
            ),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
        ],
        bytes,
    )
        .into_response())
}

/// 导入 `.owskill`（ZIP）并保存到用户技能包目录。
pub(super) async fn learn_import(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, String)> {
    let package = owo_agent_core::import_flow_skill_package(&body)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    let pipeline = state.pipeline.lock().map_err(poison)?;
    pipeline
        .store
        .save(&package)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    Ok(Json(json!({
        "ok": true,
        "name": package.manifest.name,
        "variables": package.manifest.variables,
        "target_apps": package.manifest.target_apps,
    })))
}

#[derive(Deserialize)]
pub(super) struct SkillVerifyRequest {
    path: PathBuf,
}

pub(super) async fn skill_verify(Json(request): Json<SkillVerifyRequest>) -> Json<Value> {
    // 与安装同源：结构 + 名字安全 + `min_app_version` 门禁（宿主版本=本进程版本）。
    match owo_agent_core::validate_skill_package_for_app(&request.path, env!("CARGO_PKG_VERSION")) {
        Ok(info) => Json(json!({
            "ok": true,
            "name": info.name,
            "version": info.version,
            "min_app_version": info.min_app_version,
            "permissions": info.permissions,
            "has_tests": info.has_tests,
        })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}
