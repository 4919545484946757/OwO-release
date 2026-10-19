//! P1 §4.1：Daemon 后端交互 REPL（`owo-agent repl --daemon`）。
//!
//! 与旧 `Repl` 的根本区别：本模块**不**打开 SQLite、不构造 Agent、不连 MCP——
//! 会话/回合/权限/工具/审计全部由 Daemon 持有，CLI 只消费事件流。这是"唯一运行时"
//! 在交互路径上的第一步；`--local` 保留旧实现用于命令迁移期的功能对照。
//!
//! 已迁移命令：help/exit/new/sessions/symbols/resume/model/diff/hunks/undo/revert/rewind/redo/fork/
//! rename/archive/pin/abort/status/audit/skills/permissions/settings/traces/plan/build/clear。
//! 其余命令在 daemon 模式下给出明确提示（`--local` 使用旧 REPL）。

use crate::support::{ensure_daemon_client, ensure_data_root};
use crate::ui_output::{
    parse_approval_response, print_permission_card, PermissionCard, StreamPrinter,
};
use colored::Colorize;
use owo_agent_protocol::{PermissionResponse, SseEvent};
use rustyline::error::ReadlineError;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[path = "repl_daemon/team.rs"]
pub(crate) mod team;
pub(crate) use team::*;

const GRANT_REVOKE_PATH: &str = "/permissions/grants/revoke";

