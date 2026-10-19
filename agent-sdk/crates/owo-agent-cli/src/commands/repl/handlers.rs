// §12.3 CLI 拆分：repl 斜杠命令处理器（自 repl.rs 机械外移，零行为变化）。
// impl 块延续父模块的 Repl 类型；方法 pub(crate) 以便核心循环（父模块）分发。

use super::Repl;
use crate::support::*;
use colored::Colorize;
use owo_agent_core::permissions::PermissionProfile;
use owo_agent_core::session::SessionStore;
use owo_agent_core::{
    export_html, export_markdown, list_traces, load_trace, McpClient, McpServerConfig,
    SkillRegistry, SuggestionAction,
};
use std::sync::Arc;

impl Repl {
    pub(crate) fn list_skills(&self) {
        let skills = self.agent.skills().list();
        if skills.is_empty() {
            println!(
                "暂无技能（放置到 {}/skills 或 .agents/skills/，每技能一个含 SKILL.md 的目录）",
                display_path(&self.data_root)
            );
            return;
        }
        for skill in skills {
            let marker = if self.skills.is_enabled(&skill.name) {
                String::new()
            } else {
                " [禁用]".dimmed().to_string()
            };
            println!("{}：{}{}", skill.name.cyan(), skill.description, marker);
        }
    }

    pub(crate) fn reload_skills(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut skills = SkillRegistry::discover(&self.workspace, &self.data_root);
        apply_disabled_skills(&mut skills, &self.settings);
        self.skills = skills;
        self.rebuild_agent()?;
        println!(
            "{} 已重新加载 {} 个技能",
            "✓".green(),
            self.skills.list().len()
        );
        Ok(())
    }

