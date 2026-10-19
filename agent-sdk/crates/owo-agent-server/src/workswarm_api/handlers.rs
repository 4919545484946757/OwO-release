use super::dto::*;
use super::runtime::*;
use super::workswarm_metrics;
use super::{change_set_api, project_workspace};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use owo_agent_core::project_space_store::{ProjectSpaceStoreBackend, SqliteProjectSpaceStore};
use owo_agent_core::workswarm::{
    CreateTeamRequest, HandoffFields, RoleSpec, SharedContextFactDraft, SteerCommand,
    TeamCoordinator, WorkSwarmError,
};
use owo_agent_protocol::{TeamMode, TeamRunStatus};
use owo_agent_server::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::ReceiverStream;

// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateTeamHttpRequest {
    goal_id: Option<String>,
    objective: String,
    mode: Option<String>,
    template_id: Option<String>,
    #[serde(default)]
    roles: Vec<RoleSpec>,
    #[serde(default)]
    budget: Value,
    human_policy: Option<String>,
    /// 五期：组队策略 auto|single|team（缺省 auto；未知值 → 400）。
    #[serde(default)]
    strategy: Option<String>,
    /// 十一期：团队统一模型（所有 agent 步骤缺省使用；roles[].model 显式覆盖）。
    #[serde(default)]
    model: Option<String>,
    /// 十一期：并行开发模式（lead 拆解 → w1..wN 并行 → leader 汇总；
    /// 运行期把 lead 产物的 subtasks 动态应用到 writer）。
    #[serde(default)]
    parallel: bool,
    /// 十一期：Agent 成员上限覆盖（并行模式 lead+writers+leader > 默认 5）。
    #[serde(default)]
    max_agent_members: Option<usize>,
    /// 六期（第二路）：可选项目工作区绑定（root 必须已存在；缺省只读）。
    #[serde(default)]
    workspace: Option<project_workspace::WorkspaceSpec>,
    /// 当前 REPL 父会话；服务端验证工作区后自行生成 CoreSpec。
    #[serde(default)]
    parent_session_id: Option<String>,
    /// 已经确认过该工作流的能力面提示（社区来源 + 有风险时必须为 true）。
    #[serde(default)]
    acknowledge_risk: bool,
}

