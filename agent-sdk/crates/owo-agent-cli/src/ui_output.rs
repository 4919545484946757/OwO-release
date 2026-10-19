//! §11 CLI 统一输出：UiEvent channel + human/plain/jsonl 三种渲染。
//!
//! 完成标准（审计 §11）：`owo-agent turn …` 的 stdout 在 `--output jsonl` 下
//! 只承载稳定 JSONL 协议（重定向/管道可机器解析）；human 模式保持既有观感
//! 不变；plain 模式输出最小稳定文本。tracing 日志走 stderr，永不污染 stdout。

use colored::Colorize;
use owo_agent_core::TurnEvent;
use owo_agent_protocol::{PermissionResponse, SseEvent};
use serde_json::{json, Value};

/// `--output` 输出模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputMode {
    /// 人类可读（默认）：与既有观感一致（状态行走 EventPrinter）。
    Human,
    /// 纯文本：stdout 仅最终结果与改动文件路径；状态行走 stderr。
    Plain,
    /// JSONL：stdout 每行一个 JSON 事件/结果（协议稳定，供重定向/编排）。
    Jsonl,
}

/// `--permissions` 权限档案（§11 顶层权限参数，global）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PermissionsProfile {
    /// 默认：写/执行走审批（与现状一致）。
    Default,
    /// 只读：强制 read-only 策略（注册表无写/执行工具）。
    ReadOnly,
    /// 可信：跳过审批（等价 --no-approval；仅测试/自动化用）。
    Trusted,
}

impl OutputMode {
    /// jsonl 模式判定（UiSink/渲染器共用）。
    #[allow(dead_code)] // repl/tui JSONL 迁移完成前保留判定入口（turn 直接比对枚举）。
    pub fn is_jsonl(self) -> bool {
        matches!(self, Self::Jsonl)
    }
}

fn completion_status_label(status: owo_agent_protocol::CompletionStatusV1) -> String {
    match status {
        owo_agent_protocol::CompletionStatusV1::ResponseComplete => String::new(),
        owo_agent_protocol::CompletionStatusV1::Candidate => " · 代码变更待验收".to_string(),
        owo_agent_protocol::CompletionStatusV1::Accepted => " · 宿主验收通过".to_string(),
        owo_agent_protocol::CompletionStatusV1::Unverified => " · 结果未验证".to_string(),
        owo_agent_protocol::CompletionStatusV1::Blocked => " · 存在阻断问题".to_string(),
        owo_agent_protocol::CompletionStatusV1::Aborted => " · 已取消".to_string(),
    }
}

/// Resolve one-based option input or retain free text; empty input leaves the question pending.
pub(crate) fn resolve_question_answer(input: &str, options: &[String]) -> Option<String> {
    let answer = input.trim();
    if answer.is_empty() {
        return None;
    }
    if let Ok(index) = answer.parse::<usize>() {
        if let Some(option) = index.checked_sub(1).and_then(|index| options.get(index)) {
            return Some(option.clone());
        }
    }
    Some(answer.to_string())
}

/// Human-readable pointer to the host's durable completion evidence. Machine output keeps
/// the original protocol payload unchanged; interactive surfaces show a compact locator.
pub(crate) fn completion_record_summary(
    record: Option<&owo_agent_protocol::TaskCompletionRecordV1>,
) -> Option<String> {
    let record = record?;
    let short = |value: &str, limit: usize| value.chars().take(limit).collect::<String>();
    let receipts = record
        .evidence_receipt_ids
        .iter()
        .take(3)
        .map(|id| short(id, 10))
        .collect::<Vec<_>>();
    let mut summary = format!(
        "交付记录 task={} attempt={} 收据 {} 项",
        short(&record.task_id, 8),
        short(&record.attempt_id, 8),
        record.evidence_receipt_ids.len(),
    );
    if !receipts.is_empty() {
        summary.push_str(&format!(
            " [{}{}]",
            receipts.join(", "),
            if receipts.len() < record.evidence_receipt_ids.len() {
                ", …"
            } else {
                ""
            }
        ));
    }
    if let Some(hash) = record.candidate_version_sha256.as_deref() {
        summary.push_str(&format!(" · 候选 SHA-256 {}", short(hash, 12)));
    }
    Some(summary)
}

