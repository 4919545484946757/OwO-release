//! AI 生成用户工作流草稿：`/workflows/draft`（自然语言 → 结构化配置 → 校验 → 采纳）。
//!
//! 流程刻意做成**候选-评估-采纳**三段（借鉴 AFlow 的"生成+执行反馈"与 ADAS 的
//! "候选设计需评估后归档"）：
//! 1. `POST /workflows/draft`：模型按严格 JSON 契约生成候选（草稿落盘，**不启用**）；
//! 2. 宿主用确定性校验器逐条回报问题（模型不懂就报错，而不是直接生效）；
//! 3. `POST /workflows/draft/{id}/adopt`：人工确认后写入用户模板注册表；
//!    `DELETE /workflows/draft/{id}` 丢弃。
//!
//! 只使用 Daemon 默认 provider 做一次非流式补全（不传工具、不写会话历史），
//! 因此不会产生"第二个 Agent"或额外会话。
//!
//! 模块纪律：不引用 `crate::`/`super::`，AppState 全限定 `owo_agent_server::AppState`。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use owo_agent_core::gateway::{ChatMessage, ModelOutput};
use owo_agent_core::user_workflow::{validate_workflow_template, MAX_NAME_CHARS, MAX_ROLES};
use owo_agent_core::workflow_draft::{
    draft_repair_prompt, draft_system_prompt, draft_user_prompt_with_evidence, extract_json_object,
    render_run_evidence, WorkflowRunEvidence, MAX_DRAFT_DESCRIPTION_CHARS,
};
use owo_agent_core::workswarm::TeamTemplateRegistry;
use owo_agent_protocol::TeamTemplate;
use owo_agent_server::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// 单次生成的模型超时（草稿生成是交互式动作，不能让请求无限挂着）。
const DRAFT_TIMEOUT: Duration = Duration::from_secs(90);
/// 落盘时保留的原始回复片段长度（诊断用，避免把整段回复写进草稿）。
const RAW_EXCERPT_CHARS: usize = 2_000;
/// 生成失败后允许的自动修复次数（带宿主校验错误重生成一次；不再无限重试）。
const MAX_REPAIR_ATTEMPTS: usize = 1;

/// 收敛请求：自然语言描述 + 有界轮次（生成→校验→按错误重生成，直到通过或轮次用尽）。
#[derive(Debug, Default, Deserialize)]
struct ConvergeRequest {
    description: String,
    #[serde(default)]
    constraints: Option<String>,
    #[serde(default)]
    max_roles: Option<usize>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    /// 轮次上限（缺省 3，硬顶 5）——防止无限烧模型。
    #[serde(default)]
    max_rounds: Option<usize>,
    /// 基于已保存工作流迭代（同上：注入真实运行证据）。
    #[serde(default)]
    based_on: Option<String>,
}

const MAX_CONVERGE_ROUNDS: usize = 5;