#[derive(Debug, Deserialize)]
struct PublishTeamContextHttpRequest {
    expected_revision: u64,
    key: String,
    value: String,
    producer: String,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    source_refs: Vec<String>,
    #[serde(default)]
    file_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SteerHttpRequest {
    /// continue | steer | replace | cancel | retry
    command: String,
    #[serde(default)]
    step_id: Option<String>,
    #[serde(default)]
    new_input: Option<Value>,
    #[serde(default)]
    note: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    new_worker: Option<String>,
    #[serde(default)]
    new_user_id: Option<String>,
}

impl SteerHttpRequest {
    fn into_command(self) -> Result<SteerCommand, String> {
        match self.command.as_str() {
            "continue" => Ok(SteerCommand::Continue),
            "cancel" => Ok(SteerCommand::Cancel),
            "steer" => Ok(SteerCommand::Steer {
                step_id: self.step_id,
                new_input: self.new_input,
                note: self.note,
            }),
            // R2 冻结契约：POST /teams/{id}/steer {"command":"retry","step_id":"builder","note":"…"}
            // 缺少/空 step_id → 400（Validation）。
            "retry" => {
                let step_id = self
                    .step_id
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| "retry 需要 step_id 字段".to_string())?;
                Ok(SteerCommand::Retry {
                    step_id,
                    note: self.note,
                })
            }
            "replace" => {
                let role = self
                    .role
                    .filter(|r| !r.trim().is_empty())
                    .ok_or_else(|| "replace 需要 role 字段".to_string())?;
                Ok(SteerCommand::Replace {
                    role,
                    new_worker: self.new_worker,
                    new_user_id: self.new_user_id,
                    note: self.note,
                })
            }
            other => Err(format!(
                "未知 steer 指令：{other}（支持 continue/retry/steer/replace/cancel）"
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct HandoffHttpRequest {
    team_id: String,
    from_member: String,
    #[serde(default)]
    to_member: Option<String>,
    #[serde(default)]
    completed_summary: String,
    #[serde(default)]
    open_issues: Vec<String>,
    #[serde(default)]
    output_artifact_refs: Vec<String>,
    #[serde(default)]
    evidence_refs: Vec<String>,
    #[serde(default)]
    suggested_next_actions: Vec<String>,
    #[serde(default)]
    known_risks: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HumanResultHttpRequest {
    team_id: String,
    result: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/teams", post(create_team).get(list_teams))
        .route("/teams/{id}", get(get_team))
        .route("/teams/{id}/tasks", get(get_team_tasks))
        .route("/teams/{id}/events", get(team_events))
        .route("/teams/{id}/metrics", get(team_metrics))
        .route(
            "/teams/{id}/context",
            get(get_team_context).post(publish_team_context),
        )
        .route("/teams/{id}/diagnostic", get(team_diagnostic))
        .route("/teams/{id}/steer", post(steer_team))
        // 八期（第二路）：ChangeSet 审批、接受与安全撤销。
        .route(
            "/teams/{id}/change-sets",
            get(change_set_api::list_team_change_sets),
        )
        .route("/change-sets/{id}", get(change_set_api::get_change_set))
        .route(
            "/change-sets/{id}/accept",
            post(change_set_api::accept_change_set),
        )
        .route(
            "/change-sets/{id}/reject",
            post(change_set_api::reject_change_set),
        )
        .route(
            "/change-sets/{id}/revert",
            post(change_set_api::revert_change_set),
        )
        .route("/projects/{id}", get(get_project_space))
        .route("/projects/{id}/artifacts", get(list_artifacts))
        // 六期（第二路）：项目工作区绑定（真实目录 / 只读 / 写白名单 / 树 / git 状态）。
        .route(
            "/projects/{id}/workspace",
            get(project_workspace::get_workspace).put(project_workspace::put_workspace),
        )
        .route(
            "/projects/{id}/workspace/tree",
            get(project_workspace::get_workspace_tree),
        )
        // §8.1 启动器：团队创建前预览候选工作区目录树（写入路径多选数据源）。
        .route(
            "/workspace/tree",
            get(project_workspace::preview_workspace_tree),
        )
        .route(
            "/projects/{id}/workspace/git-status",
            get(project_workspace::get_workspace_git_status),
        )
        // 七期（第二路）：Worker 代码变更追踪（变更文件 + diff 摘要 + 逐步骤记录）。
        .route(
            "/projects/{id}/workspace/changes",
            get(project_workspace::get_workspace_changes),
        )
        .route("/tasks/{id}/handoff", post(submit_handoff))
        .route("/tasks/{id}/human-result", post(submit_human_result))
        .route("/teams/templates", get(list_templates))
        .route("/teams/templates/proposals", get(list_proposals))
        .route(
            "/teams/templates/proposals/{proposal_id}/adopt",
            post(adopt_proposal),
        )
        .route(
            "/teams/templates/proposals/{proposal_id}/reject",
            post(reject_proposal),
        )
        .with_state(state)
}

// ---------------- handlers ----------------

/// POST /teams：创建运行（202）+ 后台运行循环。
async fn create_team(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTeamHttpRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let mode = match req.mode.as_deref() {
        Some("single") => TeamMode::Single,
        Some("team") | None => TeamMode::Team,
        Some("swarmflow") => TeamMode::Swarmflow,
        Some(other) => {
            return Err(error_response(&WorkSwarmError::Validation(format!(
                "未知 mode：{other}（支持 single/team/swarmflow）"
            ))));
        }
    };
    let strategy = match req.strategy.as_deref() {
        None | Some("") => None,
        Some(raw) => match owo_agent_core::team_strategy::TeamSelectionMode::parse(raw) {
            Ok(s) => Some(s),
            Err(msg) => {
                return Err(error_response(&WorkSwarmError::Validation(msg)));
            }
        },
    };
    // 社区工作流治理：来源为 community 且带风险面（执行/联网/MCP/任意写/人工节点）时，
    // 必须由调用方显式确认（UI 确认框 / CLI --yes / API acknowledge_risk）。
    // 这是宿主侧兜底：即使客户端跳过了提示，也不会在未确认的情况下把社区配置跑起来。
    if let Some(template_id) = req.template_id.as_deref() {
        let registry = owo_agent_core::workswarm::TeamTemplateRegistry::new(
            state.data_root.join("workswarm").join("templates"),
        );
        if let Some(template) = registry.get_template(template_id) {
            if template.origin.as_deref() == Some("community") {
                let health = owo_agent_core::workflow_health(&template);
                let findings = owo_agent_core::review_workflow_template(&template);
                let warnings = findings
                    .iter()
                    .filter(|finding| finding.level == "warn")
                    .count();
                if !req.acknowledge_risk && (health.risk_level == "high" || warnings > 0) {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(json!({
                            "error": format!(
                                "社区工作流 {template_id} 带 {warnings} 条能力面提示（风险 {}）；确认后加 acknowledge_risk=true 重试",
                                health.risk_level
                            ),
                            "risk_level": health.risk_level,
                            "warnings": warnings,
                            "findings": findings,
                        })),
                    ));
                }
            }
        }
    }

    // 六期（第二路）：工作区绑定先校验（不依赖团队存在；路径非法 → 400，不建队）。
    let workspace_binding = match &req.workspace {
        Some(spec) => Some(
            project_workspace::validate_workspace_spec(spec)
                .map_err(|msg| (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))))?,
        ),
        None => None,
    };
    let parent_context_snapshot = if let Some(parent_id) = req.parent_session_id.as_deref() {
        let parent = crate::session_api::load_session(&state, parent_id)
            .map_err(|(status, message)| (status, Json(json!({ "error": message }))))?;
        let parent_workspace = parent.workspace.canonicalize().map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("父会话工作区不可访问：{error}") })),
            )
        })?;
        let active_workspace = state.workspace.canonicalize().map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Daemon 工作区不可访问：{error}") })),
            )
        })?;
        if parent_workspace != active_workspace {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "父会话必须属于当前工作区" })),
            ));
        }
        let system_constraints = parent
            .system_prompt
            .as_deref()
            .unwrap_or_default()
            .chars()
            .take(8000)
            .collect::<String>();
        let mut remaining = 24000usize;
        let mut recent_user_requirements = Vec::new();
        for message in parent
            .messages
            .iter()
            .rev()
            .filter(|message| message.role == "user")
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let content = message.content.as_deref().unwrap_or_default();
            let bounded = content.chars().take(remaining).collect::<String>();
            remaining = remaining.saturating_sub(bounded.chars().count());
            if !bounded.is_empty() {
                recent_user_requirements.push(bounded);
            }
            if remaining == 0 {
                break;
            }
        }
        Some(
            serde_json::json!({
                "kind": "source_session_context_v1",
                "source_session_id": parent.id,
                "source_updated_at": parent.updated_at,
                "core_spec": {
                    "system_constraints": system_constraints,
                    "recent_user_requirements": recent_user_requirements
                }
            })
            .to_string(),
        )
    } else {
        None
    };
    let mut req = CreateTeamRequest {
        goal_id: req.goal_id,
        objective: req.objective,
        mode,
        template_id: req.template_id,
        roles: req.roles,
        budget: req.budget,
        human_policy: req.human_policy,
        strategy,
        model: req.model,
        parallel: req.parallel,
        max_agent_members: req.max_agent_members,
        parent_context_snapshot,
    };
    // 十一期：工作区 `settings.json` 的 `team` 段统一配置（模型/并行/自定义角色）；
    // 请求显式字段优先，配置只在缺省时补位（CLI/UI 传参即覆盖）。
    let settings = owo_agent_core::Settings::load(&state.workspace);
    let settings_applied = apply_team_settings(&mut req, &settings);
    for role in &req.roles {
        if role.assignee != "human"
            && role.worker.as_deref().is_some_and(|worker| {
                !matches!(worker.trim(), "" | "agent" | "echo" | "sleep" | "fail")
            })
        {
            return Err(error_response(&WorkSwarmError::Validation(format!(
                "未注册的执行器：{}；外部 Agent 需要显式适配器，不能仅指定名称",
                role.worker.as_deref().unwrap_or_default()
            ))));
        }
    }

    let team = coordinator
        .create_team_run(&req)
        .await
        .map_err(|e| error_response(&e))?;
    let team_id = team.team_id.clone();
    // 绑定在运行循环启动前落盘（首迭代 build_run_registry 即生效）。
    let mut bound_workspace = false;
    if let Some(mut binding) = workspace_binding {
        binding.team_id = team_id.clone();
        binding.project_id = team.project_space_id.clone().unwrap_or_default();
        project_workspace::save_binding(coordinator.run_dir(), &binding).map_err(|msg| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": msg })),
            )
        })?;
        bound_workspace = true;
    }
    // 后台运行循环（单写者：阶段边界串行化；人节点等待 = 门闩）。
    tokio::spawn(run_team_loop(state, coordinator, team_id.clone()));
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "team_id": team_id,
            "project_space_id": team.project_space_id,
            "mode": format!("{:?}", team.mode),
            "template_id": team.template_id,
            "members": team.members,
            "status": format!("{:?}", team.status),
            "strategy_decision": team.strategy_decision,
            "workspace_bound": bound_workspace,
            "team_settings_applied": settings_applied,
        })),
    ))
}

