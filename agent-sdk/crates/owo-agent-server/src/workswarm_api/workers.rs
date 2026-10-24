#[path = "worker_artifact_tool.rs"]
mod artifact_tool;
#[path = "worker_context_tool.rs"]
mod context_tool;
#[cfg(test)]
use artifact_tool::team_artifact_read_effect;
use artifact_tool::TeamArtifactReadTool;
#[cfg(test)]
use context_tool::{
    team_context_fact_freshness, team_context_fact_is_visible, team_context_publish_effect,
    team_context_read_effect,
};
use context_tool::{TeamContextPublishTool, TeamContextReadTool};

use super::write_lease::{WriteLease, WriteScope};
use super::{project_workspace, workspace_change_tracker, workswarm_metrics};
use async_trait::async_trait;
use owo_agent_core::agent::TurnEvent;
use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::goal::Worker;
use owo_agent_core::permissions::AutoApprover;
use owo_agent_core::subagent::SubagentRunner;
use owo_agent_core::task_context::ResolvedTaskContext;
#[cfg(test)]
use owo_agent_core::tool_effects::EffectClass;
use owo_agent_core::tools::Tool;
use owo_agent_core::worker_profile::{ProfileSubagentRunner, TurnEventSink, WorkerProfile};
use owo_agent_core::workswarm::TeamCoordinator;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

pub(super) fn resolve_agent_model(input: &Value, configured_model: Option<&str>) -> String {
    owo_agent_core::gateway::resolve_subagent_model(
        input.get("model").and_then(Value::as_str),
        configured_model,
    )
}

/// 真实 Agent 子代理 worker（name="agent"）：prompt → 子代理执行。
///
/// 五期（第三路）：与 `Agent::run_subagent` 同口径（顶层 depth=0、
/// 子代理 max_turns 上限 12），但 Provider 支持 [`workswarm_metrics::MeasuredProvider`]
/// 计数装饰器注入——`model_calls` 由 `MeasuredRoleWorker` 逐 span 精确统计
/// （每次模型调用恰好经过 `complete`/`complete_stream` 其一）。
/// 七期（第二路）：角色画像驱动——工具注册表按 `WorkerProfile` 装配（注册表面即
/// 权限边界）、回合上限取模板预算、写面为「角色 ∩ 绑定」交集白名单。
pub struct AgentSubagentWorker {
    pub(super) agent: Arc<owo_agent_core::Agent>,
    pub(super) workspace: PathBuf,
    /// 指标层注入的 per-span 模型调用计数（None = 不计数，行为不变）。
    pub(super) model_calls: Option<Arc<AtomicU64>>,
    pub(super) request_usage: Option<Arc<workswarm_metrics::RequestUsageCollector>>,
    pub(super) team_request_budget: Option<Arc<workswarm_metrics::TeamModelRequestBudget>>,
    /// 六期（第二路）：项目工作区绑定作用域（None = 全局工作区，行为不变）。
    /// 绑定后：运行目录 = 绑定根；只读绑定强制 read_only；写白名单经审批器强制。
    pub(super) workspace_scope: Option<project_workspace::WorkspaceScope>,
    /// 七期（第二路）：角色画像（工具面/只读/回合上限；None = 防御分支退回通用执行器）。
    pub(super) profile: Option<WorkerProfile>,
    /// 七期（第二路）：critic 角色代理（服务端口径：`role == "critic"` 字面量；
    /// 引擎注入的 read_only 只覆盖 critic，其余内置角色都是 producer，画像另管只读面）。
    pub(super) is_critic: bool,
    /// 七期（第二路）：团队取消桥共享标志（None = 本地标志，行为退化为不可中断）。
    pub(super) cancel_flag: Option<Arc<AtomicBool>>,
    /// 七期（第二路）：最终写白名单（角色 ∩ 绑定交集；空 = 工作区内可写）。
    pub(super) write_allowed: Vec<PathBuf>,
    pub(super) coordinator: Arc<TeamCoordinator>,
    pub(super) session_store: Arc<dyn owo_agent_core::SessionStore>,
    pub(super) external_approver: Arc<dyn owo_agent_core::permissions::Approver>,
    /// 建队时勾选「允许按声明范围写入」后的预授权规则（None = 不预授权）。
    pub(super) scoped_write_rule: Option<super::scoped_write_approver::ScopedWriteRule>,
    pub(super) parent_session_id: Option<String>,
    pub(super) team_id: String,
    pub(super) role: String,
}

#[async_trait]
impl Worker for AgentSubagentWorker {
    fn name(&self) -> &str {
        "agent"
    }