#[derive(Default)]
pub(crate) struct TurnCompletion {
    final_text: Option<String>,
    status: Option<owo_agent_protocol::CompletionStatusV1>,
    record: Option<owo_agent_protocol::TaskCompletionRecordV1>,
    failure: Option<String>,
    stats: Option<SseEvent>,
    steps: usize,
}

#[derive(Debug)]
pub(crate) struct CompletedTurn {
    pub final_text: Option<String>,
    pub status: owo_agent_protocol::CompletionStatusV1,
    pub record: Option<owo_agent_protocol::TaskCompletionRecordV1>,
    pub failure: Option<String>,
    pub stats: Option<SseEvent>,
    pub steps: usize,
}

impl TurnCompletion {
    pub(crate) fn observe(&mut self, event: &SseEvent) {
        match event {
            SseEvent::Final { text } => self.final_text = Some(text.clone()),
            SseEvent::ToolResult { .. } => self.steps = self.steps.saturating_add(1),
            SseEvent::TurnStats {
                completion_status,
                completion_record,
                ..
            } => {
                self.stats = Some(event.clone());
                if self.failure.is_none() {
                    self.status = Some(*completion_status);
                    self.record = completion_record.clone();
                }
            }
            SseEvent::TurnFailed {
                message,
                completion_status,
                completion_record,
            } => {
                self.failure = Some(message.clone());
                self.status = Some(*completion_status);
                self.record = completion_record.clone();
            }
            _ => {}
        }
    }

    pub(crate) fn finish(self) -> Result<CompletedTurn, String> {
        let status = self
            .status
            .ok_or_else(|| "turn/missing_terminal: 缺少宿主终态，不能确认结果".to_string())?;
        if self.failure.is_none() && self.final_text.is_none() {
            return Err("turn/missing_final: 宿主结束但缺少最终回答".into());
        }
        Ok(CompletedTurn {
            final_text: self.final_text,
            status,
            record: self.record,
            failure: self.failure,
            stats: self.stats,
            steps: self.steps,
        })
    }
}

