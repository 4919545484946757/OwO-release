//! OpenCode 式全屏 TUI（ratatui + crossterm）。
//!
//! P1（指南 §8 P1 第 4 条）：**状态完全来自 Daemon 事件流**——本模块不再
//! `Agent::new` / `SqliteSessionStore::open` / `connect_mcp_clients`，不持有第二套
//! Session/Agent/SQLite/MCP。会话、回合、权限、工具、审计全部由 Daemon 持有，
//! TUI 只是 `AgentClient` 的事件消费者（由 `tests/turn_path_guard_tests.rs` 守卫）。

use crate::support::{display_path, ensure_daemon_client, ensure_data_root, AGENTS_TEMPLATE};
use crate::ui_output::parse_approval_response;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use owo_agent_client::AgentClient;
use owo_agent_protocol::{PermissionResponse, SseEvent};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;
#[path = "tui/keys.rs"]
mod keys;
mod reasoning;
use keys::*;

#[derive(clap::Args)]
pub struct TuiArgs {
    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, default_value = "build")]
    pub agent: String,
    /// 自动允许所有审批（仅测试用）
    #[arg(long)]
    pub no_approval: bool,
    /// 覆盖数据目录
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

pub fn run(
    args: TuiArgs,
    permissions: crate::ui_output::PermissionsProfile,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let workspace = args.workspace.canonicalize()?;
    let root = ensure_data_root(args.data_dir.clone(), &workspace);
    let client = runtime.block_on(ensure_daemon_client(&root, &workspace))?;
    let settings = load_tui_settings(&root, &workspace);
    let read_only = args.agent == "plan"
        || settings.read_only
        || matches!(permissions, crate::ui_output::PermissionsProfile::ReadOnly);
    let mut app = TuiApp::new(
        client,
        workspace,
        args.model.clone(),
        read_only,
        args.no_approval || matches!(permissions, crate::ui_output::PermissionsProfile::Trusted),
        settings.theme.as_deref(),
        &settings.keybinds,
    );
    app.read_only_locked = matches!(permissions, crate::ui_output::PermissionsProfile::ReadOnly);
    let terminal = ratatui::init();
    let result = app.run(&runtime, terminal);
    ratatui::restore();
    app.shutdown(&runtime);
    result
}

/// TUI 本地设置（只取主题/键位/只读，避免依赖 core `Settings`）。
#[derive(Default, serde::Deserialize)]
struct TuiSettings {
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    keybinds: HashMap<String, String>,
    #[serde(default)]
    read_only: bool,
}

fn load_tui_settings(root: &Path, workspace: &Path) -> TuiSettings {
    let mut settings = TuiSettings::default();
    for path in [
        root.join("settings.json"),
        workspace.join(".owo").join("settings.json"),
    ] {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<TuiSettings>(&text) else {
            continue;
        };
        if parsed.theme.is_some() {
            settings.theme = parsed.theme;
        }
        settings.keybinds.extend(parsed.keybinds);
        settings.read_only |= parsed.read_only;
    }
    settings
}

/// 待审批请求（来自 SSE `permission_request`）。
#[derive(Default, Clone)]
struct ApprovalInfo {
    request_id: String,
    args: serde_json::Value,
    redacted_args: Option<serde_json::Value>,
    risk_note: Option<String>,
    explain: Option<serde_json::Value>,
    tool: String,
    reason: String,
    level: String,
}

struct QuestionInfo {
    question_id: String,
    question: String,
    options: Vec<String>,
}

/// 回合结束摘要。
struct TurnSummary {
    completed: crate::ui_output::CompletedTurn,
    diff_count: usize,
}

enum TuiMsg {
    Event(SseEvent),
    Approval {
        info: ApprovalInfo,
        responder: tokio::sync::oneshot::Sender<PermissionResponse>,
    },
    Question {
        info: QuestionInfo,
        responder: tokio::sync::oneshot::Sender<String>,
    },
    QuestionResponseFailed(String),
    ApprovalAcknowledged {
        tool: String,
        accepted: bool,
        granted: bool,
        error: Option<String>,
    },
    Finished(Result<TurnSummary, String>),
}

struct TuiApp {
    client: AgentClient,
    workspace: PathBuf,
    model: Option<String>,
    read_only: bool,
    read_only_locked: bool,
    no_approval: bool,
    session_id: Option<String>,
    turn_id: Option<String>,
    approval: Option<(
        ApprovalInfo,
        tokio::sync::oneshot::Sender<PermissionResponse>,
    )>,
    question: Option<(QuestionInfo, tokio::sync::oneshot::Sender<String>)>,
    event_rx: Option<Receiver<TuiMsg>>,
    input: String,
    transcript: Vec<(String, Style)>,
    diff_view: Vec<(String, Style)>,
    show_diff_panel: bool,
    streaming: String,
    scroll: usize,
    running: bool,
    status: String,
    /// 思考通道指示器：状态栏显示"思考中 + 字数 + 尾部预览"（正文流不被打断）。
    reasoning: reasoning::ReasoningIndicator,
    should_exit: bool,
    abort: Arc<AtomicBool>,
    theme: Theme,
    keybinds: HashMap<String, KeyEvent>,
}

impl TuiApp {
    fn new(
        client: AgentClient,
        workspace: PathBuf,
        model: Option<String>,
        read_only: bool,
        no_approval: bool,
        theme_name: Option<&str>,
        keybinds: &HashMap<String, String>,
    ) -> Self {
        Self {
            client,
            workspace,
            model,
            read_only,
            read_only_locked: false,
            no_approval,
            session_id: None,
            turn_id: None,
            approval: None,
            question: None,
            event_rx: None,
            input: String::new(),
            transcript: vec![(
                "OwO Agent TUI — 输入文字开始任务，Tab 切换 build/plan，Ctrl+C 中止/退出，/help 查看命令"
                    .to_string(),
                dim(),
            )],
            diff_view: Vec::new(),
            show_diff_panel: false,
            streaming: String::new(),
            scroll: 0,
            running: false,
            status: "就绪".to_string(),
            reasoning: reasoning::ReasoningIndicator::default(),
            should_exit: false,
            abort: Arc::new(AtomicBool::new(false)),
            theme: theme(theme_name),
            keybinds: build_keybinds(keybinds),
        }
    }

