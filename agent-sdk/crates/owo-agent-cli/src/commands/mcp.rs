//! `owo-agent mcp`：MCP 服务器管理的命令行入口（对标 Codex `codex mcp`）。
//!
//! 两条路径，行为对用户一致：
//! - **在线**：已有存活 Daemon（`owo_agent_client::connect` 不启动新进程）→ 走
//!   `/mcp/add|remove`，服务端热连接、审计并落配置（连接失败返回 502 且不写配置）；
//! - **离线**：无 Daemon → 直接读写 `<data>/mcp-servers.json`，并明确提示
//!   "下次启动核心生效"（不谎报已连接）。
//!
//! 名字规则与服务端共用 `owo_agent_core::validate_mcp_server_name`。

use std::path::PathBuf;

use clap::{Args, Subcommand};
use owo_agent_core::McpServerConfig;

use crate::support::{data_root, load_mcp_configs, mcp_config_path, save_mcp_configs};

#[derive(Debug, Args)]
pub struct McpArgs {
    /// 数据目录（默认取 OWO_AGENT_DATA / 系统数据根）
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    /// 机器可读输出（JSON）
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub action: McpAction,
}

#[derive(Debug, Subcommand)]
pub enum McpAction {
    /// 列出已配置的 MCP 服务器
    List,
    /// 查看单个服务器的完整配置
    Get { name: String },
    /// 新增服务器（stdio 需要 --command；http 需要 --url）
    Add {
        name: String,
        /// stdio 启动命令（如 python、npx）
        #[arg(long)]
        command: Option<String>,
        /// 逗号分隔的启动参数
        #[arg(long, value_delimiter = ',')]
        args: Vec<String>,
        /// stdio | http
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// http 传输端点
        #[arg(long)]
        url: Option<String>,
    },
    /// 移除服务器
    Remove { name: String },
}

pub(crate) fn run_mcp_cmd(args: McpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = data_root(args.data_dir.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_mcp(args, root))
}

async fn run_mcp(args: McpArgs, root: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    match args.action {
        McpAction::List => {
            // 在线时以服务端视图为准（含连接状态）；离线读配置文件。
            let configs = load_mcp_configs(&root);
            if args.json {
                println!("{}", serde_json::to_string_pretty(&configs)?);
                return Ok(());
            }
            if configs.is_empty() {
                println!("（未配置 MCP 服务器；用法：owo-agent mcp add <名> --command <命令>）");
                return Ok(());
            }
            println!(
                "已配置 MCP 服务器（{} 个，配置：{}）：",
                configs.len(),
                display(&mcp_config_path(&root))
            );
            for config in &configs {
                println!("  {}  {}", config.name, summarize(config));
            }
            Ok(())
        }
        McpAction::Get { name } => {
            let name = normalize_name(&name)?;
            let configs = load_mcp_configs(&root);
            let config = configs
                .iter()
                .find(|config| config.name == name)
                .ok_or_else(|| format!("未配置 MCP 服务器：{name}"))?;
            println!("{}", serde_json::to_string_pretty(config)?);
            Ok(())
        }
        McpAction::Add {
            name,
            command,
            args: command_args,
            transport,
            url,
        } => {
            let name = normalize_name(&name)?;
            let transport = transport.trim().to_ascii_lowercase();
            if !matches!(transport.as_str(), "stdio" | "http") {
                return Err(format!("不支持的 transport：{transport}（可用 stdio/http）").into());
            }
            let command = command.unwrap_or_default().trim().to_string();
            let url = url
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            if transport == "stdio" && command.is_empty() {
                return Err("stdio 传输需要 --command".into());
            }
            if transport == "http" && url.is_none() {
                return Err("http 传输需要 --url".into());
            }
            // 在线：交给服务端（热连接 + 审计 + 落配置，失败不写配置）。
            if let Some(client) = live_client(&root).await {
                let added = client
                    .mcp_add(&name, &transport, &command, &command_args, url.as_deref())
                    .await?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&added)?);
                } else {
                    println!(
                        "已新增 MCP 服务器：{}（已热连接，工具 {} 个）",
                        name,
                        added
                            .get("tools")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0)
                    );
                }
                return Ok(());
            }
            // 离线：写配置文件；不谎报"已连接"。
            let mut configs = load_mcp_configs(&root);
            if configs.iter().any(|config| config.name == name) {
                return Err(format!("MCP 服务器 {name} 已存在（先 remove 再 add）").into());
            }
            configs.push(McpServerConfig {
                name: name.clone(),
                transport: transport.clone(),
                command,
                args: command_args,
                url,
                timeout_ms: None,
                network_allowlist: Vec::new(),
                trusted_readonly: Vec::new(),
            });
            save_mcp_configs(&root, &configs);
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "ok": true,
                        "name": name,
                        "connected": false,
                        "applies": "next_core_start",
                        "config": display(&mcp_config_path(&root)),
                    }))?
                );
            } else {
                println!(
                    "已新增 MCP 服务器：{}（未运行核心：已写入 {}，下次启动核心生效）",
                    name,
                    display(&mcp_config_path(&root))
                );
            }
            Ok(())
        }
        McpAction::Remove { name } => {
            let name = normalize_name(&name)?;
            if let Some(client) = live_client(&root).await {
                let removed = client.mcp_remove(&name).await?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&removed)?);
                } else {
                    println!("已移除 MCP 服务器：{name}");
                }
                return Ok(());
            }
            let mut configs = load_mcp_configs(&root);
            let before = configs.len();
            configs.retain(|config| config.name != name);
            if configs.len() == before {
                return Err(format!("未配置 MCP 服务器：{name}").into());
            }
            save_mcp_configs(&root, &configs);
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "ok": true,
                        "name": name,
                        "applies": "next_core_start",
                    }))?
                );
            } else {
                println!(
                    "已移除 MCP 服务器：{}（未运行核心：已从配置移除，下次启动核心生效）",
                    name
                );
            }
            Ok(())
        }
    }
}