    async fn run(&self, input: &Value) -> Result<String, String> {
        let input_read_only = input
            .get("read_only")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let task_context = ResolvedTaskContext::from_worker_input(input)?;
        // 只读三层叠加（七期 · 二路）：步骤输入 → 团队绑定只读（团队级上限）→
        // 角色画像只读（角色级上限）。任一只读即只读。
        let read_only = input_read_only
            || self
                .workspace_scope
                .as_ref()
                .map(|s| s.read_only)
                .unwrap_or(false)
            || self.profile.as_ref().map(|p| p.read_only).unwrap_or(false);
        let task_write_allowed =
            assigned_task_write_allowlist(&task_context, &self.workspace, &self.write_allowed)?;
        let task_has_no_write_path = task_write_allowed.as_ref().is_some_and(Vec::is_empty);
        let task_has_no_write_capability = task_lacks_file_write_capability(&task_context);
        let task_has_no_write_scope = task_has_no_write_path || task_has_no_write_capability;
        let read_only = read_only || task_has_no_write_scope;
        let effective_write_allowed = task_write_allowed
            .clone()
            .unwrap_or_else(|| self.write_allowed.clone());
        // Resolve the task-local profile once. The same value controls model-visible
        // prompt guards, ToolRegistry, write lease, and request budget.
        let mut task_profile = self.profile.clone();
        if let Some(profile) = task_profile.as_mut() {
            if task_context.is_task_graph_assignment() {
                apply_task_capability_scope(profile, &task_context, task_has_no_write_scope);
                if let Some(total_calls) = task_context.model_calls_per_attempt {
                    *profile = profile
                        .clone()
                        .with_task_model_call_budget(total_calls as usize)?;
                }
            }
            if task_write_allowed.is_some() {
                let command_was_assigned = task_context
                    .required_capabilities
                    .as_deref()
                    .is_some_and(|capabilities| {
                        capabilities.iter().any(|item| item == "run_command")
                    });
                profile.can_run_command &= command_was_assigned;
                if !command_was_assigned {
                    profile.visible_tools.retain(|tool| tool != "run_command");
                }
                if task_has_no_write_scope {
                    profile.read_only = true;
                }
            }
            if read_only {
                // Workspace read-only does not forbid an explicitly selected external API.
                // Keep its own approval path; never restore workspace mutation tools.
                let external = profile
                    .visible_tools
                    .iter()
                    .filter(|name| {
                        !self.is_critic
                            && (name.starts_with("mcp__")
                                || matches!(name.as_str(), "web_fetch" | "web_search"))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                *profile = profile.clone().restricted_to_read_only();
                if !external.is_empty() {
                    profile
                        .visible_tools
                        .retain(|name| name != "__owo_no_task_tool__");
                    profile.visible_tools.extend(external);
                }
            }
            profile.write_allowed_paths = if profile.read_only {
                Vec::new()
            } else if effective_write_allowed.is_empty() {
                vec![".".to_string()]
            } else {
                prompt_workspace_write_paths(&self.workspace, &effective_write_allowed)
            };
        }
        let prompt = if let Some(team_context) = input.get("team_prompt_context") {
            let (mut prompt, prompt_meta) = match task_profile.as_ref() {
                Some(profile) => {
                    TeamCoordinator::compile_role_prompt_with_profile(team_context, profile)
                }
                None => TeamCoordinator::compile_role_prompt_with_meta(team_context),
            };
            if let Some(instruction) = input.get("team_prompt_rework").and_then(Value::as_str) {
                prompt.push_str(&format!(
                    "\n\n## 本次评审返修（保持原任务范围与写权限）\n{instruction}\n"
                ));
            }
            let mut event =
                json!({"kind": "context", "role": self.role, "step_id": task_context.step_id});
            if let (Some(obj), Some(meta)) = (event.as_object_mut(), prompt_meta.as_object()) {
                for (key, value) in meta {
                    obj.insert(key.clone(), value.clone());
                }
            }
            self.coordinator
                .note_adaptive_event(&self.team_id, event)
                .await;
            prompt
        } else {
            input
                .get("prompt")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .ok_or_else(|| "agent 步骤缺少 prompt 参数".to_string())?
                .to_string()
        };
        let configured_model = std::env::var("OWO_AGENT_MODEL").ok();
        let model = resolve_agent_model(input, configured_model.as_deref());
        // 指标计数注入：MeasuredProvider 包装共享 provider（计数仅对本 span 生效）。
        let provider: Arc<dyn ModelProvider> =
            if self.model_calls.is_some() || self.team_request_budget.is_some() {
                let counter = self
                    .model_calls
                    .clone()
                    .unwrap_or_else(|| Arc::new(AtomicU64::new(0)));
                let request_usage = self.request_usage.clone().unwrap_or_else(|| {
                    Arc::new(workswarm_metrics::RequestUsageCollector::default())
                });
                let phase_scope_key = workswarm_metrics::request_scope_key(
                    task_context.step_id.as_deref().unwrap_or("unknown"),
                    task_context.phase_epoch,
                );
                let attempt_scope_key = workswarm_metrics::task_attempt_budget_scope_key(
                    &phase_scope_key,
                    task_context.task_id.as_deref(),
                    task_context.attempt_id.as_deref(),
                );
                Arc::new(
                    workswarm_metrics::MeasuredProvider::new_with_task_attempt_request_budget(
                        self.agent.provider(),
                        counter,
                        request_usage,
                        phase_scope_key,
                        attempt_scope_key,
                        self.team_request_budget.clone(),
                        task_context.model_calls_per_attempt.map(u64::from),
                    ),
                )
            } else {
                self.agent.provider()
            };
        // 审批器（七期 · 二路）：绑定作用域 → 白名单审批器，白名单取「角色 ∩ 绑定」
        // 交集（角色白名单空 = 绑定原样）；未绑定保持 AutoApprover 原行为。
        let scope = self.workspace_scope.as_ref();
        let allow_writes = !read_only && !task_has_no_write_scope;
        let workspace_approver;
        let fallback_approver = AutoApprover { allow: read_only };
        let scoped_approver: &dyn owo_agent_core::permissions::Approver = match scope {
            Some(s) => {
                let allowed = if let Some(task_allowed) = &task_write_allowed {
                    task_allowed.clone()
                } else if self.write_allowed.is_empty() {
                    s.allowed.clone()
                } else {
                    self.write_allowed.clone()
                };
                workspace_approver = project_workspace::WorkspaceScopeApprover {
                    allow_writes,
                    root: s.root.clone(),
                    allowed,
                };
                &workspace_approver
            }
            None => &fallback_approver,
        };
        let preauthorized_writes = self.scoped_write_rule.as_ref().map(|rule| {
            rule.clone().for_scope(
                &effective_write_allowed,
                &self
                    .workspace_scope
                    .as_ref()
                    .map(|scope| scope.root.clone())
                    .unwrap_or_else(|| self.workspace.clone()),
            )
        });
        let workflow_approver = WorkflowApprover {
            scoped: scoped_approver,
            external: self.external_approver.as_ref(),
            preauthorized_writes,
        };
        let approver: &dyn owo_agent_core::permissions::Approver = &workflow_approver;
        // 中断标志（七期 · 二路）：优先团队取消桥共享标志（cancel → 即时置位，
        // run_turn 协作式中断）；None = 本地标志（行为退化为不可中断）。
        let local_abort = AtomicBool::new(false);
        let abort: &AtomicBool = match &self.cancel_flag {
            Some(flag) => flag,
            None => &local_abort,
        };
        // 七期（第二路）：画像执行器——注册表面即权限边界 + 模板预算回合上限；
        // 无画像（防御分支，现网构建路径恒有画像）退回通用执行器保持旧行为。
        let output = match task_profile.as_ref() {
            Some(task_profile) => {
                let raw_step_id = task_context
                    .step_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                let assigned_task_id = task_context.task_id.clone();
                let refs: Vec<String> = task_context
                    .read_refs
                    .iter()
                    .chain(task_context.contract_refs.iter())
                    .flatten()
                    .cloned()
                    .collect();
                let context_tool: Arc<dyn Tool> = Arc::new(TeamContextReadTool {
                    coordinator: Arc::clone(&self.coordinator),
                    team_id: self.team_id.clone(),
                    member_id: format!("m-{}", self.role),
                    step_id: raw_step_id.clone(),
                    task_id: assigned_task_id.clone(),
                    refs: refs.clone(),
                    workspace_root: self.workspace.clone(),
                });
                let artifact_tool: Arc<dyn Tool> = Arc::new(TeamArtifactReadTool {
                    coordinator: Arc::clone(&self.coordinator),
                    team_id: self.team_id.clone(),
                    member_id: format!("m-{}", self.role),
                    step_id: raw_step_id.clone(),
                });
                let mut extra_tools: Vec<Arc<dyn Tool>> = vec![context_tool, artifact_tool];
                extra_tools.extend(self.agent.declared_workflow_tools(&task_profile.visible_tools)?);
                let can_publish_context = task_context
                    .required_capabilities
                    .as_deref()
                    .map(|capabilities| {
                        capabilities
                            .iter()
                            .any(|capability| capability == "team_context_publish")
                    })
                    .unwrap_or(true);
                if !task_profile.read_only && allow_writes && can_publish_context {
                    extra_tools.push(Arc::new(TeamContextPublishTool {
                        coordinator: Arc::clone(&self.coordinator),
                        team_id: self.team_id.clone(),
                        member_id: format!("m-{}", self.role),
                        task_id: assigned_task_id.clone().unwrap_or_else(|| raw_step_id.clone()),
                        allowed_source_refs: refs,
                    }));
                }
                let tool_started_at = Arc::new(Mutex::new(HashMap::<String, Instant>::new()));
                let coordinator = Arc::clone(&self.coordinator);
                let team_id = self.team_id.clone();
                let role = self.role.clone();
                let event_step_id = raw_step_id.clone();
                let event_task_id = assigned_task_id.clone().unwrap_or_else(|| raw_step_id.clone());
                let event_attempt_id = task_context.attempt_id.clone();
                let replay_store = Arc::clone(&self.session_store);
                let replay_session_id = worker_session_id(
                    &self.team_id,
                    assigned_task_id.as_deref().unwrap_or(&raw_step_id),
                    task_context.attempt_id.as_deref().unwrap_or("unbound"),
                );
                let replay_role = self.role.clone();
                let replay_model = model.clone();
                let event_sink: TurnEventSink = Arc::new(move |event| {
                    let replay = match event {
                        TurnEvent::ModelCall => Some(owo_agent_protocol::SseEvent::Progress {
                            message: format!("{} · 模型 {} · 正在执行", replay_role, replay_model),
                        }),
                        TurnEvent::TokenDelta { delta } => Some(owo_agent_protocol::SseEvent::TokenDelta { delta: delta.clone() }),
                        TurnEvent::Final { text } => Some(owo_agent_protocol::SseEvent::Final { text: text.clone() }),
                        TurnEvent::ToolStart { id, tool, args_preview } => Some(owo_agent_protocol::SseEvent::ToolUse {
                            id: id.clone(), tool: tool.clone(), args: serde_json::json!(args_preview),
                        }),
                        TurnEvent::ToolResult { id, tool, ok, error, preview, outputs, .. } => Some(owo_agent_protocol::SseEvent::ToolResult {
                            id: id.clone(), tool: tool.clone(), ok: *ok, error: error.clone(), preview: preview.clone(), outputs: outputs.clone(),
                        }),
                        TurnEvent::PermissionRequest(request) => Some(owo_agent_protocol::SseEvent::Progress {
                            message: format!("等待审批：{}", request.tool),
                        }),
                        TurnEvent::PlanUpdate { steps } => Some(owo_agent_protocol::SseEvent::PlanUpdate { steps: steps.clone() }),
                        _ => None,
                    };
                    if let Some(payload) = replay {
                        // The same store supplies ordinary chat replay; no parallel transcript database.
                        if let Err(error) = replay_store.append_turn_event(&replay_session_id, &replay_role, &payload) {
                            coordinator.record_runtime_event(&team_id, "team.conversation.failed", error.to_string());
                        }
                    }
                    if let TurnEvent::ToolResult {
                        command_receipt: Some(receipt),
                        ..
                    } = event
                    {
                        let detail = serde_json::json!({
                            "step_id": &event_step_id,
                            "task_id": &event_task_id,
                            "attempt_id": &event_attempt_id,
                            "receipt": receipt,
                        })
                        .to_string();
                        coordinator.record_runtime_event(
                            &team_id,
                            "team.command.executed",
                            detail,
                        );
                    }
                    if let Some(detail) = safe_team_model_event(event, &role, &event_step_id) {
                        coordinator.record_runtime_event(&team_id, "team.model.started", detail);
                        return;
                    }
                    let duration_ms = match event {
                        TurnEvent::ToolStart { id, .. } => {
                            if let Ok(mut started) = tool_started_at.lock() {
                                started.insert(id.clone(), Instant::now());
                            }
                            None
                        }
                        TurnEvent::ToolResult { id, .. } => tool_started_at.lock().ok()
                            .and_then(|mut started| started.remove(id))
                            .map(|started| started.elapsed().as_millis() as u64),
                        _ => return,
                    };
                    if let Some((event_name, detail)) =
                        safe_team_tool_event(event, &role, &event_step_id, duration_ms)
                    {
                        coordinator.record_runtime_event(&team_id, event_name, detail);
                    }
                });
                let runner = ProfileSubagentRunner {
                    provider, parent_policy: self.agent.policy(), approver, abort, depth: 0, model,
                    is_critic: self.is_critic,
                    write_allowed: effective_write_allowed,
                    profile: task_profile.clone(),
                    agent_config: None,
                    budget_note_override: task_context.model_calls_per_attempt.map(|total| {
                        format!(
                            "宿主分配的当前任务尝试总预算为 {total} 次模型请求；Agent 执行最多 {} 轮，最多保留 1 次请求修复 WorkerOutputV1 格式。达到上限必须如实返回状态，不得将未完成工作报告为 Done。\n",
                            usize::from(total).saturating_sub(1)
                        )
                    }),
                    extra_tools,
                    extra_system_prompt: Some(
                         "如需获取执行期间新增的团队事实，请调用 team_context_read。需要上游产物相关正文时使用 team_artifact_read，它仅允许读取当前直接依赖。大产物按 offset_bytes 分页；eof=false 时使用 next_offset_bytes 和 sha256 作为下次 offset_bytes、expected_sha256；不要重复读取第一页，按需读取，确需全文时读到 eof=true。具备当前任务写权限时可使用 team_context_publish：source_refs 仅填写当前任务已分配的 read_refs/contract_refs；带 file_hash 时 source_refs 必须是唯一安全工作区相对路径。先读取 revision，再以 expected_revision 发布；冲突后重读。发布结果始终是 candidate/unverified\n".to_string(),
                    ),
                    event_sink: Some(event_sink),
                    session_store: Some(Arc::clone(&self.session_store)),
                    worker_session_id: Some(worker_session_id(
                        &self.team_id,
                        assigned_task_id.as_deref().unwrap_or(&raw_step_id),
                        task_context.attempt_id.as_deref().unwrap_or("unbound"),
                    )),
                    parent_session_id: self.parent_session_id.clone(),
                };
                let result = runner.run(&self.workspace, &prompt).await;
                if let Err(error) = &result {
                    let session_id = worker_session_id(
                        &self.team_id,
                        assigned_task_id.as_deref().unwrap_or(&raw_step_id),
                        task_context.attempt_id.as_deref().unwrap_or("unbound"),
                    );
                    let _ = self.session_store.append_turn_event(
                        &session_id, &self.role,
                        &owo_agent_protocol::SseEvent::Progress {
                            message: format!("节点执行失败：{error}"),
                        },
                    );
                }
                result
            }
            None => {
                let runner = SubagentRunner {
                    provider,
                    parent_policy: self.agent.policy(),
                    approver,
                    abort,
                    depth: 0,
                    max_turns: task_context
                        .model_calls_per_attempt
                        .map(|total| WorkerProfile::task_agent_turn_cap(usize::from(total)))
                        .transpose()?
                        .unwrap_or(12),
                    model,
                    events: None,
                };
                runner.run(&self.workspace, &prompt, read_only).await
            }
        }
        .map_err(|e| format!("agent 子代理执行失败：{e}"))?;
        Ok(output)
    }
}

struct WorkflowApprover<'a> {
    scoped: &'a dyn owo_agent_core::permissions::Approver,
    external: &'a dyn owo_agent_core::permissions::Approver,
    /// 用户自定义工作流：建队时勾选「允许节点按声明的范围写入」后的预授权规则。
    ///
    /// 团队运行没有交互式审批通道（`/approvals/pending` 里永远没有条目），
    /// 范围内写入若还走询问就只有超时拒绝一条路——这正是"自定义工作流必然失败"
    /// 的根因。预授权只覆盖该角色 `write_paths` 内的落盘工具，其余请求原样交给
    /// 下面的 scoped/external 审批器（写范围外、命令、联网、MCP 都不放行）。
    preauthorized_writes: Option<super::scoped_write_approver::ScopedWriteRule>,
}
#[async_trait]
impl owo_agent_core::permissions::Approver for WorkflowApprover<'_> {
    async fn decide(
        &self,
        request: &owo_agent_core::permissions::PermissionRequest,
    ) -> owo_agent_core::permissions::Decision {
        if let Some(rule) = self.preauthorized_writes.as_ref() {
            if rule.is_scoped_write(request) {
                return rule.authorize().0;
            }
        }
        let external = request.tool.starts_with("mcp__")
            || matches!(
                request.tool.as_str(),
                "web_fetch" | "web_search" | "generate_image" | "text_to_speech"
            );
        if external {
            self.external.decide(request).await
        } else {
            self.scoped.decide(request).await
        }
    }
    async fn decide_with_notification(
        &self,
        request: &owo_agent_core::permissions::PermissionRequest,
        notify: &mut (dyn FnMut() + Send),
    ) -> owo_agent_core::permissions::ApprovalOutcome {
        if let Some(rule) = self.preauthorized_writes.as_ref() {
            if rule.is_scoped_write(request) {
                return rule.authorize().1;
            }
        }
        let external = request.tool.starts_with("mcp__")
            || matches!(
                request.tool.as_str(),
                "web_fetch" | "web_search" | "generate_image" | "text_to_speech"
            );
        if external {
            self.external
                .decide_with_notification(request, notify)
                .await
        } else {
            self.scoped.decide_with_notification(request, notify).await
        }
    }
}