    pub(crate) async fn handle_mcp(
        &mut self,
        command: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let rest = command.trim_start_matches("mcp").trim_start().to_string();
        let mut parts = rest.split_whitespace();
        match parts.next() {
            Some("add") => {
                let name = parts
                    .next()
                    .ok_or("用法：/mcp add <名称> <命令> [参数...]")?
                    .to_string();
                let command_line: Vec<&str> = parts.collect();
                let config = if matches!(command_line.first().copied(), Some("http" | "https")) {
                    let url = command_line
                        .get(1)
                        .copied()
                        .ok_or("HTTP MCP 用法：/mcp add <名称> http <URL>")?;
                    McpServerConfig::http(&name, url)
                } else {
                    let command = command_line
                        .first()
                        .ok_or("缺少 MCP 服务器命令（如 npx、node、python）")?;
                    McpServerConfig::stdio(
                        &name,
                        *command,
                        command_line[1..]
                            .iter()
                            .map(|arg| arg.to_string())
                            .collect(),
                    )
                };
                match McpClient::connect(&config).await {
                    Ok(client) => {
                        let tool_count = client.tools().len();
                        self.mcp_clients
                            .push((name.clone(), Arc::new(tokio::sync::Mutex::new(client))));
                        self.mcp_configs.push(config);
                        save_mcp_configs(&self.data_root, &self.mcp_configs);
                        self.rebuild_agent()?;
                        println!("{} MCP {name}（{tool_count} 个工具）", "已添加".green());
                    }
                    Err(error) => println!("{} 连接失败：{error}", "✘".red()),
                }
            }
            Some("list") => {
                if self.mcp_clients.is_empty() {
                    println!("未配置 MCP 服务器（/mcp add <名称> <命令>）");
                } else {
                    for (name, client) in &self.mcp_clients {
                        let transport = self
                            .mcp_configs
                            .iter()
                            .find(|config| config.name == *name)
                            .map(|config| config.transport.as_str())
                            .unwrap_or("stdio");
                        let tool_names = match client.try_lock() {
                            Ok(guard) => guard
                                .tools()
                                .into_iter()
                                .map(|tool| tool.name)
                                .collect::<Vec<_>>()
                                .join(", "),
                            Err(_) => "（忙碌）".to_string(),
                        };
                        println!("{name}（{transport}）：{tool_names}");
                    }
                }
            }
            Some("trust") => {
                let name = parts
                    .next()
                    .ok_or("用法：/mcp trust <名称> <工具名> [更多工具名...]")?;
                let tool_names: Vec<String> = parts.map(|part| part.to_string()).collect();
                if tool_names.is_empty() {
                    return Err("未指定要声明为可信只读的工具名".into());
                }
                let index = self
                    .mcp_configs
                    .iter()
                    .position(|config| config.name == name)
                    .ok_or_else(|| format!("MCP 服务器不存在：{name}"))?;
                let config = &mut self.mcp_configs[index];
                for tool_name in &tool_names {
                    if !config.trusted_readonly.contains(tool_name) {
                        config.trusted_readonly.push(tool_name.clone());
                    }
                }
                let config_snapshot = config.clone();
                save_mcp_configs(&self.data_root, &self.mcp_configs);
                // 热生效：重新声明 + 重建 Agent 让注册时校验 hash。
                if let Some((_, client)) = self
                    .mcp_clients
                    .iter()
                    .find(|(existing, _)| existing == name)
                {
                    let guard = client.lock().await;
                    let tools = guard.tools();
                    let declared = owo_agent_core::tool_effects::declare_trusted_from_config(
                        &config_snapshot,
                        &tools,
                    );
                    println!(
                        "{} MCP {name} 声明可信只读 {declared}/{} 个工具（schema hash 匹配才生效）",
                        "✓".green(),
                        tool_names.len()
                    );
                }
                self.rebuild_agent()?;
            }
            Some("remove") => {
                let name = parts.next().ok_or("用法：/mcp remove <名称>")?;
                if let Some(position) = self
                    .mcp_clients
                    .iter()
                    .position(|(existing, _)| existing == name)
                {
                    let (_, client) = self.mcp_clients.remove(position);
                    let _ = client.lock().await.shutdown().await;
                }
                self.mcp_configs.retain(|config| config.name != name);
                save_mcp_configs(&self.data_root, &self.mcp_configs);
                self.rebuild_agent()?;
                println!("已移除 MCP 服务器：{name}");
            }
            Some("untrust") => {
                let name = parts
                    .next()
                    .ok_or("用法：/mcp untrust <名称> <工具名> [更多工具名...]")?;
                let tool_names: Vec<String> = parts.map(|part| part.to_string()).collect();
                if tool_names.is_empty() {
                    return Err("未指定要撤销可信只读声明的工具名".into());
                }
                let index = self
                    .mcp_configs
                    .iter()
                    .position(|config| config.name == name)
                    .ok_or_else(|| format!("MCP 服务器不存在：{name}"))?;
                let config = &mut self.mcp_configs[index];
                let before = config.trusted_readonly.len();
                config.trusted_readonly.retain(|t| !tool_names.contains(t));
                let after = config.trusted_readonly.len();
                save_mcp_configs(&self.data_root, &self.mcp_configs);
                println!(
                    "{} MCP {name} 撤销可信只读声明（{} → {} 个）",
                    "✓".green(),
                    before,
                    after
                );
                self.rebuild_agent()?;
            }
            _ => {
                println!("用法：/mcp add <名称> <命令> [参数...] | /mcp list | /mcp remove <名称> | /mcp trust <名称> <工具名...> | /mcp untrust <名称> <工具名...>")
            }
        }
        Ok(())
    }

    pub(crate) async fn new_session(
        &mut self,
        model: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(session) = &self.session {
            self.store.save(session)?;
        }
        let model = model
            .map(str::to_string)
            .unwrap_or_else(|| self.model.clone());
        self.model = model.clone();
        let session = self.store.create(&self.workspace, &model, None)?;
        println!("{} {}", "新会话：".green(), session.id.dimmed());
        self.session = Some(session);
        Ok(())
    }

