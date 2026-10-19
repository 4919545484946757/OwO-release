// §12.3 CLI 拆分批次五：repl 交互域（自 main.rs 机械外移，零行为变化）。
// Repl 主循环：会话/审批/恢复/子命令；审批者与共享 stdin 经 crate::support 显式引用。

use crate::support::*;
use crate::ui_output::StreamPrinter;
use clap::Args;
use colored::Colorize;
use owo_agent_core::permissions::{Approver, AutoApprover};
use owo_agent_core::session::SessionStore;
use owo_agent_core::{
    discover_plugins, install_builtin_packages, save_trace, Agent, LearnRecorder, McpClient,
    McpServerConfig, PluginManifest, ProactiveEngine, Session, Settings, SituationStore,
    SkillRegistry, SqliteSessionStore, TraceRecord, TurnEvent, Whitelist,
};
use rustyline::error::ReadlineError;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Args)]
pub(crate) struct ReplArgs {
    #[arg(long, default_value = ".")]
    pub(crate) workspace: PathBuf,
    #[arg(long)]
    pub(crate) model: Option<String>,
    /// 初始 agent：build（默认）或 plan（只读）
    #[arg(long, default_value = "build")]
    pub(crate) agent: String,
    /// 自动允许所有审批（仅测试用）
    #[arg(long)]
    pub(crate) no_approval: bool,
    /// 覆盖数据目录（默认 %LOCALAPPDATA%\OwO\Agent 或 OWO_AGENT_DATA）
    #[arg(long)]
    pub(crate) data_dir: Option<PathBuf>,
    /// 使用旧本地 REPL（迁移期对照；默认走唯一 Daemon 客户端，不再本地建 Agent/SQLite/MCP）。
    #[arg(long)]
    pub(crate) local: bool,
}

mod handlers;

pub(crate) struct Repl {
    workspace: PathBuf,
    model: String,
    read_only: bool,
    no_approval: bool,
    data_root: PathBuf,
    store: SqliteSessionStore,
    session: Option<Session>,
    agent: Arc<Agent>,
    abort: Arc<AtomicBool>,
    stdin: SharedStdin,
    approvals: Arc<SessionApprovals>,
    goal: Option<GoalState>,
    mcp_configs: Vec<McpServerConfig>,
    mcp_clients: Vec<(String, Arc<tokio::sync::Mutex<McpClient>>)>,
    skills: SkillRegistry,
    settings: Settings,
    plugins: Vec<PluginManifest>,
    audit_flushed: usize,
    perception: SituationStore,
    learn: LearnRecorder,
    whitelist: Whitelist,
    proactive: ProactiveEngine,
}