fn worker_session_id(team_id: &str, task_id: &str, attempt_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{team_id}\0{task_id}\0{attempt_id}").as_bytes());
    let suffix = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{team_id}-worker-{suffix}")
}

fn safe_team_model_event(event: &TurnEvent, role: &str, step_id: &str) -> Option<String> {
    if !matches!(event, TurnEvent::ModelCall) {
        return None;
    }
    let safe_role = role.chars().take(80).collect::<String>();
    let safe_step = step_id.chars().take(100).collect::<String>();
    Some(format!("role={safe_role} step_id={safe_step}"))
}

fn safe_team_tool_event(
    event: &TurnEvent,
    role: &str,
    step_id: &str,
    duration_ms: Option<u64>,
) -> Option<(&'static str, String)> {
    let (event_name, tool, outcome) = match event {
        TurnEvent::ToolStart { tool, .. } => ("team.tool.started", tool.as_str(), None),
        TurnEvent::ToolResult { tool, ok, .. } => (
            "team.tool.finished",
            tool.as_str(),
            Some(if *ok { "succeeded" } else { "failed" }),
        ),
        _ => return None,
    };
    let safe_tool = tool.chars().take(80).collect::<String>();
    let safe_role = role.chars().take(80).collect::<String>();
    let safe_step = step_id.chars().take(100).collect::<String>();
    let detail = match (outcome, duration_ms) {
        (Some(outcome), Some(ms)) => format!("role={safe_role} step_id={safe_step} tool={safe_tool} outcome={outcome} duration_ms={ms}"),
        (Some(outcome), None) => format!("role={safe_role} step_id={safe_step} tool={safe_tool} outcome={outcome}"),
        (None, _) => format!("role={safe_role} step_id={safe_step} tool={safe_tool}"),
    };
    Some((event_name, detail))
}