/// 十一期：把 `<workspace>/settings.json` 的 `team` 段应用到建队请求（请求显式优先）。
///
/// 优先级与补位规则：
/// - `model`：请求 > `team.model` > `Settings.model`（空串视为未配置）；
/// - `roles`：请求未给角色且未指定模板 → 用 `team.roles` 自定义编排；
/// - `parallel`：请求未开 → `team.parallel`（2..=8）或角色里含 `lead` 时启用；
///   并行且无角色 → 生成内置 `parallel_roles(N)`（N = `team.parallel`，缺省 4）；
/// - `budget.max_parallel`：请求未显式给 → `team.max_parallel` / `team.parallel`；
/// - `max_agent_members`：请求未给 → 角色数（覆盖默认上限 5）。
///
/// 返回是否应用了配置（响应回执/排障用）。
fn apply_team_settings(req: &mut CreateTeamRequest, settings: &owo_agent_core::Settings) -> bool {
    let team = &settings.team;
    let mut applied = false;

    if req.model.is_none() {
        // 候选逐级回退：空串/空白视为未配置（不能因为 `Some(" ")` 短路掉下游回退）。
        let fallback = team
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string)
            .or_else(|| {
                settings
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|model| !model.is_empty())
                    .map(str::to_string)
            });
        if let Some(model) = fallback {
            req.model = Some(model);
            applied = true;
        }
    }

    if req.roles.is_empty() && req.template_id.is_none() && !team.roles.is_empty() {
        req.roles = team.roles.clone();
        applied = true;
    }

    let config_parallel = team.parallel.filter(|writers| (2..=8).contains(writers));
    if !req.parallel
        && (config_parallel.is_some() || req.roles.iter().any(|role| role.role == "lead"))
    {
        req.parallel = true;
        applied = true;
    }
    if req.parallel && req.roles.is_empty() {
        req.roles = owo_agent_core::workswarm::parallel_roles(config_parallel.unwrap_or(4));
        applied = true;
    }

    let has_budget_parallel = req
        .budget
        .get("max_parallel")
        .and_then(Value::as_u64)
        .is_some();
    if !has_budget_parallel {
        if let Some(limit) = team
            .max_parallel
            .or(team.parallel)
            .filter(|limit| (1..=8).contains(limit))
        {
            let mut budget = req.budget.as_object().cloned().unwrap_or_default();
            budget.insert("max_parallel".to_string(), json!(limit));
            req.budget = Value::Object(budget);
            applied = true;
        }
    }

    if req.max_agent_members.is_none() && !req.roles.is_empty() {
        req.max_agent_members = Some(req.roles.len());
    }
    applied
}

