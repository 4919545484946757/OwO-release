//! Profile-to-runtime adapter for task-scoped Team workers.
//!
//! Capability resolution remains in the parent `worker_profile` module; this adapter
//! turns that resolved profile into a single `WorkerRuntime` invocation.

use super::{compile_worker_system_prompt, TurnEventSink, WorkerProfile, PROFILE_MAX_TURNS_CAP};
use crate::agent::AgentConfig;
use crate::gateway::ModelProvider;
use crate::permissions::{Approver, Policy};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Hard per-turn tool-call ceiling derived from the role profile. The global
/// bounded cap (64) is sized for interactive subagents; team workers repeatedly
/// burned it retrying the same failed edit/test cycle. Writers get enough room
/// for a few read/edit/verify passes; reviewers stay read-only and tighter.
fn profile_tool_call_cap(is_critic: bool, profile: &WorkerProfile) -> usize {
    if is_critic || !profile.is_writer() {
        16
    } else {
        24
    }
}

/// 画像驱动子代理执行器（七期 · 二路）：与一路 `ContractSubagentRunner` 同口径
/// （完整回合循环 + `WorkerOutputV1` 输出契约执行 + 至多一次定向修复），区别仅在：
///
/// - 工具注册表由 [`WorkerProfile::build_registry`] 按角色装配（注册表面即权限边界）；
/// - 回合上限取画像值（模板预算，硬上限 32）；
/// - 写面为「角色 ∩ 绑定」交集白名单工具（越界写入在工具层被拒）；
/// - `is_critic` 由服务端按角色名判定（`role == "critic"`；引擎注入的 `read_only`
///   只覆盖 critic，其余内置角色都是 producer，画像另管只读面）。
pub struct ProfileSubagentRunner<'a> {
    pub provider: Arc<dyn ModelProvider>,
    /// Host restrictions are inherited live; role constraints may only narrow them.
    pub parent_policy: &'a Policy,
    /// Optional live host tool enablement ceiling; Team workers always supply it.
    pub parent_agent: Option<Arc<crate::Agent>>,
    pub approver: &'a dyn Approver,
    /// 中断标志：团队取消桥共享置位，`run_turn` 协作式检查。
    pub abort: &'a AtomicBool,
    pub depth: usize,
    pub model: String,
    /// critic 角色代理（true = 只读探索口径，禁带 artifact；false = producer）。
    pub is_critic: bool,
    /// 最终写白名单（角色 ∩ 绑定交集；空 = 工作区内可写）。
    pub write_allowed: Vec<PathBuf>,
    pub profile: WorkerProfile,
    /// Optional runtime limits supplied by a controlled harness; execution still uses this runner.
    pub agent_config: Option<AgentConfig>,
    /// Optional caller-specific budget wording; role contract/tool assembly stay shared.
    pub budget_note_override: Option<String>,
    /// Team 宿主提供的额外受控工具（仍由 ToolHost 执行）。
    pub extra_tools: Vec<Arc<dyn crate::tools::Tool>>,
    /// 可选的 Team 共享上下文使用说明。
    pub extra_system_prompt: Option<String>,
    /// 可选的脱敏回合事件出口；调用方只应记录安全元数据，不记录参数/结果正文。
    pub event_sink: Option<TurnEventSink>,
    /// Daemon session store, used to resume this team/task history on local rework.
    pub session_store: Option<Arc<dyn crate::session::SessionStore>>,
    pub worker_session_id: Option<String>,
    /// Source user session retained as the worker session parent.
    pub parent_session_id: Option<String>,
}

/// Measured result from the shared Team worker runtime.
#[derive(Debug, Clone)]
pub struct ProfileSubagentRunReport {
    pub output: String,
    pub duration_ms: u64,
    pub steps: usize,
    pub model_calls: u32,
    pub usage: crate::gateway::TokenUsage,
    /// False whenever any request, including contract repair, lacks attributable usage.
    pub usage_known: bool,
    pub output_repairs: u32,
}

/// Failure telemetry is retained so eval and production diagnostics do not hide
/// the cost of a rejected worker submission or its contract-repair request.
#[derive(Debug, Clone)]
pub struct ProfileSubagentRunError {
    pub message: String,
    pub duration_ms: u64,
    pub steps: usize,
    pub model_calls: u32,
    pub usage: crate::gateway::TokenUsage,
    pub usage_known: bool,
    pub output_repairs: u32,
}

impl From<String> for ProfileSubagentRunError {
    fn from(message: String) -> Self {
        Self {
            message,
            duration_ms: 0,
            steps: 0,
            model_calls: 0,
            usage: crate::gateway::TokenUsage::default(),
            usage_known: false,
            output_repairs: 0,
        }
    }
}