impl Repl {
    pub(crate) async fn run(
        args: ReplArgs,
        permissions: crate::ui_output::PermissionsProfile,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // P1 §4.1：**默认**走唯一 Daemon 客户端（无本地 Agent/SQLite/MCP）；
        // `--local` 保留旧本地 REPL 一个发布周期，供未迁移命令对照。
        if !args.local {
            return crate::commands::repl_daemon::run(args, permissions).await;
        }
        let workspace = args.workspace.canonicalize()?;
        let settings = Settings::load(&workspace);
        apply_egress_setting(&settings);
        settings.apply_usage_env();
        settings.apply_reasoning_env();
        let model = resolve_model(args.model, settings.model.as_deref());
        let read_only = args.agent == "plan" || settings.read_only;
        let root = ensure_data_root(args.data_dir, &workspace);
        // 审查 N1：工作区信任门（与 serve/桌面同一数据面 `trust.json`）。
        let trust = owo_agent_core::trust::TrustStore::new(root.clone()).status(&workspace);
        let trust_gated = !trust.trusted && trust.gated;
        if trust_gated {
            eprintln!(
                "⚠ 工作区未信任：跳过工作区 hooks/插件/MCP/白名单（受限模式）。\n  原因：{}\n  确认可信后用桌面端或 POST /workspace/trust 授权。",
                trust.reasons.join("；")
            );
        }
        let store = SqliteSessionStore::open(&root.join("index.db"))?;
        let mut mcp_configs = load_mcp_configs(&root);
        if !trust_gated {
            for server in settings.mcp_servers.clone() {
                if !mcp_configs.iter().any(|config| config.name == server.name) {
                    mcp_configs.push(server);
                }
            }
        }
        let discovered_plugins = discover_plugins(&workspace, &root);
        let plugin_state =
            owo_agent_core::PluginStateStore::new(Some(root.join("plugin_state.json")));
        let enabled_plugins = if trust_gated {
            Vec::new()
        } else {
            owo_agent_core::plugin::discover_enabled_plugins(&workspace, &root, &plugin_state)
        };
        merge_plugin_mcp(&enabled_plugins, &mut mcp_configs);
        let plugins: Vec<PluginManifest> = discovered_plugins
            .into_iter()
            .map(|(_, manifest)| manifest)
            .collect();
        let mcp_clients = connect_mcp_clients(&mcp_configs).await;
        match install_builtin_packages(&builtin_skills_root(), &root, env!("CARGO_PKG_VERSION")) {
            Ok(report) => {
                for (name, reason) in &report.skipped {
                    eprintln!("内置技能 {name} 未安装：{reason}");
                }
            }
            Err(error) => eprintln!("内置技能安装失败：{error}"),
        }
        let mut skills = SkillRegistry::discover(&workspace, &root);
        apply_disabled_skills(&mut skills, &settings);
        let mut whitelist = Whitelist::default();
        if !trust_gated {
            for entry in settings.whitelist.clone() {
                whitelist.upsert(entry);
            }
        }
        let proactive = ProactiveEngine::new(settings.proactive.clone());
        let agent = Arc::new(build_agent_with_mcp(
            &workspace,
            &model,
            read_only,
            &mcp_clients,
            &skills,
            &settings.deny_commands,
        )?);
        // A2-1：本地 REPL 装载 settings.json 的 hooks（与 server 同一出口）；
        // 未信任工作区跳过（审查 N1）。
        if !trust_gated {
            agent.set_hooks(owo_agent_core::hooks::HookManager::from_configs(
                &settings.hooks,
            ));
        }
        // §11：--no-approval 弃用告警（兼容期显式映射 + 高风险提示）。
        if args.no_approval {
            eprintln!(
                "⚠ 已弃用：--no-approval 将在未来版本移除；兼容期等价 --permissions trusted（高风险：全部操作自动批准，含写/执行/联网）"
            );
        }
        let mut repl = Repl {
            workspace,
            model,
            read_only,
            no_approval: args.no_approval,
            data_root: root.clone(),
            store,
            session: None,
            agent,
            abort: Arc::new(AtomicBool::new(false)),
            stdin: SharedStdin::new(),
            approvals: Arc::new(SessionApprovals::new()),
            goal: None,
            mcp_configs,
            mcp_clients,
            skills,
            settings,
            plugins,
            audit_flushed: 0,
            perception: SituationStore::new(),
            learn: LearnRecorder::new(),
            whitelist,
            proactive,
        };

        println!(
            "{} {}（{}）",
            "OwO Agent".bold(),
            env!("CARGO_PKG_VERSION").cyan(),
            display_path(&repl.workspace).dimmed()
        );
        println!("输入 /help 查看命令；直接输入文字开始任务。");

        let history_path = root.join("history.txt");
        if std::io::stdin().is_terminal() {
            repl.run_terminal(&history_path).await?;
        } else {
            repl.run_piped().await?;
        }
        if let Some(session) = &repl.session {
            let _ = repl.store.save(session);
        }
        Ok(())
    }