#[derive(Debug, Default, Deserialize)]
struct VerifyOutcomeRequest {
    /// 验证团队 id（必填；缺省时由处理器给出明确 400 而不是提取器 422）。
    #[serde(default)]
    team_id: String,
    /// 结果状态（缺省从团队记录读取）。
    #[serde(default)]
    status: Option<String>,
    /// 本次验证使用的目标（沿用基线目标时会与基线一致）。
    #[serde(default)]
    objective: Option<String>,
    /// 采纳前的基线状态（用于"是否康复"的并列展示）。
    #[serde(default)]
    compared_status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct DraftRequest {
    /// 自然语言描述：要这条工作流完成什么特定功能。
    description: String,
    /// 附加约束（可选）：必须有人工节点、只能用某些工具、预算/轮次限制等。
    #[serde(default)]
    constraints: Option<String>,
    /// 角色数上限（默认 8，硬顶 [`MAX_ROLES`]）。
    #[serde(default)]
    max_roles: Option<usize>,
    /// 可用工具白名单（可选；给出后提示词只允许这些工具）。
    #[serde(default)]
    tools: Option<Vec<String>>,
    /// 基于既有草稿迭代（草稿 id）。
    #[serde(default)]
    refine_of: Option<String>,
    /// 基于**已保存工作流**迭代（模板 id）：把该工作流的真实运行证据（成败/需要修复/
    /// 最近失败定位）注入提示词，让模型按证据改配置，而不是凭描述重写。
    #[serde(default)]
    based_on: Option<String>,
}

/// 载入已保存（用户/社区/内置）工作流模板；不存在 → 404。
fn load_saved_template(
    state: &Arc<AppState>,
    template_id: &str,
) -> Result<TeamTemplate, (StatusCode, Json<Value>)> {
    let registry = TeamTemplateRegistry::new(state.data_root.join("workswarm").join("templates"));
    registry.get_template(template_id).ok_or_else(|| {
        error(
            StatusCode::NOT_FOUND,
            format!("工作流不存在：{template_id}"),
        )
    })
}

/// 该工作流最近一次"按证据改进 → 验证"的结果（供提示词做迭代反馈）。
///
/// 只读草稿记录：没有回写过 → None（不编造"上次成功了"）。
fn latest_verify_outcome(state: &AppState, template_id: &str) -> Option<String> {
    let dir = drafts_dir(state);
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(String, Value)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(draft) = serde_json::from_str::<Draft>(&text) else {
            continue;
        };
        if draft.based_on.as_deref() != Some(template_id) {
            continue;
        }
        let Some(outcome) = draft.verify_outcome.clone() else {
            continue;
        };
        let verified_at = outcome["verified_at"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if best
            .as_ref()
            .map(|(at, _)| verified_at > *at)
            .unwrap_or(true)
        {
            best = Some((verified_at, outcome));
        }
    }
    let (_, outcome) = best?;
    let status = outcome["status"].as_str().unwrap_or("?");
    let baseline = outcome["compared_status"].as_str().unwrap_or("-");
    Some(if status == baseline {
        format!("上次按证据改进后用同一目标验证：{status}（未康复，与基线相同）")
    } else {
        format!("上次按证据改进后用同一目标验证：{status}（基线 {baseline}）")
    })
}

/// 该工作流的真实运行证据（统计口径；聚合失败降级为 None，不阻断生成）。
async fn run_evidence_for(state: &Arc<AppState>, template_id: &str) -> Option<WorkflowRunEvidence> {
    let stats = crate::workflow_stats_api::collect_template_stats(state)
        .await
        .ok()?;
    let entry = stats.get(template_id)?;
    Some(WorkflowRunEvidence {
        // 上一次"按证据改进"的结果（若已回写）：让模型知道上次那样改有没有用。
        previous_attempt: latest_verify_outcome(state, template_id),
        template_id: template_id.to_string(),
        runs: entry.runs,
        succeeded: entry.succeeded,
        failed: entry.failed,
        cancelled: entry.cancelled,
        success_rate: entry.success_rate(),
        attention: crate::workflow_stats_api::attention_for(entry),
        last_failure: entry.last_failure.as_ref().map(|trace| {
            format!(
                "{} · {}{}",
                trace.event,
                trace.detail,
                if trace.at.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", trace.at)
                }
            )
        }),
    })
}

/// 落盘的一份草稿（候选）：模板 + 校验结果 + 生成上下文。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Draft {
    draft_id: String,
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    constraints: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refine_of: Option<String>,
    /// 迭代来源工作流 id（`based_on`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    based_on: Option<String>,
    /// 实际注入提示词的运行证据段落（诊断/审计：让用户看到模型看到了什么）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_evidence: Option<String>,
    /// 采纳后的验证结果（沿用基线目标再跑一次的结果；未验证为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verify_outcome: Option<Value>,
    template: TeamTemplate,
    /// 校验问题（空 = 可直接采纳）。
    errors: Vec<String>,
    /// 模型原始回复片段（诊断；解析失败时尤其有用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw_excerpt: Option<String>,
    model: String,
    created_at: String,
}