/// 归一 + 校验名字（与服务端 `/mcp/add` 同一实现）。
fn normalize_name(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    owo_agent_core::validate_mcp_server_name(name).map_err(|error| error.to_string().into())
}

/// 存活 Daemon（存在且 API 版本兼容）；不启动新进程。
async fn live_client(root: &std::path::Path) -> Option<owo_agent_client::AgentClient> {
    owo_agent_client::connect(root, Some(owo_build_info::API_VERSION))
        .await
        .ok()
}

fn summarize(config: &McpServerConfig) -> String {
    if config.transport == "http" {
        return format!("http {}", config.url.clone().unwrap_or_default());
    }
    // stdio：只显示命令本体；参数可能含令牌，列表视图不摊开（get 看全量）。
    format!("stdio {}（参数 {} 个）", config.command, config.args.len())
}

fn display(path: &std::path::Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("owo-mcp-cli-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn add_and_remove_round_trip_offline_config() {
        // 离线路径：无 Daemon 时直接读写 mcp-servers.json。
        let root = temp_root("round-trip");
        let mut configs = load_mcp_configs(&root);
        configs.push(McpServerConfig {
            name: "translate".to_string(),
            transport: "stdio".to_string(),
            command: "python".to_string(),
            args: vec!["server.py".to_string()],
            url: None,
            timeout_ms: None,
            network_allowlist: Vec::new(),
            trusted_readonly: Vec::new(),
        });
        save_mcp_configs(&root, &configs);

        let reloaded = load_mcp_configs(&root);
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].name, "translate");
        assert_eq!(reloaded[0].command, "python");
        assert!(summarize(&reloaded[0]).contains("stdio python（参数 1 个）"));
        assert!(mcp_config_path(&root).exists());

        save_mcp_configs(&root, &[]);
        assert!(load_mcp_configs(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn http_summary_shows_endpoint_without_args() {
        let config = McpServerConfig {
            name: "remote".to_string(),
            transport: "http".to_string(),
            command: String::new(),
            args: vec![],
            url: Some("https://example.test/mcp".to_string()),
            timeout_ms: None,
            network_allowlist: Vec::new(),
            trusted_readonly: Vec::new(),
        };
        assert_eq!(summarize(&config), "http https://example.test/mcp");
    }

    #[test]
    fn name_validation_is_shared_with_the_server_rule() {
        assert_eq!(normalize_name(" translate ").unwrap(), "translate");
        assert!(normalize_name("a__b").is_err());
        assert!(normalize_name("").is_err());
    }
}