    fn run(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        mut terminal: Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if self.handle_key(key, runtime)? {
                        break;
                    }
                }
            }
            self.drain_events();
            if self.running
                && self.approval.is_none()
                && self.question.is_none()
                && !self.abort.load(Ordering::Acquire)
            {
                self.status = "回合进行中（Ctrl+C 中止）".to_string();
            }
        }
        Ok(())
    }

    fn shutdown(&mut self, runtime: &tokio::runtime::Runtime) {
        self.abort.store(true, Ordering::Relaxed);
        if self.running {
            if let (Some(id), Some(turn)) = (self.session_id.clone(), self.turn_id.clone()) {
                let _ = runtime.block_on(async {
                    tokio::time::timeout(
                        Duration::from_secs(5),
                        self.client.cancel_turn_id(&id, &turn),
                    )
                    .await
                });
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(8),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(area);

        let model_label = self.model.as_deref().unwrap_or("（默认）");
        let title = Line::from(vec![
            Span::styled(
                " OwO Agent ",
                Style::default()
                    .fg(Color::Black)
                    .bg(self.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                display_path(&self.workspace),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(" | "),
            Span::styled(model_label, Style::default().fg(Color::Blue)),
            Span::raw(" | "),
            Span::styled(
                if self.read_only { "plan" } else { "build" },
                Style::default().fg(if self.read_only {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
            Span::raw(" | "),
            Span::styled(
                if self.running {
                    "● 运行中"
                } else {
                    "○ 空闲"
                },
                Style::default().fg(if self.running {
                    Color::Red
                } else {
                    Color::Green
                }),
            ),
            Span::raw(" | "),
            Span::styled(
                if self.show_diff_panel { "diff" } else { "chat" },
                Style::default().fg(if self.show_diff_panel {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            ),
        ]);
        frame.render_widget(Paragraph::new(title), chunks[0]);

        let source = if self.show_diff_panel {
            &self.diff_view
        } else {
            &self.transcript
        };
        let mut visible = self
            .visible_lines_of(source, chunks[1].height as usize)
            .into_iter()
            .collect::<Vec<_>>();
        if !self.streaming.is_empty() {
            visible.push((
                format!("▍{}", self.streaming),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ));
        }
        let lines: Vec<Line> = visible
            .iter()
            .map(|(text, style)| Line::from(Span::styled(text.clone(), *style)))
            .collect();
        let transcript = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" 会话 "))
            .wrap(Wrap { trim: false });
        frame.render_widget(transcript, chunks[1]);

        let mode_hint = if self.read_only {
            "plan（只读）"
        } else {
            "build"
        };
        let input_title = if self.question.is_some() {
            " 回答 Agent 的问题 ".to_string()
        } else {
            format!(" 输入（{mode_hint}） ")
        };
        let input_block = Block::default().borders(Borders::ALL).title(input_title);
        frame.render_widget(
            Paragraph::new(self.input.as_str())
                .block(input_block)
                .wrap(Wrap { trim: false }),
            chunks[2],
        );

        let status_text = if let Some((info, _)) = &self.approval {
            format!(
                "审批：{}（{}）{}——{} / Ctrl+C 取消",
                info.tool,
                info.level,
                info.reason,
                if info.level == "read" {
                    "y 本次 / t 任务 / w 工作区 / n 拒绝"
                } else {
                    "y 本次 / n 拒绝（写和执行不创建长期授权）"
                }
            )
        } else if self.question.is_some() {
            "回答 Agent 的问题：输入序号或补充说明，Enter 提交；Ctrl+C 中止".to_string()
        } else {
            self.status.clone()
        };
        let status_line = Line::from(vec![
            Span::styled(" Tab ", Style::default().fg(self.theme.accent)),
            Span::raw("模式 "),
            Span::styled(" Ctrl+C ", Style::default().fg(self.theme.accent)),
            Span::raw("中止/退出 "),
            Span::styled(" PgUp/PgDn ", Style::default().fg(self.theme.accent)),
            Span::raw("滚动 | "),
            Span::styled(status_text, Style::default().fg(Color::Yellow)),
        ]);
        frame.render_widget(Paragraph::new(status_line), chunks[3]);
    }

    fn visible_lines_of(&self, source: &[(String, Style)], height: usize) -> Vec<(String, Style)> {
        let viewport = height.saturating_sub(2).max(1);
        let end = source.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(viewport);
        source[start..end].to_vec()
    }

    #[cfg(test)]
    fn visible_lines(&self, height: usize) -> Vec<(String, Style)> {
        self.visible_lines_of(&self.transcript, height)
    }

    fn handle_key(
        &mut self,
        key: KeyEvent,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        // Windows 下同一个按键会同时派发 Press 与 Release（以及长按 Repeat），
        // 只处理 Press 防止输入重复。
        if key.kind != KeyEventKind::Press {
            return Ok(false);
        }
        if self.running && self.matches("abort", &key) {
            self.abort.store(true, Ordering::Release);
            self.approval = None;
            self.question = None;
            if let (Some(session), Some(turn)) = (self.session_id.clone(), self.turn_id.clone()) {
                let client = self.client.clone();
                runtime.spawn(async move {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(5),
                        client.cancel_turn_id(&session, &turn),
                    )
                    .await;
                });
            }
            self.status = "取消已请求，等待宿主确认…".into();
            self.push_system("取消已请求，正在等待实际终态".into(), yellow());
            return Ok(false);
        }
        if self.approval.is_some() {
            match key.code {
                KeyCode::Char('y' | 'Y') => self.respond_approval("once"),
                KeyCode::Char('t' | 'T') => self.respond_approval("task"),
                KeyCode::Char('w' | 'W') => self.respond_approval("workspace"),
                KeyCode::Char('n' | 'N') => self.respond_approval("deny"),
                _ => {}
            }
            return Ok(false);
        }
        if self.running {
            if self.question.is_some() {
                self.handle_question_key(key);
            }
            return Ok(false);
        }

        match key.code {
            KeyCode::Enter => self.submit(runtime)?,
            _ if self.matches("abort", &key) => {
                return Ok(true);
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => {
                if self.show_diff_panel {
                    self.show_diff_panel = false;
                    self.status = "就绪".to_string();
                } else {
                    self.input.clear();
                }
            }
            _ => {}
        }
        if self.matches("toggle_mode", &key) {
            self.toggle_mode()?;
        }
        if self.matches("scroll_up", &key) {
            self.scroll += 8;
        }
        if self.matches("scroll_down", &key) {
            self.scroll = self.scroll.saturating_sub(8);
        }
        if self.matches("clear", &key) {
            self.transcript.clear();
        }
        if self.matches("toggle_diff", &key) && !self.diff_view.is_empty() {
            self.show_diff_panel = !self.show_diff_panel;
        }
        Ok(self.should_exit)
    }

    fn handle_question_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                let options = self
                    .question
                    .as_ref()
                    .map(|(info, _)| info.options.clone())
                    .unwrap_or_default();
                if let Some(answer) =
                    crate::ui_output::resolve_question_answer(&self.input, &options)
                {
                    self.input.clear();
                    self.respond_question(answer);
                } else {
                    self.status = "回答不能为空；Agent 仍在等待。".to_string();
                }
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => self.input.clear(),
            _ => {}
        }
    }

    fn respond_question(&mut self, answer: String) {
        if let Some((info, responder)) = self.question.take() {
            let _ = responder.send(answer.clone());
            self.push_system(format!("已提交回答：{answer}"), green());
            self.status = format!("等待 Agent 继续（问题 {}）…", info.question_id);
        }
    }

    fn matches(&self, action: &str, key: &KeyEvent) -> bool {
        self.keybinds
            .get(action)
            .map(|expected| expected == key)
            .unwrap_or(false)
    }

    fn submit(
        &mut self,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let line = std::mem::take(&mut self.input);
        let line = line.trim().to_string();
        if line.is_empty() {
            return Ok(());
        }
        if line.starts_with("@explore ") || line.starts_with("@subagent ") {
            self.push_system(
                "子代理命令尚未迁移到 daemon 模式（用 --local 使用旧 REPL）".to_string(),
                yellow(),
            );
            return Ok(());
        }
        if let Some(command) = line.strip_prefix('/') {
            self.handle_command(command, runtime)?;
            return Ok(());
        }
        self.start_turn(runtime, &line);
        Ok(())
    }

    fn respond_approval(&mut self, action: &str) {
        if let Some((info, responder)) = self.approval.take() {
            let mut response = parse_approval_response(action);
            if info.level != "read" && response.allow {
                response.scope = Some("once".into());
            }
            let message = match response.scope.as_deref() {
                Some("once") => "允许决定已提交（仅本次）",
                Some("task") => "允许决定已提交（本任务，只读）",
                Some("workspace") => "允许决定已提交（工作区，只读）",
                _ => "拒绝决定已提交",
            };
            let style = if response.allow { green() } else { red() };
            let tool = info.tool;
            let _ = responder.send(response);
            self.push_system(format!("{message}：{tool}"), style);
            self.status = "执行中…".to_string();
        }
    }

    fn toggle_mode(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.read_only_locked {
            self.push_system("本次 CLI 强制只读，不能切换到 build".to_string(), yellow());
            return Ok(());
        }
        self.read_only = !self.read_only;
        self.push_system(
            if self.read_only {
                "已切换 plan（只读）".to_string()
            } else {
                "已切换 build".to_string()
            },
            yellow(),
        );
        Ok(())
    }

    fn handle_command(
        &mut self,
        command: &str,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut parts = command.split_whitespace();
        match parts.next().unwrap_or_default() {
            "help" => self.push_help(),
            "exit" | "quit" => {
                self.should_exit = true;
                self.status = "正在退出…".to_string();
            }
            "new" => self.new_session(runtime, parts.next())?,
            "sessions" => self.list_sessions(runtime),
            "resume" => {
                let id = parts.next().ok_or("用法：/resume <会话ID>")?.to_string();
                self.resume_session(runtime, &id)?;
            }
            "model" => match parts.next() {
                Some(model) => {
                    self.model = Some(model.to_string());
                    if let Some(id) = self.session_id.clone() {
                        let _ = runtime.block_on(self.client.session_set_model(&id, Some(model)));
                    }
                    self.push_system(format!("模型已切换：{model}"), green());
                }
                None => self.push_system(
                    format!("当前模型：{}", self.model.as_deref().unwrap_or("（默认）")),
                    dim(),
                ),
            },
            "diff" => self.refresh_diff(runtime),
            "undo" | "revert" => {
                if let Some(id) = self.session_id.clone() {
                    match runtime.block_on(self.client.session_revert(&id)) {
                        Ok(_) => self.push_system("已回滚本次会话全部写操作".to_string(), green()),
                        Err(error) => self.push_system(format!("回滚失败：{error}"), red()),
                    }
                } else {
                    self.push_system("暂无会话".to_string(), dim());
                }
            }
            "fork" => self.fork_session(runtime, parts.next())?,
            "rewind" => {
                let keep = parts
                    .next()
                    .ok_or("用法：/rewind <保留消息数>")?
                    .to_string();
                self.rewind_session(runtime, &keep)?;
            }
            "redo" => {
                if let Some(id) = self.session_id.clone() {
                    match runtime.block_on(self.client.session_redo(&id)) {
                        Ok(_) => self.push_system("已恢复最近一次 rewind".to_string(), green()),
                        Err(error) => self.push_system(format!("恢复失败：{error}"), red()),
                    }
                } else {
                    self.push_system("暂无会话".to_string(), dim());
                }
            }
            "tree" => self.show_tree(runtime),
            "traces" => self.print_json(runtime, "轨迹", "/traces"),
            "trace" => {
                if let Some(index) = parts.next() {
                    self.print_json(runtime, "轨迹", &format!("/traces/{index}"));
                } else {
                    self.push_system("用法：/trace <序号>（/traces 查看）".to_string(), dim());
                }
            }
            "settings" => self.print_json(runtime, "设置", "/settings"),
            "plugins" => self.print_json(runtime, "插件", "/plugins"),
            "skills" => self.print_json(runtime, "技能", "/skills"),
            "mcp" => self.print_json(runtime, "MCP", "/mcp"),
            "theme" => self.set_theme(parts.next()),
            "keybinds" => self.show_keybinds(),
            "plan" => {
                if !self.read_only {
                    self.toggle_mode()?;
                }
            }
            "build" => {
                if self.read_only {
                    self.toggle_mode()?;
                }
            }
            "status" => self.push_status(),
            "init" => {
                let target = self.workspace.join("AGENTS.md");
                if target.exists() {
                    self.push_system(
                        format!("AGENTS.md 已存在：{}", display_path(&target)),
                        yellow(),
                    );
                } else {
                    std::fs::write(&target, AGENTS_TEMPLATE)?;
                    self.push_system(format!("已生成 {}", display_path(&target)), green());
                }
            }
            "clear" => self.transcript.clear(),
            "share" | "export" | "undo-msg" | "redo-msg" => self.push_system(
                format!("/{command} 尚未迁移到 daemon 模式（用 --local）"),
                yellow(),
            ),
            other => self.push_system(format!("未知命令：/{other}（/help 查看）"), red()),
        }
        Ok(())
    }

    fn new_session(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        model: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let model = model.map(str::to_string).or_else(|| self.model.clone());
        let workspace = self.workspace.to_string_lossy().to_string();
        match runtime.block_on(self.client.create_session_with_model(&workspace, model)) {
            Ok(session) => {
                self.session_id = Some(session.id.clone());
                self.push_system(format!("新会话：{}", session.id), green());
            }
            Err(error) => self.push_system(format!("创建会话失败：{error}"), red()),
        }
        Ok(())
    }

    fn list_sessions(&mut self, runtime: &tokio::runtime::Runtime) {
        match runtime.block_on(self.client.list_sessions()) {
            Ok(sessions) => {
                if sessions.is_empty() {
                    self.push_system("暂无会话（/new 创建）".to_string(), dim());
                    return;
                }
                for session in sessions {
                    let active = self.session_id.as_deref() == Some(session.id.as_str());
                    let mut badges = String::new();
                    if session.pinned {
                        badges.push_str(" 📌");
                    }
                    if session.archived {
                        badges.push_str(" 🗄");
                    }
                    self.push_line(
                        format!(
                            "{}{}{}  model={}  updated={}",
                            if active { "▶ " } else { "  " },
                            session.title.unwrap_or_else(|| session.id.clone()),
                            badges,
                            session.model,
                            session.updated_at,
                        ),
                        if active { green() } else { default() },
                    );
                }
            }
            Err(error) => self.push_system(format!("读取会话失败：{error}"), red()),
        }
    }

    fn resume_session(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match runtime.block_on(self.client.get_session(id)) {
            Ok(session) => {
                self.session_id = Some(session.id.clone());
                self.push_system(format!("已恢复会话：{}", session.id), green());
            }
            Err(error) => self.push_system(format!("恢复失败：{error}"), red()),
        }
        Ok(())
    }

    fn fork_session(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        index: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(id) = self.session_id.clone() else {
            self.push_system("暂无会话".to_string(), dim());
            return Ok(());
        };
        let index: usize = index
            .unwrap_or("0")
            .parse()
            .map_err(|_| "消息序号需为数字")?;
        match runtime.block_on(self.client.session_fork(&id, index)) {
            Ok(child) => {
                self.push_system(format!("已创建子会话 {}", child.id), green());
                self.session_id = Some(child.id);
            }
            Err(error) => self.push_system(format!("fork 失败：{error}"), red()),
        }
        Ok(())
    }

    fn rewind_session(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        keep: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(id) = self.session_id.clone() else {
            self.push_system("暂无会话".to_string(), dim());
            return Ok(());
        };
        let keep: usize = keep.parse().map_err(|_| "保留消息数需为数字")?;
        match runtime.block_on(self.client.session_rewind(&id, keep)) {
            Ok(_) => self.push_system(format!("已回退到 {keep} 条消息（/redo 可恢复）"), yellow()),
            Err(error) => self.push_system(format!("回退失败：{error}"), red()),
        }
        Ok(())
    }

    fn show_tree(&mut self, runtime: &tokio::runtime::Runtime) {
        let Some(id) = self.session_id.clone() else {
            self.push_system("暂无会话".to_string(), dim());
            return;
        };
        self.print_json(runtime, "会话树", &format!("/session/{id}/children"));
    }

    fn print_json(&mut self, runtime: &tokio::runtime::Runtime, label: &str, path: &str) {
        match runtime.block_on(self.client.get_json::<serde_json::Value>(path)) {
            Ok(value) => {
                let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                for line in text.lines() {
                    self.push_line(format!("[{label}] {line}"), default());
                }
            }
            Err(error) => self.push_system(format!("[{label}] 读取失败：{error}"), red()),
        }
    }

    fn set_theme(&mut self, name: Option<&str>) {
        let name = name.unwrap_or("dark");
        self.theme = theme(Some(name));
        self.push_system(format!("已切换主题：{name}"), green());
    }

    fn show_keybinds(&mut self) {
        let mut lines: Vec<String> = self
            .keybinds
            .iter()
            .map(|(action, key)| format!("{action} = {}", format_key(key)))
            .collect();
        lines.sort();
        for line in lines {
            self.push_line(line, default());
        }
    }

    fn refresh_diff(&mut self, runtime: &tokio::runtime::Runtime) {
        let diffs = self
            .session_id
            .clone()
            .map(|id| {
                runtime
                    .block_on(self.client.session_diff(&id))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let (lines, active) = build_diff_panel(&diffs);
        self.diff_view = lines;
        self.show_diff_panel = active;
        if active {
            self.scroll = 0;
            self.status = "差异视图（Esc 返回）".to_string();
        }
    }

    fn push_status(&mut self) {
        self.push_line(
            format!("工作区：{}", display_path(&self.workspace)),
            default(),
        );
        self.push_line(
            format!("模型：{}", self.model.as_deref().unwrap_or("（默认）")),
            default(),
        );
        self.push_line(
            format!(
                "模式：{}",
                if self.read_only {
                    "plan（只读）"
                } else {
                    "build"
                }
            ),
            default(),
        );
        match &self.session_id {
            Some(id) => self.push_line(format!("会话：{id}"), default()),
            None => self.push_line("会话：无（任务时自动创建）".to_string(), dim()),
        }
    }

    fn push_help(&mut self) {
        for line in [
            "直接输入文字 发起任务",
            "/new /sessions /resume <id>  会话管理",
            "/fork [序号] /rewind <条数> /redo /tree  会话分支/回退/恢复/树",
            "/traces /trace <n>  回合轨迹",
            "/settings /plugins /skills /mcp  服务端状态",
            "/theme [dark|light] /keybinds  主题与键位",
            "/model [名称]  查看/切换模型",
            "/plan /build  切换只读/执行模式（或 Tab）",
            "/diff（d 差异视图）/undo  查看改动 / 回滚",
            "/status /init /clear",
            "/exit 退出（或 Ctrl+C）",
        ] {
            self.push_line(format!("  {line}"), dim());
        }
    }

    fn push_line(&mut self, text: String, style: Style) {
        self.transcript.push((text, style));
    }

    /// B2（取优合并自远端 engine）：流式正文去 Markdown 标记后再入 transcript
    /// ——ratatui 无法消费 `**粗体**`/围栏等原始标记，直接渲染会很脏。
    fn flush_streaming(&mut self) {
        if !self.streaming.is_empty() {
            let text = std::mem::take(&mut self.streaming);
            self.push_line(crate::markdown::strip_markdown(&text), default());
        }
    }

    fn push_system(&mut self, text: String, style: Style) {
        self.push_line(text, style);
    }

    fn start_turn(&mut self, runtime: &tokio::runtime::Runtime, prompt: &str) {
        if self.session_id.is_none() {
            if let Err(error) = self.new_session(runtime, None) {
                self.push_system(format!("创建会话失败：{error}"), red());
                return;
            }
        }
        let Some(session_id) = self.session_id.clone() else {
            return;
        };
        self.abort.store(false, Ordering::Relaxed);
        self.scroll = 0;
        self.running = true;
        self.status = "调用模型…".to_string();
        self.streaming.clear();
        self.push_line(format!("▶ {prompt}"), cyan());

        let client = self.client.clone();
        let no_approval = self.no_approval;
        let read_only = self.read_only || self.read_only_locked;
        let turn_id = uuid::Uuid::new_v4().to_string();
        self.turn_id = Some(turn_id.clone());
        let abort = Arc::clone(&self.abort);
        let prompt_owned = prompt.to_string();
        let (tx, rx) = mpsc::channel::<TuiMsg>();
        self.event_rx = Some(rx);
        runtime.spawn(async move {
            let result = run_turn_task(
                client,
                TurnInvocation {
                    session_id,
                    prompt: prompt_owned,
                    read_only,
                    turn_id,
                    abort,
                },
                tx.clone(),
                no_approval,
            )
            .await;
            let _ = tx.send(TuiMsg::Finished(result));
        });
    }

    fn drain_events(&mut self) {
        while let Some(message) = self.next_event() {
            match message {
                Some(TuiMsg::Event(event)) => self.push_event(event),
                Some(TuiMsg::Approval { info, responder }) => {
                    self.status = format!("审批：{}", info.tool);
                    self.flush_streaming();
                    self.push_line(format!("审批 {}：{}", info.request_id, info.tool), yellow());
                    if let Some(view) = &info.redacted_args {
                        self.push_line(
                            format!(
                                "参数：{}",
                                crate::ui_output::summarize_permission_args(view, 800)
                            ),
                            default(),
                        );
                    } else if !info.args.is_null() {
                        self.push_line("未收到脱敏参数，原始值已隐藏".into(), yellow());
                    }
                    if let Some(risk) = &info.risk_note {
                        self.push_line(format!("风险：{risk}"), yellow());
                    }
                    if let Some(explain) = &info.explain {
                        self.push_line(
                            format!(
                                "影响：{}",
                                crate::ui_output::summarize_permission_args(explain, 800)
                            ),
                            default(),
                        );
                    }
                    self.approval = Some((info, responder));
                }
                Some(TuiMsg::Question { info, responder }) => {
                    self.flush_streaming();
                    self.push_line(format!("? Agent 需要补充信息：{}", info.question), yellow());
                    for (index, option) in info.options.iter().enumerate() {
                        self.push_line(format!("  {}. {option}", index + 1), dim());
                    }
                    self.status = "输入选项序号或补充说明，按 Enter 提交".to_string();
                    self.question = Some((info, responder));
                }

                Some(TuiMsg::ApprovalAcknowledged {
                    tool,
                    accepted,
                    granted,
                    error,
                }) => {
                    let message = if let Some(error) = error {
                        format!("审批决定未送达：{tool}，{error}")
                    } else if accepted && granted {
                        format!("宿主已接受决定并创建可复用授权：{tool}")
                    } else if accepted {
                        format!("宿主已接受本次审批决定：{tool}")
                    } else {
                        format!("宿主没有确认审批决定：{tool}")
                    };
                    self.push_system(message, if accepted { cyan() } else { red() });
                }
                Some(TuiMsg::QuestionResponseFailed(error)) => {
                    self.question = None;
                    self.input.clear();
                    self.push_system(format!("回答未送达，已请求停止回合：{error}"), red());
                }
                Some(TuiMsg::Finished(result)) => {
                    self.running = false;
                    self.event_rx = None;
                    self.turn_id = None;
                    self.approval = None;
                    self.question = None;
                    self.input.clear();
                    match result {
                        Ok(summary) => {
                            // Final is authoritative; do not append the streamed text a second time.
                            self.streaming.clear();
                            if let Some(text) = &summary.completed.final_text {
                                self.push_line("── 结果 ──".to_string(), bold());
                                self.push_line(crate::markdown::strip_markdown(text), default());
                            }
                            let label = summary.completed.label();
                            let style = if summary.completed.status
                                == owo_agent_protocol::CompletionStatusV1::Accepted
                                && summary.completed.failure.is_none()
                            {
                                green()
                            } else {
                                yellow()
                            };
                            self.push_system(
                                format!(
                                    "{label}：工具 {} 步，改动 {} 个文件（/diff 查看）",
                                    summary.completed.steps, summary.diff_count
                                ),
                                style,
                            );
                            if let Some(error) = &summary.completed.failure {
                                self.push_system(error.clone(), red());
                            }
                            self.status = label.to_string();
                        }
                        Err(error) => {
                            self.push_system(format!("回合失败：{error}"), red());
                            self.status = "出错".to_string();
                        }
                    }
                }
                None => {}
            }
        }
    }

    fn next_event(&mut self) -> Option<Option<TuiMsg>> {
        let receiver = self.event_rx.as_ref()?;
        match receiver.try_recv() {
            Ok(message) => Some(Some(message)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.push_system("回合通道已断开".to_string(), red());
                self.running = false;
                self.event_rx = None;
                Some(None)
            }
        }
    }

    fn push_event(&mut self, event: SseEvent) {
        match event {
            SseEvent::TokenDelta { delta } => {
                // 正文开始：把思考指示器收起成一句小结（并让状态栏回到"生成中"），
                // 避免状态栏与回答流抢注意力。
                if let Some(note) = self.reasoning.settle() {
                    self.push_line(note, dim());
                    self.status = "生成中…".to_string();
                }
                self.streaming.push_str(&delta);
            }
            SseEvent::Final { .. } => {}
            SseEvent::Progress { message } => {
                self.flush_streaming();
                self.push_line(format!("  ↻ {message}"), cyan());
            }
            SseEvent::ToolUse { tool, .. } => {
                self.flush_streaming();
                self.push_line(format!("  ▶ {tool} …"), blue());
            }
            SseEvent::ToolResult {
                tool, ok, error, ..
            } => {
                self.flush_streaming();
                if ok {
                    self.push_line(format!("  ✔ {tool}"), green());
                } else {
                    self.push_line(
                        format!("  ✘ {tool}：{}", error.as_deref().unwrap_or("未知错误")),
                        red(),
                    );
                }
            }
            SseEvent::Compaction { summary } => {
                self.push_line(format!("  ✦（上下文已压缩：{summary}）"), yellow());
            }
            SseEvent::PermissionRequest { .. } => {}
            SseEvent::PermissionResolved { .. } => {}
            SseEvent::UserQuestion { .. } => {}
            SseEvent::UserAnswered {
                question_id,
                source,
                ..
            } => {
                self.question = None;
                self.input.clear();
                if source == "user" {
                    self.push_line(format!("✓ Agent 已收到回答（{question_id}）"), green());
                } else {
                    self.push_line(format!("? 提问 {question_id} 以 {source} 结束"), yellow());
                }
            }
            // 回合失败终态（取优合并自远端 engine）。
            SseEvent::TurnFailed {
                message,
                completion_status,
                completion_record,
            } => {
                let status = match completion_status {
                    owo_agent_protocol::CompletionStatusV1::ResponseComplete => "",
                    owo_agent_protocol::CompletionStatusV1::Candidate => " · 代码变更待验收",
                    owo_agent_protocol::CompletionStatusV1::Accepted => " · 宿主验收通过",
                    owo_agent_protocol::CompletionStatusV1::Unverified => " · 结果未验证",
                    owo_agent_protocol::CompletionStatusV1::Blocked => " · 存在阻断问题",
                    owo_agent_protocol::CompletionStatusV1::Aborted => " · 已取消",
                };
                self.push_line(format!("  ✘ 回合失败：{message}{status}"), red());
                self.reasoning.reset();
                if let Some(summary) =
                    crate::ui_output::completion_record_summary(completion_record.as_ref())
                {
                    self.push_line(format!("    {summary}"), dim());
                }
            }
            // 思考通道（取优合并自远端 engine）：TUI 不打断回答流，但状态栏必须
            // 看得见"模型在思考"（行式 REPL 是暗色实时流；TUI 折叠为一行指示器）。
            SseEvent::ReasoningDelta { delta } => {
                let text = self.reasoning.push(&delta);
                if !text.is_empty() {
                    self.status = text;
                }
            }
            // 计划更新（todo 工具）：清空流后提示步骤数。
            SseEvent::PlanUpdate { steps } => {
                self.flush_streaming();
                let count = steps.as_array().map(Vec::len).unwrap_or(0);
                self.push_line(format!("  ☰ 计划已更新（{count} 步）"), cyan());
            }
            // 回合统计（取优合并自远端 engine）：汇报耗时/步数/token。
            SseEvent::TurnStats {
                steps,
                duration_ms,
                total_tokens,
                completion_status,
                completion_record,
                model_calls,
                ..
            } => {
                let status = match completion_status {
                    owo_agent_protocol::CompletionStatusV1::ResponseComplete => "",
                    owo_agent_protocol::CompletionStatusV1::Candidate => " · 代码变更待验收",
                    owo_agent_protocol::CompletionStatusV1::Accepted => " · 宿主验收通过",
                    owo_agent_protocol::CompletionStatusV1::Unverified => " · 结果未验证",
                    owo_agent_protocol::CompletionStatusV1::Blocked => " · 存在阻断问题",
                    owo_agent_protocol::CompletionStatusV1::Aborted => " · 已取消",
                };
                self.push_line(
                    format!(
                        "  ⏱ {steps} 步 / {duration_ms} ms / {total_tokens} tokens / {} 次模型请求{status}",
                        model_calls.len()
                    ),
                    cyan(),
                );
                // 回合终态：思考指示器复位（下一回合重新计数）。
                self.reasoning.reset();
                if let Some(summary) =
                    crate::ui_output::completion_record_summary(completion_record.as_ref())
                {
                    self.push_line(format!("    {summary}"), dim());
                }
            }
        }
    }
}

/// Keeps the scoped identity, request restriction and cancellation token together.
struct TurnInvocation {
    session_id: String,
    prompt: String,
    read_only: bool,
    turn_id: String,
    abort: Arc<AtomicBool>,
}

/// 回合执行任务：消费 Daemon SSE，权限请求经 UI 决策后回传服务端。
async fn run_turn_task(
    client: AgentClient,
    invocation: TurnInvocation,
    tx: mpsc::Sender<TuiMsg>,
    no_approval: bool,
) -> Result<TurnSummary, String> {
    let TurnInvocation {
        session_id,
        prompt,
        read_only,
        turn_id,
        abort,
    } = invocation;
    let mut stream = tokio::select! {
        result = client.open_turn_with_id(&session_id, &prompt, read_only, &turn_id) =>
            result.map_err(|error| error.to_string())?,
        _ = crate::ui_output::wait_for_local_abort(&abort) => {
            let _ = tokio::time::timeout(Duration::from_secs(5), client.cancel_turn_id(&session_id, &turn_id)).await;
            return Err("turn/cancellation_unconfirmed: 已请求停止启动；请按回合 ID 查看宿主状态".into());
        }
    };
    let mut completion = crate::ui_output::TurnCompletion::default();
    let mut cancelling = false;
    let mut cancel_deadline = tokio::time::Instant::now();
    loop {
        let event = tokio::select! {
            event = stream.next_event() => event,
            _ = crate::ui_output::wait_for_local_abort(&abort), if !cancelling => {
                cancelling = true;
                cancel_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                let _ = tokio::time::timeout(Duration::from_secs(5), client.cancel_turn_id(&session_id, &turn_id)).await;
                continue;
            }
            _ = tokio::time::sleep_until(cancel_deadline), if cancelling => {
                return Err("turn/cancellation_unconfirmed: 宿主尚未确认终止，不能声明已完成".into());
            }
        };
        let Some(event) = event else { break };
        let event = event.map_err(|error| error.to_string())?;
        completion.observe(&event);
        match &event {
            SseEvent::PermissionRequest {
                request_id,
                tool,
                reason,
                level,
                args,
                redacted_args,
                risk_note,
                explain,
            } => {
                let info = ApprovalInfo {
                    request_id: request_id.clone(),
                    tool: tool.clone(),
                    reason: reason.clone(),
                    level: level.clone().unwrap_or_else(|| "unknown".into()),
                    args: args.clone(),
                    redacted_args: redacted_args.clone(),
                    risk_note: risk_note.clone(),
                    explain: explain.clone(),
                };
                let (responder, receiver) = tokio::sync::oneshot::channel();
                if no_approval {
                    let _ = responder.send(parse_approval_response("once"));
                } else {
                    tx.send(TuiMsg::Approval {
                        info: info.clone(),
                        responder,
                    })
                    .map_err(|_| "TUI 审批界面已关闭")?;
                }
                let response_client = client.clone();
                let response_session = session_id.clone();
                let response_tx = tx.clone();
                let response_abort = Arc::clone(&abort);
                tokio::spawn(async move {
                    let response = tokio::select! {
                        result = tokio::time::timeout(Duration::from_secs(300), receiver) =>
                            result.ok().and_then(|result| result.ok()),
                        _ = crate::ui_output::wait_for_local_abort(&response_abort) => None,
                    };
                    let Some(response) = response else { return };
                    let requested_allow = response.allow;
                    let result = response_client
                        .respond_permission(&response_session, &info.request_id, &response)
                        .await;
                    let (accepted, granted, error) = match result {
                        Ok(value) => (
                            value.get("allowed").and_then(serde_json::Value::as_bool)
                                == Some(requested_allow),
                            value
                                .get("granted")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                            None,
                        ),
                        Err(error) => (false, false, Some(error.to_string())),
                    };
                    let _ = response_tx.send(TuiMsg::ApprovalAcknowledged {
                        tool: info.tool,
                        accepted,
                        granted,
                        error,
                    });
                });
            }
            SseEvent::UserQuestion {
                question_id,
                question,
                options,
            } => {
                let (responder, receiver) = tokio::sync::oneshot::channel();
                let info = QuestionInfo {
                    question_id: question_id.clone(),
                    question: question.clone(),
                    options: options.clone(),
                };
                tx.send(TuiMsg::Question { info, responder })
                    .map_err(|_| "TUI 提问界面已关闭")?;
                let answer_client = client.clone();
                let answer_session = session_id.clone();
                let question_id = question_id.clone();
                let answer_tx = tx.clone();
                let answer_turn = turn_id.clone();
                tokio::spawn(async move {
                    let Ok(answer) = receiver.await else { return };
                    if let Err(error) = answer_client
                        .answer_question(&answer_session, &question_id, &answer)
                        .await
                    {
                        let _ = answer_client
                            .cancel_turn_id(&answer_session, &answer_turn)
                            .await;
                        let _ = answer_tx.send(TuiMsg::QuestionResponseFailed(error.to_string()));
                    }
                });
            }
            _ => {
                let _ = tx.send(TuiMsg::Event(event.clone()));
            }
        }
    }
    let completed = completion.finish()?;
    let diff_count = client
        .session_diff(&session_id)
        .await
        .map(|diffs| diffs.len())
        .unwrap_or(0);
    Ok(TurnSummary {
        completed,
        diff_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use owo_agent_client::ClientConfig;
    use owo_agent_protocol::FileDiff;

    fn test_app() -> TuiApp {
        // 渲染测试不发请求；用一个不可达地址构造 client 即可。
        let client = AgentClient::new(ClientConfig::new("http://127.0.0.1:1", None)).unwrap();
        TuiApp::new(
            client,
            std::env::temp_dir(),
            Some("mock".to_string()),
            false,
            true,
            None,
            &HashMap::new(),
        )
    }

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn reasoning_deltas_surface_in_status_bar_and_settle_into_a_note() {
        let mut app = test_app();
        app.push_event(SseEvent::ReasoningDelta {
            delta: "先看文件".to_string(),
        });
        assert!(app.status.contains("思考中"), "{}", app.status);
        assert!(app.status.contains("4 字"), "{}", app.status);

        app.push_event(SseEvent::ReasoningDelta {
            delta: "\n再跑测试".to_string(),
        });
        assert!(app.status.contains("8 字"), "{}", app.status);
        assert!(!app.status.contains('\n'), "状态栏必须单行：{}", app.status);

        // 正文开始：指示器收起为一句小结，状态栏不再停留在"思考中"。
        app.push_event(SseEvent::TokenDelta {
            delta: "答案".to_string(),
        });
        assert_eq!(app.streaming, "答案");
        assert!(
            app.transcript
                .iter()
                .any(|(text, _)| text.contains("思考 8 字")),
            "{:?}",
            app.transcript
        );
        assert!(!app.status.contains("思考中"), "{}", app.status);
        assert_eq!(app.status, "生成中…");
    }

    #[test]
    fn timeout_event_closes_question_input_and_reports_terminal_source() {
        let mut app = test_app();
        let (responder, _receiver) = tokio::sync::oneshot::channel();
        app.question = Some((
            QuestionInfo {
                question_id: "q-timeout".to_string(),
                question: "补充信息？".to_string(),
                options: Vec::new(),
            },
            responder,
        ));
        app.input = "尚未提交".to_string();
        app.push_event(SseEvent::UserAnswered {
            question_id: "q-timeout".to_string(),
            answer: String::new(),
            source: "timeout".to_string(),
        });
        assert!(app.question.is_none());
        assert!(app.input.is_empty());
        assert!(app
            .transcript
            .iter()
            .any(|(line, _)| line.contains("以 timeout 结束")));
    }

    #[test]
    fn question_responder_receives_selected_option() {
        let mut app = test_app();
        let (responder, mut receiver) = tokio::sync::oneshot::channel();
        app.question = Some((
            QuestionInfo {
                question_id: "q-1".to_string(),
                question: "继续还是停止？".to_string(),
                options: vec!["继续".to_string(), "停止".to_string()],
            },
            responder,
        ));
        app.respond_question("停止".to_string());
        assert!(app.question.is_none());
        assert_eq!(receiver.try_recv().expect("answer delivered"), "停止");
    }

    #[test]
    fn approval_responder_receives_decision() {
        let mut app = test_app();
        let (responder, mut receiver) = tokio::sync::oneshot::channel();
        app.approval = Some((
            ApprovalInfo {
                tool: "write_file".to_string(),
                reason: "测试".to_string(),
                level: "write".to_string(),
                ..Default::default()
            },
            responder,
        ));

        app.respond_approval("task");

        assert!(app.approval.is_none());
        let response = receiver.try_recv().expect("应收到决策");
        assert!(response.allow);
        assert_eq!(response.scope.as_deref(), Some("once"));
    }

    #[test]
    fn approval_shortcut_w_selects_workspace_scope() {
        let mut app = test_app();
        let (responder, mut receiver) = tokio::sync::oneshot::channel();
        app.approval = Some((
            ApprovalInfo {
                tool: "read_file".to_string(),
                reason: "测试".to_string(),
                level: "read".to_string(),
                ..Default::default()
            },
            responder,
        ));

        app.handle_key(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
            &test_runtime(),
        )
        .expect("应处理快捷键");

        let response = receiver.try_recv().expect("应收到决策");
        assert!(response.allow);
        assert_eq!(response.scope.as_deref(), Some("workspace"));
    }

    #[test]
    fn transcript_visible_lines_respects_scroll() {
        let mut app = test_app();
        for index in 0..20 {
            app.push_line(format!("line {index}"), default());
        }
        app.scroll = 0;
        assert_eq!(app.visible_lines(5).last().unwrap().0, "line 19");
        app.scroll = 5;
        assert_eq!(app.visible_lines(5).last().unwrap().0, "line 14");
    }

    #[test]
    fn parses_keybind_specs_and_builds_defaults() {
        let ctrl_c = parse_keybind("ctrl+c").unwrap();
        assert_eq!(ctrl_c.code, KeyCode::Char('c'));
        assert!(ctrl_c.modifiers.contains(KeyModifiers::CONTROL));
        assert_eq!(parse_keybind("f2").unwrap().code, KeyCode::F(2));
        assert_eq!(parse_keybind("tab").unwrap().code, KeyCode::Tab);
        assert!(parse_keybind("").is_none());
        assert!(parse_keybind("+").is_none());

        let mut configured = HashMap::new();
        configured.insert("toggle_mode".to_string(), "f2".to_string());
        let binds = build_keybinds(&configured);
        assert_eq!(binds.get("toggle_mode").unwrap().code, KeyCode::F(2));
        assert_eq!(binds.get("abort").unwrap().code, KeyCode::Char('c'));
        assert_eq!(format_key(binds.get("scroll_up").unwrap()), "pageup");
    }

    #[test]
    fn diff_panel_builds_from_file_diffs() {
        let diffs = vec![FileDiff {
            path: "a.txt".to_string(),
            before: Some("before".to_string()),
            after: Some("after".to_string()),
        }];
        let (lines, active) = build_diff_panel(&diffs);
        assert!(active);
        assert!(lines.iter().any(|(text, _)| text.contains("after")));
        assert!(lines.iter().any(|(text, _)| text.contains("before")));
        let (empty_lines, empty_active) = build_diff_panel(&[]);
        assert!(!empty_active);
        assert!(empty_lines[0].0.contains("没有未回滚"));
    }

    #[test]
    fn windows_release_and_repeat_events_do_not_double_input() {
        let mut app = test_app();
        let runtime = test_runtime();
        let press =
            KeyEvent::new_with_kind(KeyCode::Char('你'), KeyModifiers::NONE, KeyEventKind::Press);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('你'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        let repeat = KeyEvent::new_with_kind(
            KeyCode::Char('好'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        );

        app.handle_key(press, &runtime).unwrap();
        app.handle_key(release, &runtime).unwrap();
        app.handle_key(repeat, &runtime).unwrap();
        app.handle_key(
            KeyEvent::new_with_kind(KeyCode::Char('好'), KeyModifiers::NONE, KeyEventKind::Press),
            &runtime,
        )
        .unwrap();

        assert_eq!(app.input, "你好");
    }

    #[test]
    fn exit_command_requests_clean_shutdown_instead_of_process_exit() {
        let mut app = test_app();
        let runtime = test_runtime();

        app.handle_command("exit", &runtime).unwrap();

        assert!(app.should_exit);
        assert_eq!(app.status, "正在退出…");
    }

    #[test]
    fn disconnected_turn_channel_does_not_leave_app_running() {
        let mut app = test_app();
        let (tx, rx) = mpsc::channel();
        drop(tx);
        app.event_rx = Some(rx);
        app.running = true;

        app.drain_events();

        assert!(!app.running);
        assert!(app.event_rx.is_none());
        assert!(app
            .transcript
            .iter()
            .any(|(line, _)| line.contains("回合通道已断开")));
    }

    /// 任务 11 快照矩阵：TUI 渲染离线快照（TestBackend，无需真实终端）。
    fn snapshot_cells(terminal: &Terminal<ratatui::backend::TestBackend>) -> String {
        use unicode_width::UnicodeWidthStr;
        let buffer = terminal.backend().buffer();
        let area = buffer.area();
        let mut rows = Vec::new();
        for y in area.top()..area.bottom() {
            let mut row = String::new();
            let mut x = area.left();
            while x < area.right() {
                let symbol: &str = buffer[(x, y)].symbol();
                let width = symbol.width().max(1) as u16;
                row.push_str(symbol);
                x += width;
            }
            rows.push(row.trim_end().to_string());
        }
        rows.join("\n")
    }

    fn draw_to_buffer(app: &TuiApp, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        snapshot_cells(&terminal)
    }

    #[test]
    fn snapshot_idle_build_chat_shows_idle_chip_and_build_hint() {
        let app = test_app();
        let cells = draw_to_buffer(&app, 80, 24);
        assert!(cells.contains("○ 空闲"), "空闲芯片缺失");
        assert!(cells.contains("build"), "build 模式芯片缺失");
        assert!(cells.contains("输入（build）"), "build 输入提示缺失");
        assert!(cells.contains("就绪"), "状态行缺失");
        assert!(cells.contains("会话"), "会话面板标题缺失");
        let rows: Vec<&str> = cells.split('\n').collect();
        assert_eq!(rows.len(), 24, "80×24 快照应恰好 24 行");
        assert!(rows[0].contains("○ 空闲"), "模式芯片必须位于标题行");
        assert!(rows[0].contains("| build"), "build 芯片位于标题行");
        assert!(
            rows.iter().any(|r| r.contains("输入（build）")),
            "输入框标题独立成行"
        );
        assert!(
            rows[23].contains("Tab") && rows[23].contains("中止/退出"),
            "状态栏必须位于最后一行"
        );
    }

    #[test]
    fn snapshot_running_plan_shows_running_chip_plan_hint_and_streaming_marker() {
        let mut app = test_app();
        app.read_only = true;
        app.running = true;
        app.streaming = "正在生成…".to_string();
        let cells = draw_to_buffer(&app, 80, 24);
        assert!(cells.contains("● 运行中"), "运行中芯片缺失");
        assert!(cells.contains("plan"), "plan 模式芯片缺失");
        assert!(cells.contains("plan（只读）"), "只读输入提示缺失");
        assert!(cells.contains("▍正在生成…"), "流式指示符缺失");
    }

    #[test]
    fn snapshot_diff_panel_replaces_transcript_source() {
        let mut app = test_app();
        app.show_diff_panel = true;
        app.diff_view = vec![("+ 新增行".to_string(), green())];
        let cells = draw_to_buffer(&app, 80, 24);
        assert!(cells.contains("diff"), "diff 芯片缺失");
        assert!(cells.contains("新增行"), "diff 内容缺失");
        assert!(!cells.contains("新 增"), "不得出现宽字符续格伪影");
    }

    #[test]
    fn snapshot_wide_viewport_120x30_keeps_title_and_status_rows() {
        let mut app = test_app();
        app.streaming = "流式输出中…".to_string();
        let cells = draw_to_buffer(&app, 120, 30);
        let rows: Vec<&str> = cells.split('\n').collect();
        assert_eq!(rows.len(), 30, "120×30 快照应恰好 30 行");
        assert!(rows[0].contains("○ 空闲"), "标题行芯片在场");
        assert!(
            rows.iter().any(|r| r.contains("▍流式输出中…")),
            "流式指示符在场"
        );
        assert!(rows[29].contains("滚动"), "状态栏位于最后一行");
        assert!(rows.iter().all(|r| !r.ends_with(' ')), "行尾空白必须去除");
        assert!(rows[0].contains(" | "), "标题行内分隔空格必须保留");
    }

    #[test]
    fn snapshot_chinese_input_renders_without_cell_artifacts() {
        let mut app = test_app();
        app.input = "你好，世界！".to_string();
        let cells = draw_to_buffer(&app, 80, 24);
        assert!(
            cells.contains("你好，世界！"),
            "中文输入必须连续呈现，实际：{cells}"
        );
        assert!(!cells.contains("你 好"), "不得出现宽字符续格伪影");
    }

    #[test]
    fn snapshot_long_workspace_path_truncates_title_only() {
        let long_root = std::env::temp_dir().join(format!("owo-tui-长路径-{}", "目录".repeat(24)));
        let mut app = test_app();
        app.workspace = long_root.clone();
        let cells = draw_to_buffer(&app, 80, 24);
        let rows: Vec<&str> = cells.split('\n').collect();
        assert_eq!(rows.len(), 24);
        let title_display_width: usize = {
            use unicode_width::UnicodeWidthStr;
            rows[0].width()
        };
        assert!(
            title_display_width <= 80,
            "标题行显示宽度不得越界，实际 {title_display_width}"
        );
        assert!(cells.contains("输入（build）"), "长路径不得破坏输入框");
        assert!(rows[23].contains("Tab"), "长路径不得破坏状态栏");
    }
}