/// 解析模型回复为模板并做最小归一（缺 id/name/created_at 时补齐），失败给 422。
fn parse_draft_template(
    text: &str,
    description: &str,
) -> Result<TeamTemplate, (StatusCode, Json<Value>)> {
    let raw_excerpt: String = text.chars().take(RAW_EXCERPT_CHARS).collect();
    let Some(json_text) = extract_json_object(text) else {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("模型回复里没有 JSON 对象；原文片段：{raw_excerpt}"),
        ));
    };
    let mut template: TeamTemplate = serde_json::from_str(json_text).map_err(|parse_error| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("生成的 JSON 不符合 TeamTemplate 结构：{parse_error}；原文片段：{raw_excerpt}"),
        )
    })?;
    if template.template_id.trim().is_empty() {
        template.template_id = owo_agent_core::slugify_template_id(&template.name)
            .unwrap_or_else(|| format!("draft-{}", &uuid::Uuid::new_v4().to_string()[..8]));
    }
    if template.name.chars().count() > MAX_NAME_CHARS || template.name.trim().is_empty() {
        template.name = description.chars().take(MAX_NAME_CHARS).collect();
    }
    if template.created_at.trim().is_empty() {
        template.created_at = chrono::Utc::now().to_rfc3339();
    }
    Ok(template)
}

/// 治理动作写审计（AI 生成/采纳会改变注册表内容）。
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

fn drafts_dir(state: &AppState) -> PathBuf {
    state.data_root.join("workswarm").join("drafts")
}

fn error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.into() })))
}

/// 草稿 id 安全校验（进文件名）。
fn validate_draft_id(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 64 {
        return Err("draft_id 长度非法".to_string());
    }
    if trimmed.contains("..") || trimmed.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(format!("draft_id 含路径/保留字符：{trimmed}"));
    }
    Ok(())
}

fn draft_path(state: &AppState, id: &str) -> Result<PathBuf, String> {
    validate_draft_id(id)?;
    Ok(drafts_dir(state).join(format!("{id}.json")))
}

fn load_draft(state: &AppState, id: &str) -> Result<Draft, (StatusCode, Json<Value>)> {
    let path = draft_path(state, id).map_err(|reason| error(StatusCode::BAD_REQUEST, reason))?;
    let raw = std::fs::read_to_string(&path)
        .map_err(|_| error(StatusCode::NOT_FOUND, format!("草稿不存在：{id}")))?;
    serde_json::from_str(&raw).map_err(|parse_error| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("草稿解析失败：{parse_error}"),
        )
    })
}

fn save_draft(state: &AppState, draft: &Draft) -> Result<(), (StatusCode, Json<Value>)> {
    let dir = drafts_dir(state);
    std::fs::create_dir_all(&dir).map_err(|write_error| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("草稿目录创建失败：{write_error}"),
        )
    })?;
    let path = dir.join(format!("{}.json", draft.draft_id));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(draft).unwrap_or_else(|_| "{}".to_string()),
    )
    .map_err(|write_error| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("草稿落盘失败：{write_error}"),
        )
    })
}

fn draft_summary(draft: &Draft) -> Value {
    json!({
        "draft_id": draft.draft_id,
        "description": draft.description,
        "template_id": draft.template.template_id,
        "name": draft.template.name,
        "roles": draft.template.roles.len(),
        "valid": draft.errors.is_empty(),
        "errors": draft.errors,
        "created_at": draft.created_at,
        "refine_of": draft.refine_of,
        "based_on": draft.based_on,
        "verify_outcome": draft.verify_outcome,
        "model": draft.model,
    })
}

/// 预览：角色/依赖边/工具/写范围概览（UI 直接用，不必自己解析模板）。
fn draft_preview(template: &TeamTemplate) -> Value {
    let roles: Vec<Value> = template
        .roles
        .iter()
        .map(|role| {
            json!({
                "role": role.role,
                "assignee": role.assignee,
                "worker": role.worker,
                "depends_on": role.depends_on,
                "tools": role.tools,
                "write_paths": role.write_paths,
                "verify": role.verify,
                "handoff_contract": role.handoff_contract,
                "capabilities": role.capabilities,
            })
        })
        .collect();
    let mut edges: Vec<Value> = Vec::new();
    for role in &template.roles {
        for dependency in &role.depends_on {
            edges.push(json!({ "from": dependency, "to": role.role }));
        }
    }
    json!({
        "template_id": template.template_id,
        "name": template.name,
        "mode": template.mode,
        "applicability": template.applicability,
        "description": template.description,
        "version": template.version,
        "tags": template.tags,
        "roles": roles,
        "edges": edges,
    })
}