impl CompletedTurn {
    pub(crate) fn exit_code(&self) -> u8 {
        use owo_agent_protocol::CompletionStatusV1::*;
        if self.status == Aborted {
            return 130;
        }
        if self.failure.is_some() {
            return 1;
        }
        match self.status {
            ResponseComplete | Accepted => 0,
            Candidate => 2,
            Blocked => 3,
            Unverified => 4,
            Aborted => 130,
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        use owo_agent_protocol::CompletionStatusV1::*;
        if self.status == Aborted {
            return "已取消";
        }
        if self.failure.is_some() {
            return "执行失败";
        }
        match self.status {
            ResponseComplete => "回答结束（未声明交付验收）",
            Accepted => "宿主验收通过",
            Candidate => "候选结果待验收",
            Blocked => "存在阻断问题",
            Unverified => "结果未验证",
            Aborted => "已取消",
        }
    }

    pub(crate) fn render_jsonl(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        diffs: &[String],
    ) -> String {
        json!({
            "type": "turn_result", "session_id": session_id, "turn_id": turn_id,
            "final_text": self.final_text, "steps": self.steps, "diffs": diffs,
            "completion_status": self.status, "completion_record": self.record,
            "failure": self.failure, "stats": self.stats, "exit_code": self.exit_code(),
        })
        .to_string()
    }
}

pub(crate) async fn wait_for_local_abort(abort: &std::sync::atomic::AtomicBool) {
    while !abort.load(std::sync::atomic::Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod completion_reducer_contract_tests {
    use super::*;
    use owo_agent_protocol::CompletionStatusV1 as Status;
    fn stats(status: Status) -> SseEvent {
        serde_json::from_value(json!({
            "type":"turn_stats", "steps":1, "duration_ms":1, "prompt_tokens":0,
            "completion_tokens":0, "total_tokens":0, "cost_usd":0, "completion_status":status,
        }))
        .unwrap()
    }
    #[test]
    fn final_without_persisted_terminal_is_never_successful() {
        let mut result = TurnCompletion::default();
        result.observe(&SseEvent::Final {
            text: "fixture".into(),
        });
        assert!(result.finish().unwrap_err().contains("missing_terminal"));
    }
    #[test]
    fn failure_wins_even_when_stats_arrive_later() {
        let mut result = TurnCompletion::default();
        result.observe(&SseEvent::Final {
            text: "candidate".into(),
        });
        result.observe(&SseEvent::TurnFailed {
            message: "save failed".into(),
            completion_status: Status::Unverified,
            completion_record: None,
        });
        result.observe(&stats(Status::Accepted));
        let result = result.finish().unwrap();
        assert_eq!(result.exit_code(), 1);
        assert_eq!(result.label(), "执行失败");
        assert_eq!(result.status, Status::Unverified);
        let json: Value =
            serde_json::from_str(&result.render_jsonl("session", Some("turn"), &[])).unwrap();
        assert_eq!(json["completion_status"], "unverified");
        assert_eq!(json["failure"], "save failed");
    }
    #[test]
    fn nonaccepted_statuses_keep_distinct_exit_codes_and_labels() {
        for (status, code) in [
            (Status::Accepted, 0),
            (Status::Candidate, 2),
            (Status::Blocked, 3),
            (Status::Unverified, 4),
            (Status::Aborted, 130),
        ] {
            let mut result = TurnCompletion::default();
            result.observe(&SseEvent::Final {
                text: "fixture".into(),
            });
            result.observe(&stats(status));
            assert_eq!(result.finish().unwrap().exit_code(), code);
        }
    }
}

// ---------- 协议渲染纯函数（协议稳定性由测试锁定） ----------

/// jsonl：把任意可序列化事件包裹为稳定协议行（turn/repl 共用同一 schema）。
pub fn render_event_jsonl<T: serde::Serialize>(event: &T) -> String {
    json!({ "type": "turn_event", "event": event }).to_string()
}

/// jsonl：错误行。
pub fn render_error_jsonl(message: &str) -> String {
    json!({ "type": "error", "message": message }).to_string()
}

/// plain：最终结果 stdout 行集（最终文本在前，改动文件带 `M ` 前缀）。
pub fn render_final_result_plain(text: Option<&str>, diff_paths: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(text) = text {
        lines.push(text.to_string());
    }
    for diff in diff_paths {
        lines.push(format!("M {diff}"));
    }
    lines
}

/// 把 CLI 审批输入映射到与桌面端相同的四种授权动作。
/// 未识别输入始终拒绝；`s/session` 是旧版“本次会话”的兼容别名，
/// 统一降为任务范围，不再向服务端发送旧 `session` scope。
pub fn parse_approval_response(input: &str) -> PermissionResponse {
    let (allow, scope) = match input.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" | "1" | "once" => (true, Some("once")),
        "t" | "task" | "s" | "session" => (true, Some("task")),
        "w" | "workspace" => (true, Some("workspace")),
        "n" | "no" | "d" | "deny" | "" => (false, None),
        _ => (false, None),
    };
    PermissionResponse {
        allow,
        remember: None,
        scope: scope.map(str::to_string),
    }
}

// ---------- 审批卡（本地 Approver / Daemon SSE 两条路径共用同一观感） ----------

/// 审批卡数据：两条路径字段不完全一致，用 Option 收敛；参数优先用脱敏视图。
pub(crate) struct PermissionCard<'a> {
    pub tool: &'a str,
    pub level: Option<&'a str>,
    pub reason: &'a str,
    pub args: Option<&'a Value>,
    pub redacted_args: Option<&'a Value>,
    pub risk_note: Option<&'a str>,
    pub explain: Option<&'a Value>,
}

/// 渲染审批卡：让用户看清"批准的是什么"（工具/等级/原因/风险/影响/参数）。
/// 工具名展示：MCP 命名空间 `mcp__{server}__{tool}` → `MCP · server · tool`（审查 N5）。
///
/// 审批卡/授权清单是用户"批准谁"的地方：直接摊开 `mcp__owo_plugin_translate__translate`
/// 既难读也难核对；这里给出分层可读名，调用方同时保留原始注册名以便排查。
/// 非法/旧格式（历史会话与 trace 里的 `mcp_*`、`owo_plugin_*`）同样降级为可读文本。
pub(crate) fn tool_display_name(tool: &str) -> String {
    let trimmed = tool.trim();
    if let Some(rest) = trimmed.strip_prefix("mcp__") {
        return match rest.split_once("__") {
            Some((server, remote)) if !remote.is_empty() && remote != server => {
                format!("MCP · {server} · {remote}")
            }
            Some((server, _)) => format!("MCP · {server}"),
            None => format!("MCP · {rest}"),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("mcp_") {
        return format!("MCP · {rest}");
    }
    if let Some(rest) = trimmed.strip_prefix("owo_plugin_") {
        return format!("插件 · {rest}");
    }
    trimmed.to_string()
}

pub(crate) fn print_permission_card(card: &PermissionCard<'_>) {
    let label = tool_display_name(card.tool);
    let level = card.level.unwrap_or("unknown");
    if label == card.tool {
        println!("  {} {} 请求 {} 权限", "审批".yellow(), card.tool, level);
    } else {
        // 外部工具：可读名在前、注册名在后（审批必须对得上实际工具）。
        println!(
            "  {} {}（{}） 请求 {} 权限",
            "审批".yellow(),
            label,
            card.tool,
            level
        );
    }
    if !card.reason.trim().is_empty() {
        println!("    原因：{}", card.reason);
    }
    if let Some(note) = card.risk_note {
        // 内置工具无风险声明时核心会填统一占位串——展示侧抑制，避免每张卡都刷噪声。
        let is_placeholder =
            note.contains("未声明风险信息") || note.contains("未提供 MCP annotations");
        if !note.trim().is_empty() && !is_placeholder {
            println!("    {} {}", "风险：".red(), note);
        }
    }
    if let Some(explain) = card.explain {
        if !explain.is_null() && *explain != json!({}) {
            println!("    影响：{}", summarize_permission_args(explain, 240));
        }
    }
    if let Some(view) = card.redacted_args.or(card.args) {
        let summary = summarize_permission_args(view, 240);
        if !summary.is_empty() {
            println!("    参数：{summary}");
        }
    }
}

/// 审批卡参数摘要：紧凑 JSON，超长按字符截断（秘密字段已由 redacted_args 脱敏）。
pub(crate) fn summarize_permission_args(value: &Value, max_chars: usize) -> String {
    let raw = value.to_string();
    if raw == "null" || raw == "{}" {
        return String::new();
    }
    if raw.chars().count() <= max_chars {
        return raw;
    }
    let truncated: String = raw.chars().take(max_chars).collect();
    format!("{truncated}…")
}

// ---------- human 流式打印（本地 TurnEvent / Daemon SseEvent 共用） ----------

/// 统一流式打印器：Markdown 增量渲染 + 实时状态（思考中 / 工具参数 / 耗时）。
///
/// - `⏳ 思考中…` 在模型调用开始时显示，首个 token 到达时原地清除（仅 tty）；
/// - `▶ tool <参数预览>` 在工具启动时立即显示（含并发组，核心已改为实时外发）；
/// - `✔/✘ tool（1.2s）` 在工具完成时显示耗时。
pub struct StreamPrinter {
    markdown: crate::markdown::MarkdownStream,
    streamed: bool,
    status_visible: bool,
    tools: std::collections::HashMap<String, std::time::Instant>,
}

impl StreamPrinter {
    pub fn new() -> Self {
        Self {
            markdown: crate::markdown::MarkdownStream::new(),
            streamed: false,
            status_visible: false,
            tools: std::collections::HashMap::new(),
        }
    }

    /// 本地 `TurnEvent`。
    pub fn print_turn(&mut self, event: &TurnEvent) {
        match event {
            TurnEvent::ModelCall => {
                self.clear_status();
                self.show_status("  ⏳ 思考中…");
            }
            TurnEvent::TokenDelta { delta } => {
                self.clear_status();
                self.streamed = true;
                self.markdown.push(delta);
            }
            TurnEvent::ToolStart {
                id,
                tool,
                args_preview,
            } => {
                self.clear_status();
                self.tools.insert(id.clone(), std::time::Instant::now());
                println!(
                    "  {} {tool}{}",
                    "▶".blue(),
                    preview_suffix(args_preview.as_deref())
                );
            }
            TurnEvent::ToolResult {
                id,
                tool,
                ok,
                error,
                // 预览由 TUI 步骤面板消费；行式 REPL 保持单行输出。
                preview: _,
                // Host receipts are consumed by completion validation, not rendered as tool text.
                command_receipt: _,
            } => {
                self.clear_status();
                let suffix = self.elapsed_suffix(id);
                if *ok {
                    println!("  {} {tool}{suffix}", "✔".green());
                } else {
                    println!(
                        "  {} {tool}：{}{suffix}",
                        "✘".red(),
                        error.as_deref().unwrap_or("未知错误")
                    );
                }
            }
            TurnEvent::PermissionRequest(request) => {
                self.clear_status();
                print_permission_card(&PermissionCard {
                    tool: &request.tool,
                    level: Some(request.level.label()),
                    reason: &request.reason,
                    args: Some(&request.args),
                    redacted_args: request.redacted_args.as_ref(),
                    risk_note: request.risk_note.as_deref(),
                    explain: None,
                });
            }
            TurnEvent::Compaction { summary } => {
                self.clear_status();
                println!("  {}（上下文已压缩：{}）", "✦".yellow(), summary);
            }
            TurnEvent::ReasoningDelta { delta } => {
                // 思考通道（取优合并自远端 engine）：行式 REPL 以暗色实时输出，
                // 与正文区分；不写入最终回答文本。
                use std::io::Write;
                print!("{}", delta.dimmed());
                let _ = std::io::stdout().flush();
            }
            TurnEvent::PlanUpdate { steps } => {
                // 计划更新：行式 REPL 打印变更后的步骤清单（取优合并自远端 engine）。
                self.clear_status();
                let rendered = steps
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.get("content").and_then(|value| value.as_str()))
                            .collect::<Vec<_>>()
                            .join("；")
                    })
                    .unwrap_or_default();
                if !rendered.is_empty() {
                    println!("  {} 计划更新：{rendered}", "☰".cyan());
                }
            }
            TurnEvent::Final { text } => self.finish_final(text),
        }
    }