struct TurnDoneGuard(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for TurnDoneGuard {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

pub(crate) async fn run(
    args: super::repl::ReplArgs,
    permissions: crate::ui_output::PermissionsProfile,
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = args.workspace.canonicalize()?;
    let root = ensure_data_root(args.data_dir.clone(), &workspace);
    let client = ensure_daemon_client(&root, &workspace).await?;
    let mut repl = DaemonRepl {
        client,
        workspace,
        data_root: root,
        session: None,
        model: args.model.clone(),
        read_only: args.agent == "plan"
            || matches!(permissions, crate::ui_output::PermissionsProfile::ReadOnly),
        read_only_locked: matches!(permissions, crate::ui_output::PermissionsProfile::ReadOnly),
        no_approval: args.no_approval
            || matches!(permissions, crate::ui_output::PermissionsProfile::Trusted),
        abort: Arc::new(AtomicBool::new(false)),
        goal: None,
        hinted_goal: None,
        team_id: None,
    };
    println!(
        "{} {}（daemon 模式 · {}）",
        "OwO Agent".bold(),
        env!("CARGO_PKG_VERSION").cyan(),
        repl.workspace.display()
    );
    println!("输入 /help 查看命令；直接输入文字开始任务。");
    if std::io::stdin().is_terminal() {
        repl.run_terminal().await?;
    } else {
        repl.run_piped().await?;
    }
    Ok(())
}

struct DaemonRepl {
    client: owo_agent_client::AgentClient,
    workspace: PathBuf,
    data_root: PathBuf,
    session: Option<String>,
    model: Option<String>,
    read_only: bool,
    read_only_locked: bool,
    no_approval: bool,
    abort: Arc<AtomicBool>,
    goal: Option<crate::support::GoalState>,
    /// 已提示过的"模型登记目标"（同一目标只提示一次）。
    hinted_goal: Option<String>,
    /// 当前团队（`/team` 目标；status/steer/diff 缺省作用于它）。
    team_id: Option<String>,
}

impl DaemonRepl {
    fn prompt(&self) -> String {
        // 纯文本提示串：rustyline 的宽度计算不剥离 ANSI，颜色由 ReplHelper::highlight_prompt 渲染。
        crate::support::repl_prompt(self.read_only)
    }

    async fn run_terminal(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut editor = crate::support::new_repl_editor()?;
        loop {
            match editor.readline(&self.prompt()) {
                Ok(line) => {
                    let line = crate::support::normalize_input_line(&line);
                    if line.is_empty() {
                        continue;
                    }
                    let _ = editor.add_history_entry(line.clone());
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
        Ok(())
    }

    async fn run_piped(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        let mut line = String::new();
        loop {
            print!("{}", self.prompt());
            let _ = std::io::stdout().flush();
            line.clear();
            if std::io::stdin().read_line(&mut line)? == 0 {
                break;
            }
            let line = crate::support::normalize_input_line(&line);
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
        if line.starts_with("@explore ") || line.starts_with("@subagent ") {
            println!(
                "{}",
                "子代理命令尚未迁移到 daemon 模式（用 --local）".yellow()
            );
            return Ok(false);
        }
        let Some(command) = line.strip_prefix('/') else {
            let prompt = self.with_goal_context(line);
            self.run_turn(&prompt).await?;
            return Ok(false);
        };
        let mut parts = command.split_whitespace();
        match parts.next().unwrap_or_default() {
            "help" => print_help(),
            "exit" | "quit" => return Ok(true),
            "new" => self.new_session(parts.next()).await?,
            "sessions" => self.list_sessions().await?,
            "resume" => {
                let id = parts
                    .next()
                    .ok_or("用法：/resume <会话ID>（/sessions 查看）")?;
                self.resume(id).await?;
            }
            "model" => match parts.next() {
                Some(model) => {
                    self.model = Some(model.to_string());
                    if let Some(id) = self.session.clone() {
                        self.client.session_set_model(&id, Some(model)).await?;
                    }
                    println!("{} {}", "模型已切换：".green(), model);
                }
                None => println!("当前模型：{}", self.model.as_deref().unwrap_or("（默认）")),
            },
            "plan" => {
                self.read_only = true;
                println!("{}", "已切换到 plan 模式（只读）".yellow());
            }
            "build" => {
                if self.read_only_locked {
                    println!("本次 CLI 强制只读，不能切换到 build");
                    return Ok(false);
                }
                self.read_only = false;
                println!("{}", "已切换到 build 模式".green());
            }
            "diff" => self.show_diff().await?,
            // 逐 hunk 审阅（服务端 /hunks 与 /hunks/apply；与 CLI `review --session` 同源）。
            "hunks" => self.handle_hunks(parts).await?,
            // 符号级检索（服务端 /symbols；与 find_symbol 工具、CLI `symbols` 同内核）。
            "symbols" => self.handle_symbols(parts).await?,
            "undo" | "revert" => self.revert().await?,
            "rewind" => {
                let keep = parts.next().ok_or("用法：/rewind <保留消息数>")?;
                self.rewind(keep).await?;
            }
            "redo" => self.redo().await?,
            "fork" => self.fork(parts.next()).await?,
            "rename" => self.rename(parts.next()).await?,
            "archive" => self.archive(true).await?,
            "pin" => self.pin(true).await?,
            "abort" => {
                self.abort.store(true, Ordering::Relaxed);
                if let Some(id) = self.session.clone() {
                    let _ = self.client.cancel_turn(&id).await;
                }
                println!("已请求中止当前回合");
            }
            "status" => self.show_status(),
            "trust" => self.handle_trust(parts.next()).await?,
            "audit" => self.print_json("审计", "/audit").await,
            "skills" => match parts.next() {
                Some("health") => self.print_json("技能健康", "/skills/health").await,
                Some("reload") => println!(
                    "{}",
                    "技能热重载由 Daemon 持有；请重启 Daemon（--local 为旧行为）".yellow()
                ),
                _ => self.print_json("技能", "/skills").await,
            },
            "permissions" => match parts.next() {
                None | Some("overview") => {
                    self.print_json("权限", "/permissions/overview").await
                }
                Some("set") => {
                    let profile = parts.next().ok_or(
                        "用法：/permissions set <read_only|workspace|auto_review|full_access|unrestricted|custom> [--yes]",
                    )?;
                    let yes = parts.any(|part| part == "--yes");
                    self.set_permission_profile(profile, yes).await?;
                }
                Some("status") => self.print_json("权限", "/permissions").await,
                Some("revoke") => {
                    let grant_id = parts.next().ok_or("用法：/permissions revoke <授权ID>")?;
                    self.revoke_permission_grant(grant_id).await?;
                }
                Some(other) => println!(
                    "{}",
                    format!("未知权限子命令：{other}（用法：/permissions [overview|status] | set <档位> | revoke <授权ID>）").yellow()
                ),
            },
            "settings" => self.print_json("设置", "/settings").await,
            "traces" => self.print_json("轨迹", "/traces").await,
            "trace" => {
                let index = parts.next().ok_or("用法：/trace <序号>")?;
                self.print_json("轨迹", &format!("/traces/{index}")).await
            }
            "tree" => {
                let id = self.current_session().await?;
                self.print_json("会话树", &format!("/session/{id}/children"))
                    .await
            }
            "mcp" => match parts.next() {
                Some(sub) => println!(
                    "{}",
                    format!("/mcp {sub} 需 POST（迁移中）；/mcp 列出用 /mcp").yellow()
                ),
                None => self.print_json("MCP", "/mcp").await,
            },
            "plugins" => self.print_json("插件", "/plugins").await,
            "whitelist" => self.print_json("白名单", "/whitelist").await,
            "perception" => self.print_json("情景", "/perception/events").await,
            "learn" => match parts.next().unwrap_or("status") {
                "status" => self.print_json("学习", "/learn/status").await,
                other => println!(
                    "{}",
                    format!("/learn {other} 尚未迁移到 daemon 模式（用 --local）").yellow()
                ),
            },
            "capabilities" => self.print_json("能力", "/capabilities").await,
            "proactive" => println!(
                "{}",
                "/proactive 尚未迁移到 daemon 模式（用 --local）".yellow()
            ),
            "share" | "export" => {
                println!("{}", "/share 尚未迁移到 daemon 模式（用 --local）".yellow())
            }
            "init" => {
                let target = self.workspace.join("AGENTS.md");
                if target.exists() {
                    println!("{} {}", "AGENTS.md 已存在：".yellow(), target.display());
                } else {
                    std::fs::write(&target, crate::support::AGENTS_TEMPLATE)?;
                    println!("{} {}", "已生成".green(), target.display());
                }
            }
            "clear" => print!("\x1b[2J\x1b[1;1H"),
            "compact" => self.compact().await?,
            "review" => self.review(parts.next()).await?,
            "mention" => crate::support::mention_path(&self.workspace, parts.next()),
            "history" => crate::support::print_history(&self.data_root, parts.next()),
            "editor" => self.editor_input().await?,
            "login" => crate::support::print_login(),
            "logout" => crate::support::print_logout(),
            "debug" => self.show_debug(),
            "goal" => {
                let rest = command.strip_prefix("goal").unwrap_or("").trim();
                self.handle_goal(rest).await?;
            }
            "team" => self.handle_team(parts).await?,
            "todo" => self.show_todos().await?,
            other => println!(
                "{}",
                format!("命令 /{other} 尚未迁移到 daemon 模式（用 --local 使用旧 REPL）").yellow()
            ),
        }
        Ok(false)
    }

    async fn new_session(&mut self, model: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let model = model.map(str::to_string).or_else(|| self.model.clone());
        let workspace = self.workspace.to_string_lossy().to_string();
        let session = self
            .client
            .create_session_with_model(&workspace, model)
            .await?;
        println!("{} {}", "新会话：".green(), session.id);
        self.session = Some(session.id);
        // 新会话没有目标（旧目标不跨会话残留）。
        self.goal = None;
        Ok(())
    }

    async fn list_sessions(&self) -> Result<(), Box<dyn std::error::Error>> {
        let sessions = self.client.list_sessions().await?;
        if sessions.is_empty() {
            println!("（无会话）");
        }
        for session in sessions {
            let title = session.title.unwrap_or_default();
            println!(
                "  {}{}  {}  {}",
                session.id,
                if session.pinned { " 📌" } else { "" },
                session.model,
                title
            );
        }
        Ok(())
    }

    async fn resume(&mut self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let session = self.client.get_session(id).await?;
        println!("{} {}", "已恢复会话：".green(), session.id);
        let workspace = PathBuf::from(&session.workspace).canonicalize()?;
        self.workspace = workspace;
        self.model = Some(session.model);
        self.session = Some(session.id);
        // 目标状态随会话落库（migration v10）：恢复后不丢目标与已推进轮次。
        self.goal = session.goal.map(|goal| crate::support::GoalState {
            objective: goal.objective,
            iterations: goal.iterations,
            done: goal.done,
        });
        if let Some(goal) = self.goal.as_ref() {
            println!(
                "{} {}（{}/{}，{}）",
                "已恢复目标：".yellow(),
                goal.objective,
                goal.iterations,
                crate::support::goal_iteration_label(crate::support::goal_max_iterations()),
                if goal.done { "已完成" } else { "推进中" }
            );
        }
        Ok(())
    }

    /// 目标状态写入会话（daemon 为权威）：失败只告警，不打断本地目标循环。
    async fn persist_goal(&self, action: &str, objective: Option<&str>) {
        let Some(id) = self.session.clone() else {
            return;
        };
        if let Err(error) = self.client.session_goal(&id, action, objective).await {
            eprintln!("（目标状态未写入会话：{error}）");
        }
    }

    async fn current_session(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        if self.session.is_none() {
            self.new_session(None).await?;
        }
        Ok(self.session.clone().expect("session just created"))
    }

    async fn show_diff(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let id = self.current_session().await?;
        let diffs = self.client.session_diff(&id).await?;
        if diffs.is_empty() {
            println!("（无改动）");
        }
        for diff in diffs {
            println!("  M {}", diff.path);
        }
        Ok(())
    }

    async fn revert(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let id = self.current_session().await?;
        self.client.session_revert(&id).await?;
        println!("{}", "已回滚本次会话全部写操作".green());
        Ok(())
    }

    async fn rewind(&mut self, keep: &str) -> Result<(), Box<dyn std::error::Error>> {
        let keep: usize = keep.parse().map_err(|_| "保留消息数必须是整数")?;
        let id = self.current_session().await?;
        self.client.session_rewind(&id, keep).await?;
        println!("已回退到保留 {keep} 条消息");
        Ok(())
    }

    async fn redo(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let id = self.current_session().await?;
        self.client.session_redo(&id).await?;
        println!("已恢复最近一次 rewind");
        Ok(())
    }

    async fn fork(&mut self, index: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let index: usize = index
            .unwrap_or("0")
            .parse()
            .map_err(|_| "消息序号必须是整数")?;
        let id = self.current_session().await?;
        let child = self.client.session_fork(&id, index).await?;
        println!("{} {}", "子会话：".green(), child.id);
        self.session = Some(child.id);
        // 分支继承父目标（core fork 语义）；本地状态与之一致。
        self.goal = child.goal.map(|goal| crate::support::GoalState {
            objective: goal.objective,
            iterations: goal.iterations,
            done: goal.done,
        });
        Ok(())
    }

    async fn rename(&mut self, title: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let title = title.ok_or("用法：/rename <标题>")?;
        let id = self.current_session().await?;
        self.client.session_rename(&id, title).await?;
        println!("已重命名会话");
        Ok(())
    }

    async fn archive(&mut self, archived: bool) -> Result<(), Box<dyn std::error::Error>> {
        let id = self.current_session().await?;
        self.client.session_archive(&id, archived).await?;
        println!("会话已归档");
        Ok(())
    }

    async fn pin(&mut self, pinned: bool) -> Result<(), Box<dyn std::error::Error>> {
        let id = self.current_session().await?;
        self.client.session_pin(&id, pinned).await?;
        println!("会话已置顶");
        Ok(())
    }

    async fn print_json(&self, label: &str, path: &str) {
        match self.client.get_json::<serde_json::Value>(path).await {
            Ok(value) => println!(
                "[{label}] {}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            ),
            Err(error) => println!("[{label}] 读取失败：{error}"),
        }
    }

    /// `/permissions set <profile>`：切换权限档位（高风险档位需确认）。
    async fn set_permission_profile(
        &self,
        profile: &str,
        allow_yes_flag: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !crate::support::confirm_high_risk_profile(profile, allow_yes_flag) {
            println!(
                "{}",
                "已取消（管道模式切换 unrestricted 需显式追加 --yes）".yellow()
            );
            return Ok(());
        }
        let result: serde_json::Value = self
            .client
            .post_json("/permissions", &serde_json::json!({ "profile": profile }))
            .await?;
        let applied = result
            .get("profile")
            .and_then(|value| value.as_str())
            .unwrap_or(profile);
        println!("{} 已切换权限档位：{applied}", "✓".green());
        Ok(())
    }

    async fn revoke_permission_grant(
        &self,
        grant_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let result: serde_json::Value = self
            .client
            .post_json(GRANT_REVOKE_PATH, &grant_revoke_payload(grant_id))
            .await?;
        println!("{}", format!("已撤销授权：{result}").green());
        self.print_json("权限", "/permissions/overview").await;
        Ok(())
    }

    /// `/compact`：显示上下文占用并触发服务端压缩（对齐 Codex）。
    async fn compact(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let id = self.current_session().await?;
        if let Ok(value) = self
            .client
            .get_json::<serde_json::Value>(&format!("/session/{id}/context"))
            .await
        {
            let tokens = value
                .get("estimated_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let budget = value
                .get("token_budget")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let over = value
                .get("over_budget")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let suffix = if over {
                "（超预算）".yellow().to_string()
            } else {
                String::new()
            };
            println!("上下文：{tokens} / {budget} tokens{suffix}");
        }
        let result = self
            .client
            .post_empty::<serde_json::Value>(&format!("/session/{id}/compact"))
            .await?;
        let compacted = result
            .get("compacted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if compacted {
            let before = result
                .get("tokens_before")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let after = result
                .get("tokens_after")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!("{} 已压缩：{before} → {after} tokens", "✓".green());
            if let Some(summary) = result.get("summary").and_then(|v| v.as_str()) {
                println!("{}", "── 摘要 ──".bold());
                for line in summary.lines() {
                    println!("{}", crate::markdown::render_block_line(line));
                }
            }
        } else {
            println!("{}", "未压缩（历史不足或模型未产出摘要）".yellow());
        }
        Ok(())
    }

    /// `/review [提示]`：发起一次「审查当前改动」的回合。
    async fn review(&mut self, extra: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let prompt = match extra {
            Some(extra) => format!(
                "请审查工作区当前改动（git diff），指出缺陷、风险与改进建议。额外关注：{extra}"
            ),
            None => "请审查工作区当前改动（git diff），指出缺陷、风险与改进建议。".to_string(),
        };
        self.run_turn(&prompt).await
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
        println!(
            "[debug] 模型：{}",
            self.model.as_deref().unwrap_or("（默认）")
        );
        println!(
            "[debug] 会话：{}",
            self.session.as_deref().unwrap_or("（无）")
        );
        println!("[debug] 后端：daemon {}", self.client.base_url());
        println!(
            "[debug] trace 目录：{}",
            self.data_root.join("traces").display()
        );
    }

    fn show_status(&self) {
        println!("工作区：{}", self.workspace.display());
        println!("模型：{}", self.model.as_deref().unwrap_or("（默认）"));
        println!(
            "会话：{}",
            self.session.as_deref().unwrap_or("（尚未创建）")
        );
        println!(
            "模式：{}",
            if self.read_only {
                "plan（只读）"
            } else {
                "build"
            }
        );
    }

    /// `/trust [status|grant|revoke]`：查看/切换工作区信任（审查 N1）。
    ///
    /// 未信任的工作区不会加载 hooks / 插件 MCP / 权限放宽 / 白名单（受限模式）。
    /// 授予是**显式用户动作**，与桌面端信任按钮、`owo-agent trust` 子命令、
    /// `GET/POST /workspace/trust` 共用数据根里的同一份记录。
    async fn handle_trust(&self, action: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let status = match action.unwrap_or("status") {
            "status" => self.client.workspace_trust().await?,
            "grant" | "trust" => self.client.set_workspace_trust(true).await?,
            "revoke" => self.client.set_workspace_trust(false).await?,
            other => {
                println!("未知子命令：{other}（用法：/trust [status|grant|revoke]）");
                return Ok(());
            }
        };
        let trusted = status
            .get("trusted")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let gated = status
            .get("gated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        println!(
            "工作区：{}",
            status
                .get("workspace")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("-")
        );
        if trusted {
            println!("信任：{}", "已信任".green());
        } else if gated {
            println!("信任：{}", "未信任（受限模式）".yellow());
        } else {
            println!("信任：未信任（该工作区没有会自动执行的配置，无需确认）");
        }
        if let Some(reasons) = status.get("reasons").and_then(serde_json::Value::as_array) {
            for reason in reasons {
                if let Some(text) = reason.as_str() {
                    println!("  - {text}");
                }
            }
        }
        if status
            .get("changed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            println!(
                "{}",
                "注意：配置内容已变化，需要重新确认（/trust grant）".yellow()
            );
        }
        if let Some(note) = status.get("note").and_then(serde_json::Value::as_str) {
            if !note.is_empty() {
                println!("{note}");
            }
        }
        Ok(())
    }

    async fn run_turn(&mut self, prompt: &str) -> Result<(), Box<dyn std::error::Error>> {
        let result = self.run_turn_capture(prompt).await?;
        let _ = result;
        // 模型可能经 goal_request 登记了长任务：回合结束后提示用户是否交给宿主循环。
        self.notice_model_goal().await;
        Ok(())
    }

    /// `/hunks [--path <文件>] [--accept <序号|all>] [--reject <序号|all>]`：
    /// 会话内逐 hunk 审阅（服务端 `/session/{id}/hunks[/apply]`）。
    async fn handle_hunks(
        &mut self,
        mut parts: std::str::SplitWhitespace<'_>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut path: Option<String> = None;
        let mut accept: Option<String> = None;
        let mut reject: Option<String> = None;
        while let Some(token) = parts.next() {
            match token {
                "--path" | "-p" => path = parts.next().map(str::to_string),
                "--accept" | "-a" => accept = parts.next().map(str::to_string),
                "--reject" | "-r" => reject = parts.next().map(str::to_string),
                other => {
                    return Err(format!(
                        "未知参数：{other}（用法：/hunks [--path <文件>] [--accept <序号|all>] [--reject <序号|all>]）"
                    )
                    .into())
                }
            }
        }
        let id = self.current_session().await?;
        // `all` 需要先知道该文件的 hunk 数（服务端视图是唯一真相）。
        let expand = |spec: &str,
                      view: Option<&serde_json::Value>,
                      path: &str|
         -> Result<Vec<usize>, String> {
            if spec.trim().eq_ignore_ascii_case("all") {
                let total = view
                    .and_then(|view| crate::support::hunk_count_for_path(view, path))
                    .ok_or_else(|| format!("会话改动里没有该文件：{path}"))?;
                return Ok((0..total).collect());
            }
            crate::support::parse_hunk_indices(spec)
        };
        match (path, accept, reject) {
            (None, None, None) => {
                let view = self
                    .client
                    .get_json::<serde_json::Value>(&format!("/session/{id}/hunks"))
                    .await?;
                println!("{}", crate::support::render_hunks_view(&id, &view));
            }
            (None, _, _) => {
                return Err("应用 hunk 需要 --path（一次审阅一个文件，避免误写）".into())
            }
            (Some(path), Some(spec), None) => {
                let needs_view = spec.trim().eq_ignore_ascii_case("all");
                let view = if needs_view {
                    Some(
                        self.client
                            .get_json::<serde_json::Value>(&format!("/session/{id}/hunks"))
                            .await?,
                    )
                } else {
                    None
                };
                let indices = expand(&spec, view.as_ref(), &path)?;
                let applied = self
                    .client
                    .post_json::<_, serde_json::Value>(
                        &format!("/session/{id}/hunks/apply"),
                        &serde_json::json!({ "path": path, "accept": indices }),
                    )
                    .await?;
                println!("{}", crate::support::render_hunks_apply(&applied));
            }
            (Some(path), None, Some(spec)) => {
                let needs_view = spec.trim().eq_ignore_ascii_case("all");
                let view = if needs_view {
                    Some(
                        self.client
                            .get_json::<serde_json::Value>(&format!("/session/{id}/hunks"))
                            .await?,
                    )
                } else {
                    None
                };
                let indices = expand(&spec, view.as_ref(), &path)?;
                let applied = self
                    .client
                    .post_json::<_, serde_json::Value>(
                        &format!("/session/{id}/hunks/apply"),
                        &serde_json::json!({ "path": path, "reject": indices }),
                    )
                    .await?;
                println!("{}", crate::support::render_hunks_apply(&applied));
            }
            (Some(_), Some(_), Some(_)) => return Err("--accept 与 --reject 不能同时使用".into()),
            (Some(_), None, None) => {
                return Err("需要 --accept 或 --reject（只列清单时不带这两个参数）".into())
            }
        }
        Ok(())
    }

    /// `/symbols <名字> [--kind <种类>] [--path <子目录>] [--max <N>]`：
    /// 符号级检索（服务端 `/symbols`；与 `find_symbol` 工具同内核）。
    async fn handle_symbols(
        &mut self,
        mut parts: std::str::SplitWhitespace<'_>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut query: Option<String> = None;
        let mut kind: Option<String> = None;
        let mut path: Option<String> = None;
        let mut max_results: Option<String> = None;
        while let Some(token) = parts.next() {
            match token {
                "--kind" | "-k" => kind = parts.next().map(str::to_string),
                "--path" | "-p" => path = parts.next().map(str::to_string),
                "--max" | "--max-results" => max_results = parts.next().map(str::to_string),
                other => {
                    if query.is_some() {
                        return Err(format!(
                            "只能给一个查询词（多余：{other}）；用法：/symbols <名字> [--kind <种类>] [--path <子目录>] [--max <N>]"
                        )
                        .into());
                    }
                    query = Some(other.to_string());
                }
            }
        }
        let query =
            query.ok_or("用法：/symbols <名字> [--kind <种类>] [--path <子目录>] [--max <N>]")?;
        let mut uri = format!(
            "/symbols?query={}",
            crate::support::encode_query_component(&query)
        );
        if let Some(kind) = kind.as_deref() {
            uri.push_str(&format!(
                "&kind={}",
                crate::support::encode_query_component(kind)
            ));
        }
        if let Some(path) = path.as_deref() {
            uri.push_str(&format!(
                "&path={}",
                crate::support::encode_query_component(path)
            ));
        }
        if let Some(max) = max_results.as_deref() {
            // 非数字直接由服务端 400 报错，不在这里吞掉。
            uri.push_str(&format!("&max_results={max}"));
        }
        let value = self
            .client
            .get_json::<serde_json::Value>(&uri)
            .await
            .map_err(|error| format!("符号检索失败（{error}）"))?;
        println!("{}", crate::support::render_symbol_report(&value));
        Ok(())
    }

    /// 模型登记的目标（origin=model）只提示一次：`/goal run` 交给宿主循环接管。
    async fn notice_model_goal(&mut self) {
        if self.goal.is_some() {
            return; // 本地循环已在推进目标。
        }
        let Some(id) = self.session.clone() else {
            return;
        };
        let Ok(session) = self.client.get_session(&id).await else {
            return;
        };
        let Some(goal) = session.goal else {
            return;
        };
        if goal.done || goal.origin != "model" {
            return;
        }
        if self.hinted_goal.as_deref() == Some(goal.objective.as_str()) {
            return;
        }
        self.hinted_goal = Some(goal.objective.clone());
        println!(
            "{} 模型请求接管长任务：{}\n  用 /goal run 由宿主循环推进；/goal clear 忽略",
            "◈".yellow(),
            goal.objective
        );
    }

    /// 执行一回合并返回最终文本与宿主完成状态（`/goal` 据此闭合任务）。
    async fn run_turn_capture(
        &mut self,
        prompt: &str,
    ) -> Result<crate::support::GoalTurnResult, Box<dyn std::error::Error>> {
        let id = self.current_session().await?;
        self.abort.store(false, Ordering::Relaxed);
        let turn_id = uuid::Uuid::new_v4().to_string();
        let cancel_turn_id = turn_id.clone();

        // Ctrl+C 监听随回合结束而退出（旧实现每回合 spawn 一个永不结束的任务：
        // `/goal` 多轮会累积监听器，回合结束后按 Ctrl+C 还会误置 abort）。
        let (turn_done_tx, turn_done_rx) = tokio::sync::oneshot::channel();
        let _turn_done = TurnDoneGuard(Some(turn_done_tx));
        let cancel_client = self.client.clone();
        let cancel_id = id.clone();
        let abort_flag = Arc::clone(&self.abort);
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    abort_flag.store(true, Ordering::Relaxed);
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), cancel_client.cancel_turn_id(&cancel_id,&cancel_turn_id)).await;
                }
                _ = turn_done_rx => {}
            }
        });

        println!("{} {}", "▶".green(), prompt.dimmed());
        let mut stream = self
            .client
            .open_turn_with_id(
                &id,
                prompt,
                self.read_only || self.read_only_locked,
                &turn_id,
            )
            .await?;
        let mut completion = crate::ui_output::TurnCompletion::default();
        let mut printer = StreamPrinter::new();
        while let Some(event) = stream.next_event().await {
            let event = event?;
            completion.observe(&event);
            match &event {
                SseEvent::PermissionRequest { request_id, .. } => {
                    printer.print_sse(&event);
                    let response = self.decide_permission(&event)?;
                    let _ = self
                        .client
                        .respond_permission(&id, request_id, &response)
                        .await;
                }
                SseEvent::UserQuestion { question_id, .. } => {
                    printer.print_sse(&event);
                    let answer = self.read_question_answer(&event)?;
                    self.client
                        .answer_question(&id, question_id, &answer)
                        .await?;
                }
                other => printer.print_sse(other),
            }
        }
        printer.finish();
        let completed = completion.finish()?;
        let diff_count = self
            .client
            .session_diff(&id)
            .await
            .map(|diffs| diffs.len())
            .unwrap_or(0);
        println!(
            "{} 工具 {} 步，改动 {} 个文件（/diff 查看）",
            completed.label(),
            completed.steps,
            diff_count
        );
        Ok(crate::support::GoalTurnResult {
            failed: completed.failure.is_some()
                && completed.status != owo_agent_protocol::CompletionStatusV1::Aborted,
            final_text: completed.final_text,
            completion_status: completed.status,
        })
    }

    /// 目标激活且未完成时，把目标附到每次输入前（目标推进期间用户插话也带目标上下文）。
    fn with_goal_context(&self, line: &str) -> String {
        crate::support::goal_context_prompt(self.goal.as_ref(), line)
    }

    /// `/todo`：查看会话任务清单（`todo` 工具维护）。
    async fn show_todos(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let id = self.current_session().await?;
        let session = self
            .client
            .get_json::<serde_json::Value>(&format!("/session/{id}"))
            .await?;
        let todos = session
            .get("todos")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        if todos.is_empty() {
            println!("（任务清单为空；模型调用 todo 工具后会出现在这里）");
            return Ok(());
        }
        println!("{}", "任务清单：".bold());
        for todo in todos {
            let content = todo
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let status = todo
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("pending");
            let mark = match status {
                "completed" => "✔".green().to_string(),
                "in_progress" => "▶".yellow().to_string(),
                _ => "○".dimmed().to_string(),
            };
            println!("  {mark} {content}");
        }
        Ok(())
    }

    /// `/goal [目标|status|clear]`：目标模式——模型标记完成且宿主裁决允许交付时才停止。
    async fn handle_goal(&mut self, arg: &str) -> Result<(), Box<dyn std::error::Error>> {
        use crate::support::{goal_max_iterations, GoalState};
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
                self.persist_goal("clear", None).await;
                println!("{}", "已清除目标".green());
                Ok(())
            }
            "run" | "start" => {
                // 接管已有目标（用户设定的，或模型经 goal_request 登记的）。
                let objective = match self.goal.as_ref() {
                    Some(goal) if !goal.done => goal.objective.clone(),
                    _ => {
                        let id = self.current_session().await?;
                        match self
                            .client
                            .get_session(&id)
                            .await
                            .ok()
                            .and_then(|session| session.goal)
                            .filter(|goal| !goal.done)
                        {
                            Some(goal) => goal.objective,
                            None => {
                                println!(
                                    "（没有可推进的目标；用法：/goal <目标描述>，或等模型用 goal_request 登记）"
                                );
                                return Ok(());
                            }
                        }
                    }
                };
                self.goal = Some(GoalState {
                    objective: objective.clone(),
                    iterations: self.goal.as_ref().map(|goal| goal.iterations).unwrap_or(0),
                    done: false,
                });
                println!("{}", format!("接管目标：{objective}").green());
                self.run_goal_loop(&objective).await
            }
            objective => {
                let max = goal_max_iterations();
                self.goal = Some(GoalState {
                    objective: objective.to_string(),
                    iterations: 0,
                    done: false,
                });
                // 目标落在当前会话上（跨进程 /resume 不丢）：先确保会话存在再写入。
                self.current_session().await?;
                self.persist_goal("set", Some(objective)).await;
                println!(
                    "{}（轮次上限：{}；/goal clear 停止）",
                    format!("目标已设定：{objective}").green(),
                    crate::support::goal_iteration_label(max)
                );
                self.run_goal_loop(objective).await
            }
        }
    }

    /// 目标推进循环：模型标记完成且宿主裁决允许交付时才停止（`/goal <目标>` 与
    /// `/goal run` 共用；轮次上限由 `goal_max_iterations` 决定）。
    async fn run_goal_loop(&mut self, objective: &str) -> Result<(), Box<dyn std::error::Error>> {
        use crate::support::{goal_continue_prompt, goal_first_prompt, goal_max_iterations};
        let max = goal_max_iterations();
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
                    format!("已达最大迭代 {max}，目标未标记完成（/goal status 查看）").yellow()
                );
                break;
            }
            let iteration = goal.iterations + 1;
            let prompt = if iteration == 1 {
                goal_first_prompt(objective)
            } else if let Some(status) = previous_status {
                crate::support::goal_continue_prompt_after_status(objective, iteration, status)
            } else {
                goal_continue_prompt(objective, iteration)
            };
            let limit = crate::support::goal_iteration_label(max);
            println!("{}", format!("── 目标推进 {iteration}/{limit} ──").bold());
            let turn = self.run_turn_capture(&prompt).await?;
            if let Some(goal) = self.goal.as_mut() {
                goal.iterations = iteration;
            }
            self.persist_goal("advance", None).await;
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
                self.persist_goal("done", None).await;
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

    /// `/team ...`：WorkSwarm 多 agent 团队——创建即后台跑，CLI 实时跟踪进度，
    /// 可查看任务图 / 干预（steer/retry/cancel）/ 查看真实变更集（diff）。
    async fn handle_team(
        &mut self,
        parts: std::str::SplitWhitespace<'_>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        const USAGE: &str = "用法：/team [--single|--team|--auto] [--parallel <2..=8> | --role <名>[:依赖1|依赖2]] [--model <模型>] [--model <角色>=<模型>] [--write <角色>=<路径>[;<路径>]] <目标>（缺省自动并行开发；--auto 使用策略选择）| list | use <团队ID> | status | watch | context [publish <key> <内容>] | steer <说明> | retry <步骤ID> | cancel | diff | accept <ChangeSet ID> [说明] | reject <ChangeSet ID> [说明] | revert <ChangeSet ID> [说明]";
        // 前置参数解析（纯函数 `parse_team_args`，便于单测）。声明了自定义角色时
        // 策略缺省强制 team（避免 auto 判定裁剪显式编排）。
        let raw: Vec<String> = parts.map(str::to_string).collect();
        let parsed =
            parse_team_args(&raw).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        let TeamArgs {
            strategy: parsed_strategy,
            roles: parsed_roles,
            rest,
            model: parsed_model,
            parallel,
        } = parsed;
        let (strategy, automatic_parallel) =
            resolve_team_create_intent(parsed_strategy, &parsed_roles, parallel);
        let mut parts = rest.into_iter();
        match parts.next().as_deref() {
            None => println!("{}", USAGE.dimmed()),
            Some("list") => self.team_list().await?,
            Some("use") => {
                let id = parts
                    .next()
                    .ok_or("用法：/team use <团队ID>（/team list 查看）")?;
                self.team_id = Some(id.to_string());
                println!("{} {}", "当前团队：".green(), id);
            }
            Some("status") => {
                let id = self.team_required()?;
                self.team_status(&id).await?;
            }
            Some("watch") => {
                let id = self.team_required()?;
                self.team_watch(&id).await?;
            }
            Some("context") => {
                let id = self.team_required()?;
                match parts.next() {
                    Some(command) if command == "publish" => {
                        let key = parts
                            .next()
                            .ok_or("用法：/team context publish <key> <内容>")?;
                        let value = parts.collect::<Vec<_>>().join(" ");
                        self.team_context_publish(&id, &key, &value).await?;
                    }
                    None => self.team_context(&id).await?,
                    Some(other) => {
                        return Err(format!("未知 context 子命令：{other}（支持 publish）").into())
                    }
                }
            }
            Some("steer") => {
                let id = self.team_required()?;
                let note = parts.collect::<Vec<_>>().join(" ");
                if note.trim().is_empty() {
                    return Err("用法：/team steer <给团队的说明>".into());
                }
                self.team_steer(&id, serde_json::json!({ "command": "steer", "note": note }))
                    .await?;
            }
            Some("retry") => {
                let id = self.team_required()?;
                let step_id = parts
                    .next()
                    .ok_or("用法：/team retry <步骤ID>（/team status 查看步骤）")?;
                self.team_steer(
                    &id,
                    serde_json::json!({ "command": "retry", "step_id": step_id }),
                )
                .await?;
            }
            Some("cancel") => {
                let id = self.team_required()?;
                self.team_steer(&id, serde_json::json!({ "command": "cancel" }))
                    .await?;
            }
            Some("diff") => {
                let id = self.team_required()?;
                self.team_diff(&id).await?;
            }
            Some(action @ ("accept" | "reject" | "revert")) => {
                let team_id = self.team_required()?;
                let decision = parse_team_change_set_decision(action, &parts.collect::<Vec<_>>())
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
                self.team_change_set_decide(&team_id, &decision).await?;
            }
            Some(first) => {
                // 目标可能含空格：把剩余片段拼回。
                let objective = std::iter::once(first.to_string())
                    .chain(parts)
                    .collect::<Vec<_>>()
                    .join(" ");
                self.team_create_and_watch(
                    &objective,
                    &strategy,
                    &parsed_roles,
                    parsed_model.as_deref(),
                    parallel,
                    automatic_parallel,
                )
                .await?;
            }
        }
        Ok(())
    }

    fn team_required(&self) -> Result<String, Box<dyn std::error::Error>> {
        self.team_id
            .clone()
            .ok_or_else(|| "当前没有团队（先 /team <目标> 创建，或 /team use <团队ID>）".into())
    }

    async fn team_list(&self) -> Result<(), Box<dyn std::error::Error>> {
        let value = self.client.get_json::<serde_json::Value>("/teams").await?;
        let teams = value
            .get("teams")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if teams.is_empty() {
            println!("（没有团队运行；/team <目标> 创建）");
            return Ok(());
        }
        println!("{}", "团队运行：".bold());
        for team in teams {
            let id = team.get("team_id").and_then(|v| v.as_str()).unwrap_or("?");
            let status = team.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let mode = team.get("mode").and_then(|v| v.as_str()).unwrap_or("?");
            let active = team
                .get("active")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let interrupted = team
                .get("interrupted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let members = team
                .get("members")
                .and_then(|v| v.as_array())
                .map(|list| list.len())
                .unwrap_or(0);
            let current = self.team_id.as_deref() == Some(id);
            println!(
                "  {} {}  {}  mode={mode}  {members} 名成员{}{}",
                if current {
                    "→".green().to_string()
                } else {
                    " ".to_string()
                },
                id.dimmed(),
                status,
                if active { " · 运行中" } else { "" },
                if interrupted {
                    " · 已中断（可 /team retry）"
                } else {
                    ""
                },
            );
        }
        Ok(())
    }

    async fn team_context(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let context = self
            .client
            .get_json::<serde_json::Value>(&format!("/teams/{id}/context"))
            .await?;
        let revision = context
            .get("revision")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let facts = context
            .get("facts")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        println!("{} revision={revision}", "团队共享上下文：".bold());
        if facts.is_empty() {
            println!("（暂无共享事实；用 /team context publish <key> <内容> 发布候选事实）");
            return Ok(());
        }
        let mut seen = std::collections::HashSet::new();
        for fact in facts {
            let key = fact.get("key").and_then(|v| v.as_str()).unwrap_or("?");
            if !seen.insert(key.to_string()) {
                continue;
            }
            let fact_revision = fact.get("revision").and_then(|v| v.as_u64()).unwrap_or(0);
            let producer = fact.get("producer").and_then(|v| v.as_str()).unwrap_or("?");
            let status = fact.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let value = fact.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let preview = value.chars().take(240).collect::<String>();
            println!("  r{fact_revision} [{status}] {key} · {producer}\n    {preview}");
        }
        Ok(())
    }

    async fn team_context_publish(
        &self,
        id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if key.trim().is_empty() || value.trim().is_empty() {
            return Err("用法：/team context publish <key> <内容>".into());
        }
        let current = self
            .client
            .get_json::<serde_json::Value>(&format!("/teams/{id}/context"))
            .await?;
        let expected_revision = current
            .get("revision")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let body = serde_json::json!({
            "expected_revision": expected_revision,
            "key": key,
            "value": value,
            "producer": "user",
            "source_refs": []
        });
        let result: serde_json::Value = self
            .client
            .post_json(&format!("/teams/{id}/context"), &body)
            .await?;
        let revision = result
            .pointer("/fact/revision")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| expected_revision.saturating_add(1));
        println!(
            "{} {key}（revision {revision}，候选事实）",
            "已共享：".green()
        );
        Ok(())
    }

    async fn team_status(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let detail = self
            .client
            .get_json::<serde_json::Value>(&format!("/teams/{id}"))
            .await?;
        print_team_detail(&detail);
        Ok(())
    }

    /// 创建团队（后台跑）+ 实时跟踪到终态（Ctrl+C 只停止跟踪，团队继续跑）。
    ///
    /// - `roles` 非空 = 自定义/并行角色编排；
    /// - `model`：显式 `--model` 才随请求下发；缺省由服务端读
    ///   `<workspace>/settings.json` 的 `team.model`（→ `model` → 环境变量/内置缺省）；
    /// - `parallel` = 显式并行开发：请求带 `parallel=true` +
    ///   `max_agent_members=N+2` + `budget.max_parallel=N`；缺省由服务端按
    ///   `team.parallel` 决定；lead 拆解出的 `subtasks`（子任务 + 写范围）由服务端
    ///   运行期动态应用到 w1..wN。
    async fn team_create_and_watch(
        &mut self,
        objective: &str,
        strategy: &str,
        roles: &[serde_json::Value],
        model: Option<&str>,
        parallel: Option<usize>,
        automatic_parallel: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let model = model.map(str::to_string);
        let mut body = serde_json::json!({
            "objective": objective,
            "mode": "team",
            "strategy": strategy,
            "workspace": {
                "root": self.workspace.to_string_lossy(),
                "read_only": false,
            },
        });
        if let Some(model) = &model {
            body["model"] = serde_json::json!(model);
        }
        if let Some(session_id) = self.session.as_deref() {
            body["parent_session_id"] = serde_json::json!(session_id);
        }
        apply_team_execution_intent(&mut body, parallel, automatic_parallel);
        if automatic_parallel {
            println!(
                "{} 默认自动并行开发；并发容量按工作区 team 配置（缺省 4）",
                "编排：".green()
            );
        }
        if !roles.is_empty() {
            if let Some(obj) = body.as_object_mut() {
                obj.insert(
                    "roles".to_string(),
                    serde_json::Value::Array(roles.to_vec()),
                );
            }
            if let Some(writers) = parallel {
                println!(
                    "{} 并行 {writers} 路：lead 拆解 → w1..w{writers} 并行 → leader 汇总；\
                     写范围由 lead 拆解动态分配（也可 --write w1=路径 预声明）",
                    "编排：".green()
                );
            } else {
                // 只读提示：自定义角色既未声明写范围、角色名也不属写角色族时，服务端
                // 按权限默认 deny 只读执行（不会产出文件）——提前告知避免"成功零产出"。
                for role in roles {
                    let name = role.get("role").and_then(|v| v.as_str()).unwrap_or("?");
                    let has_write = role
                        .get("write_paths")
                        .and_then(|v| v.as_array())
                        .is_some_and(|paths| !paths.is_empty());
                    if !has_write
                        && !owo_agent_core::worker_profile::WorkerProfile::for_role(name, 0)
                            .is_writer()
                    {
                        println!(
                            "{} 角色 {name} 未声明 --write 且不在写角色族，将按只读执行（不产出文件）",
                            "提示：".yellow()
                        );
                    }
                }
            }
        }
        let created: serde_json::Value = self.client.post_json("/teams", &body).await?;
        let team_id = created
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or("创建团队响应缺少 team_id")?
            .to_string();
        let mode = created
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("Team");
        let template = created
            .get("template_id")
            .and_then(|v| v.as_str())
            .unwrap_or("（动态组队）");
        // 成员展示：role 名 +（角色级模型覆盖时的）标注；统一模型在标题行展示。
        let members = created
            .get("members")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|member| {
                        let role = member.get("role").and_then(|v| v.as_str())?;
                        let model = roles
                            .iter()
                            .find(|r| r.get("role").and_then(|v| v.as_str()) == Some(role))
                            .and_then(|r| r.get("model"))
                            .and_then(|v| v.as_str());
                        Some(match model {
                            Some(model) => format!("{role}({model})"),
                            None => role.to_string(),
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        self.team_id = Some(team_id.clone());
        println!(
            "{} 团队已创建：{}（{mode} · 模板 {template} · 模型 {}）",
            "✓".green(),
            team_id,
            model
                .as_deref()
                .unwrap_or("settings.json team.model / 服务端缺省")
        );
        if !members.is_empty() {
            println!("  成员：{members}");
        }
        println!("  团队在 Daemon 侧后台运行；下面实时跟踪（Ctrl+C 停止跟踪，团队继续跑）");
        self.team_watch(&team_id).await
    }

    /// 轮询任务图并打印状态转移，直到团队进入终态。
    async fn team_watch(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        use std::collections::{HashMap, HashSet};
        use std::time::{Duration, Instant};

        let deadline = Instant::now() + Duration::from_secs(600);
        let mut last = HashMap::<String, String>::new();
        let mut seen_audit = HashSet::<u64>::new();
        let mut last_event_id: Option<String> = None;
        let mut detail = self
            .client
            .get_json::<serde_json::Value>(&format!("/teams/{id}"))
            .await?;
        print_team_detail(&detail);
        seed_team_step_states(&detail, &mut last);
        let mut status = team_status(&detail);
        if team_status_is_terminal(&status) {
            print_team_watch_terminal(&status, &detail);
            return Ok(());
        }

        let mut retry_delay = Duration::from_millis(250);
        loop {
            if Instant::now() >= deadline {
                println!(
                    "{}",
                    "（跟踪已到 10 分钟上限；团队继续在后台运行，/team watch 继续跟踪）".yellow()
                );
                return Ok(());
            }

            let stream_path = format!("/teams/{id}/events");
            match self
                .client
                .open_event_stream_after(&stream_path, last_event_id.as_deref())
                .await
            {
                Ok(mut stream) => loop {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {
                            println!("{}", "（已停止跟踪；团队继续在后台运行，/team status 查看）".yellow());
                            return Ok(());
                        }
                        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                            println!("{}", "（跟踪已到 10 分钟上限；团队继续在后台运行，/team watch 继续跟踪）".yellow());
                            return Ok(());
                        }
                        event = stream.next_event() => match event {
                            Some(Ok(event)) => {
                                if let Some(cursor) = stream.last_event_id() {
                                    last_event_id = Some(cursor.to_string());
                                }
                                match event.get("type").and_then(|v| v.as_str()) {
                                    Some("progress") => {
                                        if let Some(steps) = event.pointer("/progress/current_steps").and_then(|v| v.as_array()) {
                                            print_team_step_updates(steps, &mut last);
                                        }
                                    }
                                    Some("audit") => {
                                        if let Some(signature) = team_audit_signature(&event) {
                                            if seen_audit.insert(signature) {
                                                if seen_audit.len() > 2048 {
                                                    seen_audit.clear();
                                                    seen_audit.insert(signature);
                                                }
                                                if let Some(summary) = team_audit_summary(&event) {
                                                    println!("  · {summary}");
                                                }
                                            }
                                        }
                                    }
                                    Some("state") => {
                                        status = event.get("status").and_then(|v| v.as_str()).unwrap_or("running").to_string();
                                        if team_status_is_terminal(&status) {
                                            detail = self.client.get_json(&format!("/teams/{id}")).await?;
                                            print_team_watch_terminal(&status, &detail);
                                            return Ok(());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Some(Err(error)) => {
                                if let Some(cursor) = stream.last_event_id() {
                                    last_event_id = Some(cursor.to_string());
                                }
                                eprintln!("{} {error}", "团队事件流中断，正在刷新状态并重连：".yellow());
                                break;
                            }
                            None => {
                                if let Some(cursor) = stream.last_event_id() {
                                    last_event_id = Some(cursor.to_string());
                                }
                                break;
                            }
                        }
                    }
                },
                Err(error) => {
                    eprintln!(
                        "{} {error}",
                        "团队事件流暂不可用，正在刷新状态并重连：".yellow()
                    );
                }
            }

            // SSE 断开时先用快照补齐状态，再重连；不会让断流丢失进度。
            detail = self
                .client
                .get_json::<serde_json::Value>(&format!("/teams/{id}"))
                .await?;
            status = team_status(&detail);
            if let Some(steps) = detail
                .get("progress")
                .and_then(|progress| progress.get("current_steps"))
                .and_then(|steps| steps.as_array())
            {
                print_team_step_updates(steps, &mut last);
            }
            if let Some(tasks) = detail.get("tasks").and_then(|tasks| tasks.as_array()) {
                print_team_step_updates(tasks, &mut last);
            }
            if team_status_is_terminal(&status) {
                print_team_watch_terminal(&status, &detail);
                return Ok(());
            }

            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("{}", "（已停止跟踪；团队继续在后台运行，/team status 查看）".yellow());
                    return Ok(());
                }
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                    println!("{}", "（跟踪已到 10 分钟上限；团队继续在后台运行，/team watch 继续跟踪）".yellow());
                    return Ok(());
                }
                _ = tokio::time::sleep(retry_delay) => {}
            }
            retry_delay = (retry_delay * 2).min(Duration::from_secs(5));
        }
    }

    async fn team_steer(
        &self,
        id: &str,
        payload: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let value: serde_json::Value = self
            .client
            .post_json(&format!("/teams/{id}/steer"), &payload)
            .await?;
        println!("{} {}", "已提交团队指令：".green(), value);
        Ok(())
    }

    async fn team_change_set_decide(
        &self,
        team_id: &str,
        decision: &TeamChangeSetDecision,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Resolve the target through the selected team first; ChangeSet action routes use a
        // global ID, so this prevents a typo or stale selection from mutating another team.
        let listed = self
            .client
            .get_json::<serde_json::Value>(&format!("/teams/{team_id}/change-sets"))
            .await?;
        let change_sets = listed
            .get("change_sets")
            .and_then(serde_json::Value::as_array)
            .ok_or("服务端没有返回团队 ChangeSet 列表")?;
        let target = change_sets.iter().find(|item| {
            item.get("change_set_id")
                .and_then(serde_json::Value::as_str)
                == Some(decision.change_set_id.as_str())
        });
        let Some(target) = target else {
            return Err(format!(
                "ChangeSet {} 不属于当前团队 {team_id}；请先 /team diff 核对 ID",
                decision.change_set_id
            )
            .into());
        };
        if target.get("team_id").and_then(serde_json::Value::as_str) != Some(team_id) {
            return Err("服务端 ChangeSet 的 team_id 与当前团队不一致，拒绝继续".into());
        }

        let idempotency_key = team_change_set_idempotency_key(team_id, decision);
        let mut body = serde_json::json!({ "idempotency_key": idempotency_key });
        if let Some(note) = &decision.note {
            body["note"] = serde_json::Value::String(note.clone());
        }
        let result = self
            .client
            .post_json::<_, serde_json::Value>(
                &format!(
                    "/change-sets/{}/{}",
                    decision.change_set_id, decision.action
                ),
                &body,
            )
            .await?;
        let change_set = result
            .get("change_set")
            .filter(|value| {
                value
                    .get("change_set_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(decision.change_set_id.as_str())
                    && value.get("team_id").and_then(serde_json::Value::as_str) == Some(team_id)
            })
            .ok_or("ChangeSet 决定响应的团队或变更集身份不匹配")?;
        let status = change_set
            .get("status")
            .and_then(serde_json::Value::as_str)
            .ok_or("ChangeSet 决定响应缺少状态")?;
        let replayed = result
            .get("replayed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        println!(
            "{} ChangeSet {} → {}{}",
            "团队变更已处理：".green(),
            decision.change_set_id,
            status,
            if replayed { "（幂等重放）" } else { "" }
        );
        Ok(())
    }

    async fn team_diff(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let value = self
            .client
            .get_json::<serde_json::Value>(&format!("/teams/{id}/change-sets"))
            .await?;
        let change_sets = value
            .get("change_sets")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if change_sets.is_empty() {
            println!("（团队还没有变更集；写角色产出真实文件改动后会出现）");
            return Ok(());
        }
        println!("{}", "团队变更集：".bold());
        if let Some(reason) = value.get("approval_block_reason").and_then(|v| v.as_str()) {
            println!("  {} {reason}", "批准门：".yellow());
        }
        for change_set in change_sets {
            let cs_id = change_set
                .get("change_set_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let role = change_set
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let state = change_set
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let files = change_set
                .get("changed_files")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            println!(
                "  {} role={role} status={state} 文件 {} 个",
                cs_id.dimmed(),
                files.len()
            );
            for file in files.iter().take(10) {
                if let Some(path) = file.as_str() {
                    println!("      {path}");
                }
            }
            if let Some(diff_ref) = change_set.get("diff_ref").and_then(|v| v.as_str()) {
                println!("      diff: {diff_ref}");
            }
        }
        println!("  处理方式：/team accept <ChangeSet ID> [说明] | reject <ChangeSet ID> [说明] | revert <ChangeSet ID> [说明]");
        Ok(())
    }
    fn read_question_answer(&self, event: &SseEvent) -> Result<String, Box<dyn std::error::Error>> {
        let SseEvent::UserQuestion {
            question_id,
            options,
            ..
        } = event
        else {
            return Err("无效的用户提问事件".into());
        };
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return Ok("请基于现有信息继续；缺失的信息保持未知，不要虚构授权或验收".into());
        }
        use std::io::Write;
        loop {
            if options.is_empty() {
                eprint!("回答 {question_id}（输入后按 Enter）：");
            } else {
                eprint!("回答 {question_id}（输入序号或文字，按 Enter）：");
            }
            let _ = std::io::stderr().flush();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line)? == 0 {
                return Err("标准输入已关闭，无法回答 Agent 的问题".into());
            }
            if let Some(answer) = crate::ui_output::resolve_question_answer(&line, options) {
                return Ok(answer);
            }
            eprintln!("回答不能为空；回合仍在等待你的补充信息。");
        }
    }

    fn decide_permission(
        &self,
        event: &SseEvent,
    ) -> Result<PermissionResponse, Box<dyn std::error::Error>> {
        let SseEvent::PermissionRequest {
            tool,
            reason,
            level,
            args,
            redacted_args,
            risk_note,
            explain,
            ..
        } = event
        else {
            return Ok(parse_approval_response("deny"));
        };
        if self.no_approval {
            return Ok(PermissionResponse {
                allow: true,
                remember: None,
                scope: Some("once".to_string()),
            });
        }
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("approval_required: 非交互任务未获得授权，本次拒绝且不消耗下一条任务输入");
            return Ok(parse_approval_response("deny"));
        }
        print_permission_card(&PermissionCard {
            tool,
            level: level.as_deref(),
            reason,
            args: Some(args),
            redacted_args: redacted_args.as_ref(),
            risk_note: risk_note.as_deref(),
            explain: explain.as_ref(),
        });
        eprint!("允许？[y=仅本次 / t=本任务 / w=工作区长期 / n=拒绝] ");
        use std::io::Write;
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        Ok(parse_approval_response(&line))
    }
}