/// `POST /workflows/draft`：自然语言 → 候选工作流（模型 + 校验，落盘不启用）。
async fn create_draft(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DraftRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let description = request.description.trim().to_string();
    if description.is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "缺少 description（用自然语言说明这条工作流要完成什么）",
        ));
    }
    if description.chars().count() > MAX_DRAFT_DESCRIPTION_CHARS {
        return Err(error(
            StatusCode::BAD_REQUEST,
            format!("description 过长（>{MAX_DRAFT_DESCRIPTION_CHARS}）"),
        ));
    }
    let based_on = request
        .based_on
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if based_on.is_some() && request.refine_of.is_some() {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "based_on（基于已保存工作流）与 refine_of（基于草稿）不能同时使用",
        ));
    }
    let previous = match request.refine_of.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => Some(load_draft(&state, id)?.template),
        // 基于已保存工作流迭代：把现有配置也回灌给模型（保留仍然合理的角色/依赖）。
        _ => match based_on.as_deref() {
            Some(id) => Some(load_saved_template(&state, id)?),
            None => None,
        },
    };
    // 运行证据：只来自宿主统计（从未运行 → 空证据，不注入、不编造成功率）。
    let evidence = match based_on.as_deref() {
        Some(id) => run_evidence_for(&state, id).await,
        None => None,
    };
    let evidence_block = evidence
        .as_ref()
        .map(render_run_evidence)
        .filter(|block| !block.is_empty());
    let max_roles = request.max_roles.unwrap_or(8).clamp(1, MAX_ROLES);
    let tools = request.tools.clone().unwrap_or_default();

    let messages = vec![
        ChatMessage::system(draft_system_prompt(max_roles, &tools)),
        ChatMessage::user(draft_user_prompt_with_evidence(
            &description,
            request.constraints.as_deref(),
            previous.as_ref(),
            evidence.as_ref(),
        )),
    ];
    let provider = state.agent.provider();
    let completion = tokio::time::timeout(DRAFT_TIMEOUT, provider.complete(&messages, &[])).await;
    let output = match completion {
        Ok(Ok(output)) => output,
        Ok(Err(model_error)) => {
            return Err(error(
                StatusCode::BAD_GATEWAY,
                format!("模型调用失败：{model_error}（检查模型配置 / OWO_CLOUD_ENABLED）"),
            ))
        }
        Err(_) => {
            return Err(error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("模型调用超时（>{}s）", DRAFT_TIMEOUT.as_secs()),
            ))
        }
    };
    let text = match output {
        ModelOutput::Text(text) => text,
        ModelOutput::ToolCalls(_) => {
            return Err(error(
                StatusCode::BAD_GATEWAY,
                "模型返回了工具调用而不是 JSON（草稿生成不传工具，请重试）".to_string(),
            ))
        }
    };
    let mut attempts = 1usize;
    let mut template = parse_draft_template(&text, &description)?;
    let mut errors = validate_workflow_template(&template);
    let mut repaired = false;

    // 生成 → 宿主校验 →（一次）带着错误回灌重生成：把确定性校验器的反馈闭环给模型，
    // 但最多一次（不做无限重试），修复失败就保留第一版与错误，让用户看到真实原因。
    let mut repair_index = 0usize;
    while !errors.is_empty() && repair_index < MAX_REPAIR_ATTEMPTS {
        repair_index += 1;
        attempts += 1;
        let repair_messages = vec![
            ChatMessage::system(draft_system_prompt(max_roles, &tools)),
            ChatMessage::user(draft_repair_prompt(
                &description,
                request.constraints.as_deref(),
                &template,
                &errors,
            )),
        ];
        let completion =
            tokio::time::timeout(DRAFT_TIMEOUT, provider.complete(&repair_messages, &[])).await;
        let Ok(Ok(output)) = completion else {
            break;
        };
        let ModelOutput::Text(text) = output else {
            break;
        };
        let Ok(candidate) = parse_draft_template(&text, &description) else {
            break;
        };
        let candidate_errors = validate_workflow_template(&candidate);
        if candidate_errors.len() < errors.len() {
            template = candidate;
            errors = candidate_errors;
            repaired = true;
        } else {
            break;
        }
    }

    let draft = Draft {
        draft_id: format!("draft-{}", &uuid::Uuid::new_v4().to_string()[..8]),
        description: description.clone(),
        constraints: request
            .constraints
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        refine_of: request.refine_of.clone(),
        based_on: based_on.clone(),
        run_evidence: evidence_block.clone(),
        verify_outcome: None,
        template: template.clone(),
        errors: errors.clone(),
        raw_excerpt: None,
        model: "daemon-default".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    save_draft(&state, &draft)?;
    audit(
        &state,
        "workflow_draft_generate",
        Some(draft.draft_id.clone()),
        Some(errors.is_empty()),
        format!(
            "AI 生成工作流草稿：attempts={attempts} repaired={repaired} errors={} based_on={} evidence={}",
            errors.len(),
            based_on.as_deref().unwrap_or("-"),
            evidence_block.is_some()
        ),
    );
    Ok(Json(json!({
        "ok": errors.is_empty(),
        "draft": draft_summary(&draft),
        "template": template,
        "preview": draft_preview(&template),
        "errors": errors,
        "attempts": attempts,
        "repaired": repaired,
        // 迭代来源与证据透明化：用户能看到"凭什么改"，而不是黑盒。
        "based_on": based_on,
        "run_evidence": evidence_block,
        // 风险面随生成结果一起返回：通过不等于低风险（UI/CLI 要能提示）。
        "health": owo_agent_core::workflow_health(&template),
    })))
}