/// GET /teams：团队运行列表。
///
/// `active` = 运行循环正在执行阶段（人节点等待窗口 / 终态为 false = 暂停）。
async fn list_teams(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let runs = coordinator
        .list_team_runs()
        .await
        .map_err(|e| error_response(&e))?;
    let mut items = Vec::with_capacity(runs.len());
    for team in runs {
        let mut item = serde_json::to_value(&team).unwrap_or_else(|_| json!({}));
        if let Some(obj) = item.as_object_mut() {
            obj.insert(
                "active".to_string(),
                json!(coordinator.is_run_active(&team.team_id)),
            );
            // R2：中断标记（磁盘 Running 但无活动运行，等待显式恢复）。
            obj.insert(
                "interrupted".to_string(),
                json!(coordinator.is_interrupted(&team.team_id)),
            );
        }
        items.push(item);
    }
    Ok(Json(json!({ "teams": items })))
}

/// GET /teams/{id}：成员、预算、状态 + 任务视图 + 审计尾迹。
///
/// R2：响应含 `interrupted` 标记（磁盘 Running 但无活动运行 → 已被识别为中断、
/// 等待显式 continue/retry 恢复）。损坏的状态文件在此明确 500（原文件保留）。
async fn get_team(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    // 请求时先做一次单团队中断识别（幂等；CorruptState 在下方 load_run_state 明确报错）。
    // 响应性（R3）：运行中/循环存活的团队跳过识别——该路径取 team 锁，
    // 长 Worker 阶段绝不等待（运行中的团队按定义不可能是中断残留）。
    if !coordinator.is_run_active(&id) && !coordinator.is_loop_alive(&id) {
        let _ = coordinator.detect_interrupted_for(&id).await;
    }
    let team = coordinator
        .get_team_run(&id)
        .await
        .map_err(|e| error_response(&e))?;
    let run_state = coordinator
        .load_run_state(&id)
        .map_err(|e| error_response(&e))?;
    let progress = coordinator.progress_snapshot(&id).await.ok();
    Ok(Json(json!({
        "team": team,
        "interrupted": coordinator.is_interrupted(&id),
        "tasks": task_view(&run_state),
        "progress": progress,
        "audit_tail": audit_tail(&coordinator, &id),
    })))
}

