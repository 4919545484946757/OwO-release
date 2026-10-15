//! OwO 输入法 Agent IPC 协议 v3 类型层。
//!
//! 本模块是 `OwO-release/docs/plugins/agent-ipc-v3.schema.json` 的 Rust 镜像：
//! 字段集合、必填性与取值范围逐条对齐（schema 中所有字段均为 `required`，
//! `additionalProperties: false`——因此这里全部为必填字段 + 严格字段集合，
//! 可选语义用空串 / 空数组 / 0 表示）。
//!
//! 行为语义以官方参考实现 `OwO-release/apps/agent_mock/main.cpp` 为准。
//! 常量对齐 `OwO-release/include/owo/agent/agent_protocol.h`。

use serde::{Deserialize, Serialize};

// ────────────────────────────── 协议常量 ──────────────────────────────

/// 当前协议版本（请求/响应 `schema_version` 的 const 值）。
pub const AGENT_PROTOCOL_VERSION: u32 = 3;
/// 连接器仍可解析的最低协议版本（v2 兼容由连接器侧负责）。
pub const MINIMUM_AGENT_PROTOCOL_VERSION: u32 = 2;
/// 单个 JSON 载荷上限（字节）。帧长度超过它必须拒绝。
pub const MAXIMUM_AGENT_PAYLOAD_BYTES: usize = 256 * 1024;
/// 响应候选上限。
pub const MAXIMUM_AGENT_COMMANDS: usize = 64;
/// `context_entries` 上限。
pub const MAXIMUM_AGENT_CONTEXT_ENTRIES: usize = 32;
/// 单条上下文文本上限（字节）。
pub const MAXIMUM_AGENT_CONTEXT_ENTRY_BYTES: usize = 32 * 1024;
/// `context_entries` 文本总量上限（字节）。
pub const MAXIMUM_AGENT_CONTEXT_BYTES: usize = 128 * 1024;
/// 任务草稿槽位上限。
pub const MAXIMUM_AGENT_TASK_SLOTS: usize = 16;
/// 拼音区间数量上限（每个 ranges 数组）。
pub const MAXIMUM_AGENT_TASK_RANGES: usize = 32;
/// 单次 slot_updates 上限。
pub const MAXIMUM_AGENT_SLOT_UPDATES: usize = 8;
/// 官方连接器插件 ID。
pub const AGENT_PLUGIN_ID: &str = "org.owo.agent-ipc";
/// 官方连接器插件服务名。
pub const AGENT_PLUGIN_SERVICE: &str = "owo.agent.exchange.v1";

/// 本适配器声明的全部能力（响应返回与请求 `capabilities` 的交集）。
pub const SUPPORTED_CAPABILITIES: &[&str] = &[
    "protocol.negotiation",
    "input.structured",
    "context.entries",
    "context.privacy",
    "commands.risk",
    "errors.structured",
    "state.progress",
    "tasks.slots",
    "tasks.incremental",
];

// ────────────────────────────── 枚举 ──────────────────────────────

/// 请求动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Submit,
    Select,
    Page,
    Cancel,
    Poll,
}

/// 响应状态（13 态，schema `response.status` 全集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Connecting,
    AgentMode,
    Submitting,
    Thinking,
    WaitingForUser,
    Executing,
    WaitingForConfirmation,
    Cancelling,
    Cancelled,
    Completed,
    Disconnected,
    Timeout,
    Error,
}

/// 命令风险级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

// ────────────────────────────── 结构体 ──────────────────────────────

/// 拼音区间：相对去掉开头 `v` 后的原始 ASCII 拼音，从 0 开始、左闭右开 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinyinRange {
    pub start: u32,
    pub end: u32,
}

/// 槽位补丁（select 携带 / 候选携带）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotUpdate {
    pub slot_id: String,
    pub value: String,
    pub consumed_ranges: Vec<PinyinRange>,
    pub lock: bool,
}

/// 任务草稿槽位。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSlot {
    pub id: String,
    pub label: String,
    pub value: String,
    pub source_ranges: Vec<PinyinRange>,
    pub locked: bool,
    pub confidence_milli: u32,
}

/// 任务草稿（槽位模型）。空草稿 = 默认值（intent 空串、全空数组、revision 0）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraft {
    pub intent: String,
    pub intent_ranges: Vec<PinyinRange>,
    pub slots: Vec<TaskSlot>,
    pub unconsumed_ranges: Vec<PinyinRange>,
    pub revision: u64,
}

/// 结构化候选命令（唯一可选来源；`message` 永不解析为操作）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub high_risk: bool,
    pub description: String,
    pub category: String,
    pub risk_level: RiskLevel,
    pub requires_confirmation: bool,
    pub preview: String,
    pub enabled: bool,
    pub disabled_reason: String,
    pub task_revision: u64,
    pub slot_updates: Vec<SlotUpdate>,
    pub commit_task: bool,
}

