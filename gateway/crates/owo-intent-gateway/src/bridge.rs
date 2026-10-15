//! IME ↔ agent-server 桥接：turn 异步适配与命令执行（E1.4）。
//!
//! 数据流：管道请求 → [`ImeState::dispatch`]（纯状态决策）→
//! - `Respond`：直接回包；
//! - `StartTurn`：spawn 本模块的 [`run_turn_task`]（HTTP 回环 + SSE 消费），
//!   先把 `thinking` 立即回包；turn 完成后经 `complete_turn` 写入槽位；
//! - `RunCommand`：执行真实动作（回滚 / 打开可信界面 / 任务提交）。
//!
//! 取消：`slot.cancelled()` 触发 → `POST /abort` → 继续消费流至多 30s 宽限，
//! 然后强制结束并回写取消响应。

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use owo_agent_protocol::SseEvent;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::commands;
use crate::http::{BridgeError, OwoHttpClient};
use crate::protocol::{AgentIpcRequest, AgentIpcResponse, Status};
use crate::sse::{SseFrame, SseParser};
use crate::state::{
    CommandAction, ImeState, PendingHandle, PermissionNotice, StartTurnRequest, StateAction,
};

/// 取消后的流消费宽限期（之后强制结束）。
const CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(30);
/// 最终消息字符上限（协议 `message` maxLength 16384）。
const MESSAGE_LIMIT: usize = 16_000;

/// 桥接器：管道帧处理器 + HTTP 回环客户端 + 会话映射。
pub struct ImeBridge {
    state: Arc<ImeState>,
    client: Arc<OwoHttpClient>,
    workspace: String,
    web_url: String,
    /// IME 会话 → agent-server 会话 ID。
    sessions: Arc<Mutex<HashMap<String, String>>>,
}