/// GET /teams/{id}/context：返回版本号、事实来源与有界 CAS 正文。
async fn get_team_context(
    State(state): State<Arc<AppState>>,
    AxumPath(team_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let snapshot = coordinator
        .read_team_context(&team_id)
        .await
        .map_err(|e| error_response(&e))?;
    let mut remaining = 128 * 1024usize;
    let mut facts = Vec::new();
    for fact in snapshot.facts.iter().rev().take(64) {
        if remaining == 0 {
            break;
        }
        let value = fact
            .value_ref
            .strip_prefix("cas://sha256:")
            .and_then(|hash| coordinator.cas().get_text(hash))
            .unwrap_or_default();
        let bounded = value.chars().take(remaining).collect::<String>();
        remaining = remaining.saturating_sub(bounded.chars().count());
        facts.push(json!({
            "key": fact.key, "value": bounded, "value_ref": fact.value_ref,
            "revision": fact.revision, "producer": fact.producer,
            "task_id": fact.task_id, "source_refs": fact.source_refs,
            "file_hash": fact.file_hash, "confidence": fact.confidence,
            "status": fact.status, "created_at": fact.created_at
        }));
    }
    Ok(Json(
        json!({ "team_id": team_id, "revision": snapshot.revision, "facts": facts }),
    ))
}

/// POST /teams/{id}/context：expected_revision 保护的 CAS 发布。
async fn publish_team_context(
    State(state): State<Arc<AppState>>,
    AxumPath(team_id): AxumPath<String>,
    Json(request): Json<PublishTeamContextHttpRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let fact = coordinator
        .publish_team_context_fact(
            &team_id,
            request.expected_revision,
            SharedContextFactDraft {
                key: request.key,
                value: request.value,
                producer: request.producer,
                task_id: request.task_id,
                source_refs: request.source_refs,
                file_hash: request.file_hash,
            },
        )
        .await
        .map_err(|e| error_response(&e))?;
    Ok((StatusCode::CREATED, Json(json!({ "fact": fact }))))
}

/// GET /teams/{id}/tasks：任务图（步骤 × 状态）。
async fn get_team_tasks(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let team = coordinator
        .get_team_run(&id)
        .await
        .map_err(|e| error_response(&e))?;
    let run_state = coordinator
        .load_run_state(&id)
        .map_err(|e| error_response(&e))?;
    Ok(Json(json!({
        "team_id": team.team_id,
        "tasks": task_view(&run_state),
    })))
}

#[path = "team_observability.rs"]
mod team_observability;

use team_observability::{team_diagnostic, team_events, team_metrics};

/// POST /teams/{id}/steer：continue/retry/steer/replace/cancel。
async fn steer_team(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<SteerHttpRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let cmd = req
        .into_command()
        .map_err(|m| (StatusCode::BAD_REQUEST, Json(json!({ "error": m }))))?;
    // continue / retry 成功后都需要重新启动团队运行循环（R2：retry 恢复同样重启循环；
    // 运行循环每轮自行重建 worker 注册表，构建失败会在循环内记录并退出）。
    let restarts_loop = matches!(cmd, SteerCommand::Continue | SteerCommand::Retry { .. });
    let team = coordinator
        .apply_steer(&id, &cmd)
        .await
        .map_err(|e| error_response(&e))?;
    if restarts_loop && !coordinator.is_run_active(&id) {
        let state2 = state.clone();
        let coordinator2 = Arc::clone(&coordinator);
        tokio::spawn(run_team_loop(state2, coordinator2, id.clone()));
    }
    Ok((
        StatusCode::OK,
        Json(json!({
            "team_id": team.team_id,
            "status": format!("{:?}", team.status),
            "interrupted": coordinator.is_interrupted(&team.team_id),
        })),
    ))
}

/// GET /projects/{id}：Project Space 摘要。
async fn get_project_space(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let space = coordinator
        .get_project_space(&id)
        .await
        .map_err(|e| error_response(&e))?;
    Ok(Json(json!({ "project": space })))
}

/// GET /projects/{id}/artifacts：版本化共享产物（含 CAS 内容解析）。
async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    // 空间不存在 → 404（区分"空项目"与"不存在"）。
    let space = coordinator
        .get_project_space(&id)
        .await
        .map_err(|e| error_response(&e))?;
    let artifacts = coordinator
        .list_artifacts(&space)
        .await
        .map_err(|e| error_response(&e))?;
    let mut items = Vec::with_capacity(artifacts.len());
    for a in artifacts {
        let content_preview = coordinator
            .cas()
            .get_text(a.content_ref.strip_prefix("cas://sha256:").unwrap_or(""))
            .map(|c| c.chars().take(200).collect::<String>())
            .unwrap_or_default();
        items.push(json!({
            "artifact_id": a.artifact_id,
            "kind": a.kind,
            "version": a.version,
            "producer": a.producer,
            "content_ref": a.content_ref,
            "source_refs": a.source_refs,
            "review_state": format!("{:?}", a.review_state),
            // 五期：版本链链接（返工重跑登记时指向前版）——前端版本时间线/
            // v1v2 差异按此字段合并链（null = 首版/无链接）。
            "supersedes_artifact_id": a.supersedes_artifact_id,
            "created_at": a.created_at,
            "preview": content_preview,
        }));
    }
    Ok(Json(json!({ "project_id": id, "artifacts": items })))
}