/// 单轮生成：有 `previous` 时走"带错误的修复提示"，否则走首次生成提示。
#[allow(clippy::too_many_arguments)]
async fn generate_round(
    provider: &Arc<dyn owo_agent_core::gateway::ModelProvider>,
    description: &str,
    constraints: Option<&str>,
    previous: Option<&TeamTemplate>,
    errors: &[String],
    max_roles: usize,
    tools: &[String],
    evidence: Option<&WorkflowRunEvidence>,
) -> Result<(TeamTemplate, Vec<String>), (StatusCode, Json<Value>)> {
    let user_prompt = match previous {
        Some(previous) => draft_repair_prompt(description, constraints, previous, errors),
        None => draft_user_prompt_with_evidence(description, constraints, None, evidence),
    };
    let messages = vec![
        ChatMessage::system(draft_system_prompt(max_roles, tools)),
        ChatMessage::user(user_prompt),
    ];
    let completion = tokio::time::timeout(DRAFT_TIMEOUT, provider.complete(&messages, &[])).await;
    let output = match completion {
        Ok(Ok(output)) => output,
        Ok(Err(model_error)) => {
            return Err(error(
                StatusCode::BAD_GATEWAY,
                format!("模型调用失败：{model_error}（检查模型配置 / OWO_CLOUD_ENABLED）"),
            ))
        }
        Err(_) => {
            return Err(error(
                StatusCode::GATEWAY_TIMEOUT,
                format!("模型调用超时（>{}s）", DRAFT_TIMEOUT.as_secs()),
            ))
        }
    };
    let text = match output {
        ModelOutput::Text(text) => text,
        ModelOutput::ToolCalls(_) => {
            return Err(error(
                StatusCode::BAD_GATEWAY,
                "模型返回了工具调用而不是 JSON（草稿生成不传工具，请重试）".to_string(),
            ))
        }
    };
    let template = parse_draft_template(&text, description)?;
    let errors = validate_workflow_template(&template);
    Ok((template, errors))
}