    async fn run_terminal(
        &mut self,
        history_path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut rl = new_repl_editor()?;
        if let Ok(content) = std::fs::read_to_string(history_path) {
            for line in content.lines() {
                let _ = rl.add_history_entry(line);
            }
        }
        loop {
            // 纯文本提示串：rustyline 的宽度计算不剥离 ANSI，颜色由 ReplHelper::highlight_prompt 渲染。
            let prompt = repl_prompt(self.read_only);
            match rl.readline(&prompt) {
                Ok(line) => {
                    let line = normalize_input_line(&line);
                    if line.is_empty() {
                        continue;
                    }
                    let _ = rl.add_history_entry(line.clone());
                    match self.handle_line(&line).await {
                        Ok(true) => break,
                        Ok(false) => continue,
                        Err(error) => eprintln!("错误：{error}"),
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("{}", "（/exit 退出；已取消当前输入）".dimmed());
                    continue;
                }
                Err(ReadlineError::Eof) => break,
                Err(error) => {
                    eprintln!("输入错误：{error}");
                    break;
                }
            }
        }
        let history: Vec<String> = rl.history().iter().cloned().collect();
        let _ = std::fs::write(history_path, history.join("\n"));
        Ok(())
    }

    async fn run_piped(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut line = String::new();
        loop {
            print!(
                "{} ",
                if self.read_only {
                    "plan ❯".yellow()
                } else {
                    "build ❯".green()
                }
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();
            line.clear();
            let read = self.stdin.read_line(&mut line).await?;
            if read == 0 {
                break;
            }
            let line = normalize_input_line(&line);
            if line.is_empty() {
                continue;
            }
            match self.handle_line(&line).await {
                Ok(true) => break,
                Ok(false) => continue,
                Err(error) => eprintln!("错误：{error}"),
            }
        }
        Ok(())
    }

    async fn handle_line(&mut self, line: &str) -> Result<bool, Box<dyn std::error::Error>> {
        if let Some(query) = line.strip_prefix("@explore ") {
            self.run_at_subagent(query, true, false).await?;
            return Ok(false);
        }
        if let Some(task) = line.strip_prefix("@subagent ") {
            // 命令执行显式 opt-in：`@subagent --allow-commands <任务>`（缺省拒绝）。
            let (allow_commands, task) = match task.strip_prefix("--allow-commands ") {
                Some(rest) => (true, rest),
                None => (false, task),
            };
            self.run_at_subagent(task, false, allow_commands).await?;
            return Ok(false);
        }
        if let Some(command) = line.strip_prefix('/') {
            let mut parts = command.split_whitespace();
            match parts.next().unwrap_or_default() {
                "help" => crate::print_help(),
                "exit" | "quit" => return Ok(true),
                "new" => self.new_session(parts.next()).await?,
                "sessions" => self.list_sessions(),
                "resume" => {
                    let id = parts
                        .next()
                        .ok_or("用法：/resume <会话ID>（/sessions 查看）")?;
                    self.resume(id).await?;
                }
                "model" => match parts.next() {
                    Some(model) => {
                        self.model = model.to_string();
                        self.rebuild_agent()?;
                        println!("{} {}", "模型已切换：".green(), self.model);
                    }
                    None => println!("当前模型：{}", self.model),
                },
                "plan" => {
                    self.set_mode(true)?;
                }
                "build" => {
                    self.set_mode(false)?;
                }
                "agent" => match parts.next() {
                    Some("plan") => self.set_mode(true)?,
                    Some("build") => self.set_mode(false)?,
                    Some(other) => println!("未知 agent：{other}（build / plan）"),
                    None => println!(
                        "当前 agent：{}",
                        if self.read_only {
                            "plan（只读）"
                        } else {
                            "build"
                        }
                    ),
                },
                "diff" => self.show_diff(),
                "undo" | "revert" => self.undo().await?,
                "mcp" => self.handle_mcp(command).await?,
                "skills" => match parts.next() {
                    Some("reload") => {
                        self.reload_skills()?;
                    }
                    _ => self.list_skills(),
                },
                "fork" => self.fork_session(parts.next()).await?,
                "rewind" => {
                    let keep = parts.next().ok_or("用法：/rewind <保留消息数>")?;
                    self.rewind_session(keep).await?;
                }
                "redo" => self.redo_session().await?,
                "undo-msg" => self.undo_message(parts.next()).await?,
                "redo-msg" => self.redo_message().await?,
                "tree" => self.show_tree(),
                "share" => self.share_session(parts.next())?,
                "traces" => self.list_traces(),
                "trace" => self.show_trace(parts.next())?,
                "settings" => self.show_settings(),
                "plugins" => self.list_plugins(),
                "whitelist" => self.show_whitelist(),
                "perception" => self.show_perception(),
                "learn" => self.handle_learn(command)?,
                "proactive" => self.handle_proactive(command)?,
                "status" => self.show_status(),
                "permissions" => self.handle_permissions(command)?,
                "approvals" => self.handle_approvals(parts.next()),
                "goal" => {
                    let rest = command.strip_prefix("goal").unwrap_or("").trim();
                    self.handle_goal(rest).await?;
                }
                "todo" => self.show_todos(),
                "audit" => self.show_audit(),
                "init" => {
                    let target = self.workspace.join("AGENTS.md");
                    if target.exists() {
                        println!("{} {}", "AGENTS.md 已存在：".yellow(), target.display());
                    } else {
                        std::fs::write(&target, AGENTS_TEMPLATE)?;
                        println!("{} {}", "已生成".green(), target.display());
                    }
                }
                "abort" => {
                    self.abort.store(true, Ordering::Relaxed);
                    println!("已请求中止当前回合");
                }
                "clear" => print!("\x1b[2J\x1b[1;1H"),
                "compact" => self.compact().await?,
                "review" => self.run_turn(&review_prompt(parts.next())).await?,
                "mention" => mention_path(&self.workspace, parts.next()),
                "history" => print_history(&self.data_root, parts.next()),
                "editor" => self.editor_input().await?,
                "login" => print_login(),
                "logout" => print_logout(),
                "debug" => self.show_debug(),
                other => println!("未知命令：/{other}（/help 查看全部）"),
            }
            return Ok(false);
        }
        let prompt = self.with_goal_context(line);
        self.run_turn(&prompt).await?;
        Ok(false)
    }

    /// 目标激活且未完成时，把目标附到每次输入前（目标推进期间用户插话也带目标上下文）。
    fn with_goal_context(&self, line: &str) -> String {
        goal_context_prompt(self.goal.as_ref(), line)
    }

    /// `/approvals [clear]`：查看/清除本会话的「总是允许」工具记忆。
    fn handle_approvals(&self, action: Option<&str>) {
        match action {
            Some("clear") | Some("reset") => {
                self.approvals.clear();
                println!("{}", "已清除本会话的「总是允许」记忆".green());
            }
            _ => {
                let tools = self.approvals.list();
                if tools.is_empty() {
                    println!("本会话尚无「总是允许」的工具（审批时按 s 添加）");
                } else {
                    println!("{}", "本会话「总是允许」的工具：".bold());
                    for tool in tools {
                        // 与审批卡同口径：MCP/插件工具给可读名（注册名仍可见，便于核对）。
                        let label = crate::ui_output::tool_display_name(&tool);
                        if label == tool {
                            println!("  • {tool}");
                        } else {
                            println!("  • {label}（{tool}）");
                        }
                    }
                    println!("（/approvals clear 清除）");
                }
            }
        }
    }

    /// `/todo`：查看会话任务清单（`todo` 工具维护）。
    fn show_todos(&self) {
        let todos = self
            .session
            .as_ref()
            .map(|session| session.todos.as_slice())
            .unwrap_or(&[]);
        if todos.is_empty() {
            println!("（任务清单为空；模型调用 todo 工具后会出现在这里）");
            return;
        }
        println!("{}", "任务清单：".bold());
        for todo in todos {
            let mark = match todo.status.as_str() {
                "completed" => "✔".green().to_string(),
                "in_progress" => "▶".yellow().to_string(),
                _ => "○".dimmed().to_string(),
            };
            println!("  {mark} {}", todo.content);
        }
    }

    /// `/goal [目标|status|clear]`：目标模式——模型标记完成且宿主裁决允许交付时才停止。
    async fn handle_goal(&mut self, arg: &str) -> Result<(), Box<dyn std::error::Error>> {
        match arg {
            "" | "status" => {
                match &self.goal {
                    Some(goal) => println!(
                        "目标：{}\n  轮次：{}/{}  状态：{}",
                        goal.objective,
                        goal.iterations,
                        crate::support::goal_iteration_label(goal_max_iterations()),
                        if goal.done { "已完成" } else { "推进中" }
                    ),
                    None => println!("（未设定目标；用法：/goal <目标描述>）"),
                }
                Ok(())
            }
            "clear" | "stop" => {
                self.goal = None;
                println!("{}", "已清除目标".green());
                Ok(())
            }
            objective => {
                let max = goal_max_iterations();
                self.goal = Some(GoalState {
                    objective: objective.to_string(),
                    iterations: 0,
                    done: false,
                });
                println!(
                    "{}（轮次上限：{}；/goal clear 停止）",
                    format!("目标已设定：{objective}").green(),
                    crate::support::goal_iteration_label(max)
                );
                let mut previous_status = None;
                loop {
                    if self.abort.load(Ordering::Relaxed) {
                        println!("{}", "（目标推进已中止）".yellow());
                        break;
                    }
                    let Some(goal) = self.goal.as_ref() else {
                        break;
                    };
                    if goal.done {
                        break;
                    }
                    if crate::support::goal_iteration_limit_reached(goal.iterations, max) {
                        println!(
                            "{}",
                            format!("已达最大迭代 {max}，目标未标记完成（/goal status 查看）")
                                .yellow()
                        );
                        break;
                    }
                    let iteration = goal.iterations + 1;
                    let prompt = if iteration == 1 {
                        goal_first_prompt(objective)
                    } else if let Some(status) = previous_status {
                        crate::support::goal_continue_prompt_after_status(
                            objective, iteration, status,
                        )
                    } else {
                        goal_continue_prompt(objective, iteration)
                    };
                    let limit = crate::support::goal_iteration_label(max);
                    println!("{}", format!("── 目标推进 {iteration}/{limit} ──").bold());
                    let turn = self.run_turn_capture(&prompt).await?;
                    if let Some(goal) = self.goal.as_mut() {
                        goal.iterations = iteration;
                    }
                    if turn.failed {
                        println!(
                            "{} 回合失败，目标未完成（/goal status 查看；修复问题后可重新提交）",
                            "!".red()
                        );
                        break;
                    }
                    if turn.completion_status == owo_agent_protocol::CompletionStatusV1::Aborted {
                        println!("{} 目标推进已中止", "!".yellow());
                        break;
                    }
                    if crate::support::goal_claim_is_accepted(&turn) {
                        if let Some(goal) = self.goal.as_mut() {
                            goal.done = true;
                        }
                        println!("{} 目标完成（第 {iteration} 轮）", "✓".green());
                        break;
                    }
                    if turn
                        .final_text
                        .as_deref()
                        .is_some_and(crate::support::goal_is_done)
                    {
                        println!(
                            "{} 模型报告完成，但宿主状态为 {:?}，继续推进验收",
                            "!".yellow(),
                            turn.completion_status
                        );
                    }
                    previous_status = Some(turn.completion_status);
                }
                Ok(())
            }
        }
    }

    /// `/compact`：显式压缩会话历史（对齐 Codex）。
    async fn compact(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.session.is_none() {
            self.new_session(None).await?;
        }
        let mut session = self.session.take().expect("session just created");
        let before = owo_agent_core::estimate_tokens(&session.messages);
        let budget = self.agent.config().token_budget;
        let suffix = if before > budget {
            "（超预算）".yellow().to_string()
        } else {
            String::new()
        };
        println!("上下文：{before} / {budget} tokens{suffix}");
        let summary = self.agent.compact_session(&mut session).await?;
        let after = owo_agent_core::estimate_tokens(&session.messages);
        self.session = Some(session);
        if let Some(session) = &self.session {
            self.store.save(session)?;
        }
        match summary {
            Some(summary) => {
                println!("{} 已压缩：{before} → {after} tokens", "✓".green());
                println!("{}", "── 摘要 ──".bold());
                for line in summary.lines() {
                    println!("{}", crate::markdown::render_block_line(line));
                }
            }
            None => println!("{}", "未压缩（历史不足或模型未产出摘要）".yellow()),
        }
        Ok(())
    }

    /// `/editor`：用 `$VISUAL`/`$EDITOR` 编辑多行提示后作为一次输入。
    async fn editor_input(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let editor = std::env::var("VISUAL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            });
        let Some(editor) = editor else {
            println!(
                "{}",
                "未设置 $VISUAL/$EDITOR；请先设置外部编辑器（如 set EDITOR=notepad）".yellow()
            );
            return Ok(());
        };
        let path = std::env::temp_dir().join(format!("owo-prompt-{}.md", std::process::id()));
        std::fs::write(&path, "")?;
        let mut cmd = editor.split_whitespace();
        let program = cmd.next().unwrap_or_default().to_string();
        let status = std::process::Command::new(&program)
            .args(cmd)
            .arg(&path)
            .status();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        match status {
            Ok(status) if status.success() && !text.trim().is_empty() => {
                self.run_turn(text.trim()).await?;
            }
            Ok(_) => println!("（空输入，已取消）"),
            Err(error) => println!("{} 启动编辑器失败：{error}", "✘".red()),
        }
        Ok(())
    }

    /// `/debug`：诊断信息。
    fn show_debug(&self) {
        println!("[debug] 版本：{}", env!("CARGO_PKG_VERSION"));
        println!("[debug] 工作区：{}", self.workspace.display());
        println!("[debug] 数据目录：{}", self.data_root.display());
        println!(
            "[debug] 模式：{}",
            if self.read_only {
                "plan（只读）"
            } else {
                "build"
            }
        );
        println!("[debug] 模型：{}", self.model);
        println!(
            "[debug] 会话：{}",
            self.session
                .as_ref()
                .map(|s| s.id.as_str())
                .unwrap_or("（无）")
        );
        println!(
            "[debug] MCP：{} 个；插件：{} 个",
            self.mcp_configs.len(),
            self.plugins.len()
        );
        println!(
            "[debug] trace 目录：{}",
            self.data_root.join("traces").display()
        );
    }

    fn set_mode(&mut self, read_only: bool) -> Result<(), Box<dyn std::error::Error>> {
        self.read_only = read_only;
        self.rebuild_agent()?;
        println!(
            "{}",
            if read_only {
                "已切换到 plan 模式（只读，写/执行将被拒绝）".yellow()
            } else {
                "已切换到 build 模式（写/执行需要审批）".green()
            }
        );
        Ok(())
    }

    fn rebuild_agent(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.audit_flushed = 0;
        self.agent = Arc::new(build_agent_with_mcp(
            &self.workspace,
            &self.model,
            self.read_only,
            &self.mcp_clients,
            &self.skills,
            &self.settings.deny_commands,
        )?);
        Ok(())
    }

    async fn run_turn(&mut self, prompt: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.run_turn_capture(prompt).await.map(|_| ())
    }

    /// 执行一回合并返回最终文本与宿主完成状态（`/goal` 据此闭合任务）。
    async fn run_turn_capture(
        &mut self,
        prompt: &str,
    ) -> Result<crate::support::GoalTurnResult, Box<dyn std::error::Error>> {
        if self.session.is_none() {
            self.new_session(None).await?;
        }
        let mut session = self.session.take().expect("session just created");
        let prompt = prompt.to_string();
        self.abort.store(false, Ordering::Relaxed);
        let agent = Arc::clone(&self.agent);
        let abort = Arc::clone(&self.abort);
        let approver: Arc<dyn Approver> = if self.no_approval {
            Arc::new(AutoApprover { allow: true })
        } else {
            Arc::new(ConsoleApprover {
                stdin: self.stdin.clone(),
                approvals: Arc::clone(&self.approvals),
            })
        };

        println!("{} {}", "▶".green(), prompt.dimmed());
        let mut task = tokio::spawn(async move {
            let mut printer = StreamPrinter::new();
            let mut on_event = |event: &TurnEvent| printer.print_turn(event);
            let outcome = agent
                .run_turn(
                    &mut session,
                    &prompt,
                    approver.as_ref(),
                    &abort,
                    &mut on_event,
                )
                .await;
            (outcome, session)
        });

        // Ctrl+C 仅在当前回合内监听：旧实现每回合 spawn 一个永不结束的监听任务
        // （任务泄漏），且回合结束后按 Ctrl+C 仍会误报“正在中止”。改为 select!：
        // 中止后等待回合协作收尾，再走统一的保存/审计/摘要路径。
        let (outcome, session) = tokio::select! {
            joined = &mut task => {
                joined.map_err(|error| std::io::Error::other(format!("回合任务失败：{error}")))?
            }
            _ = tokio::signal::ctrl_c() => {
                self.abort.store(true, Ordering::Relaxed);
                println!("{}", "（Ctrl+C：正在中止当前回合…）".yellow());
                task.await
                    .map_err(|error| std::io::Error::other(format!("回合任务失败：{error}")))?
            }
        };
        self.session = Some(session);
        if let Some(session) = &self.session {
            self.store.save(session)?;
        }
        self.flush_audit();
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("{} {error}", "回合失败：".red());
                println!("（会话已保存；/status 查看状态，/diff 查看改动，/undo 回滚）");
                let aborted = self.abort.load(Ordering::Relaxed);
                return Ok(crate::support::GoalTurnResult {
                    final_text: None,
                    completion_status: if aborted {
                        owo_agent_protocol::CompletionStatusV1::Aborted
                    } else {
                        owo_agent_protocol::CompletionStatusV1::Blocked
                    },
                    failed: !aborted,
                });
            }
        };
        let trace =
            TraceRecord::from_outcome(self.session.as_ref().expect("session saved"), &outcome);
        if let Ok(path) = save_trace(&self.data_root.join("traces"), &trace) {
            println!("[trace] {}", display_path(&path));
        }
        println!(
            "{} 工具步数 {}，审计 {} 条，改动 {} 个文件（/diff 查看，/undo 回滚）",
            "✓".green(),
            outcome.steps,
            self.agent
                .audit_log()
                .lock()
                .map(|log| log.entries.len())
                .unwrap_or(0),
            self.session.as_ref().map(|s| s.diff().len()).unwrap_or(0),
        );
        Ok(crate::support::GoalTurnResult {
            final_text: outcome.final_text.clone(),
            completion_status: outcome.completion_status,
            failed: false,
        })
    }

    fn flush_audit(&mut self) {
        let audit_entries = self
            .agent
            .audit_log()
            .lock()
            .map(|guard| guard.entries.clone())
            .unwrap_or_default();
        if audit_entries.len() <= self.audit_flushed {
            return;
        }
        if self
            .store
            .append_audit(&audit_entries[self.audit_flushed..])
            .is_ok()
        {
            self.audit_flushed = audit_entries.len();
        }
    }
}