/// 当前输入位置的应用身份（不含窗口标题）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationView {
    pub process_id: u32,
    pub thread_id: u32,
    pub executable: String,
    pub window_class: String,
    pub focus_window_class: String,
    pub context_id: String,
    pub sensitive_input: bool,
}

/// 用户输入的结构化视图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputView {
    pub raw_pinyin: String,
    pub segmented_pinyin: String,
    pub selected_text: String,
    pub pending_pinyin: String,
    pub natural_language: String,
    pub input_mode: String,
    pub correction_enabled: bool,
}

impl Default for InputView {
    fn default() -> Self {
        Self {
            raw_pinyin: String::new(),
            segmented_pinyin: String::new(),
            selected_text: String::new(),
            pending_pinyin: String::new(),
            natural_language: String::new(),
            input_mode: "agent".to_string(),
            correction_enabled: false,
        }
    }
}

/// 上下文过滤状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivacyFlags {
    pub filtered: bool,
    pub redacted: bool,
    pub truncated: bool,
}

/// 其他输入位置/应用的上下文条目（按旧到新排列）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextEntry {
    pub context_id: String,
    pub application: ApplicationView,
    pub text: String,
    pub sequence: u64,
    pub context_type: String,
    pub source: String,
    pub created_at_ms: u64,
    pub relevance_milli: u32,
    pub is_current: bool,
    pub privacy: PrivacyFlags,
}

/// 管道请求（一问一答，每次连接一个请求）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIpcRequest {
    pub schema_version: u32,
    pub action: Action,
    pub session_id: String,
    pub request_id: String,
    pub parent_request_id: String,
    pub idempotency_key: String,
    pub capabilities: Vec<String>,
    pub protocol_min: u32,
    pub protocol_max: u32,
    pub required_features: Vec<String>,
    pub user_input: String,
    pub input: InputView,
    pub application: ApplicationView,
    pub session_context: String,
    pub context_entries: Vec<ContextEntry>,
    pub command_id: String,
    pub page: u32,
    pub task_revision: u64,
    pub slot_updates: Vec<SlotUpdate>,
}

/// 管道响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIpcResponse {
    pub schema_version: u32,
    pub session_id: String,
    pub request_id: String,
    pub message: String,
    pub commands: Vec<Command>,
    pub executing_command: String,
    pub status: Status,
    pub page: u32,
    pub has_more: bool,
    pub error_code: String,
    pub state_revision: u64,
    pub progress: u32,
    pub retry_after_ms: u32,
    pub can_cancel: bool,
    pub can_continue_input: bool,
    pub expires_at_ms: u64,
    pub error_message: String,
    pub retryable: bool,
    pub capabilities: Vec<String>,
    pub task: TaskDraft,
}

// ────────────────────────────── 错误与校验 ──────────────────────────────

/// 协议层错误。
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("载荷超限：{size} 字节 > 上限 {max} 字节")]
    PayloadTooLarge { size: usize, max: usize },
    #[error("字段 `{field}` 非法：{reason}")]
    InvalidField { field: &'static str, reason: String },
}

fn invalid(field: &'static str, reason: impl Into<String>) -> ProtocolError {
    ProtocolError::InvalidField {
        field,
        reason: reason.into(),
    }
}