/// `POST /workflows/draft/converge`：有界多轮收敛（生成 → 宿主校验 → 带错误重生成）。
///
/// 与 `/workflows/draft` 的区别：不是"一次生成 + 一次修复"，而是**最多 `max_rounds`
/// 轮**在错误反馈下迭代；每轮只在"错误数变少"时才接受新版本（否则保留最佳），
/// 通过即停。返回逐轮轨迹，便于 UI 展示"AI 是怎么收敛的"。
async fn converge_draft(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConvergeRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let description = request.description.trim().to_string();
    if description.is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "缺少 description（用自然语言说明这条工作流要完成什么）",
        ));
    }
    if description.chars().count() > MAX_DRAFT_DESCRIPTION_CHARS {
        return Err(error(
            StatusCode::BAD_REQUEST,
            format!("description 过长（>{MAX_DRAFT_DESCRIPTION_CHARS}）"),
        ));
    }
    let based_on = request
        .based_on
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    // 基于已保存工作流时：先确认存在（404 早失败），再取真实运行证据。
    if let Some(id) = based_on.as_deref() {
        load_saved_template(&state, id)?;
    }
    let evidence = match based_on.as_deref() {
        Some(id) => run_evidence_for(&state, id).await,
        None => None,
    };
    let evidence_block = evidence
        .as_ref()
        .map(render_run_evidence)
        .filter(|block| !block.is_empty());
    let max_roles = request.max_roles.unwrap_or(8).clamp(1, MAX_ROLES);
    let tools = request.tools.clone().unwrap_or_default();
    let max_rounds = request
        .max_rounds
        .unwrap_or(3)
        .clamp(1, MAX_CONVERGE_ROUNDS);
    let provider = state.agent.provider();

    let mut best: Option<(TeamTemplate, Vec<String>)> = None;
    let mut trajectory: Vec<Value> = Vec::new();
    for round in 1..=max_rounds {
        let (previous, errors) = match &best {
            Some((template, errors)) => (Some(template), errors.clone()),
            None => (None, Vec::new()),
        };
        let outcome = generate_round(
            &provider,
            &description,
            request.constraints.as_deref(),
            previous,
            &errors,
            max_roles,
            &tools,
            evidence.as_ref(),
        )
        .await;
        match outcome {
            Ok((template, new_errors)) => {
                let improved = match &best {
                    None => true,
                    Some((_, previous_errors)) => new_errors.len() < previous_errors.len(),
                };
                let round_health = owo_agent_core::workflow_health(&template);
                trajectory.push(json!({
                    "round": round,
                    "roles": template.roles.len(),
                    "errors": new_errors.len(),
                    "valid": new_errors.is_empty(),
                    "accepted": improved,
                    "risk_level": round_health.risk_level,
                    "elevated_tools": round_health.elevated_tools,
                }));
                if improved {
                    best = Some((template, new_errors.clone()));
                }
                if new_errors.is_empty() {
                    break;
                }
                // 没有改善就停：再轮下去大概率也是同样错误（省模型调用）。
                if !improved {
                    break;
                }
            }
            Err((status, body)) => {
                // 中途模型失败：已有最佳版本就返回它（轨迹里说明失败），否则原样报错。
                if best.is_none() {
                    return Err((status, body));
                }
                trajectory.push(json!({
                    "round": round,
                    "error": body.0.to_string(),
                    "detail": body.0.get("error").cloned().unwrap_or(Value::Null),
                }));
                break;
            }
        }
    }

    let (template, errors) =
        best.ok_or_else(|| error(StatusCode::BAD_GATEWAY, "未产出任何草稿"))?;
    let draft = Draft {
        draft_id: format!("draft-{}", &uuid::Uuid::new_v4().to_string()[..8]),
        description: description.clone(),
        constraints: request
            .constraints
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        refine_of: None,
        based_on: based_on.clone(),
        run_evidence: evidence_block.clone(),
        verify_outcome: None,
        template: template.clone(),
        errors: errors.clone(),
        raw_excerpt: None,
        model: "daemon-default".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    save_draft(&state, &draft)?;
    audit(
        &state,
        "workflow_draft_converge",
        Some(draft.draft_id.clone()),
        Some(errors.is_empty()),
        format!(
            "AI 收敛生成工作流：轮次={} 校验错误={} 风险={} based_on={} evidence={}",
            trajectory.len(),
            errors.len(),
            owo_agent_core::workflow_health(&template).risk_level,
            based_on.as_deref().unwrap_or("-"),
            evidence_block.is_some()
        ),
    );
    Ok(Json(json!({
        "ok": errors.is_empty(),
        "draft": draft_summary(&draft),
        "template": template,
        "preview": draft_preview(&template),
        "errors": errors,
        "trajectory": trajectory,
        "rounds": trajectory.len(),
        "health": owo_agent_core::workflow_health(&template),
        "based_on": based_on,
        "run_evidence": evidence_block,
    })))
}