    /// Daemon `SseEvent`。
    pub fn print_sse(&mut self, event: &SseEvent) {
        match event {
            SseEvent::Progress { message } => {
                self.clear_status();
                if message.contains("模型调用") {
                    self.show_status("  ⏳ 思考中…");
                } else {
                    println!("{} {message}", "  ↻".cyan());
                }
            }
            SseEvent::TokenDelta { delta } => {
                self.clear_status();
                self.streamed = true;
                self.markdown.push(delta);
            }
            SseEvent::ToolUse { id, tool, args } => {
                self.clear_status();
                self.tools.insert(id.clone(), std::time::Instant::now());
                println!("  {} {tool}{}", "▶".blue(), preview_suffix(args.as_str()));
            }
            SseEvent::ToolResult {
                id,
                tool,
                ok,
                error,
                preview: _,
            } => {
                self.clear_status();
                let suffix = self.elapsed_suffix(id);
                if *ok {
                    println!("  {} {tool}{suffix}", "✔".green());
                } else {
                    println!(
                        "  {} {tool}：{}{suffix}",
                        "✘".red(),
                        error.as_deref().unwrap_or("未知错误")
                    );
                }
            }
            SseEvent::PermissionRequest {
                tool,
                reason,
                level,
                args,
                redacted_args,
                risk_note,
                explain,
                ..
            } => {
                self.clear_status();
                print_permission_card(&PermissionCard {
                    tool,
                    level: level.as_deref(),
                    reason,
                    args: Some(args),
                    redacted_args: redacted_args.as_ref(),
                    risk_note: risk_note.as_deref(),
                    explain: explain.as_ref(),
                });
            }
            SseEvent::PermissionResolved { .. } => {}
            // 回合失败终态（取优合并自远端 engine）：明确打印失败，不留在「执行中」。
            SseEvent::TurnFailed {
                message,
                completion_status,
                completion_record,
            } => {
                self.clear_status();
                println!(
                    "  {} 回合失败：{message}{}",
                    "✘".red(),
                    completion_status_label(*completion_status)
                );
                if let Some(summary) = completion_record_summary(completion_record.as_ref()) {
                    println!("    {summary}");
                }
            }
            SseEvent::Compaction { summary } => {
                self.clear_status();
                println!("  {}（上下文已压缩：{}）", "✦".yellow(), summary);
            }
            // 思考通道（取优合并自远端 engine）：暗色流式，不进入最终回答。
            SseEvent::ReasoningDelta { delta } => {
                self.clear_status();
                use std::io::Write;
                print!("{}", delta.dimmed());
                let _ = std::io::stdout().flush();
            }
            SseEvent::PlanUpdate { steps } => {
                self.clear_status();
                let count = steps.as_array().map(Vec::len).unwrap_or(0);
                println!("  {} 计划已更新（{count} 步）", "☰".cyan());
            }
            SseEvent::TurnStats {
                steps,
                duration_ms,
                total_tokens,
                completion_status,
                completion_record,
                model_calls,
                ..
            } => {
                self.clear_status();
                println!(
                    "  {} {steps} 步 / {duration_ms} ms / {total_tokens} tokens / {} 次模型请求{}",
                    "⏱".cyan(),
                    model_calls.len(),
                    completion_status_label(*completion_status)
                );
                if let Some(summary) = completion_record_summary(completion_record.as_ref()) {
                    println!("    {summary}");
                }
            }
            SseEvent::UserQuestion {
                question_id,
                question,
                options,
            } => {
                self.clear_status();
                println!("  {} 模型提问（{question_id}）：{question}", "?".yellow());
                if !options.is_empty() {
                    println!("     选项：{}", options.join(" / "));
                }
            }
            SseEvent::UserAnswered {
                question_id,
                answer,
                source,
            } => {
                self.clear_status();
                println!(
                    "  {} 已答复（{question_id}/{source}）：{answer}",
                    "✔".green()
                );
            }
            SseEvent::Final { text } => self.finish_final(text),
        }
    }