fn task_lacks_file_write_capability(task: &ResolvedTaskContext) -> bool {
    let scoped_media_output = task
        .required_capabilities
        .as_deref()
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .any(|tool| matches!(tool.as_str(), "generate_image" | "text_to_speech"))
        })
        && task
            .write_paths
            .as_deref()
            .is_some_and(|paths| paths.iter().any(|path| path.trim() == "outputs"));
    task.lacks_file_write_capability() && !scoped_media_output
}

/// Compatibility adapter; the shared capability narrowing lives in Core.
fn apply_task_capability_scope(
    profile: &mut WorkerProfile,
    task: &ResolvedTaskContext,
    task_has_no_write_scope: bool,
) {
    profile.apply_task_capability_scope(task, task_has_no_write_scope);
}

fn prompt_workspace_write_paths(
    workspace_root: &std::path::Path,
    paths: &[PathBuf],
) -> Vec<String> {
    WorkerProfile::workspace_relative_write_paths(workspace_root, paths)
}

/// Compatibility adapter for existing server call sites and regression tests.
fn assigned_task_write_allowlist(
    task: &ResolvedTaskContext,
    workspace_root: &std::path::Path,
    role_allowed: &[PathBuf],
) -> Result<Option<Vec<PathBuf>>, String> {
    WorkerProfile::resolve_task_write_allowlist(task, workspace_root, role_allowed)
}

#[cfg(test)]
mod team_context_scope_tests {
    use super::*;

    #[test]
    fn prompt_write_paths_are_relative_and_never_expose_outside_paths() {
        let workspace = tempfile::tempdir().unwrap();
        let allowed = vec![
            workspace.path().join("apps/api"),
            workspace.path().join("src/lib.rs"),
        ];
        assert_eq!(
            prompt_workspace_write_paths(workspace.path(), &allowed),
            vec!["apps/api", "src/lib.rs"]
        );
        let outside = vec![workspace
            .path()
            .parent()
            .unwrap()
            .join("outside-secret-path")];
        assert_eq!(
            prompt_workspace_write_paths(workspace.path(), &outside),
            vec!["<outside-workspace-denied>"]
        );
    }