/// `POST /workflows/draft/{id}/verify`：回写"采纳后验证运行"的结果。
///
/// 这是迭代闭环的反馈位：下一次 `based_on` 同一工作流生成时，会把上次改进的结果
/// 一起交给模型（"上次这样改，验证还是失败"），避免重复无效改动。
/// `status` 缺省时从团队记录读取（不编造）。
async fn record_verify_outcome(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<VerifyOutcomeRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut draft = load_draft(&state, &id)?;
    if request.team_id.trim().is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "缺少 team_id"));
    }
    let team_id = request.team_id.trim().to_string();
    let coordinator = state.workswarm.coordinator().map_err(|service_error| {
        error(StatusCode::SERVICE_UNAVAILABLE, service_error.to_string())
    })?;
    let run = coordinator
        .get_team_run(&team_id)
        .await
        .map_err(|run_error| error(StatusCode::NOT_FOUND, run_error.to_string()))?;
    let status = match request.status.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => match run.status {
            owo_agent_protocol::TeamRunStatus::Created => "created".to_string(),
            owo_agent_protocol::TeamRunStatus::Running => "running".to_string(),
            owo_agent_protocol::TeamRunStatus::AwaitingHuman => "awaiting_human".to_string(),
            owo_agent_protocol::TeamRunStatus::Succeeded => "succeeded".to_string(),
            owo_agent_protocol::TeamRunStatus::Failed => "failed".to_string(),
            owo_agent_protocol::TeamRunStatus::Cancelled => "cancelled".to_string(),
        },
    };
    let outcome = json!({
        "team_id": team_id,
        "status": status,
        "objective": request.objective.as_deref().map(str::trim).filter(|value| !value.is_empty()),
        // 与采纳前的基线状态并列，便于"是否康复"一眼可读（不替代人工判断）。
        "compared_status": request.compared_status.as_deref().map(str::trim).filter(|value| !value.is_empty()),
        "verified_at": chrono::Utc::now().to_rfc3339(),
    });
    draft.verify_outcome = Some(outcome.clone());
    save_draft(&state, &draft)?;
    audit(
        &state,
        "workflow_draft_verify",
        Some(id.clone()),
        Some(status == "succeeded"),
        format!(
            "回写验证结果：草稿 {id} team={} status={} 基线={}",
            outcome["team_id"].as_str().unwrap_or("-"),
            outcome["status"].as_str().unwrap_or("-"),
            outcome["compared_status"].as_str().unwrap_or("-")
        ),
    );
    Ok(Json(json!({
        "ok": true,
        "draft": draft_summary(&draft),
        "verify_outcome": outcome,
    })))
}

/// `GET /workflows/draft`：草稿列表（新在前）。
async fn list_drafts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let dir = drafts_dir(&state);
    let mut drafts: Vec<Draft> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(draft) = serde_json::from_str::<Draft>(&raw) {
                    drafts.push(draft);
                }
            }
        }
    }
    drafts.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    let items: Vec<Value> = drafts.iter().map(draft_summary).collect();
    Ok(Json(json!({
        "drafts": items,
        "count": items.len(),
    })))
}

/// `GET /workflows/draft/{id}`：草稿详情（含完整模板与预览）。
async fn get_draft(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let draft = load_draft(&state, &id)?;
    Ok(Json(json!({
        "draft": draft_summary(&draft),
        "template": draft.template,
        "preview": draft_preview(&draft.template),
        "errors": draft.errors,
        "raw_excerpt": draft.raw_excerpt,
    })))
}