    pub(crate) async fn fork_session(
        &mut self,
        index: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(current) = &self.session else {
            println!("暂无会话");
            return Ok(());
        };
        let index = match index {
            Some(value) => value.parse().map_err(|_| "消息序号需为数字".to_string())?,
            None => current.messages.len().saturating_sub(1),
        };
        let child = current.fork(index);
        self.store.save(&child)?;
        let id = child.id.clone();
        self.session = Some(child);
        println!("{} 子会话 {id}（在消息 {index} 处 fork）", "已创建".green());
        Ok(())
    }

    pub(crate) async fn rewind_session(
        &mut self,
        keep: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(session) = &mut self.session else {
            println!("暂无会话");
            return Ok(());
        };
        let keep: usize = keep.parse().map_err(|_| "保留消息数需为数字".to_string())?;
        if keep < session.messages.len() {
            session.revert().await?;
        }
        let removed = session.rewind(keep);
        self.store.save(session)?;
        println!(
            "{} 已回退到 {keep} 条消息（移除 {} 条，/redo 可恢复）",
            "↶".yellow(),
            removed.len()
        );
        Ok(())
    }

    pub(crate) async fn redo_session(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(session) = &mut self.session else {
            println!("暂无会话");
            return Ok(());
        };
        let restored = session.redo().map(|tail| tail.len()).unwrap_or(0);
        self.store.save(session)?;
        if restored == 0 {
            println!("没有可恢复的历史");
        } else {
            println!("{} 已恢复 {restored} 条消息", "↷".green());
        }
        Ok(())
    }

    pub(crate) async fn undo_message(
        &mut self,
        count: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(session) = &mut self.session else {
            println!("暂无会话");
            return Ok(());
        };
        let count = match count {
            Some(value) => value.parse().map_err(|_| "数量需为数字".to_string())?,
            None => 1,
        };
        let removed = session.undo_message(count);
        match removed {
            Some(messages) => {
                self.store.save(session)?;
                println!(
                    "{} 已撤销 {} 条消息（/redo-msg 恢复）",
                    "↶".yellow(),
                    messages.len()
                );
            }
            None => println!("没有可撤销的消息"),
        }
        Ok(())
    }

    pub(crate) async fn redo_message(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(session) = &mut self.session else {
            println!("暂无会话");
            return Ok(());
        };
        let restored = session.redo_message().map(|tail| tail.len()).unwrap_or(0);
        self.store.save(session)?;
        if restored == 0 {
            println!("没有可恢复的消息");
        } else {
            println!("{} 已恢复 {restored} 条消息", "↷".green());
        }
        Ok(())
    }

    pub(crate) fn show_tree(&self) {
        let ids = self.store.list();
        if ids.is_empty() {
            println!("暂无会话");
            return;
        }
        for id in ids {
            if let Ok(session) = self.store.load(&id) {
                let active = self
                    .session
                    .as_ref()
                    .map(|current| current.id == id)
                    .unwrap_or(false);
                let parent = session.parent_id.as_deref().unwrap_or("(根)");
                println!(
                    "{} {} parent={} fork={} msgs={}",
                    if active {
                        "▶".green().to_string()
                    } else {
                        "  ".to_string()
                    },
                    id,
                    parent,
                    session
                        .fork_point
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".into()),
                    session.messages.len()
                );
            }
        }
    }

    pub(crate) fn share_session(
        &self,
        format: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(session) = &self.session else {
            println!("暂无会话");
            return Ok(());
        };
        let format = format.unwrap_or("md");
        let shares = self.data_root.join("shares");
        std::fs::create_dir_all(&shares)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let path = match format {
            "html" => shares.join(format!("{}-{stamp}.html", session.id)),
            _ => shares.join(format!("{}-{stamp}.md", session.id)),
        };
        let content = match format {
            "html" => export_html(session),
            _ => export_markdown(session),
        };
        std::fs::write(&path, content)?;
        println!("{} 会话已导出：{}", "已分享".green(), display_path(&path));
        Ok(())
    }

