// §12.3 CLI 拆分批次二：plugins 子命令域（自 main.rs 机械外移，零行为变化）。
// 签名/扫描/回滚由 core PluginManager 提供；远端拉取走 HTTP 面。

use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args)]
pub(crate) struct PluginArgs {
    #[command(subcommand)]
    action: PluginAction,
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    /// 数据目录（默认 OWO_AGENT_DATA 或 %LOCALAPPDATA%\OwO\Agent）
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// 远端市场 URL（预留；当前实现为本地离线模式）
    #[arg(long)]
    url: Option<String>,
    /// 离线模式（默认 true；不联网）
    #[arg(long)]
    offline: bool,
}
#[derive(Subcommand)]
pub(crate) enum PluginAction {
    /// 列出本地插件目录
    Catalog,
    /// 校验插件目录（签名/扫描/版本）
    Check { dir: PathBuf },
    /// 校验插件目录（与 check 相同）
    Verify { dir: PathBuf },
    /// 校验 Agent Plugins 1.0.0 规范包（plugin.json + skills/ + mcp.json）
    Validate { dir: PathBuf },
    /// 安装插件目录
    Install { dir: PathBuf },
    /// 更新已安装插件（id 为已安装插件 id）
    Update { id: String, dir: PathBuf },
    /// 卸载插件
    Uninstall { id: String },
}
/// 校验 Agent Plugins 1.0.0 规范包并打印逐组件结果。
///
/// 规范语义：清单非法 = 整个插件拒绝（exit 1）；组件级问题只降级并列出。
fn validate_agent_plugin(dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use owo_agent_core::agent_plugins::{
        load_agent_plugin, DiagnosticLevel, AGENT_PLUGINS_SPEC_VERSION,
    };
    match load_agent_plugin(dir, None) {
        Ok(loaded) => {
            println!(
                "Agent Plugins {AGENT_PLUGINS_SPEC_VERSION} 规范包：{} v{}",
                loaded.id(),
                loaded.manifest.version.as_deref().unwrap_or("(未声明)")
            );
            println!("  技能 {} 个：", loaded.skills.len());
            for skill in &loaded.skills {
                println!("    {} —— {}", skill.name, skill.description);
            }
            println!("  MCP 服务器 {} 个：", loaded.mcp_servers.len());
            for server in &loaded.mcp_servers {
                println!(
                    "    {}（{}）{}",
                    server.key,
                    server.transport.as_str(),
                    server
                        .config
                        .url
                        .as_deref()
                        .unwrap_or(server.config.command.as_str())
                );
            }
            println!("  hooks {} 条：", loaded.hooks.len());
            for hook in &loaded.hooks {
                println!(
                    "    {}（matcher={}，fail_mode={}）← {}",
                    hook.event,
                    hook.matcher.as_deref().unwrap_or("*"),
                    hook.fail_mode.as_deref().unwrap_or("open"),
                    hook.script.display()
                );
            }
            let warnings: Vec<_> = loaded
                .diagnostics
                .iter()
                .filter(|item| item.level >= DiagnosticLevel::Warning)
                .collect();
            if warnings.is_empty() {
                println!("  诊断：无（符合 Agent Plugins {AGENT_PLUGINS_SPEC_VERSION}）");
            } else {
                println!("  诊断 {} 条：", warnings.len());
                for diagnostic in warnings {
                    println!("    {diagnostic}");
                }
            }
            Ok(())
        }
        Err(error) => {
            println!("校验失败：{error}");
            std::process::exit(1);
        }
    }
}