    #[test]
    fn worker_sessions_are_stable_per_team_task_and_attempt() {
        let first = worker_session_id("team-a", "task-a", "attempt-1");
        assert_eq!(first, worker_session_id("team-a", "task-a", "attempt-1"));
        assert_ne!(first, worker_session_id("team-a", "task-a", "attempt-2"));
        assert_ne!(first, worker_session_id("team-a", "task-b", "attempt-1"));
        assert_ne!(first, worker_session_id("team-b", "task-a", "attempt-1"));
        assert!(first.starts_with("team-a-worker-"));
        assert_eq!(first.len(), "team-a-worker-".len() + 64);
    }

    fn resolved_task(value: &Value) -> ResolvedTaskContext {
        ResolvedTaskContext::from_worker_input(value).expect("test task context must parse")
    }

    #[test]
    fn task_capabilities_narrow_role_tools_and_do_not_grant_write() {
        let mut profile = WorkerProfile::explicit_writer(4);
        apply_task_capability_scope(
            &mut profile,
            &resolved_task(&json!({
                "required_capabilities": ["read_file"],
                "assigned_write_paths": ["apps/api"]
            })),
            false,
        );
        assert!(profile.read_only);
        assert!(!profile.can_run_command);
        assert_eq!(profile.visible_tools, vec!["read_file".to_string()]);
        let read_tools = profile
            .build_registry(Vec::new())
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(read_tools, vec!["read_file".to_string()]);

        let mut writer = WorkerProfile::explicit_writer(4);
        apply_task_capability_scope(
            &mut writer,
            &resolved_task(&json!({ "required_capabilities": ["write_file"] })),
            false,
        );
        assert!(!writer.read_only);
        assert_eq!(writer.visible_tools, vec!["write_file".to_string()]);
        let write_tools = writer
            .build_registry(Vec::new())
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(write_tools, vec!["write_file".to_string()]);

        let mut scoped_out = WorkerProfile::explicit_writer(4);
        apply_task_capability_scope(
            &mut scoped_out,
            &resolved_task(&json!({ "required_capabilities": ["write_file"] })),
            true,
        );
        assert!(scoped_out.read_only);
        assert!(!scoped_out
            .visible_tools
            .iter()
            .any(|tool| tool == "write_file"));

        let mut bounded = WorkerProfile::explicit_writer(4);
        apply_task_capability_scope(
            &mut bounded,
            &resolved_task(&json!({
                "required_capabilities": ["write_file", "run_command"],
                "assigned_verification": {
                    "requirements": [{
                        "validator_id": "workspace-command-success-v1",
                        "resources": {"timeout_ms": 1234}
                    }]
                }
            })),
            false,
        );
        assert_eq!(bounded.verification_timeout_ms, Some(1234));

        let mut empty = WorkerProfile::explicit_writer(4);
        apply_task_capability_scope(
            &mut empty,
            &resolved_task(&json!({ "required_capabilities": [] })),
            false,
        );
        assert!(empty.read_only);
        assert_eq!(
            empty.visible_tools,
            vec!["__owo_no_task_tool__".to_string()]
        );
        assert!(empty.build_registry(Vec::new()).specs().is_empty());

        assert!(task_lacks_file_write_capability(&resolved_task(&json!({
            "assigned_task_id": "read-only",
            "assigned_task": "read task",
            "required_capabilities": ["read_file"]
        }))));
        assert!(!task_lacks_file_write_capability(&resolved_task(&json!({
            "assigned_task_id": "writer",
            "assigned_task": "write task",
            "required_capabilities": ["write_file"]
        }))));
        assert!(!task_lacks_file_write_capability(&resolved_task(&json!({
            "required_capabilities": []
        }))));
        assert!(task_lacks_file_write_capability(&resolved_task(&json!({
            "assigned_task_id": "missing-capabilities",
            "assigned_task": "task with omitted capabilities"
        }))));
    }

    #[test]
    fn context_tools_have_read_and_explicit_write_effects() {
        let read = team_context_read_effect();
        assert_eq!(read.class, EffectClass::Read);
        assert!(read.host_verified_readonly);
        let publish = team_context_publish_effect();
        assert_eq!(publish.class, EffectClass::Write);
        assert!(!publish.host_verified_readonly);
        let artifact = team_artifact_read_effect();
        assert_eq!(artifact.class, EffectClass::Read);
        assert!(artifact.host_verified_readonly);
    }

    #[test]
    fn context_facts_are_limited_to_global_task_step_or_refs() {
        let fact = |task_id: Option<&str>, refs: &[&str]| owo_agent_protocol::SharedContextFact {
            key: "k".into(),
            value_ref: "cas://sha256:x".into(),
            revision: 1,
            producer: "m-worker".into(),
            task_id: task_id.map(str::to_string),
            source_refs: refs.iter().map(|s| (*s).into()).collect(),
            file_hash: None,
            confidence: "unverified".into(),
            status: "candidate".into(),
            created_at: "now".into(),
        };
        let refs = vec!["src/api.rs".to_string()];
        assert!(team_context_fact_is_visible(
            &fact(None, &[]),
            Some("task"),
            "step",
            &refs
        ));
        assert!(team_context_fact_is_visible(
            &fact(Some("task"), &[]),
            Some("task"),
            "step",
            &refs
        ));
        assert!(team_context_fact_is_visible(
            &fact(Some("step"), &[]),
            Some("task"),
            "step",
            &refs
        ));
        assert!(team_context_fact_is_visible(
            &fact(Some("other"), &["src/api.rs"]),
            Some("task"),
            "step",
            &refs
        ));
        assert!(!team_context_fact_is_visible(
            &fact(Some("other"), &["docs/readme.md"]),
            Some("task"),
            "step",
            &refs
        ));
    }