    pub(crate) fn list_traces(&self) {
        let traces = list_traces(&self.data_root.join("traces"));
        if traces.is_empty() {
            println!("暂无 trace（完成回合后自动记录）");
            return;
        }
        for (index, path) in traces.iter().enumerate() {
            if let Ok(trace) = load_trace(path) {
                let preview: String = trace.prompt.chars().take(40).collect();
                println!(
                    "{}: {} steps={} {}ms final={}",
                    index,
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    trace.steps,
                    trace.duration_ms,
                    trace.final_text.is_some()
                );
                println!("      {}", preview.dimmed());
            }
        }
    }

    pub(crate) fn show_trace(&self, index: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let index: usize = index
            .ok_or("用法：/trace <序号>（/traces 查看）")?
            .parse()?;
        let traces = list_traces(&self.data_root.join("traces"));
        let path = traces
            .get(index)
            .ok_or_else(|| format!("trace 序号越界（共 {} 条）", traces.len()))?;
        let trace = load_trace(path)?;
        println!("{}", serde_json::to_string_pretty(&trace)?);
        Ok(())
    }

    pub(crate) fn show_settings(&self) {
        println!(
            "{}",
            serde_json::to_string_pretty(&self.settings).unwrap_or_default()
        );
    }

    pub(crate) fn list_plugins(&self) {
        if self.plugins.is_empty() {
            println!("未加载插件（放置到 <workspace>/.owo/plugins/ 或 <data>/plugins/，每插件一个含 manifest.json 的目录）");
            return;
        }
        for manifest in &self.plugins {
            let tool_count = self
                .mcp_clients
                .iter()
                .find(|(name, _)| name == &manifest.id)
                .and_then(|(_, client)| client.try_lock().ok())
                .map(|guard| guard.tools().len())
                .unwrap_or(0);
            println!(
                "{} v{}（{}）——{}",
                manifest.name,
                manifest.version,
                manifest.id,
                if manifest.description.is_empty() {
                    tool_count.to_string() + " 个工具"
                } else {
                    format!("{}，{} 个工具", manifest.description, tool_count)
                }
            );
        }
    }

    pub(crate) fn list_sessions(&self) {
        let ids = self.store.list();
        if ids.is_empty() {
            println!("暂无会话（/new 创建）");
            return;
        }
        for id in ids {
            match self.store.load(&id) {
                Ok(session) => {
                    let active = self
                        .session
                        .as_ref()
                        .map(|s| s.id == session.id)
                        .unwrap_or(false);
                    let mut badges = String::new();
                    if session.pinned {
                        badges.push_str(" 📌");
                    }
                    if session.archived {
                        badges.push_str(" 🗄");
                    }
                    let short_id: String = id.chars().take(8).collect();
                    println!(
                        "{} {}{}  {}  model={}  msgs={}  updated={}",
                        if active {
                            "▶".green().to_string()
                        } else {
                            "  ".to_string()
                        },
                        session.display_title(),
                        badges,
                        short_id.dimmed(),
                        session.model,
                        session.messages.len(),
                        session.updated_at,
                    );
                }
                Err(error) => println!("{} {}（{error}）", "损坏会话：".red(), id),
            }
        }
    }

    pub(crate) async fn resume(&mut self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let session = self.store.load(id)?;
        if session.workspace != self.workspace {
            println!(
                "{} 会话工作区 {} 与当前 {} 不同",
                "警告：".yellow(),
                display_path(&session.workspace),
                display_path(&self.workspace)
            );
        }
        self.session = Some(session);
        println!("{}", format!("已恢复会话：{id}").green());
        Ok(())
    }

    pub(crate) fn show_diff(&self) {
        let Some(session) = &self.session else {
            println!("暂无会话");
            return;
        };
        let diffs = session.diff();
        if diffs.is_empty() {
            println!("{}", "当前会话没有未回滚的文件改动".dimmed());
            return;
        }
        for diff in diffs {
            println!("{} {}", "●".cyan(), diff.path.bold());
            if let Some(before) = &diff.before {
                for line in before.lines() {
                    println!("{} {}", "-".red(), line);
                }
            } else {
                println!("{}", "(新建文件)".green());
            }
            if let Some(after) = &diff.after {
                for line in after.lines() {
                    println!("{} {}", "+".green(), line);
                }
            } else {
                println!("{}", "(已删除)".red());
            }
        }
    }

