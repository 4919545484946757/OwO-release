use crate::tools::ToolApprovalGrant;
pub(super) const MAX_TOOL_RESULT_CHARS: usize = 50_000;
pub(crate) const DEFAULT_BOUNDED_TOOL_CALL_CAP: usize = 64;

/// §9.1 阶段一产物：按原始 tool-call 顺序完成的权限判定（Ask 已归并为 Allow/Deny）。
pub(super) struct PreparedCall {
    /// Policy/Approver 归并后的类型化放行凭证；拒绝调用没有执行凭证。
    pub(super) approval: Option<ToolApprovalGrant>,
    /// 权限理由（拒绝消息文案）。
    pub(super) reason: String,
    /// 循环保护拦截原因（Some = 本调用被宿主拦截，不执行、不审批）。
    ///
    /// A host-side pre-execution rejection (loop guard, schema check, or hook).
    /// The model receives the reason as a tool result and may repair its call.
    pub(super) guard_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Maximum model/tool rounds in one user turn; 0 means continue until the model
    /// returns a final answer or the caller cancels. Worker runtimes set explicit caps.
    pub max_turns: usize,
    pub context_limit: usize,
    pub subagent_depth: usize,
    pub token_budget: usize,
    pub keep_recent: usize,
    pub compaction_enabled: bool,
    /// §9.1：单个并发组内只读工具的最大并发数（默认 4；下限 1）。
    pub tool_concurrency: usize,
    /// Optional hard cap on tool calls for one user turn; 0 means uncapped.
    ///
    /// Normal conversations may keep using tools until the model finishes.
    /// Worker/evaluation budgets remain explicit in their runtime profiles.
    pub max_tool_calls_per_turn: usize,
    /// Host-enforced upper bound for run_command in a task-specific validation plan.
    pub max_command_timeout_ms: Option<u64>,
    /// 循环保护：同一 `name + 规范化参数` 的连续无进展调用上限（默认 3）；
    /// 成功的非只读操作会重置其他调用的计数。
    /// 超过即拦截该调用并回灌"改变策略"提示，不再执行。
    pub max_repeated_tool_calls: usize,
    /// §9.2：turn 级统一截止时间；None = 不限时（保持既有行为，仅记账）。
    pub turn_deadline: Option<std::time::Duration>,
    /// 单次回复的最大输出 token（配置文件 `model.max_output_tokens`）。
    pub max_output_tokens: Option<usize>,
    /// 采样温度（配置文件 `model.temperature`）。
    pub temperature: Option<f64>,
    /// 单次模型请求超时秒数（配置文件 `model.timeout_secs`）。
    pub request_timeout_secs: Option<usize>,
}

impl AgentConfig {
    /// 从环境变量覆盖上下文相关预算（**配置文件 → 环境变量 → 代码默认**）。
    ///
    /// 为什么做成环境变量而不是塞进代码：用户明确要求"模型服务地址、模型名称、
    /// 上下文等全都通过文件随时更改，不能写死"。桌面壳读 `config.json` 后把这些值
    /// 注入核心进程环境，核心在这里消费——单一实现、单一来源，改文件即生效。
    ///
    /// 非法值（0、非数字）**忽略并保留默认**，不 panic：用户手写配置文件打错字
    /// 不该让核心起不来，但也不能静默采纳一个会让压缩逻辑失效的 0。
    pub fn with_env_overrides(mut self) -> Self {
        if let Some(value) = env_usize("OWO_MODEL_CONTEXT_WINDOW") {
            self.token_budget = value;
        }
        if let Some(value) = env_usize("OWO_MODEL_MAX_OUTPUT_TOKENS") {
            self.max_output_tokens = Some(value);
        }
        if let Some(value) = env_usize("OWO_AGENT_KEEP_RECENT") {
            self.keep_recent = value;
        }
        if let Some(value) = env_bool("OWO_AGENT_COMPACTION") {
            self.compaction_enabled = value;
        }
        if let Some(value) = env_f64("OWO_MODEL_TEMPERATURE") {
            self.temperature = Some(value);
        }
        if let Some(value) = env_usize("OWO_MODEL_TIMEOUT_SECS") {
            self.request_timeout_secs = Some(value);
        }
        if let Some(value) = env_usize_allow_zero("OWO_AGENT_MAX_MODEL_TURNS") {
            self.max_turns = value;
        }
        if let Some(value) = env_usize_allow_zero("OWO_AGENT_MAX_TOOL_CALLS") {
            self.max_tool_calls_per_turn = value;
        }
        if let Some(value) = env_usize("OWO_AGENT_MAX_REPEATED_TOOL_CALLS") {
            self.max_repeated_tool_calls = value;
        }
        self
    }
}