/// POST /tasks/{id}/handoff：结构化接力（team_id 在请求体；任务 id 按团队命名空间解析）。
async fn submit_handoff(
    State(state): State<Arc<AppState>>,
    AxumPath(task_id): AxumPath<String>,
    Json(req): Json<HandoffHttpRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let fields = HandoffFields {
        to_member: req.to_member,
        completed_summary: req.completed_summary,
        open_issues: req.open_issues,
        output_artifact_refs: req.output_artifact_refs,
        evidence_refs: req.evidence_refs,
        suggested_next_actions: req.suggested_next_actions,
        known_risks: req.known_risks,
    };
    let handoff = coordinator
        .submit_handoff(&req.team_id, &task_id, &req.from_member, &fields)
        .await
        .map_err(|e| error_response(&e))?;
    Ok(Json(json!({ "handoff": handoff })))
}

/// POST /tasks/{id}/human-result：人节点提交结果（落盘后运行循环自动唤醒下游）。
async fn submit_human_result(
    State(state): State<Arc<AppState>>,
    AxumPath(task_id): AxumPath<String>,
    Json(req): Json<HumanResultHttpRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let artifact = coordinator
        .record_human_result(&req.team_id, &task_id, &req.result)
        .await
        .map_err(|e| error_response(&e))?;
    Ok(Json(json!({ "artifact": artifact })))
}