/// 校验 token：`^[A-Za-z0-9._-]{0,128}$`（ASCII 字符集 + 长度按 code point 计）。
fn check_token(value: &str, field: &'static str, allow_empty: bool) -> Result<(), ProtocolError> {
    if value.is_empty() {
        return if allow_empty {
            Ok(())
        } else {
            Err(invalid(field, "不能为空"))
        };
    }
    if value.chars().count() > 128 {
        return Err(invalid(field, "超过 128 字符上限"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(invalid(
            field,
            "仅允许 ASCII 字母、数字、点、连字符或下划线",
        ));
    }
    Ok(())
}

/// 校验字符串长度（code point 计）。
fn check_len(
    value: &str,
    field: &'static str,
    max: usize,
    min: usize,
) -> Result<(), ProtocolError> {
    let len = value.chars().count();
    if len < min {
        return Err(invalid(field, format!("长度 {len} 小于下限 {min}")));
    }
    if len > max {
        return Err(invalid(field, format!("长度 {len} 超过上限 {max}")));
    }
    Ok(())
}

/// 校验 session_id：16–64 位十六进制或带连字符标识。
fn check_session_id(value: &str) -> Result<(), ProtocolError> {
    let len = value.chars().count();
    if !(16..=64).contains(&len) {
        return Err(invalid(
            "session_id",
            format!("长度 {len} 不在 16–64 范围内"),
        ));
    }
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(invalid("session_id", "仅允许十六进制字符或连字符"));
    }
    Ok(())
}

fn check_ranges(ranges: &[PinyinRange], field: &'static str) -> Result<(), ProtocolError> {
    if ranges.len() > MAXIMUM_AGENT_TASK_RANGES {
        return Err(invalid(field, "拼音区间数量超过 32"));
    }
    for range in ranges {
        if range.start > 4095 || range.end > 4096 || range.start >= range.end {
            return Err(invalid(
                field,
                format!("区间 [{}, {}) 非法", range.start, range.end),
            ));
        }
    }
    Ok(())
}

fn check_unique_tokens(
    values: &[String],
    field: &'static str,
    max: usize,
) -> Result<(), ProtocolError> {
    if values.len() > max {
        return Err(invalid(field, format!("数量超过上限 {max}")));
    }
    let mut seen = std::collections::HashSet::new();
    for value in values {
        check_token(value, field, false)?;
        if !seen.insert(value.as_str()) {
            return Err(invalid(field, format!("存在重复项 `{value}`")));
        }
    }
    Ok(())
}

fn check_slot_updates(updates: &[SlotUpdate]) -> Result<(), ProtocolError> {
    if updates.len() > MAXIMUM_AGENT_SLOT_UPDATES {
        return Err(invalid("slot_updates", "数量超过 8"));
    }
    for update in updates {
        check_token(&update.slot_id, "slot_updates.slot_id", false)?;
        check_len(&update.value, "slot_updates.value", 4096, 0)?;
        check_ranges(&update.consumed_ranges, "slot_updates.consumed_ranges")?;
    }
    Ok(())
}

fn check_application(app: &ApplicationView) -> Result<(), ProtocolError> {
    check_len(&app.executable, "application.executable", 260, 0)?;
    check_len(&app.window_class, "application.window_class", 128, 0)?;
    check_len(
        &app.focus_window_class,
        "application.focus_window_class",
        128,
        0,
    )?;
    check_token(&app.context_id, "application.context_id", true)
}

fn check_command(command: &Command) -> Result<(), ProtocolError> {
    check_token(&command.id, "command.id", false)?;
    check_len(&command.label, "command.label", 512, 1)?;
    check_len(&command.description, "command.description", 2048, 0)?;
    check_token(&command.category, "command.category", true)?;
    check_len(&command.preview, "command.preview", 2048, 0)?;
    check_len(&command.disabled_reason, "command.disabled_reason", 512, 0)?;
    check_slot_updates(&command.slot_updates)?;
    // 风险字段不变量（协议最小接入验证第 15 条）：
    // risk_level = high 必须同时 high_risk=true 且 requires_confirmation=true。
    if matches!(command.risk_level, RiskLevel::High | RiskLevel::Critical)
        && !(command.high_risk && command.requires_confirmation)
    {
        return Err(invalid(
            "command.risk_level",
            "高风险命令必须同时设置 high_risk=true 与 requires_confirmation=true",
        ));
    }
    Ok(())
}

fn check_task(task: &TaskDraft) -> Result<(), ProtocolError> {
    check_token(&task.intent, "task.intent", true)?;
    check_ranges(&task.intent_ranges, "task.intent_ranges")?;
    check_ranges(&task.unconsumed_ranges, "task.unconsumed_ranges")?;
    if task.slots.len() > MAXIMUM_AGENT_TASK_SLOTS {
        return Err(invalid("task.slots", "槽位数量超过 16"));
    }
    for slot in &task.slots {
        check_token(&slot.id, "task.slots.id", false)?;
        check_len(&slot.label, "task.slots.label", 256, 0)?;
        check_len(&slot.value, "task.slots.value", 4096, 0)?;
        check_ranges(&slot.source_ranges, "task.slots.source_ranges")?;
        if slot.confidence_milli > 1000 {
            return Err(invalid("task.slots.confidence_milli", "超过 1000"));
        }
    }
    Ok(())
}

impl AgentIpcRequest {
    /// 按 schema 校验（v3 严格字段集合；违者按 error 响应，不执行任何回退命令）。
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.schema_version != AGENT_PROTOCOL_VERSION {
            return Err(invalid(
                "schema_version",
                format!(
                    "仅支持 v{AGENT_PROTOCOL_VERSION} 结构（收到 {}）",
                    self.schema_version
                ),
            ));
        }
        check_session_id(&self.session_id)?;
        check_token(&self.request_id, "request_id", false)?;
        check_token(&self.parent_request_id, "parent_request_id", true)?;
        check_token(&self.idempotency_key, "idempotency_key", false)?;
        check_unique_tokens(&self.capabilities, "capabilities", 32)?;
        if !(MINIMUM_AGENT_PROTOCOL_VERSION..=AGENT_PROTOCOL_VERSION).contains(&self.protocol_min) {
            return Err(invalid("protocol_min", "取值须在 2–3"));
        }
        if !(MINIMUM_AGENT_PROTOCOL_VERSION..=AGENT_PROTOCOL_VERSION).contains(&self.protocol_max) {
            return Err(invalid("protocol_max", "取值须在 2–3"));
        }
        if self.protocol_min > self.protocol_max {
            return Err(invalid("protocol_min", "不能大于 protocol_max"));
        }
        check_unique_tokens(&self.required_features, "required_features", 16)?;
        check_len(&self.user_input, "user_input", 4096, 0)?;
        check_len(&self.input.raw_pinyin, "input.raw_pinyin", 4096, 0)?;
        check_len(
            &self.input.segmented_pinyin,
            "input.segmented_pinyin",
            8192,
            0,
        )?;
        check_len(&self.input.selected_text, "input.selected_text", 16384, 0)?;
        check_len(&self.input.pending_pinyin, "input.pending_pinyin", 4096, 0)?;
        check_len(
            &self.input.natural_language,
            "input.natural_language",
            16384,
            0,
        )?;
        check_token(&self.input.input_mode, "input.input_mode", true)?;
        check_application(&self.application)?;
        check_len(&self.session_context, "session_context", 32768, 0)?;
        if self.context_entries.len() > MAXIMUM_AGENT_CONTEXT_ENTRIES {
            return Err(invalid("context_entries", "数量超过 32"));
        }
        for entry in &self.context_entries {
            check_token(&entry.context_id, "context_entries.context_id", false)?;
            check_application(&entry.application)?;
            check_len(&entry.text, "context_entries.text", 32768, 1)?;
            check_token(&entry.context_type, "context_entries.context_type", true)?;
            check_token(&entry.source, "context_entries.source", true)?;
            if entry.relevance_milli > 1000 {
                return Err(invalid("context_entries.relevance_milli", "超过 1000"));
            }
        }
        check_token(&self.command_id, "command_id", true)?;
        if self.page > 1000 {
            return Err(invalid("page", "超过 1000"));
        }
        check_slot_updates(&self.slot_updates)?;
        Ok(())
    }

    /// 从帧载荷（UTF-8 JSON 字节）解码并校验。
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAXIMUM_AGENT_PAYLOAD_BYTES {
            return Err(ProtocolError::PayloadTooLarge {
                size: bytes.len(),
                max: MAXIMUM_AGENT_PAYLOAD_BYTES,
            });
        }
        let request: Self = serde_json::from_slice(bytes)?;
        request.validate()?;
        Ok(request)
    }

    /// 编码为 JSON 字节（先校验）。
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }

    /// 请求声明的能力与适配器支持能力的交集（响应 `capabilities` 用）。
    pub fn negotiated_capabilities(&self) -> Vec<String> {
        SUPPORTED_CAPABILITIES
            .iter()
            .filter(|capability| self.capabilities.iter().any(|c| c == *capability))
            .map(|capability| (*capability).to_string())
            .collect()
    }
}