    #[tokio::test]
    async fn context_fact_freshness_detects_changes_and_rejects_escaping_refs() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        let source = root.path().join("src/api.rs");
        std::fs::write(&source, b"pub fn api() {}").unwrap();
        let digest = owo_agent_core::CasStore::hash_of(b"pub fn api() {}");
        let fact =
            |refs: Vec<String>, hash: Option<String>| owo_agent_protocol::SharedContextFact {
                key: "api".into(),
                value_ref: "cas://sha256:aa".into(),
                revision: 1,
                producer: "m-w".into(),
                task_id: None,
                source_refs: refs,
                file_hash: hash,
                confidence: "unverified".into(),
                status: "candidate".into(),
                created_at: "now".into(),
            };
        let current = fact(vec!["src/api.rs".into()], Some(format!("sha256:{digest}")));
        assert_eq!(
            team_context_fact_freshness(&current, root.path()).await,
            "current"
        );
        std::fs::write(&source, b"changed").unwrap();
        assert_eq!(
            team_context_fact_freshness(&current, root.path()).await,
            "stale"
        );
        let escape = fact(
            vec!["../outside.rs".into()],
            Some(format!("sha256:{digest}")),
        );
        assert_eq!(
            team_context_fact_freshness(&escape, root.path()).await,
            "stale"
        );
        let ambiguous = fact(
            vec!["src/api.rs".into(), "api".into()],
            Some(format!("sha256:{digest}")),
        );
        assert_eq!(
            team_context_fact_freshness(&ambiguous, root.path()).await,
            "unverifiable"
        );
    }

    #[test]
    fn safe_model_events_record_only_bounded_identifiers() {
        let event = TurnEvent::ModelCall;
        let detail = safe_team_model_event(&event, &"r".repeat(100), &"s".repeat(120)).unwrap();
        assert!(detail.starts_with(&format!("role={} step_id=", "r".repeat(80))));
        assert!(detail.chars().count() <= 5 + 80 + 9 + 100);
        assert!(safe_team_model_event(
            &TurnEvent::Final {
                text: "private".into()
            },
            "worker",
            "step"
        )
        .is_none());
        assert!(!detail.contains("private"));
    }

    #[test]
    fn safe_tool_events_exclude_arguments_previews_and_error_text() {
        let start = TurnEvent::ToolStart {
            id: "call-1".into(),
            tool: "read_file".into(),
            args_preview: Some("secret/path".into()),
        };
        let (_, detail) = safe_team_tool_event(&start, "worker", "step-1", None).unwrap();
        assert!(detail.contains("tool=read_file"));
        assert!(!detail.contains("secret/path"));
        let finish = TurnEvent::ToolResult {
            id: "call-1".into(),
            tool: "write_file".into(),
            ok: false,
            error: Some("private contents".into()),
            preview: Some("payload".into()),
            command_receipt: None,
        };
        let (_, detail) = safe_team_tool_event(&finish, "worker", "step-1", Some(12)).unwrap();
        assert!(detail.contains("outcome=failed"));
        assert!(detail.contains("duration_ms=12"));
        assert!(!detail.contains("private contents"));
        assert!(!detail.contains("payload"));
    }
}

pub(super) struct EchoWorker;

#[async_trait]
impl Worker for EchoWorker {
    fn name(&self) -> &str {
        "echo"
    }
    async fn run(&self, input: &Value) -> Result<String, String> {
        Ok(input
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| input.to_string()))
    }
}

pub(super) struct SleepWorker;

#[async_trait]
impl Worker for SleepWorker {
    fn name(&self) -> &str {
        "sleep"
    }
    async fn run(&self, input: &Value) -> Result<String, String> {
        let ms = input
            .get("ms")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(600_000);
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(format!("slept {ms}ms"))
    }
}

pub(super) struct FailWorker;

#[async_trait]
impl Worker for FailWorker {
    fn name(&self) -> &str {
        "fail"
    }
    async fn run(&self, input: &Value) -> Result<String, String> {
        Err(input
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "fail worker 注入失败".to_string()))
    }
}

/// 追踪型 worker 包装（七期 · 二路）：插在内层 worker 与 `RoleWorker` 之间——
/// 1) 写角色按其声明的写范围取得租约；不重叠的路径可并发，未声明范围按工作区
///    保守互斥；读角色不参与写租约；
/// 2) 拿到租约后采集前快照（等待租约的时间不计入变更窗口——否则会把其他写角色
///    正在进行的变更记到本步骤头上）；
/// 3) 执行后采集后快照，登记「变更文件 + diff 摘要 + diff ref」，并对窗口内新增
///    变更做写白名单校验——越界转 `scope_violation` 步骤失败（引擎按失败处理，
///    不登记成功 Artifact）。
///
/// 仅写角色包装追踪（`tracking`/`lease` 均为 None 时纯透传）：读角色没有写工具
/// 不会改文件，而并行读角色的快照窗口会误捕写角色的变更。
///
/// 八期（二路）：执行前对允许路径做内容基线快照（进 CAS）；成功路径自动生成
/// `ChangeSet`（pending_review，等人工 accept/reject/revert；未接受时该团队代码
/// Artifact 不得成为最终 approved head——门控函数见 `change_set_store`）。
pub(super) struct TrackedRoleWorker {
    pub(super) inner: Arc<dyn Worker>,
    /// 范围写租约（None = 只读角色，不参与租约）。
    pub(super) lease: Option<WriteLease>,
    pub(super) lease_waits: Option<Arc<workswarm_metrics::LeaseWaitTracker>>,
    /// 变更追踪配置（None = 不追踪，纯透传）。
    pub(super) tracking: Option<workspace_change_tracker::Tracker>,
}