impl ProfileSubagentRunner<'_> {
    /// Compatibility entry point for production call sites.
    pub async fn run(&self, workspace: &Path, prompt: &str) -> Result<String, String> {
        self.run_report(workspace, prompt)
            .await
            .map(|report| report.output)
            .map_err(|error| error.message)
    }

    pub async fn run_with_images(
        &self,
        workspace: &Path,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
    ) -> Result<String, String> {
        self.run_report_with_images(workspace, prompt, images)
            .await
            .map(|report| report.output)
            .map_err(|error| error.message)
    }

    /// Resolve the capability profile here, then delegate all worker execution to WorkerRuntime.
    pub async fn run_report(
        &self,
        workspace: &Path,
        prompt: &str,
    ) -> Result<ProfileSubagentRunReport, ProfileSubagentRunError> {
        self.run_report_with_images(workspace, prompt, &[]).await
    }

    pub async fn run_report_with_images(
        &self,
        workspace: &Path,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
    ) -> Result<ProfileSubagentRunReport, ProfileSubagentRunError> {
        self.run_report_with_input(workspace, prompt, images, None)
            .await
    }

    /// Host-validated input refs are persisted with the same turn that receives the pixels.
    pub async fn run_with_input(
        &self,
        workspace: &Path,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        input: owo_agent_protocol::StoredUserInput,
    ) -> Result<String, String> {
        self.run_report_with_input(workspace, prompt, images, Some(input))
            .await
            .map(|r| r.output)
            .map_err(|e| e.message)
    }

    pub async fn run_report_with_input(
        &self,
        workspace: &Path,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        input: Option<owo_agent_protocol::StoredUserInput>,
    ) -> Result<ProfileSubagentRunReport, ProfileSubagentRunError> {
        if let Some(parent) = &self.parent_agent {
            let issues = parent.preflight_workflow_profile(&self.profile)?;
            if !issues.is_empty() {
                return Err(format!(
                    "workflow preflight: {}",
                    serde_json::to_string(&issues).unwrap_or_default()
                )
                .into());
            }
        }
        let policy = self.parent_policy.for_workspace(workspace.to_path_buf())?;
        let has_external_tools = self.profile.visible_tools.iter().any(|name| {
            name.starts_with("mcp__") || matches!(name.as_str(), "web_fetch" | "web_search")
        });
        let policy = if self.is_critic || (self.profile.read_only && !has_external_tools) {
            policy.read_only_scope()
        } else {
            policy
        };
        let mut registry = self.profile.build_registry(self.write_allowed.clone());
        for tool in &self.extra_tools {
            registry.register_arc(crate::tools::scoped_workflow_tool(
                Arc::clone(tool),
                self.write_allowed.clone(),
            ));
        }
        if let Some(parent) = &self.parent_agent {
            parent.guard_workflow_registry(&mut registry)?;
        }
        let registered = registry.specs();
        let missing = self
            .profile
            .visible_tools
            .iter()
            .filter(|name| {
                !name.starts_with("__owo_") && !registered.iter().any(|spec| spec.name == **name)
            })
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!("workflow tools unavailable: {}", missing.join(", ")).into());
        }
        let mut config = self.agent_config.clone().unwrap_or_else(|| AgentConfig {
            max_turns: self.profile.max_turns.min(PROFILE_MAX_TURNS_CAP),
            max_tool_calls_per_turn: crate::agent::DEFAULT_BOUNDED_TOOL_CALL_CAP,
            subagent_depth: self.depth + 1,
            ..Default::default()
        });
        let profile_turn_cap = self.profile.max_turns.min(PROFILE_MAX_TURNS_CAP);
        config.max_turns = if config.max_turns == 0 {
            profile_turn_cap
        } else {
            config.max_turns.min(profile_turn_cap)
        };
        let tool_call_cap = profile_tool_call_cap(self.is_critic, &self.profile);
        config.max_tool_calls_per_turn = if config.max_tool_calls_per_turn == 0 {
            tool_call_cap
        } else {
            config.max_tool_calls_per_turn.min(tool_call_cap)
        };
        if let Some(task_timeout_ms) = self.profile.verification_timeout_ms {
            config.max_command_timeout_ms = Some(
                config
                    .max_command_timeout_ms
                    .map_or(task_timeout_ms, |configured| {
                        configured.min(task_timeout_ms)
                    }),
            );
        }
        config.subagent_depth = self.depth + 1;
        let budget_note = self.budget_note_override.clone().unwrap_or_else(|| {
            format!(
                "你的回合预算为 {} 回合：前 {} 回合完成必要的读取、写入和任务要求的定向验证；最后一个回合必须直接输出最终 JSON（不要再调用任何工具）。尽量少花回合。\n",
                self.profile.max_turns,
                self.profile.max_turns.saturating_sub(1)
            )
        });
        let system_prompt = compile_worker_system_prompt(
            &self.profile,
            self.is_critic,
            &budget_note,
            self.extra_system_prompt.as_deref(),
        );
        let runtime = crate::worker_runtime::WorkerRuntime {
            provider: Arc::clone(&self.provider),
            approver: self.approver,
            abort: self.abort,
            depth: self.depth,
            model: self.model.clone(),
            workspace: workspace.to_path_buf(),
            registry,
            policy,
            config,
            system_prompt: Some(system_prompt),
            is_critic: self.is_critic,
            event_sink: self.event_sink.clone(),
            session_store: self.session_store.clone(),
            worker_session_id: self.worker_session_id.clone(),
            parent_session_id: self.parent_session_id.clone(),
        };
        runtime
            .run_report_with_input(prompt, images, input)
            .await
            .map(|report| ProfileSubagentRunReport {
                output: report.output,
                duration_ms: report.duration_ms,
                steps: report.steps,
                model_calls: report.model_calls,
                usage: report.usage,
                usage_known: report.usage_known,
                output_repairs: report.output_repairs,
            })
            .map_err(|error| ProfileSubagentRunError {
                message: error.message,
                duration_ms: error.duration_ms,
                steps: error.steps,
                model_calls: error.model_calls,
                usage: error.usage,
                usage_known: error.usage_known,
                output_repairs: error.output_repairs,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_worker_tool_caps_bound_retry_loops() {
        let writer = WorkerProfile::for_role("implementer", 8);
        let reviewer = WorkerProfile::for_role("reviewer", 8);
        assert_eq!(profile_tool_call_cap(false, &writer), 24);
        assert_eq!(profile_tool_call_cap(true, &reviewer), 16);
        assert_eq!(profile_tool_call_cap(false, &reviewer), 16);
    }
}