impl AgentIpcResponse {
    /// 按 schema 校验（出站响应自检）。
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.schema_version != AGENT_PROTOCOL_VERSION {
            return Err(invalid("schema_version", "必须为 3"));
        }
        check_session_id(&self.session_id)?;
        check_token(&self.request_id, "request_id", false)?;
        check_len(&self.message, "message", 16384, 0)?;
        if self.commands.len() > MAXIMUM_AGENT_COMMANDS {
            return Err(invalid("commands", "数量超过 64"));
        }
        let mut seen = std::collections::HashSet::new();
        for command in &self.commands {
            check_command(command)?;
            if !seen.insert(command.id.as_str()) {
                return Err(invalid(
                    "commands.id",
                    format!("重复的命令 ID `{}`", command.id),
                ));
            }
        }
        check_len(&self.executing_command, "executing_command", 4096, 0)?;
        if self.page > 1000 {
            return Err(invalid("page", "超过 1000"));
        }
        check_token(&self.error_code, "error_code", true)?;
        if self.progress > 100 {
            return Err(invalid("progress", "超过 100"));
        }
        if self.retry_after_ms > 300_000 {
            return Err(invalid("retry_after_ms", "超过 300000"));
        }
        check_len(&self.error_message, "error_message", 4096, 0)?;
        check_unique_tokens(&self.capabilities, "capabilities", 32)?;
        check_task(&self.task)?;
        Ok(())
    }

    /// 编码为 JSON 字节（先校验）。
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }

    /// 从 JSON 字节解码（用于契约测试对照）。
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAXIMUM_AGENT_PAYLOAD_BYTES {
            return Err(ProtocolError::PayloadTooLarge {
                size: bytes.len(),
                max: MAXIMUM_AGENT_PAYLOAD_BYTES,
            });
        }
        let response: Self = serde_json::from_slice(bytes)?;
        response.validate()?;
        Ok(response)
    }
}