#[async_trait]
impl Worker for TrackedRoleWorker {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn run(&self, input: &Value) -> Result<String, String> {
        let Some(mut tracking) = self.tracking.clone() else {
            return self.inner.run(input).await;
        };
        let task_context = ResolvedTaskContext::from_worker_input(input)?;
        let task_graph = task_context.is_task_graph_assignment();
        if task_graph {
            let task_paths =
                assigned_task_write_allowlist(&task_context, &tracking.root, &tracking.allowed)?
                    .ok_or_else(|| "TaskGraph 任务缺少 assigned_write_paths".to_string())?;
            if task_lacks_file_write_capability(&task_context) || task_paths.is_empty() {
                return self.inner.run(input).await;
            }
            tracking.allowed = task_paths;
        }
        // TaskGraph 的租约与追踪均缩到当前任务写范围；同槽位的互不重叠任务
        // 可以并行，冲突范围仍由共享工作区仲裁器串行化。
        // 十期·四路 R1：租约覆盖「前快照 → 执行 → 后快照 → 变更登记」全程——
        // 变更登记（record + ChangeSet upsert）完成前不释放：并发写步骤若在
        // 本步骤后快照前插窗口，会把本步骤的变更误算进它的前基线，反之亦然。
        let step_id = task_context.step_id.as_deref().unwrap_or("unknown");
        let scope_key = workswarm_metrics::request_scope_key(step_id, task_context.phase_epoch);
        let task_lease = if task_graph {
            self.lease
                .as_ref()
                .map(|lease| lease.for_paths(&tracking.allowed))
                .transpose()?
        } else {
            None
        };
        let lease = task_lease.as_ref().or(self.lease.as_ref());
        let lease_wait_started = Instant::now();
        let _lease = match lease {
            Some(lease) => Some(lease.acquire().await),
            None => None,
        };
        if let Some(waits) = &self.lease_waits {
            waits.record(&scope_key, lease_wait_started.elapsed().as_millis() as u64);
        }
        let pre = workspace_change_tracker::GitSnapshot::snapshot(&tracking.root).await;
        // 八期（二路）：执行前对允许路径做内容基线快照（进 CAS）——ChangeSet
        // reject/revert 的恢复依据；快照尽力而为（超限/读取失败的文件基线记为
        // 「未知」，之后被改只能走 conflicted 人工路径，绝不误删）。
        let base = owo_agent_core::change_set::snapshot_allowed_paths(
            &tracking.root,
            &tracking.allowed,
            &tracking.cas,
        )
        .await;
        // 九期（一路）：执行前已脏文件的内容哈希——合并变更检测的第二基线。
        // 允许路径内的文件直接复用 base.entries（同一份数据，避免重复读盘）；
        // 之外（或基线未覆盖）的执行前脏文件现场读哈希。
        let mut pre_dirty_hashes: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        for path in pre.dirty_paths() {
            if let Some(hash) = base.entries.get(&path) {
                pre_dirty_hashes.insert(path, Some(hash.clone()));
            } else {
                let hash = workspace_change_tracker::content_hash(&tracking.root, &path);
                pre_dirty_hashes.insert(path, hash);
            }
        }
        // 执行（成败、超时、取消都以 result 承接——变更收尾对四种结局一视同仁）。
        let result = self.inner.run(input).await;
        // ---- 变更收尾：成功/失败/超时/取消都必须完成 ----
        let post = workspace_change_tracker::GitSnapshot::snapshot(&tracking.root).await;
        // 十期·四路 R1：非 git 目录（含 git 快照不可用）改用内容哈希检测——
        // 执行后对同一组允许路径再采内容快照，与执行前 CAS 基线逐文件比哈希
        // （新增/修改/删除），不依赖 git。
        let (changed, detection) = if pre.git && post.git {
            (
                workspace_change_tracker::merge_changed_files(
                    &pre,
                    &post,
                    &pre_dirty_hashes,
                    &tracking.root,
                ),
                "git 窗口+内容哈希",
            )
        } else {
            let post_base = owo_agent_core::change_set::snapshot_allowed_paths(
                &tracking.root,
                &tracking.allowed,
                &tracking.cas,
            )
            .await;
            (
                owo_agent_core::change_set::merge_content_snapshots(&base, &post_base),
                "内容哈希（非 git）",
            )
        };
        let foreign_scopes: Vec<WriteScope> = _lease
            .as_ref()
            .map(|guard| guard.foreign_scopes_in_window(pre.at, post.at))
            .unwrap_or_default();
        let changed: Vec<String> = if tracking.allowed.is_empty() {
            changed
        } else {
            let pre_dirty: HashSet<String> = pre.dirty_paths().into_iter().collect();
            changed
                .into_iter()
                .filter(|path| {
                    if workspace_change_tracker::path_in_allowed(
                        path,
                        &tracking.root,
                        &tracking.allowed,
                    ) {
                        return true;
                    }
                    if foreign_scopes
                        .iter()
                        .any(|scope| scope.covers(&tracking.root, path))
                    {
                        return false;
                    }
                    !pre_dirty.contains(path)
                })
                .collect()
        };
        let violation =
            workspace_change_tracker::check_whitelist(&changed, &tracking.root, &tracking.allowed)
                .err();
        let step = step_id.to_string();
        let (record, record_error) = match tracking
            .record(&step, &post, &changed, violation.as_deref(), Some(&base))
            .await
        {
            Ok(record) => (Some(record), None),
            Err(error) => {
                tracing::error!(
                    team_id = %tracking.team_id,
                    role = %tracking.role,
                    %error,
                    "工作区变更记录落盘失败"
                );
                (None, Some(error))
            }
        };
        if let Some(violation) = violation {
            // 越界：变更已留痕（record 含 violation 字段）但步骤失败——引擎按失败
            // 处理，不登记成功 Artifact；不生成可审批 ChangeSet（越界写入不可审查
            // 为合法工作区改动，恢复基线只会制造更大的不可控面）。
            return Err(violation);
        }
        // 九期（一路）：无实际变更 → 不创建空 pending ChangeSet（此前会生成
        // changed_files 为空的空壳待办，人工审批无法操作），只记审计留痕。
        if changed.is_empty() {
            if let Some(audit) = &tracking.audit {
                if let Ok(mut audit) = audit.lock() {
                    audit.record(
                        &tracking.team_id,
                        "change_set.no_change",
                        Some(format!("workswarm/{}", tracking.team_id)),
                        Some(true),
                        format!(
                            "步骤 {} 无实际工作区变更（检测：{detection}），跳过 ChangeSet 生成",
                            step
                        ),
                    );
                }
            }
            return match (result, record_error) {
                (Ok(_), Some(error)) => Err(format!("工作区变更记录未落盘，步骤拒绝成功：{error}")),
                (Err(original), Some(error)) => {
                    Err(format!("{original}；此外，工作区变更记录未落盘：{error}"))
                }
                (result, None) => result,
            };
        }
        // 十期·四路 R1：无论步骤成败都生成 ChangeSet（pending_review，等人工
        // accept/reject/revert）——「写入后失败」「写入中取消」留下的残留修改
        // 同样可审查、可恢复；未接受时该团队代码 Artifact 不得成为最终 approved
        // head。变更追踪或 ChangeSet 任一持久化失败时，成功输出必须 fail closed，
        // 否则 DeliveryGate 会把无法审计/复原的工作区变更当成交付成功。
        let mut change_set = owo_agent_core::change_set::build_change_set(
            &tracking.team_id,
            &step,
            &tracking.role,
            &base,
            &changed,
            &tracking.root,
            record.as_ref().and_then(|record| record.diff_ref.clone()),
        );
        change_set.attempt_id = task_context.attempt_id.clone();
        let store = owo_agent_core::change_set_store::ChangeSetStore::new(&tracking.run_dir);
        let changeset_error = match store.save_upsert(&change_set) {
            Ok(()) => {
                if let Some(audit) = &tracking.audit {
                    if let Ok(mut audit) = audit.lock() {
                        audit.record(
                            &tracking.team_id,
                            "change_set.created",
                            Some(format!("workswarm/{}", tracking.team_id)),
                            Some(true),
                            format!(
                                "{} 生成（步骤 {}，检测：{detection}，文件：{}）",
                                change_set.change_set_id,
                                step,
                                changed.join(", ")
                            ),
                        );
                    }
                }
                None
            }
            Err(error) => {
                tracing::error!(
                    team_id = %tracking.team_id,
                    role = %tracking.role,
                    error = %error,
                    "ChangeSet 落盘失败（变更审批闭环缺失）"
                );
                Some(error.to_string())
            }
        };
        let persistence_errors = record_error
            .into_iter()
            .map(|error| format!("工作区变更记录未落盘：{error}"))
            .chain(
                changeset_error
                    .into_iter()
                    .map(|error| format!("ChangeSet 未落盘：{error}")),
            )
            .collect::<Vec<_>>();
        // 租约在此作用域末尾释放。只有原执行和全部必要变更收据都持久化成功，
        // 步骤才可返回成功；原执行已失败时保留原错误并附上留证故障。
        if persistence_errors.is_empty() {
            return result;
        }
        let persistence_detail = persistence_errors.join("；");
        match result {
            Ok(_) => Err(format!(
                "工作区已变更，但交付留证不完整，步骤拒绝成功：{persistence_detail}"
            )),
            Err(error) => Err(format!(
                "{error}；此外，工作区变更留证不完整：{persistence_detail}"
            )),
        }
    }
}