    /// 收尾：输出 Markdown 残留并清除状态行。
    pub fn finish(&mut self) {
        self.clear_status();
        if self.streamed {
            self.markdown.finish();
            self.streamed = false;
        }
    }

    fn finish_final(&mut self, text: &str) {
        self.clear_status();
        if self.streamed {
            self.markdown.finish();
            self.streamed = false;
        } else {
            println!("\n{}\n", "── 结果 ──".bold());
            self.markdown.push(text);
            self.markdown.finish();
        }
    }

    fn elapsed_suffix(&mut self, id: &str) -> String {
        match self.tools.remove(id) {
            Some(started) => format!("（{:.1}s）", started.elapsed().as_secs_f64()),
            None => String::new(),
        }
    }

    fn show_status(&mut self, text: &str) {
        use std::io::{IsTerminal, Write};
        if std::io::stdout().is_terminal() {
            print!("{text}");
            let _ = std::io::stdout().flush();
            self.status_visible = true;
        } else {
            println!("{text}");
        }
    }

    fn clear_status(&mut self) {
        use std::io::Write;
        if self.status_visible {
            print!("\r\u{1b}[2K");
            let _ = std::io::stdout().flush();
            self.status_visible = false;
        }
    }
}

impl Default for StreamPrinter {
    fn default() -> Self {
        Self::new()
    }
}