    pub(crate) async fn undo(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(session) = &mut self.session else {
            println!("暂无会话");
            return Ok(());
        };
        let restored = session.revert().await?;
        if restored.is_empty() {
            println!("{}", "没有可回滚的改动".dimmed());
        } else {
            println!("{} {}", "已回滚：".green(), restored.join(", "));
        }
        self.store.save(session)?;
        Ok(())
    }

    pub(crate) fn show_status(&self) {
        println!("工作区：{}", display_path(&self.workspace));
        println!("模型：{}", self.model);
        println!(
            "模式：{}",
            if self.read_only {
                "plan（只读）".yellow()
            } else {
                "build".green()
            }
        );
        match &self.session {
            Some(session) => {
                println!("会话：{}", session.id);
                println!("消息数：{}", session.messages.len());
                println!("未回滚改动：{}", session.diff().len());
            }
            None => println!("会话：{}", "无（输入任务时自动创建）".dimmed()),
        }
        let audit_count = match self.agent.audit_log().lock() {
            Ok(guard) => guard.entries.len(),
            Err(_) => 0,
        };
        println!("审计记录：{audit_count} 条");
    }

    pub(crate) fn show_permissions(&self) {
        println!("{}", "权限档位（/permissions set <档位> 切换）：".bold());
        println!("  当前：{}", self.agent.permission_profile().label().cyan());
        println!("  read_only    只允许宿主验证的只读操作");
        println!("  workspace    工作区内普通读写自动允许；执行/联网/UI/越界/破坏性询问");
        println!("  auto_review  审批 Agent 可收紧或代批可代批操作");
        println!("  full_access  减少询问，不绕过审计/脱敏/不可逆操作确认");
        println!("  custom       按工具、路径、主机和时效组合规则");
        println!(
            "{}",
            "规则：deny 优先 → 档位 → 授权记忆（grant）→ ask 审批".dimmed()
        );
        println!("  read（read_file/list_dir/search_files）：作用域内自动放行");
        println!(
            "  write（write_file）：工作区内自动放行（workspace 档）；越界拒绝，可 /undo 回滚"
        );
        println!("  execute（run_command）：默认审批，危险命令直接拒绝，60s 超时");
        // §11：临时授权视图——当前 grant、过期时间、剩余次数与撤销入口。
        match self.agent.policy().grant_store() {
            Some(store) => {
                let pruned = store.prune_expired();
                let grants = store.list();
                if grants.is_empty() {
                    println!(
                        "{}",
                        if pruned > 0 {
                            format!("临时授权：无（本次清理 {pruned} 条过期项）")
                        } else {
                            "临时授权：无".to_string()
                        }
                        .dimmed()
                    );
                } else {
                    println!(
                        "{}",
                        format!(
                            "临时授权（{n} 条；/permissions revoke <id> 撤销）：",
                            n = grants.len()
                        )
                        .bold()
                    );
                    for grant in grants {
                        let mut parts: Vec<String> = vec![grant.tool_id.clone()];
                        if let Some(scope) = &grant.path_scope {
                            parts.push(format!("路径≤{}", scope.display()));
                        }
                        if let Some(host) = &grant.host_scope {
                            parts.push(format!("主机={host}"));
                        }
                        if let Some(expires_at) = grant.expires_at {
                            let seconds = (expires_at - chrono::Utc::now()).num_seconds();
                            parts.push(if seconds > 0 {
                                format!("约 {seconds}s 后过期")
                            } else {
                                "已过期（待清理）".to_string()
                            });
                        }
                        if let Some(uses) = grant.remaining_uses {
                            parts.push(format!("剩余 {uses} 次"));
                        }
                        let id_head = grant
                            .grant_id
                            .get(..8)
                            .unwrap_or(&grant.grant_id)
                            .to_string();
                        println!("  {} {}（{}）", "·".dimmed(), id_head, parts.join("；"));
                    }
                }
            }
            None => println!("{}", "临时授权：本会话未启用授权记忆".dimmed()),
        }
        if self.read_only || self.agent.permission_profile() == PermissionProfile::ReadOnly {
            println!("{}", "当前只读：写/执行/注入一律拒绝".yellow());
        }
    }

