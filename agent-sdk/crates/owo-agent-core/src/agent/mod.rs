use crate::audit::AuditLog;
use crate::autoreview::{ReviewVerdict, Reviewer};
use crate::context::{build_system_prompt, load_project_rules, load_project_rules_hierarchical};
use crate::deadline::{DeadlineBudget, Phase, PhaseBudgets, PhaseTiming};
use crate::error::AgentError;
use crate::gateway::{
    ChatMessage, ModelCallMetadata, ModelOutput, ModelProvider, StreamChunk, TokenUsage,
};
use crate::injection::sanitize_tool_result;
use crate::permissions::{Approver, Decision, PermissionRequest, Policy};
use crate::session::Session;
use crate::skill::SkillRegistry;
use crate::subagent::SubagentRunner;
use crate::tool_effects::EffectClass;
use crate::tools::{
    ToolApprovalGrant, ToolCapabilityContext, ToolContext, ToolHostService, ToolRegistry,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

mod config;
mod single_completion;
mod single_manual_acceptance;
mod turn;
use single_completion::{
    assess_single_turn_completion, single_missing_verification_plan_feedback,
    single_path_is_source_code, single_validation_retry_feedback,
    single_verification_plan_matches_turn, single_workspace_path_matches, workspace_file_hash,
};
mod single_review;

#[cfg(test)]
mod tests;

pub(crate) use config::DEFAULT_BOUNDED_TOOL_CALL_CAP;
use config::*;
pub use config::{AgentConfig, TurnExecutionOptions};

/// 达到最大回合数后的收尾指令：不再调用工具，强制产出可见结论
/// （审查/分析类任务据此给出结构化报告；信息不足时列出需要用户澄清的问题）。
const WRAP_UP_PROMPT: &str = "你已达到本次任务的最大执行步数上限，现在必须停止调用工具，\
     直接用 Markdown 输出最终结论：1) 已完成的工作与关键发现（审查/分析类任务给出结构化报告：结论、证据、风险）；\
     2) 仍未完成或未验证的部分；3) 如果信息不足，列出需要用户澄清的具体问题。不要再请求任何工具。";

/// 空回答的静默重试次数：第一次空响应直接再问一次（不打扰用户），仍为空才走摘要兜底。
const EMPTY_REPLY_RETRIES: usize = 1;

/// 宿主验收反馈的最大修复轮次。
///
/// 反馈指纹只能拦截「完全相同」的失败；长程任务里模型每次修复都会产生新的
/// 哈希（收据/证据引用变化），若不设上限，回合可能在"修复 → 验收失败 → 再修复"
/// 之间永不收敛。到达上限后按真实 completion_status（通常 unverified）结束回合，
/// 交付可见结论，而不是让用户无限等待。
const MAX_HOST_VALIDATION_REPAIR_ROUNDS: usize = 6;

/// 空回答重试时的追加指令：强制产出可见结论，而不是继续思考或调工具。
const EMPTY_REPLY_RETRY_PROMPT: &str = "（系统提示）你上一条回复没有产生任何可见内容。\
     请不要再调用工具，立即用 Markdown 直接输出：1) 当前已完成的工作与结论；\
     2) 仍未完成或不确定的部分。";

/// 兜底摘要中单条工具动作的参数预览长度上限。
const FALLBACK_ACTION_PREVIEW_CHARS: usize = 90;
/// 兜底摘要中最多列出的工具动作条数（去重后）。
const FALLBACK_ACTION_LIMIT: usize = 20;

/// Default safety cap for nested workers. Main user turns are uncapped by default,
/// while delegated subagents always have a finite independent request budget.
fn nested_turn_cap(configured: usize) -> usize {
    if configured == 0 {
        crate::subagent::MAX_SUBAGENT_TURNS
    } else {
        configured.min(crate::subagent::MAX_SUBAGENT_TURNS)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandExecutionReceipt {
    pub command_sha256: String,
    pub exit_code: i32,
    pub result_sha256: String,
    /// ToolHost execution duration; None means this is a legacy receipt without budget evidence.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub workspace_hashes_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator_version: Option<String>,
    #[serde(default)]
    pub workspace_hashes: std::collections::BTreeMap<String, Option<String>>,
    /// Pre-execution identity. Legacy receipts without this evidence remain Unverified.
    #[serde(default)]
    pub workspace_hashes_before: std::collections::BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TurnEvent {
    ModelCall,
    TokenDelta {
        delta: String,
    },
    /// 深度思考增量（模型 reasoning；不写入对话历史，仅 UI 展示）。
    ReasoningDelta {
        delta: String,
    },
    /// 任务计划更新（todo 工具整表替换后外发）：前端渲染步骤进度。
    PlanUpdate {
        steps: serde_json::Value,
    },
    Compaction {
        summary: String,
    },
    PermissionRequest(PermissionRequest),
    ToolStart {
        id: String,
        tool: String,
        /// 参数预览（脱敏、截断）：供 CLI/前端实时展示"正在用什么参数调用"。
        /// 序列化向后兼容（缺省为 None）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args_preview: Option<String>,
    },
    ToolResult {
        id: String,
        tool: String,
        ok: bool,
        error: Option<String>,
        /// 结果预览（截断，纯展示）：随事件下发步骤时间线，前端 chip 可展开；
        /// 写回模型上下文的净化仍由 `sanitize_tool_result` 负责。
        /// 序列化向后兼容（缺省为 None）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_receipt: Option<CommandExecutionReceipt>,
    },
    Final {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnOutcome {
    /// Per-request model identity, provider request id, usage, latency, and outcome.
    #[serde(default)]
    pub model_calls: Vec<ModelCallRecord>,
    pub final_text: Option<String>,
    /// Host assessment: response completion never implies code delivery acceptance.
    #[serde(default)]
    pub completion_status: owo_agent_protocol::CompletionStatusV1,
    /// True when an explicitly configured turn cap forced the wrap-up request.
    #[serde(default)]
    pub reached_model_turn_limit: bool,
    pub steps: usize,
    pub events: Vec<TurnEvent>,
    pub prompt: String,
    pub started_at: String,
    pub duration_ms: u64,
    /// 本回合按请求累加的模型 token 用量。
    #[serde(default)]
    pub usage: TokenUsage,
    /// true 表示回合内每个模型请求都返回了可归属的 usage 元数据。
    #[serde(default)]
    pub usage_known: bool,
    /// §9.3：阶段耗时瀑布（model/approval/tool/persistence，按发生顺序）。
    #[serde(default)]
    pub phase_timings: Vec<crate::deadline::PhaseTiming>,
    /// §9.3：工具面稳定指纹（SHA-256；schema 缓存复用的 key 基础）。
    #[serde(default)]
    pub tools_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCallRecord {
    pub metadata: ModelCallMetadata,
    pub succeeded: bool,
}

/// Agent 核心：执行循环 + 工具注册表 + 权限策略 + 审计。
///
/// 原始注册表句柄不属于下游公开面：
///
/// ```compile_fail
/// let _ = owo_agent_core::Agent::registry;
/// ```
pub struct Agent {
    provider: Arc<dyn ModelProvider>,
    /// 工具注册表（RwLock：MCP 服务器热连接/热卸载时无需重建 Agent）。
    registry: Arc<RwLock<ToolRegistry>>,
    /// 受信工具执行门面：能力签发、执行和 receipt 统一从此处经过。
    tool_host: ToolHostService,
    /// 插件热卸载：已禁用工具前缀（模型不可见、直接调用被拒）。
    disabled_tool_prefixes: Arc<RwLock<HashSet<String>>>,
    /// MCP 客户端进程生命周期注册表（进程级热卸载/退出清理）。
    mcp_clients: Arc<crate::mcp::McpRegistry>,
    /// §10：per-server 健康跟踪（熔断/限流/幂等重试）。
    mcp_health: Arc<crate::mcp_health::McpHealthTracker>,
    /// 独立审批模型（Auto-review）：Ask 先经审查链，Deny 不打扰用户。
    reviewer: Option<Arc<dyn Reviewer>>,
    policy: Policy,
    audit: Arc<Mutex<AuditLog>>,
    config: AgentConfig,
    skills: SkillRegistry,
    elements: Arc<Mutex<crate::ElementRegistry>>,
    /// §9.3：超大工具结果 artifact store（CAS）；None = 维持盲截断旧行为。
    artifact_store: Option<Arc<crate::cas_store::CasStore>>,
    /// A2-1 hooks 生命周期扩展点（settings.json 的 `hooks` 灌入；空 = 无 hook）。
    /// RwLock：服务运行中（Arc<Agent>）也能重灌（settings 保存后热生效）。
    hooks: RwLock<crate::hooks::HookManager>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        registry: ToolRegistry,
        policy: Policy,
        config: AgentConfig,
    ) -> Self {
        let registry = Arc::new(RwLock::new(registry));
        let audit = Arc::new(Mutex::new(AuditLog::default()));
        let tool_host = ToolHostService::new(Arc::clone(&registry), Arc::clone(&audit));
        Self {
            provider,
            registry,
            tool_host,
            disabled_tool_prefixes: Arc::new(RwLock::new(HashSet::new())),
            mcp_clients: Arc::new(crate::mcp::McpRegistry::new()),
            mcp_health: Arc::new(crate::mcp_health::McpHealthTracker::new(
                crate::mcp_health::McpHealthConfig::default(),
            )),
            reviewer: None,
            policy,
            audit,
            config,
            skills: SkillRegistry::default(),
            elements: Arc::new(Mutex::new(crate::ElementRegistry::new())),
            artifact_store: None,
            hooks: RwLock::new(crate::hooks::HookManager::default()),
        }
    }

    /// A2-1：灌入 hooks 配置（settings.json 的 `hooks` 数组；exit 2 = 阻断）。
    /// 快照语义：clone 后释放锁，hook 执行（可达 10s）不阻塞重灌。
    pub fn set_hooks(&self, hooks: crate::hooks::HookManager) {
        if let Ok(mut slot) = self.hooks.write() {
            *slot = hooks;
        }
    }

    /// 当前 hooks 快照（读锁即取即放，避免跨 await 持锁）。
    fn hooks_snapshot(&self) -> crate::hooks::HookManager {
        self.hooks
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// §9.3：挂载 artifact store（CAS）。超大工具结果以「指针+预览」回填模型，
    /// 完整内容按内容寻址落盘；不挂载则维持盲截断旧行为。
    pub fn with_artifact_store(mut self, store: Arc<crate::cas_store::CasStore>) -> Self {
        self.artifact_store = Some(store);
        self
    }

    pub fn set_skills(&mut self, skills: SkillRegistry) {
        self.skills = skills;
    }

    /// 设置独立审批模型（None 表示关闭 Auto-review，恢复纯人工审批）。
    pub fn set_reviewer(&mut self, reviewer: Option<Arc<dyn Reviewer>>) {
        self.reviewer = reviewer;
    }

    /// 当前是否启用 Auto-review。
    pub fn autoreview_enabled(&self) -> bool {
        self.reviewer.is_some()
    }

    /// 注册 MCP 服务器工具（命名空间 `{server}_{tool}`）；热连接，无需重建 Agent。
    pub fn register_mcp_tools(
        &self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<crate::mcp::McpClient>>,
        tools: Vec<crate::mcp::McpTool>,
    ) {
        self.mcp_clients.insert(server_name, Arc::clone(&client));
        if let Ok(mut registry) = self.registry.write() {
            registry.register_mcp_tools_with_health(
                server_name,
                client,
                tools,
                Some(Arc::clone(&self.mcp_health)),
            );
        }
    }

    /// A2-2：注册 MCP resources/prompts 泛化工具（与 tools 一并热注册）。
    pub fn register_mcp_extras(
        &self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<crate::mcp::McpClient>>,
        resources: Vec<crate::mcp::McpResource>,
        prompts: Vec<crate::mcp::McpPrompt>,
    ) {
        if let Ok(mut registry) = self.registry.write() {
            registry.register_mcp_extras(server_name, client, resources, prompts);
        }
    }

    /// MCP 客户端进程注册表（进程级热卸载/状态查询）。
    pub fn mcp_clients(&self) -> Arc<crate::mcp::McpRegistry> {
        Arc::clone(&self.mcp_clients)
    }

    /// §10：per-server 健康跟踪器（熔断/限流状态观测接口面）。
    pub fn mcp_health(&self) -> Arc<crate::mcp_health::McpHealthTracker> {
        Arc::clone(&self.mcp_health)
    }

    /// 热连接 MCP 服务器并注册工具（插件启用/热添加）；返回工具数。
    pub async fn connect_mcp_server(
        &self,
        config: &owo_agent_plugins::McpServerConfig,
    ) -> Result<usize, String> {
        let client = crate::mcp::McpClient::connect(config).await?;
        let tools = client.tools();
        let resources = client.resources();
        let prompts = client.prompts();
        // §5.2：先按 config 声明宿主可信只读（server+tool+schema hash），
        // 随后 register 时 hash 匹配的 readOnlyHint 才能降级为 Read。
        crate::tool_effects::declare_trusted_from_config(config, &tools);
        let tool_count = tools.len();
        let client = Arc::new(tokio::sync::Mutex::new(client));
        self.register_mcp_tools(&config.name, Arc::clone(&client), tools);
        self.register_mcp_extras(&config.name, client, resources, prompts);
        Ok(tool_count)
    }

    /// 进程级热卸载 MCP 服务器：kill stdio 子进程 + 撤销工具（前缀移除且禁用）。
    /// 返回 false 表示服务器本未连接（幂等，不报错）。
    pub async fn shutdown_mcp_server(&self, name: &str) -> Result<bool, String> {
        if !self.mcp_clients.names().iter().any(|n| n == name) {
            return Ok(false);
        }
        let prefix = crate::tools::mcp_tool_prefix(name);
        self.set_tool_prefix_enabled(&prefix, false);
        self.remove_tools_prefix(&prefix);
        self.mcp_clients.shutdown(name).await?;
        Ok(true)
    }

    /// 关闭全部 MCP 客户端（服务退出前调用，防止遗留 stdio 子进程）。
    pub async fn shutdown_all_mcp(&self) -> Vec<(String, String)> {
        self.mcp_clients.shutdown_all().await
    }

    /// 按前缀撤销工具（插件热卸载）；返回移除数量。
    pub fn remove_tools_prefix(&self, prefix: &str) -> usize {
        self.registry
            .write()
            .map(|mut registry| registry.remove_prefix(prefix))
            .unwrap_or(0)
    }

    /// 插件工具前缀启停（热卸载：模型不可见 + 直接调用被拒，无需重建 Agent）。
    pub fn set_tool_prefix_enabled(&self, prefix: &str, enabled: bool) {
        if let Ok(mut prefixes) = self.disabled_tool_prefixes.write() {
            if enabled {
                prefixes.remove(prefix);
            } else {
                prefixes.insert(prefix.to_string());
            }
        }
    }

    /// 工具名是否命中任一禁用前缀。
    pub fn tool_disabled(&self, name: &str) -> bool {
        self.disabled_tool_prefixes
            .read()
            .map(|prefixes| prefixes.iter().any(|prefix| name.starts_with(prefix)))
            .unwrap_or(false)
    }

    /// §9.1：调用能否进入并发组——宿主验证只读（`EffectClass::Read` 且
    /// `host_verified_readonly=true`）。MCP 自报 readOnlyHint 未经验证、
    /// 写/执行/注入、effect 缺失与未知工具一律 `false`（保持串行）。
    fn call_is_concurrent_eligible(&self, call: &crate::gateway::ToolCall) -> bool {
        // These read-class tools update the live session or suspend for user input.
        // Running them on discarded session clones loses plans/decisions.
        if matches!(
            call.name.as_str(),
            "todo" | "verification_plan" | "ask_user" | "kill_shell"
        ) {
            return false;
        }
        let Ok(registry) = self.registry.read() else {
            return false;
        };
        let Some(tool) = registry.get(&call.name) else {
            return false;
        };
        let spec = tool.spec();
        matches!(
            spec.effect.as_ref(),
            Some(effect) if effect.class == EffectClass::Read && effect.host_verified_readonly
        )
    }

    /// 当前模型可见工具：注册表全量减去禁用前缀。
    pub fn visible_tool_specs(&self) -> Vec<crate::tools::ToolSpec> {
        self.registry
            .read()
            .map(|registry| {
                registry
                    .specs()
                    .into_iter()
                    .filter(|spec| !self.tool_disabled(&spec.name))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 设置共享窗口元素注册表（与 HTTP 感知层共用同一 ID 空间）。
    pub fn set_elements(&mut self, elements: Arc<Mutex<crate::ElementRegistry>>) {
        self.elements = elements;
    }

    pub fn elements(&self) -> Arc<Mutex<crate::ElementRegistry>> {
        Arc::clone(&self.elements)
    }

    pub fn skills(&self) -> &SkillRegistry {
        &self.skills
    }

    /// 当前 Agent 配置（只读快照，供诊断/上下文仪表展示）。
    pub fn config(&self) -> AgentConfig {
        self.config.clone()
    }

    /// A turn-local provider view sharing the existing host, policy and audit.
    /// Does not create a second registry or mutate the daemon's default provider.
    pub fn with_turn_provider(&self, provider: Arc<dyn ModelProvider>) -> Result<Self, String> {
        Ok(Self {
            provider,
            registry: Arc::clone(&self.registry),
            tool_host: self.tool_host.clone(),
            disabled_tool_prefixes: Arc::clone(&self.disabled_tool_prefixes),
            mcp_clients: Arc::clone(&self.mcp_clients),
            mcp_health: Arc::clone(&self.mcp_health),
            reviewer: self.reviewer.clone(),
            policy: self.policy.for_workspace(self.policy.workspace())?,
            audit: Arc::clone(&self.audit),
            config: self.config.clone(),
            skills: self.skills.clone(),
            elements: Arc::clone(&self.elements),
            artifact_store: self.artifact_store.clone(),
            hooks: RwLock::new(self.hooks.read().unwrap_or_else(|e| e.into_inner()).clone()),
        })
    }

    pub fn provider(&self) -> Arc<dyn ModelProvider> {
        Arc::clone(&self.provider)
    }

    /// 直呼子代理（CLI `@explore` / `@subagent`）：独立子会话执行，返回最终文本。
    ///
    /// **命令执行默认拒绝**：无交互审批通道时 Execute/Inject 必须在
    /// [`Self::run_subagent_with_options`] 显式 opt-in，避免仓库文件/网页里的
    /// 注入内容诱导子代理执行命令。
    pub async fn run_subagent(
        &self,
        workspace: &std::path::Path,
        model: &str,
        prompt: &str,
        read_only: bool,
    ) -> Result<String, AgentError> {
        self.run_subagent_with_options(workspace, model, prompt, read_only, false)
            .await
    }

    /// 直呼子代理（带命令执行开关）。
    ///
    /// - `read_only=true`：读恒放行，写/执行/注入一律拒绝；
    /// - `read_only=false, allow_commands=false`：仅工作区内写入放行，命令执行拒绝；
    /// - `read_only=false, allow_commands=true`：工作区内写/执行放行（调用方必须
    ///   已获得用户显式同意，如桌面端"允许执行命令"复选框、CLI `--allow-commands`、
    ///   workflow plan 的 `allow_commands: true`）。
    ///
    /// 越界写与越界命令仍由工作区范围审批器拒绝，危险命令片段仍由 [`Policy`] 拦截。
    pub async fn run_subagent_with_options(
        &self,
        workspace: &std::path::Path,
        model: &str,
        prompt: &str,
        read_only: bool,
        allow_commands: bool,
    ) -> Result<String, AgentError> {
        let abort = std::sync::atomic::AtomicBool::new(false);
        // 直呼子代理没有可回传到客户端的审批通道（goal/plan 后台 worker、
        // POST /subagent）：改用工作区范围审批器——读恒放行；写仅在**工作区内**
        // 放行，越界一律拒绝；命令执行只有显式 opt-in 才放行（见方法文档）。
        let approver = crate::permissions::WorkspaceApprover {
            workspace: workspace.to_path_buf(),
            allow_writes: !read_only,
            allow_execute: !read_only && allow_commands,
        };
        let runner = SubagentRunner {
            provider: Arc::clone(&self.provider),
            parent_policy: &self.policy,
            approver: &approver,
            abort: &abort,
            depth: self.config.subagent_depth,
            max_turns: nested_turn_cap(self.config.max_turns),
            model: model.to_string(),
            events: None,
        };
        runner
            .run(workspace, prompt, read_only)
            .await
            .map_err(AgentError::Tool)
    }

    pub fn audit_log(&self) -> Arc<Mutex<AuditLog>> {
        Arc::clone(&self.audit)
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// 运行时追加危险命令片段（热生效；重启后由 settings.deny_commands 恢复）。
    pub fn add_runtime_deny(&self, fragment: impl Into<String>) {
        self.policy.add_runtime_deny(fragment);
    }

    /// 应用运行时权限设置（设置页保存后立即影响下一次工具调用）。
    pub fn apply_policy_settings(&self, read_only: bool, deny_commands: &[String]) {
        self.policy.set_read_only_runtime(read_only);
        self.policy.replace_runtime_deny(deny_commands);
    }

    /// §5.4 运行时注入授权记忆（server 端与 AppState.grants 共享同一引用）。
    pub fn set_grants(&self, grants: std::sync::Arc<crate::grant_store::GrantStore>) {
        self.policy.set_grants(grants);
    }

    /// §5.3 运行时切换权限档位（UI/CLI/HTTP 统一入口）。
    pub fn set_permission_profile(&self, profile: crate::permissions::PermissionProfile) {
        self.policy.set_profile(profile);
    }

    /// 当前权限档位（诊断/设置页回显）。
    pub fn permission_profile(&self) -> crate::permissions::PermissionProfile {
        self.policy.profile()
    }

    /// 执行一轮任务。审批经 `approver` 独立决策；`abort` 可随时中止。
    pub async fn run_turn(
        &self,
        session: &mut Session,
        prompt: &str,
        approver: &dyn Approver,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
    ) -> Result<TurnOutcome, AgentError> {
        self.run_turn_with_asker(session, prompt, approver, abort, on_event, None)
            .await
    }

    /// 带用户提问通道的回合（ask_user 工具；取优合并自远端 engine）：
    /// `questioner` 为 None 时语义同 [`Agent::run_turn`]（工具会明确报错，
    /// 模型改为在最终回复里书面提问）。
    pub async fn run_turn_with_asker(
        &self,
        session: &mut Session,
        prompt: &str,
        approver: &dyn Approver,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
        questioner: Option<&dyn crate::question::Questioner>,
    ) -> Result<TurnOutcome, AgentError> {
        self.run_turn_inner(
            session,
            prompt,
            &[],
            approver,
            abort,
            on_event,
            questioner,
            &TurnExecutionOptions::default(),
        )
        .await
    }

    /// 带图片输入的回合（A1-2 多模态；取优合并自远端 engine）：`images` 为空时
    /// 语义同 [`Agent::run_turn_with_asker`]；非空时用户消息携带视觉内容
    /// （provider 层转成 image parts / image block）。
    // 图片/提问/审批/中止/事件回调同为回合执行固有维度，参数数超过 clippy
    // 默认阈值；打包成结构体反而让三处调用点可读性下降，故显式豁免。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_with_images(
        &self,
        session: &mut Session,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        approver: &dyn Approver,
        questioner: Option<&dyn crate::question::Questioner>,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
    ) -> Result<TurnOutcome, AgentError> {
        self.run_turn_inner(
            session,
            prompt,
            images,
            approver,
            abort,
            on_event,
            questioner,
            &TurnExecutionOptions::default(),
        )
        .await
    }

    /// Run with request constraints. These can only restrict the host policy.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_with_images_constrained(
        &self,
        session: &mut Session,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        approver: &dyn Approver,
        questioner: Option<&dyn crate::question::Questioner>,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
        read_only: bool,
    ) -> Result<TurnOutcome, AgentError> {
        self.run_turn_with_images_options(
            session,
            prompt,
            images,
            approver,
            questioner,
            abort,
            on_event,
            &TurnExecutionOptions {
                read_only,
                ..Default::default()
            },
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_with_images_options(
        &self,
        session: &mut Session,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        approver: &dyn Approver,
        questioner: Option<&dyn crate::question::Questioner>,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
        options: &TurnExecutionOptions,
    ) -> Result<TurnOutcome, AgentError> {
        self.run_turn_inner(
            session, prompt, images, approver, abort, on_event, questioner, options,
        )
        .await
    }

    /// 当估算 token 超过预算时，用模型把旧历史压缩为摘要（保留最近消息）。
    /// `force=true`（`/compact` 显式触发）时跳过预算判断，仍受 `compaction_enabled` 约束。
    async fn maybe_compact(
        &self,
        messages: &mut Vec<ChatMessage>,
        session_id: &str,
        force: bool,
    ) -> Result<Option<String>, AgentError> {
        if !self.config.compaction_enabled {
            return Ok(None);
        }
        if !force && estimate_tokens(messages) <= self.config.token_budget {
            return Ok(None);
        }
        // 切点必须对齐 tool 群组（A4-2 同源规则），且必须在**生成摘要之前**确定：
        // 摘要是 head、保留段是 tail，两者必须严格互补。若先按 head_end 生成摘要、
        // 再对齐 tail，被对齐让出去的那几条消息就既不在摘要里也不在保留段里——等于
        // 静默丢历史。
        let Some(tail_start) = compaction_split(messages, self.config.keep_recent) else {
            return Ok(None);
        };
        let head = messages[1..tail_start].to_vec();
        let prompt = format!(
            "请把以下 Agent 会话历史压缩成一份简洁的进展摘要（保留：已完成的动作、\
             未完成事项、关键决策、当前上下文；不要编造新信息）：\n\n{}",
            serde_json::to_string(&head).unwrap_or_else(|_| "[]".to_string())
        );
        // 压缩摘要是轻量任务：走 fast 档（OWO_MODEL_FAST 配置时路由到便宜模型，
        // 未配置回退 Provider 解析链；M4.2 任务类型路由）。
        let fast_model = crate::gateway::resolve_tier_model(crate::gateway::ModelTier::Fast);
        let summary = match self
            .provider
            .complete_with_model(fast_model.as_deref(), &[ChatMessage::user(prompt)], &[])
            .await
        {
            Ok(crate::gateway::ModelOutput::Text(text)) => text,
            Ok(_) | Err(_) => {
                // 压缩失败兜底（A4-3）：按 token 硬裁剪历史，保证本回合仍可发送，
                // 而不是带着超预算历史直接撞模型 400。返回 Some 让前端收到可见提示。
                let before = messages.len();
                compact_truncate_to_budget(messages, self.config.token_budget / 2);
                self.audit
                    .lock()
                    .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                    .record(
                        session_id,
                        "compaction_fallback",
                        None,
                        None,
                        format!(
                            "模型压缩失败，按预算硬裁剪历史：{before} → {} 条",
                            messages.len().saturating_sub(1)
                        ),
                    );
                return Ok(Some(
                    "模型压缩失败，已硬裁剪最近历史以保持在上下文预算内".to_string(),
                ));
            }
        };
        let mut compacted = vec![messages[0].clone()];
        compacted.push(ChatMessage::system(format!(
            "历史摘要（已压缩）：\n{summary}"
        )));
        // 保留段起点用对齐后的 tail_start（不是 head_end）：见上方说明，
        // 这条是"压缩后历史仍能发出去"的关键。
        compacted.extend(messages[tail_start..].to_vec());
        // 兜底归一：任何压缩路径（含摘要模型异常、未来新增切分）都不该产出非法序列。
        // 这一步只处理非法配对，正常情况下是空操作。
        sanitize_history(&mut compacted);
        *messages = compacted;
        self.audit
            .lock()
            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
            .record(
                session_id,
                "compaction",
                None,
                None,
                format!("压缩 {} 条历史消息", head.len()),
            );
        Ok(Some(summary))
    }

    /// 显式压缩会话历史（`/compact` 触发）：不依赖 token 预算，直接压缩并写回 `session`。
    /// 返回摘要文本；历史不足或模型未产出摘要时返回 `None`。
    pub async fn compact_session(
        &self,
        session: &mut Session,
    ) -> Result<Option<String>, AgentError> {
        let mut messages = session.messages.clone();
        match self.maybe_compact(&mut messages, &session.id, true).await? {
            Some(summary) => {
                session.messages = messages;
                Ok(Some(summary))
            }
            None => Ok(None),
        }
    }
}

/// 工具参数预览：脱敏（秘密字段只显示类型/长度）+ 紧凑 JSON + 截断，供 `ToolStart`
/// 实时状态展示；空参数返回 `None`。
fn tool_args_preview(args: &serde_json::Value) -> Option<String> {
    let text = crate::permissions::redact_args(args).to_string();
    if text == "null" || text == "{}" {
        return None;
    }
    const MAX_CHARS: usize = 160;
    if text.chars().count() <= MAX_CHARS {
        Some(text)
    } else {
        let truncated: String = text.chars().take(MAX_CHARS).collect();
        Some(format!("{truncated}…"))
    }
}

/// Successful serialized operations provide a new opportunity for the model to make progress.
/// Keep the current operation count so a command cannot reset its own repeat guard.
fn reset_loop_guard_after_progress(
    call_signatures: &mut HashMap<String, usize>,
    current_signature: &str,
) {
    call_signatures.retain(|signature, _| signature == current_signature);
}

/// 循环保护签名：`name` + 规范化参数（键序稳定，避免 provider 参数键序不同导致漏判）。
fn tool_call_signature(call: &crate::gateway::ToolCall) -> String {
    format!("{}:{}", call.name, canonical_json(&call.arguments))
}

/// 稳定 JSON 序列化：对象键排序、数组保序，用于「同一调用」判定。
fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<String> = map
                .iter()
                .map(|(key, value)| format!("{key}:{}", canonical_json(value)))
                .collect();
            entries.sort();
            format!("{{{}}}", entries.join(","))
        }
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

fn commit_turn_messages(session: &mut Session, messages: &[ChatMessage]) {
    session.messages = messages.iter().skip(1).cloned().collect();
    session.updated_at = Utc::now().to_rfc3339();
}

async fn wait_for_abort(abort: &AtomicBool) {
    while !abort.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn truncate_tool_result(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let mut truncated: String = content.chars().take(max_chars).collect();
    truncated.push_str("\n[工具输出已截断]");
    truncated
}

/// 步骤时间线的结果预览上限：够看清这步做了什么，又不会把 SSE 帧撑爆。
const TOOL_PREVIEW_CHARS: usize = 1600;

/// 工具结果预览（随 `ToolResult` 事件下发给前端 chip 展开区）。
/// 纯展示用途：写回模型上下文的净化仍由 `sanitize_tool_result` 负责。
fn command_execution_receipt(
    tool: &str,
    outcome: &Result<serde_json::Value, String>,
    session: &Session,
    before: Option<crate::command_evidence::CommandSnapshot>,
) -> Option<CommandExecutionReceipt> {
    if tool != "run_command" {
        return None;
    }
    let value = outcome.as_ref().ok()?;
    let command = value.get("command")?.as_str()?.trim();
    let exit_code = i32::try_from(value.get("exit_code")?.as_i64()?).ok()?;
    let duration_ms = value.get("duration_ms").and_then(serde_json::Value::as_u64);
    let registered_behavior_command = crate::verification::is_registered_behavior_command(command);
    let (workspace_hashes_complete, workspace_hashes_before, workspace_hashes) = match before {
        Some(before) if registered_behavior_command => {
            let after = crate::command_evidence::capture_command_snapshot(session);
            (
                before.complete && after.complete,
                before.hashes,
                after.hashes,
            )
        }
        _ => (false, Default::default(), Default::default()),
    };
    Some(CommandExecutionReceipt {
        command_sha256: crate::CasStore::hash_of(command.as_bytes()),
        exit_code,
        result_sha256: crate::CasStore::hash_of(value.to_string().as_bytes()),
        duration_ms,
        workspace_hashes_complete,
        validator_id: registered_behavior_command
            .then(|| "workspace-command-success-v1".to_string()),
        validator_version: registered_behavior_command.then(|| "1".to_string()),
        workspace_hashes,
        workspace_hashes_before,
    })
}

fn tool_preview(outcome: &Result<serde_json::Value, String>) -> Option<String> {
    let text = match outcome {
        Ok(value) => value.to_string(),
        Err(error) => format!("工具错误：{error}"),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncate_tool_result(trimmed, TOOL_PREVIEW_CHARS))
}

/// 合成兜底回复：模型在回合结束时没有产出任何内容（空回答/收尾失败）时，
/// 把本回合已执行的工具动作整理成可读摘要，保证用户总能得到明确结论，
/// 而不是只看到一段越来越长的「思考过程」后什么都没有。
fn synthesize_fallback_reply(messages: &[ChatMessage], steps: usize, reason: &str) -> String {
    let mut actions: Vec<String> = Vec::new();
    for message in messages {
        let Some(calls) = &message.tool_calls else {
            continue;
        };
        for call in calls {
            let preview = tool_call_preview(&call.arguments);
            let line = if preview.is_empty() {
                format!("- `{}`", call.name)
            } else {
                format!("- `{}`：{preview}", call.name)
            };
            if !actions.contains(&line) {
                actions.push(line);
            }
        }
    }
    let shown = actions.len().min(FALLBACK_ACTION_LIMIT);
    let mut body = String::new();
    body.push_str("> ⚠️ 本回合模型没有产出正式回答（");
    body.push_str(reason);
    body.push_str("）。以下为系统自动整理的工作摘要，供你确认或让我继续。\n\n");
    if actions.is_empty() {
        body.push_str("**本轮没有执行任何工具操作，也没有产出文本内容。**\n\n");
    } else {
        body.push_str(&format!(
            "**本回合共执行 {steps} 步工具操作（列出前 {shown} 条）：**\n\n"
        ));
        for line in actions.iter().take(shown) {
            body.push_str(line);
            body.push('\n');
        }
        if actions.len() > shown {
            body.push_str(&format!("- …（其余 {} 条已省略）\n", actions.len() - shown));
        }
        body.push('\n');
    }
    body.push_str("你可以直接回复「继续」让我接着完成剩余部分，或指出需要调整的地方。");
    body
}

/// 工具调用参数摘要：优先取路径/命令等最具信息量的字段，截断到预览长度上限。
fn tool_call_preview(arguments: &serde_json::Value) -> String {
    const KEYS: [&str; 6] = ["path", "command", "file", "pattern", "query", "url"];
    let raw = KEYS
        .iter()
        .find_map(|key| arguments.get(key).and_then(serde_json::Value::as_str))
        .or_else(|| {
            arguments
                .as_object()
                .and_then(|map| map.values().find_map(serde_json::Value::as_str))
        })
        .unwrap_or_default()
        .trim();
    if raw.is_empty() {
        return String::new();
    }
    let mut preview: String = raw.chars().take(FALLBACK_ACTION_PREVIEW_CHARS).collect();
    if raw.chars().count() > FALLBACK_ACTION_PREVIEW_CHARS {
        preview.push('…');
    }
    preview.replace('\n', " ")
}

/// 每条消息的固定开销（role / 分隔符等）。
pub const MESSAGE_OVERHEAD: usize = 4;

/// 图片消息的 token 估算（取优合并自远端 engine）：视觉输入 token 随分辨率
/// 浮动（数百到数千），按 1100 中位值计入预算，防止带图消息把 token 估算打穿。
pub const IMAGE_TOKEN_ESTIMATE: usize = 1_100;

/// 单段文本的 token 估算：CJK ≈ 1 token/字，ASCII 按 4 字符/token。
/// （不引入 tiktoken 原生依赖；与真实 cl100k 计数同量级，用于预算与压缩触发。）
fn text_token_estimate(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut ascii_run = 0usize;
    for ch in text.chars() {
        let wide = (ch as u32) >= 0x2E80; // CJK 及全角标点
        if wide {
            tokens += 1;
            ascii_run = 0;
        } else if ch.is_ascii_alphanumeric() || ch == ' ' {
            ascii_run += 1;
            if ascii_run == 4 {
                tokens += 1;
                ascii_run = 0;
            }
        } else {
            tokens += 1;
            ascii_run = 0;
        }
    }
    if ascii_run > 0 {
        tokens += 1;
    }
    tokens
}

/// token 估算（P1-1 口径）：CJK 按 1 token/字，图片按 [`IMAGE_TOKEN_ESTIMATE`]。
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|message| {
            text_token_estimate(message.content.as_deref().unwrap_or_default())
                + MESSAGE_OVERHEAD
                + message.images.len() * IMAGE_TOKEN_ESTIMATE
        })
        .sum()
}

/// 压缩保留段起点对齐（A4-2）：切点落在 tool 群组内时——
/// 有前置 assistant(tool_calls) 则回退到该消息（调用与结果同进同出）；
/// 否则跳过整个孤儿 tool 群组（脏历史直接发给模型会 400）。
fn align_keep_start(messages: &[ChatMessage], start: usize) -> usize {
    if start >= messages.len() || messages[start].role != "tool" {
        return start;
    }
    let mut group_start = start;
    while group_start > 1 && messages[group_start - 1].role == "tool" {
        group_start -= 1;
    }
    if group_start > 1
        && messages[group_start - 1].role == "assistant"
        && messages[group_start - 1].tool_calls.is_some()
    {
        return group_start - 1;
    }
    let mut after = start;
    while after < messages.len() && messages[after].role == "tool" {
        after += 1;
    }
    after
}

/// 摘要压缩的切分点：返回保留段起点，`None` 表示不该压缩（历史太短或对齐后无内容）。
///
/// 为什么单独抽出来：`keep_recent` 是**按条数**切的，而一个工具回合里
/// `assistant(tool_calls)` 与它的若干 `tool` 结果是多条消息。切点若落进群组中间，
/// 保留段会以**孤立 tool 消息**开头，拼上摘要直接发模型就是 400：
///   "Messages with role 'tool' must be a response to a preceding message with
///    'tool_calls'"
/// 实测症状：跑了 33 个工具的回合，压缩提示刚出现，下一轮请求整体失败。
/// 同文件另两条裁剪路径（[`compact_truncate`] / [`compact_truncate_to_budget`]）
/// 都已对齐，只有摘要压缩这条漏了；抽成纯函数也便于回归测试钉住。
fn compaction_split(messages: &[ChatMessage], keep_recent: usize) -> Option<usize> {
    let head_end = messages.len().saturating_sub(keep_recent);
    if head_end < 4 {
        return None;
    }
    let tail_start = align_keep_start(messages, head_end);
    // 对齐把整段都让出去了（切点后全是孤儿 tool 群组）：不压缩，保持原样。
    if tail_start >= messages.len() {
        return None;
    }
    Some(tail_start)
}

/// 存量脏历史归一（取优合并自远端 engine）：发请求前调用——
/// 丢弃无配对的孤立 tool 消息；给有 tool_calls 但没有结果的 assistant 补占位结果
/// （中断半截提交/外部导入的会话直接发给模型会 400）。
fn sanitize_history(messages: &mut Vec<ChatMessage>) {
    fn flush_pending(pending: &mut Vec<String>, cleaned: &mut Vec<ChatMessage>) {
        for id in pending.drain(..) {
            cleaned.push(ChatMessage::tool(
                id,
                "工具结果缺失（该回合被中断或未完成）".to_string(),
            ));
        }
    }

    let mut cleaned: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    let mut pending: Vec<String> = Vec::new();
    for message in messages.drain(..) {
        match message.role.as_str() {
            "tool" => {
                let id = message.tool_call_id.clone().unwrap_or_default();
                if let Some(position) = pending.iter().position(|call_id| *call_id == id) {
                    pending.remove(position);
                    cleaned.push(message);
                }
                // 无配对（孤立 tool）：丢弃。
            }
            "assistant" => {
                flush_pending(&mut pending, &mut cleaned);
                if let Some(calls) = &message.tool_calls {
                    pending.extend(calls.iter().map(|call| call.id.clone()));
                }
                cleaned.push(message);
            }
            _ => {
                flush_pending(&mut pending, &mut cleaned);
                cleaned.push(message);
            }
        }
    }
    flush_pending(&mut pending, &mut cleaned);
    *messages = cleaned;
}

/// 按 token 预算硬裁剪历史（A4-3 兜底）：从尾部保留尽量多的消息（对齐 tool
/// 群组切点），使 [`estimate_tokens`] 回到 `budget` 内。
///
/// 与条数版 [`compact_truncate`] 的区别：模型上下文是 token 硬约束，压缩模型
/// 调用失败时条数检查挡不住 token 超限。
fn compact_truncate_to_budget(messages: &mut Vec<ChatMessage>, budget: usize) {
    let token_of = |message: &ChatMessage| {
        text_token_estimate(message.content.as_deref().unwrap_or_default())
            + MESSAGE_OVERHEAD
            + message.images.len() * IMAGE_TOKEN_ESTIMATE
    };
    // system（messages[0]）必保留；从尾部往前累计，找出预算内可保留的 tail。
    let mut acc = messages.first().map(&token_of).unwrap_or(0);
    let mut keep = 0usize;
    for message in messages.iter().skip(1).rev() {
        let tokens = token_of(message);
        if acc + tokens > budget && keep > 0 {
            break;
        }
        acc += tokens;
        keep += 1;
    }
    if keep == 0 || keep >= messages.len() {
        return;
    }
    let tail_start = align_keep_start(messages, messages.len() - keep);
    // 对齐可能把 tail 推到群组之后甚至越界：越界时不裁（保守，不破坏序列）。
    if tail_start >= messages.len() {
        return;
    }
    let mut tail = messages[tail_start..].to_vec();
    let system = messages[0].clone();
    tail.insert(0, system);
    *messages = tail;
}

/// 回合事件出口的共享单元：父回合与嵌套子代理（`SubagentRunner.events`）共用同一份，
/// 保证子代理的审批请求/工具进度在父回合 await 期间也能即时外发。
pub(crate) type EventCell<'a> = Arc<Mutex<&'a mut (dyn FnMut(&TurnEvent) + Send + 'a)>>;

/// 回合事件出口：`on_event` 用共享单元传递（子代理/嵌套回合持同一单元即时回传）。
fn emit(events: &mut Vec<TurnEvent>, on_event: &EventCell<'_>, event: TurnEvent) {
    // 中毒也继续转发（见 nested_sink 注释）。
    let mut forward = on_event
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    forward(&event);
    events.push(event);
}

fn compact_truncate(messages: &mut Vec<ChatMessage>, limit: usize) {
    if messages.len() <= limit {
        return;
    }
    let keep = limit.saturating_sub(1);
    let tail_start = align_keep_start(messages, messages.len().saturating_sub(keep));
    if tail_start >= messages.len() {
        return;
    }
    let mut tail = messages[tail_start..].to_vec();
    let system = messages[0].clone();
    tail.insert(0, system);
    *messages = tail;
}