/// `POST /workflows/draft/{id}/adopt`：采纳草稿 → 用户模板（校验不过 422，不落盘）。
async fn adopt_draft(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut draft = load_draft(&state, &id)?;
    let errors = validate_workflow_template(&draft.template);
    if !errors.is_empty() {
        draft.errors = errors.clone();
        save_draft(&state, &draft)?;
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "ok": false, "errors": errors })),
        ));
    }
    let registry = registry(&state);
    if let Some(existing) = registry.get_template(&draft.template.template_id) {
        if !matches!(existing.origin.as_deref(), Some("user") | Some("community")) {
            return Err(error(
                StatusCode::CONFLICT,
                format!(
                    "模板 id 与内置模板冲突：{}（在草稿里改 template_id 后重试）",
                    draft.template.template_id
                ),
            ));
        }
    }
    let mut template = draft.template.clone();
    template.origin = Some("user".to_string());
    if template.created_at.trim().is_empty() {
        template.created_at = chrono::Utc::now().to_rfc3339();
    }
    registry.save_template(&template).map_err(|write_error| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("模板落盘失败：{write_error}"),
        )
    })?;
    // 采纳成功后草稿保留（可追溯），但标注已采纳。
    draft.errors = Vec::new();
    draft.template = template.clone();
    save_draft(&state, &draft)?;
    // 采纳即进入"待验证"：如果这版候选是按某条工作流的运行证据改出来的，
    // 就把那条工作流**上一次运行的状态/目标**一并返回，让 CLI/面板能用同一个目标
    // 再跑一次做前后对比（改配置不是目的，康复才是）。
    let verify = match draft.based_on.as_deref() {
        Some(previous_id) => {
            let previous = crate::workflow_stats_api::latest_run_for_template(&state, previous_id)
                .await
                .unwrap_or(None);
            let stats = crate::workflow_stats_api::collect_template_stats(&state)
                .await
                .unwrap_or_default();
            previous.map(|run| {
                json!({
                    "previous_template_id": previous_id,
                    "previous_status": run["status"],
                    "objective": run["objective"],
                    "previous_team_id": run["team_id"],
                    // 采纳前的"需要修复"清单（对比用基线；采纳后由新运行刷新）。
                    "previous_attention": stats
                        .get(previous_id)
                        .map(crate::workflow_stats_api::attention_for)
                        .unwrap_or_default(),
                    "same_template_id": previous_id == template.template_id,
                    "adopted_template_id": template.template_id,
                })
            })
        }
        None => None,
    };
    audit(
        &state,
        "workflow_draft_adopt",
        Some(template.template_id.clone()),
        Some(true),
        format!(
            "采纳草稿 {id} 为用户工作流（{} 个角色）基于={} 待验证={}",
            template.roles.len(),
            draft.based_on.as_deref().unwrap_or("-"),
            verify.is_some()
        ),
    );
    Ok(Json(json!({
        "ok": true,
        "template": template,
        "draft_id": id,
        "based_on": draft.based_on,
        // 有基线时才给验证计划（没有运行记录 → 无从对比，不编造）。
        "verify": verify,
    })))
}

/// `DELETE /workflows/draft/{id}`：丢弃草稿。
async fn delete_draft(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = draft_path(&state, &id).map_err(|reason| error(StatusCode::BAD_REQUEST, reason))?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(Json(json!({ "ok": true, "deleted": id }))),
        Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => {
            Err(error(StatusCode::NOT_FOUND, format!("草稿不存在：{id}")))
        }
        Err(remove_error) => Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("删除失败：{remove_error}"),
        )),
    }
}

/// 草稿路由（由 build_router 合并；与工作流路由同形：自带 state）。
pub fn workflow_draft_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/workflows/draft", post(create_draft).get(list_drafts))
        .route("/workflows/draft/converge", post(converge_draft))
        .route("/workflows/draft/{id}", get(get_draft).delete(delete_draft))
        .route("/workflows/draft/{id}/adopt", post(adopt_draft))
        .route("/workflows/draft/{id}/verify", post(record_verify_outcome))
        .with_state(state)
}