/// 读一个正整数环境变量；空串/非法/0 一律当作"未设置"。
fn env_usize(name: &str) -> Option<usize> {
    env_value(name).and_then(|value| parse_env_usize(&value).filter(|value| *value > 0))
}

fn env_usize_allow_zero(name: &str) -> Option<usize> {
    env_value(name).and_then(|value| parse_env_usize(&value))
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_env_usize(value: &str) -> Option<usize> {
    value.trim().parse::<usize>().ok()
}

fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn env_bool(name: &str) -> Option<bool> {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: 0,
            context_limit: 200,
            subagent_depth: 0,
            token_budget: 60_000,
            keep_recent: 20,
            compaction_enabled: true,
            tool_concurrency: 4,
            max_tool_calls_per_turn: 0,
            max_command_timeout_ms: None,
            max_repeated_tool_calls: 3,
            turn_deadline: None,
            max_output_tokens: None,
            temperature: None,
            request_timeout_secs: None,
        }
    }
}

#[cfg(test)]
mod env_limit_tests {
    use super::parse_env_usize;

    #[test]
    fn zero_is_a_valid_value_for_uncapped_turn_limits() {
        assert_eq!(parse_env_usize("0"), Some(0));
        assert_eq!(parse_env_usize(" 12 "), Some(12));
        assert_eq!(parse_env_usize(""), None);
        assert_eq!(parse_env_usize("-1"), None);
        assert_eq!(parse_env_usize("unlimited"), None);
    }

    /// 纯对话模式的工具白名单必须只有联网检索与提问：
    /// 一旦有人往里加本地文件/命令/桌面工具，这里会立刻失败。
    #[test]
    fn chat_mode_allowlist_excludes_local_capabilities() {
        use super::CHAT_MODE_TOOLS;
        assert!(CHAT_MODE_TOOLS.contains(&"web_search"));
        assert!(CHAT_MODE_TOOLS.contains(&"web_fetch"));
        for forbidden in [
            "write_file",
            "read_file",
            "list_dir",
            "run_command",
            "apply_patch",
            "generate_image",
            "computer_use",
        ] {
            assert!(
                !CHAT_MODE_TOOLS.contains(&forbidden),
                "纯对话模式不应开放 {forbidden}"
            );
        }
    }
}

/// 纯对话模式允许的工具（按名字精确匹配）：
/// 只有联网检索和"向用户提问"，本地文件读写、命令执行、桌面控制一律不开放。
/// 这是执行层的硬边界，模型就算臆造出工具名也会被拒绝，而不只是 schema 不可见。
pub const CHAT_MODE_TOOLS: &[&str] = &["web_search", "web_fetch", "ask_user"];

#[derive(Debug, Clone, Default)]
pub struct TurnExecutionOptions {
    /// A narrowing-only constraint; false never changes the host policy.
    pub read_only: bool,
    /// Host-validated identity shared by SSE, cancellation, traces and receipts.
    pub turn_id: Option<String>,
    /// 会话工作模式；`Chat` 只暴露 [`CHAT_MODE_TOOLS`]，其余调用在执行层被拒绝。
    pub mode: crate::session::SessionMode,
}