#[cfg(test)]
mod worker_context_integration_tests {
    use super::*;
    use owo_agent_core::tools::ToolContext;

    #[tokio::test]
    async fn worker_context_publish_is_visible_on_next_read_and_prompt_slice() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        let store = Arc::new(
            owo_agent_core::SqliteProjectSpaceStore::open(&dir.path().join("space.db")).unwrap(),
        );
        let cas = owo_agent_core::CasStore::new(dir.path().join("cas")).unwrap();
        let templates = Arc::new(owo_agent_core::TeamTemplateRegistry::new(
            dir.path().join("templates"),
        ));
        let coordinator = Arc::new(owo_agent_core::TeamCoordinator::new(
            store, templates, cas, runs,
        ));
        let team = coordinator
            .create_team_run(&owo_agent_core::CreateTeamRequest::new(
                "验证 Worker 共享上下文",
                owo_agent_protocol::TeamMode::Single,
            ))
            .await
            .unwrap();
        let step_id = coordinator
            .load_run_state(&team.team_id)
            .unwrap()
            .plan
            .steps[0]
            .id
            .clone();

        let policy = owo_agent_core::Policy::new(dir.path());
        let mut session = owo_agent_core::Session::new(dir.path(), "test-model", None);
        let audit = Arc::new(std::sync::Mutex::new(
            owo_agent_core::audit::AuditLog::default(),
        ));
        let skills = owo_agent_core::SkillRegistry::default();
        let elements = Arc::new(std::sync::Mutex::new(owo_agent_core::ElementRegistry::new()));
        let mut tool_context = ToolContext {
            workspace: dir.path(),
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        let publisher = TeamContextPublishTool {
            coordinator: Arc::clone(&coordinator),
            team_id: team.team_id.clone(),
            member_id: "m-runner".into(),
            task_id: step_id.clone(),
            allowed_source_refs: vec!["src/api.rs".into()],
        };
        let published = publisher
            .run(
                &mut tool_context,
                json!({
                    "key":"api.contract",
                    "value":"GET /items returns ItemList",
                    "expected_revision":0,
                    "source_refs":["src/api.rs"]
                }),
            )
            .await
            .unwrap();
        assert_eq!(published["revision"], 1);
        assert_eq!(published["status"], "candidate");

        let draft = |key: &str| owo_agent_core::workswarm::SharedContextFactDraft {
            key: key.into(),
            value: format!("value for {key}"),
            producer: "m-runner".into(),
            task_id: None,
            source_refs: Vec::new(),
            file_hash: None,
        };
        let left = coordinator.publish_team_context_fact(&team.team_id, 1, draft("race.left"));
        let right = coordinator.publish_team_context_fact(&team.team_id, 1, draft("race.right"));
        let (left, right) = tokio::join!(left, right);
        assert_ne!(
            left.is_ok(),
            right.is_ok(),
            "相同 expected_revision 的并发发布必须恰有一个胜者"
        );
        assert_eq!(
            coordinator
                .read_team_context(&team.team_id)
                .await
                .unwrap()
                .revision,
            2
        );

        let reader = TeamContextReadTool {
            coordinator: Arc::clone(&coordinator),
            team_id: team.team_id.clone(),
            member_id: "m-runner".into(),
            step_id: step_id.clone(),
            task_id: Some(step_id.clone()),
            refs: vec!["src/api.rs".into()],
            workspace_root: dir.path().to_path_buf(),
        };
        let observed = reader
            .run(&mut tool_context, json!({"key":"api.contract"}))
            .await
            .unwrap();
        assert_eq!(observed["revision"], 2);
        assert_eq!(observed["facts"][0]["value"], "GET /items returns ItemList");

        let slice = coordinator
            .assemble_context_slice(&team.team_id, "m-runner", &step_id)
            .await
            .unwrap();
        assert_eq!(slice["shared_context_revision"], 2);
        assert!(slice["shared_facts"]
            .to_string()
            .contains("GET /items returns ItemList"));
    }
}

#[cfg(test)]
mod tracked_worker_parallel_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Barrier;

    struct ConcurrentProbe {
        barrier: Arc<Barrier>,
        entered: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Worker for ConcurrentProbe {
        fn name(&self) -> &str {
            "concurrent-probe"
        }

        async fn run(&self, _input: &Value) -> Result<String, String> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            self.barrier.wait().await;
            Ok("completed".to_string())
        }
    }

    #[tokio::test]
    async fn same_role_disjoint_task_scopes_enter_worker_concurrently() {
        let temp = tempfile::tempdir().expect("temporary workspace");
        let root = std::fs::canonicalize(temp.path()).expect("canonical workspace");
        let run_dir = root.join("run");
        std::fs::create_dir_all(root.join("src/a")).expect("task a scope");
        std::fs::create_dir_all(root.join("src/b")).expect("task b scope");
        std::fs::create_dir_all(&run_dir).expect("team run directory");

        let allowed = vec![root.join("src/a"), root.join("src/b")];
        let lease = WriteLease::new(
            super::super::write_lease::WriteLeaseManager::new(),
            WriteScope::from_paths(&allowed),
        );
        let tracker = workspace_change_tracker::Tracker {
            root: root.clone(),
            run_dir: run_dir.clone(),
            team_id: "team-parallel".to_string(),
            role: "w1".to_string(),
            allowed,
            cas: owo_agent_core::cas_store::CasStore::new(run_dir.join("cas"))
                .expect("CAS should initialize"),
            audit: None,
        };
        let entered = Arc::new(AtomicUsize::new(0));
        let worker = TrackedRoleWorker {
            inner: Arc::new(ConcurrentProbe {
                barrier: Arc::new(Barrier::new(2)),
                entered: Arc::clone(&entered),
            }),
            lease: Some(lease),
            lease_waits: Some(Arc::new(workswarm_metrics::LeaseWaitTracker::default())),
            tracking: Some(tracker),
        };
        let task_input = |task: &str, path: &str| {
            json!({
                "assigned_task_id": task,
                "assigned_task": format!("edit {task}"),
                "required_capabilities": ["write_file"],
                "assigned_write_paths": [path],
                "_workswarm": {"step_id": format!("s-{task}"), "phase_epoch": 1}
            })
        };
        let input_a = task_input("task-a", "src/a");
        let input_b = task_input("task-b", "src/b");
        let results = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(worker.run(&input_a), worker.run(&input_b))
        })
        .await
        .expect("disjoint task scopes must reach the worker together");

        assert!(results.0.is_ok(), "task-a wrapper failed: {:?}", results.0);
        assert!(results.1.is_ok(), "task-b wrapper failed: {:?}", results.1);
        assert_eq!(entered.load(Ordering::SeqCst), 2);
    }
}