fn preview_suffix(preview: Option<&str>) -> String {
    match preview {
        Some(preview) if !preview.is_empty() => format!(" {preview}"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_display_name_renders_mcp_namespace_and_legacy_forms() {
        // 内置工具原样返回（调用方据此决定是否再附注册名）。
        assert_eq!(tool_display_name("read_file"), "read_file");
        assert_eq!(tool_display_name("  run_command "), "run_command");
        // 审查 N5 的命名空间：审批卡显示分层可读名。
        assert_eq!(
            tool_display_name("mcp__owo_plugin_translate__translate"),
            "MCP · owo_plugin_translate · translate"
        );
        assert_eq!(tool_display_name("mcp__git__status"), "MCP · git · status");
        // 只有 server 段（资源/提示类工具没有 `__tool`）时不留空尾段。
        assert_eq!(
            tool_display_name("mcp__owo_plugin_demo__"),
            "MCP · owo_plugin_demo"
        );
        assert_eq!(tool_display_name("mcp__solo"), "MCP · solo");
        // 历史会话/旧 trace 里的旧前缀仍可读。
        assert_eq!(tool_display_name("mcp_demo_lookup"), "MCP · demo_lookup");
        assert_eq!(
            tool_display_name("owo_plugin_translate_translate"),
            "插件 · translate_translate"
        );
    }

    #[test]
    fn question_answer_accepts_numbered_options_or_nonempty_free_text() {
        let options = vec!["继续".to_string(), "停止".to_string()];
        assert_eq!(
            resolve_question_answer("1", &options).as_deref(),
            Some("继续")
        );
        assert_eq!(
            resolve_question_answer(" 2 ", &options).as_deref(),
            Some("停止")
        );
        assert_eq!(
            resolve_question_answer("自定义说明", &options).as_deref(),
            Some("自定义说明")
        );
        assert_eq!(resolve_question_answer("", &options), None);
        assert_eq!(resolve_question_answer("0", &options).as_deref(), Some("0"));
    }

    #[test]
    fn completion_record_summary_exposes_receipts_and_candidate_version_compactly() {
        let record = owo_agent_protocol::TaskCompletionRecordV1 {
            task_id: "task-123456789".to_string(),
            attempt_id: "attempt-abcdefghi".to_string(),
            status: owo_agent_protocol::CompletionStatusV1::Accepted,
            evidence_receipt_ids: vec!["receipt-one".to_string(), "receipt-two".to_string()],
            candidate_version_sha256: Some("0123456789abcdef".to_string()),
            decided_at: "now".to_string(),
        };
        let summary = completion_record_summary(Some(&record)).unwrap();
        assert!(summary.contains("task=task-12"));
        assert!(summary.contains("attempt=attempt"));
        assert!(summary.contains("receipt-on"));
        assert!(summary.contains("0123456789ab"));
        assert!(completion_record_summary(None).is_none());
    }

    #[test]
    fn jsonl_turn_event_wraps_event() {
        let event = owo_agent_protocol::SseEvent::Progress {
            message: "模型调用".to_string(),
        };
        let line = render_event_jsonl(&event);
        let value: serde_json::Value = serde_json::from_str(&line).expect("应为合法 JSON");
        assert_eq!(value["type"], "turn_event");
        assert!(value["event"].is_object(), "应包裹原始事件");
        assert_eq!(value["event"]["type"], "progress");
    }

    #[test]
    fn jsonl_error_line_carries_message() {
        let value: serde_json::Value =
            serde_json::from_str(&render_error_jsonl("boom")).expect("应为合法 JSON");
        assert_eq!(value["type"], "error");
        assert_eq!(value["message"], "boom");
    }

    #[test]
    fn plain_lines_are_final_text_then_diff_markers() {
        let lines =
            render_final_result_plain(Some("答案"), &["a.txt".to_string(), "b.txt".to_string()]);
        assert_eq!(
            lines,
            vec![
                "答案".to_string(),
                "M a.txt".to_string(),
                "M b.txt".to_string()
            ]
        );
        assert!(render_final_result_plain(None, &[]).is_empty());
    }

    #[test]
    fn approval_choices_match_the_four_desktop_actions() {
        for (input, expected_scope) in [
            ("y", "once"),
            ("once", "once"),
            ("t", "task"),
            ("task", "task"),
            ("w", "workspace"),
            ("workspace", "workspace"),
            ("s", "task"),
            ("session", "task"),
        ] {
            let response = parse_approval_response(input);
            assert!(response.allow, "{input} should allow");
            assert_eq!(response.scope.as_deref(), Some(expected_scope));
            assert!(response.remember.is_none());
        }
    }

    #[test]
    fn approval_rejection_and_unknown_input_fail_closed() {
        for input in ["", "n", "no", "d", "deny", "typo"] {
            let response = parse_approval_response(input);
            assert!(!response.allow, "{input} should deny");
            assert!(response.scope.is_none());
            assert!(response.remember.is_none());
        }
    }

    #[test]
    fn permission_args_summary_skips_empty_and_truncates_long() {
        assert_eq!(summarize_permission_args(&json!(null), 10), "");
        assert_eq!(summarize_permission_args(&json!({}), 10), "");
        let short = summarize_permission_args(&json!({ "path": "a.txt" }), 240);
        assert!(short.contains("a.txt"));
        let long = summarize_permission_args(&json!({ "content": "x".repeat(500) }), 20);
        assert!(long.chars().count() <= 21, "truncated must fit: {long}");
        assert!(long.ends_with('…'));
    }

    #[test]
    fn preview_suffix_formats_tool_args() {
        assert_eq!(
            preview_suffix(Some("{\"path\":\"a\"}")),
            " {\"path\":\"a\"}"
        );
        assert_eq!(preview_suffix(Some("")), "");
        assert_eq!(preview_suffix(None), "");
    }
}