/// GET /teams/templates：已采纳团队模板。
async fn list_templates(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let templates = coordinator.templates().list_templates();
    Ok(Json(json!({ "templates": templates })))
}

/// GET /teams/templates/proposals：模板提案列表（只提案，不自动启用）。
async fn list_proposals(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let proposals = coordinator.templates().list_proposals();
    Ok(Json(json!({ "proposals": proposals })))
}

/// POST /teams/templates/proposals/{proposal_id}/adopt：采纳 → 进入模板注册表（幂等）。
async fn adopt_proposal(
    State(state): State<Arc<AppState>>,
    AxumPath(proposal_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let template = coordinator
        .templates()
        .adopt_proposal(&proposal_id)
        .map_err(|m| {
            let code = if m.contains("不存在") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (code, Json(json!({ "error": m })))
        })?;
    Ok(Json(json!({ "template": template })))
}

/// POST /teams/templates/proposals/{proposal_id}/reject：拒绝提案（保留记录，可审计；已采纳 → 400）。
async fn reject_proposal(
    State(state): State<Arc<AppState>>,
    AxumPath(proposal_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    coordinator
        .templates()
        .reject_proposal(&proposal_id)
        .map_err(|m| {
            let code = if m.contains("不存在") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (code, Json(json!({ "error": m })))
        })?;
    Ok(Json(json!({
        "proposal_id": proposal_id,
        "status": "rejected",
    })))
}

// ---------------------------------------------------------------------------
// 九期（一路）：TrackedRoleWorker 合并检测 / 空 ChangeSet 守卫（真实 git 仓库）
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 十一期：`settings.json` 的 `team` 段 → 建队请求缺省（请求显式优先）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod team_settings_tests {
    use super::*;
    use owo_agent_core::{Settings, TeamSettings};

    fn base_req() -> CreateTeamRequest {
        CreateTeamRequest::new("并行目标", TeamMode::Team)
    }

    /// 配置补位：统一模型 + 并行路数 + 并行度 + 自动生成并行角色 + 成员上限。
    #[test]
    fn config_fills_defaults_and_generates_parallel_roles() {
        let settings = Settings {
            team: TeamSettings {
                model: Some("glm-5.3-flashx".to_string()),
                parallel: Some(3),
                max_parallel: Some(2),
                roles: Vec::new(),
            },
            ..Default::default()
        };
        let mut req = base_req();
        assert!(apply_team_settings(&mut req, &settings));
        assert_eq!(req.model.as_deref(), Some("glm-5.3-flashx"));
        assert!(req.parallel, "team.parallel 应启用并行模式");
        assert_eq!(req.roles.len(), 5, "lead + w1..w3 + leader");
        assert_eq!(req.roles[0].role, "lead");
        assert_eq!(req.roles[4].role, "leader");
        assert_eq!(req.budget["max_parallel"], 2, "并行度可独立配置");
        assert_eq!(req.max_agent_members, Some(5));
    }

    /// CLI 显式并行意图未携带容量时，按配置或服务端安全缺省生成 worker 槽位。
    #[test]
    fn explicit_parallel_intent_without_config_uses_default_worker_capacity() {
        let mut req = base_req();
        req.parallel = true;
        assert!(apply_team_settings(&mut req, &Settings::default()));
        assert_eq!(req.roles.len(), 6, "lead + 默认 4 个 worker + leader");
        assert_eq!(
            req.roles.first().map(|role| role.role.as_str()),
            Some("lead")
        );
        assert_eq!(
            req.roles.last().map(|role| role.role.as_str()),
            Some("leader")
        );
        assert_eq!(req.roles[1].role, "w1");
        assert_eq!(req.roles[4].role, "w4");
        assert_eq!(req.max_agent_members, Some(6));
        assert_eq!(req.budget, Value::Null, "未显式容量时保留预算默认值");
    }

    /// 请求显式字段优先：配置不得覆盖 model/roles/parallel/budget/成员上限。
    #[test]
    fn request_explicit_fields_win_over_config() {
        let mut roles = vec![RoleSpec::agent("solo")];
        roles[0].write_paths = vec!["src/x".to_string()];
        let mut req = base_req();
        req.model = Some("custom-model".to_string());
        req.parallel = true;
        req.roles = roles;
        req.budget = json!({ "max_parallel": 1 });
        req.max_agent_members = Some(9);
        let settings = Settings {
            model: Some("fallback-model".to_string()),
            team: TeamSettings {
                model: Some("cfg-model".to_string()),
                parallel: Some(4),
                max_parallel: Some(8),
                roles: vec![RoleSpec::agent("w1")],
            },
            ..Default::default()
        };
        assert!(!apply_team_settings(&mut req, &settings), "无缺省可补");
        assert_eq!(req.model.as_deref(), Some("custom-model"));
        assert_eq!(req.roles.len(), 1);
        assert_eq!(req.roles[0].role, "solo");
        assert_eq!(req.budget["max_parallel"], 1);
        assert_eq!(req.max_agent_members, Some(9));
    }

    /// 配置自定义角色编排：请求未给角色时使用；角色含 lead 时自动开并行分配。
    #[test]
    fn config_custom_roles_used_and_lead_enables_parallel() {
        let mut lead = RoleSpec::agent("lead");
        lead.depends_on.clear();
        let settings = Settings {
            team: TeamSettings {
                roles: vec![lead, {
                    let mut w1 = RoleSpec::agent("w1");
                    w1.depends_on = vec!["lead".to_string()];
                    w1.write_paths = vec!["src/a".to_string()];
                    w1
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut req = base_req();
        assert!(apply_team_settings(&mut req, &settings));
        assert_eq!(req.roles.len(), 2);
        assert!(req.parallel, "自定义编排含 lead → 启用动态分配");
        assert_eq!(req.max_agent_members, Some(2));
    }

    /// 指定模板的请求不被配置角色覆盖；模型回退 `Settings.model`；空串视为未配置。
    #[test]
    fn template_request_keeps_roles_and_model_falls_back() {
        let settings = Settings {
            model: Some("glm-5.3-flash".to_string()),
            team: TeamSettings {
                model: Some("   ".to_string()),
                roles: vec![RoleSpec::agent("w1")],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut req = base_req();
        req.template_id = Some("code-change-v1".to_string());
        assert!(apply_team_settings(&mut req, &settings));
        assert!(req.roles.is_empty(), "模板请求保留模板角色");
        assert_eq!(
            req.model.as_deref(),
            Some("glm-5.3-flash"),
            "team.model 空串回退 Settings.model"
        );
    }

    /// 非法并行度（越界）不启用并行；max_parallel 越界不写预算。
    #[test]
    fn out_of_range_parallel_is_ignored() {
        let settings = Settings {
            team: TeamSettings {
                parallel: Some(99),
                max_parallel: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut req = base_req();
        assert!(!apply_team_settings(&mut req, &settings));
        assert!(!req.parallel);
        assert!(req.roles.is_empty());
        assert_eq!(req.budget, Value::Null);
    }
}