    pub(crate) fn handle_permissions(
        &mut self,
        command: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let rest = command.trim_start_matches("permissions").trim_start();
        let mut parts = rest.split_whitespace();
        match parts.next() {
            Some("set") => {
                let profile = parts.next().ok_or(
                    "用法：/permissions set <read_only|workspace|auto_review|full_access|unrestricted|custom> [--yes]",
                )?;
                let yes = parts.any(|part| part == "--yes");
                if !crate::support::confirm_high_risk_profile(profile, yes) {
                    println!(
                        "{}",
                        "已取消（管道模式切换 unrestricted 需显式追加 --yes）".yellow()
                    );
                    return Ok(());
                }
                let parsed = PermissionProfile::parse(profile).ok_or_else(|| {
                    format!("未知档位：{profile}（可选 read_only / workspace / auto_review / full_access / unrestricted / custom）")
                })?;
                self.agent.set_permission_profile(parsed);
                if parsed == PermissionProfile::ReadOnly {
                    self.settings.read_only = true;
                } else {
                    self.settings.read_only = false;
                    self.settings.permission_profile = Some(parsed.label().to_string());
                }
                self.settings.save(&self.workspace)?;
                println!(
                    "{} 已切换权限档位：{}（已写入 settings.json）",
                    "✓".green(),
                    parsed.label().cyan()
                );
                Ok(())
            }
            Some("revoke") => {
                // §11：撤销入口——按授权 id（或工具 id）撤销临时授权。
                let Some(id) = parts.next() else {
                    println!("用法：/permissions revoke <授权id 或 工具id>");
                    return Ok(());
                };
                let Some(store) = self.agent.policy().grant_store() else {
                    println!("本会话未启用授权记忆，无授权可撤销");
                    return Ok(());
                };
                let by_id = store.revoke(id);
                let by_tool = if by_id { 0 } else { store.revoke_tool(id) };
                if by_id || by_tool > 0 {
                    println!(
                        "{} 已撤销 {id}（命中授权 {} 条）",
                        "✓".green(),
                        if by_id { 1 } else { by_tool }
                    );
                } else {
                    println!("未找到匹配的授权：{id}（/permissions 查看现有授权）");
                }
                Ok(())
            }
            Some(other) => {
                println!("未知子命令：{other}（用法：/permissions set <档位> 或 /permissions revoke <id>）");
                Ok(())
            }
            None => {
                self.show_permissions();
                Ok(())
            }
        }
    }

    pub(crate) fn show_audit(&self) {
        let audit = self.agent.audit_log();
        let Ok(audit) = audit.lock() else {
            return;
        };
        if audit.entries.is_empty() {
            println!("暂无审计记录");
            return;
        }
        for entry in audit.entries.iter().rev().take(20) {
            let tag = match entry.event.as_str() {
                "permission" => format!(
                    "[审批 {}]",
                    entry.approved.map(|v| v.to_string()).unwrap_or_default()
                ),
                "tool_call" => "[工具]".to_string(),
                _ => format!("[{}]", entry.event),
            };
            println!(
                "{} {} {} {}",
                tag.dimmed(),
                entry.tool.as_deref().unwrap_or("").cyan(),
                entry.detail.dimmed(),
                entry.ts.dimmed()
            );
        }
    }

    pub(crate) fn show_whitelist(&self) {
        println!(
            "{}",
            format!("应用白名单（{} 项）：", self.whitelist.entries().len()).bold()
        );
        for entry in self.whitelist.entries() {
            println!(
                "  {} {}（{}）操作={} 学习={} 敏感={}",
                entry.app_id,
                entry.name,
                entry.tier.label(),
                if entry.auto_ops_allowed {
                    "允许".green()
                } else {
                    "禁止".red()
                },
                if entry.learn_allowed {
                    "允许".green()
                } else {
                    "禁止".red()
                },
                entry.sensitive
            );
        }
    }