impl ImeBridge {
    pub fn new(
        state: Arc<ImeState>,
        client: Arc<OwoHttpClient>,
        workspace: impl Into<String>,
        web_url: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state,
            client,
            workspace: workspace.into(),
            web_url: web_url.into(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// 处理一帧请求载荷，返回响应载荷（`None` = 不回包直接断开）。
    ///
    /// 非法请求照官方 mock 行为：记录并断连（无法回填 session_id/request_id，不伪造响应）。
    pub async fn handle_frame(&self, payload: Vec<u8>) -> Option<Vec<u8>> {
        let request = match AgentIpcRequest::decode(&payload) {
            Ok(request) => request,
            Err(error) => {
                warn!(%error, "IME 请求解码失败，拒绝并断开");
                return None;
            }
        };
        debug!(
            action = ?request.action,
            session = %request.session_id,
            request_id = %request.request_id,
            "收到 IME 请求"
        );

        let response = match self.state.dispatch(&request) {
            StateAction::Respond(response) => response,
            StateAction::StartTurn { immediate, turn } => {
                tokio::spawn(run_turn_task(
                    Arc::clone(&self.state),
                    Arc::clone(&self.client),
                    Arc::clone(&self.sessions),
                    self.workspace.clone(),
                    turn,
                ));
                immediate
            }
            StateAction::RunCommand { session_id, action } => {
                self.run_command(&request, &session_id, action).await
            }
        };

        match response.encode() {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                error!(%error, "IME 响应编码失败，断连");
                None
            }
        }
    }

    /// 执行命令动作（真实 IO）。
    async fn run_command(
        &self,
        request: &AgentIpcRequest,
        ime_session_id: &str,
        action: CommandAction,
    ) -> AgentIpcResponse {
        match action {
            CommandAction::RevertAll => {
                let agent_session = self.sessions.lock().await.get(ime_session_id).cloned();
                let Some(agent_session) = agent_session else {
                    return self.state.complete_command(
                        ime_session_id,
                        request,
                        commands::error_response(request, "session_not_found", "会话不存在"),
                    );
                };
                match self.client.revert(&agent_session).await {
                    Ok(()) => {
                        let mut response = commands::base_response(request, Status::Completed);
                        response.message = "已撤销全部改动".to_string();
                        self.state
                            .complete_command(ime_session_id, request, response)
                    }
                    Err(error) => {
                        warn!(%error, "revert 调用失败");
                        self.state.complete_command(
                            ime_session_id,
                            request,
                            commands::error_response(
                                request,
                                "revert_failed",
                                format!("撤销失败：{error}"),
                            ),
                        )
                    }
                }
            }
            CommandAction::OpenConfirmUi { request_id } => {
                let url = self.web_url.clone();
                info!(%request_id, %url, "打开 Agent 可信界面");
                open_in_browser(&url);
                let mut response = commands::base_response(request, Status::WaitingForConfirmation);
                response.message = format!("请在 Agent 可信界面完成确认（已请求打开 {url}）");
                response.can_cancel = true;
                self.state
                    .complete_command(ime_session_id, request, response)
            }
            CommandAction::OpenWorkbench => {
                let url = self.web_url.clone();
                info!(%url, "打开 Agent 工作台（diff 审阅）");
                open_in_browser(&url);
                let mut response = commands::base_response(request, Status::Completed);
                response.message = format!("已在 Agent 工作台打开改动明细（{url}）");
                self.state
                    .complete_command(ime_session_id, request, response)
            }
            CommandAction::CommitTask => {
                // E1.5：任务提交（reminder 等）走后续实现；当前明确告知。
                self.state.complete_command(
                    ime_session_id,
                    request,
                    commands::error_response(
                        request,
                        "not_implemented",
                        "任务提交将在后续版本支持",
                    ),
                )
            }
            CommandAction::InsertReply { text } => {
                // 状态机已内联处理；此处兜底（理论上不可达）。
                let mut response = commands::base_response(request, Status::Completed);
                response.message = text;
                self.state
                    .complete_command(ime_session_id, request, response)
            }
        }
    }
}

/// turn 后台任务：确保会话 → 发 turn → 消费 SSE → 写回结果。
pub async fn run_turn_task(
    state: Arc<ImeState>,
    client: Arc<OwoHttpClient>,
    sessions: Arc<Mutex<HashMap<String, String>>>,
    workspace: String,
    turn: Box<StartTurnRequest>,
) {
    let ime_session_id = turn.session_id.clone();
    let request = turn.request.clone();

    let outcome = execute_turn(&client, &sessions, &workspace, &turn).await;
    let (mut final_response, insert_text, diff_count) = match outcome {
        Ok((accumulator, agent_session)) => {
            let (mut response, insert_text) = build_final_response(&request, &accumulator);
            // 改动审阅候选：只有成功回合才拉 diff。
            let diff_count = if response.status == Status::AgentMode {
                match client.diff(&agent_session).await {
                    Ok(diffs) if !diffs.is_empty() => Some(diffs.len()),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(count) = diff_count {
                response.message = format!("{}（本次改动 {count} 个文件）", response.message);
            }
            (response, insert_text, diff_count)
        }
        Err(error) => {
            warn!(%error, session = %ime_session_id, "IME turn 执行失败");
            (
                commands::error_response(
                    &request,
                    "agent_error",
                    format!("Agent 执行失败：{error}"),
                ),
                None,
                None,
            )
        }
    };

    // 构建候选列表与动作表（响应携带 + 状态机注册）；再写回完成结果。
    let mut candidate_commands: Vec<crate::protocol::Command> = Vec::new();
    let mut actions: HashMap<String, CommandAction> = HashMap::new();
    if let Some(text) = insert_text {
        let command = commands::insert_reply_command(&text);
        actions.insert(command.id.clone(), CommandAction::InsertReply { text });
        candidate_commands.push(command);
    }
    if let Some(count) = diff_count {
        let view = commands::view_diff_command(count);
        actions.insert(view.id.clone(), CommandAction::OpenWorkbench);
        candidate_commands.push(view);
        let revert = commands::revert_command();
        actions.insert(revert.id.clone(), CommandAction::RevertAll);
        candidate_commands.push(revert);
    }
    if !candidate_commands.is_empty() {
        final_response.commands = candidate_commands.clone();
        state.set_commands(&ime_session_id, candidate_commands, actions);
    }
    let _ = state.complete_turn(&ime_session_id, final_response);
}

/// 执行一个 turn：会话准备 → SSE 消费（含取消）→ 累积结果。
///
/// 返回 `(累积器, agent 会话 ID)`（会话 ID 供后续 diff/revert 使用）。
async fn execute_turn(
    client: &OwoHttpClient,
    sessions: &Mutex<HashMap<String, String>>,
    workspace: &str,
    turn: &StartTurnRequest,
) -> Result<(TurnAccumulator, String), BridgeError> {
    // 1. agent 会话（一次创建、会话内复用）。
    let agent_session = {
        let mut map = sessions.lock().await;
        match map.get(&turn.session_id) {
            Some(session) => session.clone(),
            None => {
                let session = client.create_session(workspace).await?;
                map.insert(turn.session_id.clone(), session.id.clone());
                session.id
            }
        }
    };

    // 2. 发 turn（长流）。
    let prompt = compose_prompt(&turn.request);
    let response = client.turn_stream(&agent_session, &prompt).await?;

    // 3. 消费 SSE（select 取消信号）。
    let mut accumulator = TurnAccumulator::default();
    let mut parser = SseParser::new();
    let mut stream = response.bytes_stream();
    let mut cancel_deadline: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            _ = turn.slot.cancelled(), if !accumulator.cancelled => {
                accumulator.cancelled = true;
                info!(session = %agent_session, "IME 取消：发送 abort");
                if let Err(error) = client.abort(&agent_session).await {
                    warn!(%error, "abort 调用失败");
                }
                cancel_deadline = Some(tokio::time::Instant::now() + CANCEL_GRACE);
            }
            _ = tokio::time::sleep_until(
                cancel_deadline.unwrap_or_else(tokio::time::Instant::now)
            ), if cancel_deadline.is_some() => {
                warn!(session = %agent_session, "取消宽限期已过，强制结束流");
                break;
            }
            chunk = stream.next() => {
                let Some(chunk) = chunk else { break };
                let bytes = chunk?;
                for frame in parser.push(&bytes) {
                    apply_frame(&frame, &mut accumulator, &turn.slot);
                }
            }
        }
    }
    for frame in parser.finish() {
        apply_frame(&frame, &mut accumulator, &turn.slot);
    }
    Ok((accumulator, agent_session))
}

/// turn 过程累积器。
#[derive(Debug, Default)]
pub struct TurnAccumulator {
    pub final_text: String,
    pub failed: Option<String>,
    pub tools_used: usize,
    pub permission: Option<PermissionNotice>,
    pub cancelled: bool,
}

/// 处理一个 SSE 帧（`data` 为 `SseEvent` tagged JSON）。
fn apply_frame(frame: &SseFrame, accumulator: &mut TurnAccumulator, slot: &Arc<PendingHandle>) {
    let Ok(event) = serde_json::from_str::<SseEvent>(&frame.data) else {
        debug!(event = %frame.event, "忽略无法解析的 SSE 帧");
        return;
    };
    match event {
        SseEvent::Final { text } => {
            accumulator.final_text = text;
        }
        SseEvent::TurnFailed { message, .. } => {
            accumulator.failed = Some(message);
        }
        SseEvent::PermissionRequest {
            request_id,
            tool,
            reason,
            ..
        } => {
            accumulator.permission = Some(PermissionNotice {
                request_id,
                tool,
                reason,
            });
        }
        SseEvent::ToolUse { .. } => {
            accumulator.tools_used += 1;
        }
        SseEvent::ToolResult { .. } => {
            // 审批已响应（放行或拒绝）后工具才会产生结果：清除等待态。
            accumulator.permission = None;
        }
        _ => {}
    }
    // 同步进槽位（poll 可见审批等待与进度）。
    let mut guard = slot.lock();
    guard.permission_waiting = accumulator.permission.clone();
    guard.tools_used = accumulator.tools_used;
}

/// 由累积器构造最终 IME 响应。
///
/// 返回 `(响应, 可选 insert-reply 文本)`。
fn build_final_response(
    request: &AgentIpcRequest,
    accumulator: &TurnAccumulator,
) -> (AgentIpcResponse, Option<String>) {
    if accumulator.cancelled {
        let mut response = commands::base_response(request, Status::Cancelled);
        response.message = "任务已取消".to_string();
        return (response, None);
    }
    if let Some(message) = &accumulator.failed {
        let mut response = commands::error_response(
            request,
            "agent_failed",
            commands::truncate_chars(message, 4096),
        );
        response.retryable = true;
        return (response, None);
    }
    let text = accumulator.final_text.trim();
    let mut response = commands::base_response(request, Status::AgentMode);
    response.message = if text.is_empty() {
        "任务已完成（无文本输出）".to_string()
    } else {
        commands::truncate_chars(text, MESSAGE_LIMIT)
    };
    let insert_text = (!text.is_empty()).then(|| accumulator.final_text.clone());
    (response, insert_text)
}

/// 从 IME 请求构造 agent prompt（正文 + 简短输入法上下文）。
///
/// 隐私红线（协议 agent-ipc-privacy.md）：`application.sensitive_input == true`
/// （密码框等）时**不附带任何上下文、也不推断应用身份**，只发用户输入正文。
pub fn compose_prompt(request: &AgentIpcRequest) -> String {
    let mut prompt = String::new();
    let base = if !request.user_input.is_empty() {
        request.user_input.as_str()
    } else {
        request.input.natural_language.as_str()
    };
    prompt.push_str(base);

    if request.application.sensitive_input {
        return prompt;
    }

    let mut context_lines = Vec::new();
    if !request.input.selected_text.is_empty() {
        context_lines.push(format!(
            "已选文字：{}",
            commands::truncate_chars(&request.input.selected_text, 200)
        ));
    }
    if !request.application.executable.is_empty() {
        context_lines.push(format!("当前应用：{}", request.application.executable));
    }
    if !request.session_context.is_empty() {
        context_lines.push(format!(
            "输入位置上下文：{}",
            commands::truncate_chars(&request.session_context, 800)
        ));
    }
    if !request.input.pending_pinyin.is_empty() {
        context_lines.push(format!("未消耗拼音：{}", request.input.pending_pinyin));
    }
    // context_entries：协议保证按旧到新排列；逐条限长防提示词膨胀。
    for entry in &request.context_entries {
        if entry.text.is_empty() {
            continue;
        }
        let app_hint = if entry.application.executable.is_empty() {
            String::new()
        } else {
            format!("（{}）", entry.application.executable)
        };
        context_lines.push(format!(
            "历史上下文{app_hint}：{}",
            commands::truncate_chars(&entry.text, 200)
        ));
    }
    if !context_lines.is_empty() {
        prompt.push_str("\n\n[输入法上下文]\n");
        prompt.push_str(&context_lines.join("\n"));
    }
    prompt
}

/// 打开默认浏览器（Windows）。
pub fn open_in_browser(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

#[cfg(windows)]
#[async_trait::async_trait]
impl crate::pipe::FrameHandler for ImeBridge {
    async fn handle(&self, payload: Vec<u8>) -> Option<Vec<u8>> {
        self.handle_frame(payload).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request() -> AgentIpcRequest {
        AgentIpcRequest {
            schema_version: 3,
            action: crate::protocol::Action::Submit,
            session_id: "0123456789abcdef0123456789abcdef".to_string(),
            request_id: "req-1".to_string(),
            parent_request_id: String::new(),
            idempotency_key: "req-1".to_string(),
            capabilities: vec![],
            protocol_min: 2,
            protocol_max: 3,
            required_features: vec![],
            user_input: "帮我找文件".to_string(),
            input: crate::protocol::InputView {
                selected_text: "选中文字".to_string(),
                ..Default::default()
            },
            application: crate::protocol::ApplicationView {
                executable: "notepad.exe".to_string(),
                ..Default::default()
            },
            session_context: "附近的输入上下文".to_string(),
            context_entries: vec![],
            command_id: String::new(),
            page: 0,
            task_revision: 0,
            slot_updates: vec![],
        }
    }

    #[test]
    fn prompt_includes_context_by_default() {
        let prompt = compose_prompt(&base_request());
        assert!(prompt.starts_with("帮我找文件"));
        assert!(prompt.contains("notepad.exe"));
        assert!(prompt.contains("附近的输入上下文"));
        assert!(prompt.contains("选中文字"));
    }

    #[test]
    fn sensitive_input_strips_all_context() {
        // 协议隐私红线：密码框等敏感输入不得携带/推断上下文。
        let mut request = base_request();
        request.application.sensitive_input = true;
        request.session_context = "秘密上下文".to_string();
        request.input.selected_text = "敏感选中".to_string();
        request.context_entries = vec![crate::protocol::ContextEntry {
            context_id: "ctx-1".to_string(),
            application: crate::protocol::ApplicationView::default(),
            text: "历史秘密".to_string(),
            sequence: 1,
            context_type: "committed_text".to_string(),
            source: "owo_commit".to_string(),
            created_at_ms: 0,
            relevance_milli: 500,
            is_current: false,
            privacy: crate::protocol::PrivacyFlags::default(),
        }];
        let prompt = compose_prompt(&request);
        assert_eq!(prompt, "帮我找文件", "敏感输入时只允许发送用户正文");
        assert!(!prompt.contains("秘密"));
        assert!(!prompt.contains("notepad.exe"));
    }

    #[test]
    fn context_entries_appended_in_order() {
        let mut request = base_request();
        request.session_context = String::new();
        request.input.selected_text = String::new();
        request.context_entries = vec![
            crate::protocol::ContextEntry {
                context_id: "ctx-1".to_string(),
                application: crate::protocol::ApplicationView {
                    executable: "word.exe".to_string(),
                    ..Default::default()
                },
                text: "第一段".to_string(),
                sequence: 1,
                context_type: "committed_text".to_string(),
                source: "owo_commit".to_string(),
                created_at_ms: 0,
                relevance_milli: 500,
                is_current: false,
                privacy: crate::protocol::PrivacyFlags::default(),
            },
            crate::protocol::ContextEntry {
                context_id: "ctx-2".to_string(),
                application: crate::protocol::ApplicationView::default(),
                text: "第二段".to_string(),
                sequence: 2,
                context_type: "committed_text".to_string(),
                source: "owo_commit".to_string(),
                created_at_ms: 0,
                relevance_milli: 600,
                is_current: false,
                privacy: crate::protocol::PrivacyFlags::default(),
            },
        ];
        let prompt = compose_prompt(&request);
        let first = prompt.find("第一段").expect("第一段必须在");
        let second = prompt.find("第二段").expect("第二段必须在");
        assert!(first < second, "上下文必须按旧到新排列");
        assert!(prompt.contains("word.exe"));
    }

    #[test]
    fn long_context_is_truncated() {
        let mut request = base_request();
        request.session_context = "很".repeat(2000);
        let prompt = compose_prompt(&request);
        // 800 字符上限 + 省略号：总长必须受控。
        assert!(prompt.chars().count() < 1200, "上下文必须限长");
    }
}