fn grant_revoke_payload(grant_id: &str) -> serde_json::Value {
    serde_json::json!({ "grant_id": grant_id })
}

fn print_help() {
    println!("{}", "── daemon 模式命令 ──".bold());
    println!("  直接输入文字       向当前会话发起任务");
    println!("  /new [模型]        新建会话");
    println!("  /sessions          列出会话");
    println!("  /resume <id>       恢复会话");
    println!("  /model [名称]      查看/切换模型");
    println!("  /plan | /build     切换只读 / 执行模式");
    println!("  /diff              查看本次会话文件改动");
    println!("  /hunks [--path <文件>] [--accept|--reject <序号|all>]  逐 hunk 审阅/接受/拒绝（写盘前 409 校验）");
    println!("  /symbols <名字> [--kind <种类>] [--path <子目录>] [--max <N>]  符号级检索（函数/类/结构体定义）");
    println!("  /undo | /revert    回滚本次会话全部写操作");
    println!("  /rewind <n>        回退会话历史到保留 n 条消息");
    println!("  /redo              恢复最近一次 rewind");
    println!("  /fork [序号]       在指定消息处创建子会话");
    println!("  /rename <标题>     重命名当前会话");
    println!("  /archive | /pin    归档 / 置顶当前会话");
    println!("  /abort             中止当前回合");
    println!("  /status            查看工作区/模型/会话状态");
    println!("  /trust [status|grant|revoke]  查看/授予/撤销工作区信任（未信任=受限模式，不加载 hooks/插件/权限放宽）");
    println!(
        "  /permissions [status] 查看档位与授权；set <档位> 切换；revoke <授权ID> 撤销单条授权"
    );
    println!("  /team [--parallel N] [--model <模型>] [--write <角色>=<路径>] <目标>");
    println!(
        "                    并行 N 路：lead 拆解 → w1..wN 并行 → leader 汇总；模型/并行/角色缺省读 <workspace>/settings.json 的 team 段（list/status/steer/retry/cancel/diff）"
    );
    println!("                    accept/reject/revert <ChangeSet ID> [说明] 处理团队变更集");
    println!("                    context 查看团队共享事实；context publish <key> <内容> 发布带 revision 的候选事实");
    println!("  /audit /skills /settings /traces  读取服务端状态");
    println!("  /clear             清屏");
    println!("  /exit | /quit      退出");
    println!("  其它命令请用 --local 使用旧 REPL（迁移中）");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_done_guard_signals_listener_even_when_stream_exits_early() {
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        let guard = TurnDoneGuard(Some(sender));
        drop(guard);
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn cli_grant_revoke_is_limited_to_one_exact_grant() {
        assert_eq!(GRANT_REVOKE_PATH, "/permissions/grants/revoke");
        assert_eq!(
            grant_revoke_payload("grant-123"),
            serde_json::json!({ "grant_id": "grant-123" })
        );
    }

    fn team_args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn team_change_set_idempotency_key_is_stable_and_action_scoped() {
        let accept =
            parse_team_change_set_decision("accept", &team_args(&["cs-team-123:task-1-456"]))
                .unwrap();
        let retry = accept.clone();
        let reject =
            parse_team_change_set_decision("reject", &team_args(&["cs-team-123:task-1-456"]))
                .unwrap();

        assert_eq!(
            team_change_set_idempotency_key("team-123", &accept),
            team_change_set_idempotency_key("team-123", &retry)
        );
        assert_ne!(
            team_change_set_idempotency_key("team-123", &accept),
            team_change_set_idempotency_key("team-123", &reject)
        );
    }

    #[test]
    fn team_change_set_decisions_validate_ids_and_preserve_notes() {
        let args = team_args(&["cs-team-123:task-1-456", "确认", "已检查"]);
        let decision = parse_team_change_set_decision("accept", &args).unwrap();
        assert_eq!(decision.action, "accept");
        assert_eq!(decision.change_set_id, "cs-team-123:task-1-456");
        assert_eq!(decision.note.as_deref(), Some("确认 已检查"));

        assert!(parse_team_change_set_decision("overwrite", &args).is_err());
        assert!(parse_team_change_set_decision("revert", &team_args(&["../outside"])).is_err());
        assert!(parse_team_change_set_decision("revert", &team_args(&[".."])).is_err());
        assert!(parse_team_change_set_decision("reject", &[]).is_err());
    }

    #[test]
    fn parse_team_args_builds_parallel_roles_with_models_and_scopes() {
        let parsed = parse_team_args(&team_args(&[
            "--role",
            "w1",
            "--role",
            "w2:reviewer",
            "--model",
            "w1=glm-5.3-flash,w2=glm-4.6",
            "--write",
            "w1=src/a;src/common",
            "--write",
            "w2=src/b",
            "实现",
            "A 与 B",
        ]))
        .unwrap();
        assert_eq!(parsed.strategy, None, "缺省策略按是否声明角色推导");
        assert_eq!(parsed.rest, vec!["实现".to_string(), "A 与 B".to_string()]);
        assert_eq!(parsed.roles.len(), 2);
        assert_eq!(parsed.roles[0]["role"], "w1");
        assert_eq!(parsed.roles[0]["assignee"], "agent");
        assert_eq!(parsed.roles[0]["model"], "glm-5.3-flash");
        assert_eq!(
            parsed.roles[0]["write_paths"],
            serde_json::json!(["src/a", "src/common"])
        );
        assert_eq!(parsed.roles[1]["role"], "w2");
        assert_eq!(
            parsed.roles[1]["depends_on"],
            serde_json::json!(["reviewer"])
        );
        assert_eq!(parsed.roles[1]["model"], "glm-4.6");
        assert_eq!(parsed.roles[1]["write_paths"], serde_json::json!(["src/b"]));
    }

    #[test]
    fn automatic_parallel_request_leaves_capacity_to_workspace_settings() {
        let mut body = serde_json::json!({"objective":"goal"});
        apply_team_execution_intent(&mut body, None, true);
        assert_eq!(body["parallel"], true);
        assert!(body.get("budget").is_none());
        assert!(body.get("max_agent_members").is_none());

        let mut explicit = serde_json::json!({"objective":"goal"});
        apply_team_execution_intent(&mut explicit, Some(3), false);
        assert_eq!(explicit["parallel"], true);
        assert_eq!(explicit["max_agent_members"], 5);
        assert_eq!(explicit["budget"]["max_parallel"], 3);
    }

    #[test]
    fn default_team_goal_selects_parallel_development_without_overriding_explicit_modes() {
        let (strategy, parallel) = resolve_team_create_intent(None, &[], None);
        assert_eq!(strategy, "team");
        assert!(parallel);

        let (strategy, parallel) = resolve_team_create_intent(Some("auto".to_string()), &[], None);
        assert_eq!(strategy, "auto");
        assert!(!parallel);

        let roles = vec![serde_json::json!({"role":"reviewer"})];
        let (strategy, parallel) = resolve_team_create_intent(None, &roles, None);
        assert_eq!(strategy, "team");
        assert!(!parallel);

        let (strategy, parallel) =
            resolve_team_create_intent(Some("single".to_string()), &[], None);
        assert_eq!(strategy, "single");
        assert!(!parallel);
    }

    #[test]
    fn parse_team_args_keeps_defaults_and_rejects_bad_mappings() {
        // 无前置参数：rest 原样，无角色；模型不写死（缺省读 settings.json team 段）。
        let parsed = parse_team_args(&team_args(&["--team", "目标"])).unwrap();
        assert_eq!(parsed.strategy.as_deref(), Some("team"));
        assert!(parsed.roles.is_empty());
        assert!(parsed.model.is_none(), "未显式 --model 时不得内置模型常量");
        assert!(parsed.parallel.is_none());
        assert_eq!(parsed.rest, vec!["目标".to_string()]);
        // 未声明角色 → 模型/写范围映射报错；缺参数报错。
        assert!(parse_team_args(&team_args(&["--model", "w1=x"])).is_err());
        assert!(parse_team_args(&team_args(&["--role", "w1", "--write", "w2=src"])).is_err());
        assert!(parse_team_args(&team_args(&["--write"])).is_err());
    }

    #[test]
    fn parse_team_args_parallel_generates_lead_writers_and_leader() {
        let parsed = parse_team_args(&team_args(&[
            "--parallel",
            "3",
            "--model",
            "glm-5.3-flashx",
            "--write",
            "w2=src/b",
            "开发",
            "功能",
        ]))
        .unwrap();
        assert_eq!(parsed.parallel, Some(3));
        assert_eq!(parsed.model.as_deref(), Some("glm-5.3-flashx"));
        let roles: Vec<&str> = parsed
            .roles
            .iter()
            .map(|r| r["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, vec!["lead", "w1", "w2", "w3", "leader"]);
        assert_eq!(parsed.roles[1]["depends_on"], serde_json::json!(["lead"]));
        assert_eq!(
            parsed.roles[4]["depends_on"],
            serde_json::json!(["w1", "w2", "w3"])
        );
        assert_eq!(parsed.roles[2]["write_paths"], serde_json::json!(["src/b"]));
        assert_eq!(parsed.rest, vec!["开发".to_string(), "功能".to_string()]);
    }

    #[test]
    fn team_audit_summary_exposes_runtime_measurements_without_control_characters() {
        let event = serde_json::json!({
            "ts": "2026-10-02T05:00:00Z",
            "event": "team.model.request_completed",
            "detail": "role=w1 model=glm-5 latency_ms=123 usage_tokens=42 request_id=secret"
        });
        let summary = team_audit_summary(&event).expect("known runtime event");
        assert!(summary.contains("w1"));
        assert!(summary.contains("glm-5"));
        assert!(summary.contains("123 ms"));
        assert!(summary.contains("42 tokens"));
        assert!(!summary.contains("secret"));

        let model_started = serde_json::json!({
            "event": "team.model.started",
            "detail": "role=w1 step_id=step-1"
        });
        let model_summary = team_audit_summary(&model_started).expect("model start is visible");
        assert!(model_summary.contains("w1"));
        assert!(model_summary.contains("step-1"));

        let tool_event = serde_json::json!({
            "event": "team.tool.finished",
            "detail": "role=implementer step_id=step-2 tool=apply_patch outcome=succeeded duration_ms=13",
        });
        let tool_summary = team_audit_summary(&tool_event).expect("tool event is visible");
        assert!(tool_summary.contains("apply_patch"));
        assert!(tool_summary.contains("succeeded"));
        assert!(tool_summary.contains("13 ms"));

        let validation_event = serde_json::json!({
            "event": "team.artifact.validation_passed",
            "detail": "member=m-w1 step=step-2 format=markdown duration_ms=4",
        });
        let validation_summary =
            team_audit_summary(&validation_event).expect("validation timing is visible");
        assert!(validation_summary.contains("4 ms"));

        let unsafe_field = serde_json::json!({
            "event": "team.worker.started",
            "detail": "role=w1\u{001b}[31m step_id=step-1"
        });
        let summary = team_audit_summary(&unsafe_field).expect("worker start");
        assert!(!summary.contains('\u{1b}'));
        assert!(team_audit_summary(&serde_json::json!({"event":"unknown.event"})).is_none());
    }

    #[test]
    fn parse_team_args_parallel_rejects_bad_or_conflicting_flags() {
        assert!(parse_team_args(&team_args(&["--parallel", "1"])).is_err());
        assert!(parse_team_args(&team_args(&["--parallel", "9"])).is_err());
        assert!(parse_team_args(&team_args(&["--parallel", "2", "--role", "w1"])).is_err());
        assert!(parse_team_args(&team_args(&["--parallel"])).is_err());
    }
}