/// 插件市场治理（本地离线模式）：catalog/check/install/update/uninstall/verify。
/// 签名/扫描/回滚由 core PluginManager 提供；远端拉取走 HTTP 面（POST /plugins/market/refresh）。
pub(crate) fn run_plugin(args: PluginArgs) -> Result<(), Box<dyn std::error::Error>> {
    use owo_agent_core::plugin::{
        discover_plugins, scan_plugin_for_risks, PluginManager, PluginManifest,
    };

    let workspace = args.workspace;
    let data_root = if let Some(dir) = args.data_dir {
        dir
    } else if let Ok(env_dir) = std::env::var("OWO_AGENT_DATA") {
        std::path::PathBuf::from(env_dir)
    } else if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        std::path::PathBuf::from(local).join("OwO").join("Agent")
    } else {
        std::path::PathBuf::from("data")
    };
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let _ = &args.url; // 远端 URL 预留：当前为本地离线模式（--offline 恒真）。
    let _ = args.offline;

    match args.action {
        PluginAction::Catalog => {
            let plugins = discover_plugins(&workspace, &data_root);
            println!(
                "本地插件目录（workspace={} data={}）：",
                workspace.display(),
                data_root.display()
            );
            for (path, manifest) in &plugins {
                let base = path.parent().unwrap_or(path);
                let is_spec = path.file_name().and_then(|name| name.to_str())
                    == Some(owo_agent_core::agent_plugins::AGENT_PLUGINS_MANIFEST_FILE);
                // 规范包只扫可执行内容；清单元数据里的 $schema/homepage 不是网络请求。
                let risks = if is_spec {
                    owo_agent_core::agent_plugins::scan_agent_plugin_for_risks(base, &[])
                } else {
                    let manifest_content = std::fs::read_to_string(path).unwrap_or_default();
                    let entry_content = manifest
                        .entry
                        .as_ref()
                        .and_then(|entry| std::fs::read_to_string(base.join(entry)).ok());
                    scan_plugin_for_risks(
                        &manifest_content,
                        entry_content.as_deref(),
                        &manifest.network_allowlist,
                    )
                };
                let risk = if risks.is_empty() { "clean" } else { "RISK" };
                let format = if is_spec {
                    "agent-plugins-1.0.0"
                } else {
                    "owo-manifest"
                };
                println!(
                    "  {} v{}（{}）[{}][{}] 服务器 {} 个 / hooks {} 条{}",
                    manifest.id,
                    manifest.version,
                    manifest.name,
                    format,
                    risk,
                    manifest.all_mcp_servers().count(),
                    manifest.hooks.len(),
                    if risks.is_empty() {
                        String::new()
                    } else {
                        format!("：{}", risks.join("；"))
                    }
                );
            }
            if plugins.is_empty() {
                println!("  （无插件）");
            }
        }
        PluginAction::Check { dir } | PluginAction::Verify { dir } => {
            // Agent Plugins 1.0.0 规范包走规范校验（先校验清单，再发现组件）。
            if owo_agent_core::agent_plugins::is_agent_plugin_dir(&dir) {
                return validate_agent_plugin(&dir);
            }
            let manager = PluginManager::new(data_root.clone(), app_version);
            match manager.verify_plugin_dir(&dir) {
                Ok(report) => {
                    println!(
                        "校验通过：{} v{}（{:?}）",
                        report.id, report.version, report.state
                    );
                    for line in report.audit {
                        println!("  {line}");
                    }
                }
                Err(error) => {
                    println!("校验失败：{error}");
                    std::process::exit(1);
                }
            }
        }
        PluginAction::Validate { dir } => {
            return validate_agent_plugin(&dir);
        }
        PluginAction::Install { dir } => {
            let manager = PluginManager::new(data_root.clone(), app_version);
            match manager.install(&dir) {
                Ok(report) => {
                    println!(
                        "安装完成：{} v{}（{:?}）",
                        report.id, report.version, report.state
                    );
                    for line in report.audit {
                        println!("  {line}");
                    }
                }
                Err(error) => {
                    println!("安装失败：{error}");
                    std::process::exit(1);
                }
            }
        }
        PluginAction::Update { id, dir } => {
            let manager = PluginManager::new(data_root.clone(), app_version);
            let backup = data_root.join("plugins").join("backups");
            match manager.update(&dir, &backup) {
                Ok(report) => {
                    println!("更新完成：{id} → v{}（{:?}）", report.version, report.state);
                    for line in report.audit {
                        println!("  {line}");
                    }
                }
                Err(error) => {
                    println!("更新失败（已回滚或旧版保留）：{error}");
                    std::process::exit(1);
                }
            }
        }
        PluginAction::Uninstall { id } => {
            let manager = PluginManager::new(data_root.clone(), app_version);
            match manager.uninstall(&id) {
                Ok(audit_lines) => {
                    println!("已卸载 {id}");
                    for line in audit_lines {
                        println!("  {line}");
                    }
                }
                Err(error) => {
                    println!("卸载失败：{error}");
                    std::process::exit(1);
                }
            }
        }
    }
    let _ = PluginManifest::load; // 类型引用保活（防未使用告警变体依赖）。
    Ok(())
}