    pub(crate) fn show_perception(&self) {
        let snapshot = self.perception.snapshot();
        println!("{}", "当前情景快照（按权限过滤）：".bold());
        println!(
            "  {}",
            serde_json::to_string_pretty(&snapshot).unwrap_or_default()
        );
    }

    pub(crate) fn handle_learn(&mut self, command: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut parts = command.split_whitespace();
        parts.next(); // "learn"
        match parts.next() {
            Some("start") => {
                self.learn.start();
                println!(
                    "{}",
                    "开始录制示范操作（Ctrl+C 前的操作会保留在当前样本）".green()
                );
            }
            Some("pause") => {
                self.learn.pause();
                println!("录制已暂停");
            }
            Some("resume") => {
                self.learn.resume();
                println!("录制已恢复");
            }
            Some("stop") => {
                let samples = self.learn.stop();
                println!("录制结束，共 {} 条动作样本", samples.len());
            }
            Some("clear") => {
                self.learn.clear();
                println!("已清空本次样本");
            }
            Some("status") | None => {
                println!(
                    "状态：{:?} ｜ 样本：{} ｜ 敏感面熔断：{}",
                    self.learn.state(),
                    self.learn.samples(),
                    self.learn.sensitive_break()
                );
            }
            Some(other) => println!("未知子命令：{other}（start/pause/resume/stop/clear/status）"),
        }
        Ok(())
    }

    pub(crate) fn handle_proactive(
        &mut self,
        command: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut parts = command.split_whitespace();
        parts.next(); // "proactive"
        match parts.next() {
            Some("status") | None => {
                let suggestions = self.proactive.suggestions();
                if suggestions.is_empty() {
                    println!("暂无主动建议（默认仅提示，不执行）");
                } else {
                    for suggestion in suggestions {
                        println!(
                            "  [{}] {} ｜ {}",
                            suggestion.id, suggestion.app_id, suggestion.summary
                        );
                    }
                }
            }
            Some("observe") => {
                let app_id = parts
                    .next()
                    .ok_or("用法：/proactive observe <应用ID> <动作序列>")?;
                let actions: Vec<String> = parts
                    .flat_map(|part| part.split(','))
                    .map(|action| action.trim().to_string())
                    .filter(|action| !action.is_empty())
                    .collect();
                if actions.is_empty() {
                    return Err("动作序列不能为空".into());
                }
                match self.proactive.observe(app_id, actions) {
                    Some(suggestion) => println!(
                        "{} [{}] {}",
                        "建议：".yellow(),
                        suggestion.id,
                        suggestion.summary
                    ),
                    None => println!("未达到建议阈值"),
                }
            }
            Some("decide") => {
                let id = parts
                    .next()
                    .ok_or("用法：/proactive decide <建议ID> <learn|execute|ignore|mute>")?;
                let action = match parts.next() {
                    Some("learn") => SuggestionAction::Learn,
                    Some("execute") => SuggestionAction::ExecuteOnce,
                    Some("ignore") => SuggestionAction::Ignore,
                    Some("mute") => SuggestionAction::MuteForever,
                    _ => return Err("动作需为 learn/execute/ignore/mute".into()),
                };
                self.proactive.decide(id, action)?;
                println!("已处理建议 {id}");
            }
            Some(other) => println!("未知子命令：{other}（status/observe/decide）"),
        }
        Ok(())
    }

    pub(crate) async fn run_at_subagent(
        &mut self,
        prompt: &str,
        read_only: bool,
        allow_commands: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let workspace = self.workspace.clone();
        let model = self.model.clone();
        let agent = Arc::clone(&self.agent);
        let text = agent
            .run_subagent_with_options(&workspace, &model, prompt, read_only, allow_commands)
            .await?;
        println!(
            "{} {}",
            if read_only {
                "探索结果：".cyan()
            } else if allow_commands {
                "子代理结果（含命令执行）：".green()
            } else {
                "子代理结果：".green()
            },
            text
        );
        Ok(())
    }
}
