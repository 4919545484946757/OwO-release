//! 响应与候选命令构造器。
//!
//! E1.3 提供基础构造器（响应骨架 / 错误 / 思考中 / 槽位提交候选）；
//! E1.5 在此基础上扩展 turn 结果 → 候选的完整映射。

use crate::protocol::{
    AgentIpcRequest, AgentIpcResponse, Command, RiskLevel, Status, TaskDraft,
    AGENT_PROTOCOL_VERSION,
};

/// 响应骨架：回填 session_id / request_id / 协商能力，其余字段取默认值。
pub fn base_response(request: &AgentIpcRequest, status: Status) -> AgentIpcResponse {
    AgentIpcResponse {
        schema_version: AGENT_PROTOCOL_VERSION,
        session_id: request.session_id.clone(),
        request_id: request.request_id.clone(),
        message: String::new(),
        commands: Vec::new(),
        executing_command: String::new(),
        status,
        page: request.page,
        has_more: false,
        error_code: String::new(),
        state_revision: 0,
        progress: 0,
        retry_after_ms: 0,
        can_cancel: false,
        can_continue_input: true,
        expires_at_ms: 0,
        error_message: String::new(),
        retryable: false,
        capabilities: request.negotiated_capabilities(),
        task: TaskDraft::default(),
    }
}

/// 结构化错误响应（`session_not_found` 标记为可重试，对齐 mock 语义）。
pub fn error_response(
    request: &AgentIpcRequest,
    code: &str,
    message: impl Into<String>,
) -> AgentIpcResponse {
    let message = message.into();
    let mut response = base_response(request, Status::Error);
    response.error_code = code.to_string();
    response.error_message = message.clone();
    response.message = message;
    response.retryable = code == "session_not_found";
    response
}

/// 异步处理中的响应（连接器按 `retry_after_ms` 继续查询）。
pub fn thinking_response(
    request: &AgentIpcRequest,
    progress: u32,
    retry_after_ms: u32,
) -> AgentIpcResponse {
    let mut response = base_response(request, Status::Thinking);
    response.message = "正在处理，请稍候…".to_string();
    response.can_cancel = true;
    response.can_continue_input = true;
    response.progress = progress;
    response.retry_after_ms = retry_after_ms;
    response
}

/// 把 Agent 回复作为可选上屏内容（是否上屏由连接器决定，见方案附录 B Q2）。
pub fn insert_reply_command(reply: &str) -> Command {
    let mut command = command(
        "insert-reply",
        &format!("插入回复：{}", truncate_chars(reply, 30)),
        "text.insert",
        RiskLevel::Low,
    );
    command.description = "把 Agent 的回复文字作为候选内容".to_string();
    command.preview = "不会执行系统操作；回复全文见消息区".to_string();
    command
}

/// 按字符数截断（超长补省略号）。
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

/// 通用候选构造。
pub fn command(id: &str, label: &str, category: &str, risk_level: RiskLevel) -> Command {
    Command {
        id: id.to_string(),
        label: label.to_string(),
        high_risk: false,
        description: String::new(),
        category: category.to_string(),
        risk_level,
        requires_confirmation: false,
        preview: String::new(),
        enabled: true,
        disabled_reason: String::new(),
        task_revision: 0,
        slot_updates: Vec::new(),
        commit_task: false,
    }
}

/// 槽位补丁应用后的「确认提交任务」候选（对齐 mock `execute-reminder` 语义）。
pub fn task_commit_command(task_revision: u64) -> Command {
    let mut command = command("task-commit", "确认提交任务", "task.commit", RiskLevel::Low);
    command.description = "提交当前已锁定的任务草稿".to_string();
    command.preview = "由第三方 Agent 按自身权限规则继续确认或执行".to_string();
    command.task_revision = task_revision;
    command.commit_task = true;
    command
}

/// 查看改动明细候选（打开工作台，只读）。
pub fn view_diff_command(file_count: usize) -> Command {
    let mut command = command(
        "view-diff",
        &format!("查看 {file_count} 处改动"),
        "file.diff",
        RiskLevel::None,
    );
    command.description = "在 Agent 工作台中查看改动明细".to_string();
    command.preview = "只读展示，不会修改文件".to_string();
    command
}

/// 回滚全部改动候选（供 diff 场景使用）。
pub fn revert_command() -> Command {
    let mut command = command("revert-all", "撤销全部改动", "file.revert", RiskLevel::Low);
    command.description = "把本次会话改动的文件恢复原状".to_string();
    command.preview = "只影响本会话的 workspace 改动".to_string();
    command
}

/// 打开 Agent 可信界面确认（高风险操作唯一通路，协议红线）。
pub fn confirm_ui_command(request_id: &str, tool_hint: &str) -> Command {
    let mut command = command(
        &format!("confirm-{request_id}"),
        &format!("打开确认界面：允许 {tool_hint}"),
        "agent.confirm_ui",
        RiskLevel::Low,
    );
    command.description = "在 Agent 可信界面（桌宠 / Web 工作台）中确认".to_string();
    command.preview = "候选框不直接授权高风险操作".to_string();
    command
}
