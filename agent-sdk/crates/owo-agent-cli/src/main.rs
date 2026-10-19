// §12.3 CLI 拆分完成态：main.rs 仅承载参数定义 + 子命令分派（每域一模块，经 commands:: 显式引用）。

use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod commands;
mod markdown;
mod product_eval_cmd;
mod support;
mod tui;
mod ui_output;
mod worker_child;

use crate::support::run_async;
use colored::Colorize;
use ui_output::{OutputMode, PermissionsProfile};

#[derive(Parser)]
#[command(
    name = "owo-agent",
    // §7.1：--version 与 /health.build、doctor、release manifest 同构
    // （单一来源 owo_build_info::identity()，含 commit/dirty/built_at/api）。
    // clap 4 derive 的 version 槽不接 String（Str: From<String> 不满足），
    // identity() 为进程级 OnceLock 缓存，leak 一次即常驻（量级 <1KB）。
    version = &*owo_build_info::identity().oneline().leak(),
    about = "OwO Agent SDK CLI（Codex 式 / OpenCode 式交互终端）"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    /// §11：输出模式（human|plain|jsonl）；plain/jsonl 支持 turn、capabilities、trust。
    #[arg(long, value_enum, global = true, default_value = "human")]
    output: OutputMode,
    /// §11：权限档案（default|read-only|trusted），支持 turn、默认 REPL 与 TUI。
    #[arg(long, value_enum, global = true, default_value = "default")]
    permissions: PermissionsProfile,
    /// 内部受控子进程协议入口（A1，主文档 §9.1）：stdout 仅承载 JSONL 协议。
    #[arg(long, hide = true)]
    owo_worker_child: bool,
    /// 与 --owo-worker-child 搭配：选择首期受限处理器（echo|sleep|fail）。
    #[arg(long, hide = true, requires = "owo_worker_child")]
    handler: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// 执行一轮一次性任务（非交互）
    Turn(commands::turn::TurnArgs),
    /// 启动本地 HTTP API 服务
    Serve(commands::serve::ServeArgs),
    /// Daemon 生命周期（P1：status/stop，经 discovery + 共享客户端）
    Daemon(commands::daemon::DaemonArgs),
    /// 进入交互式终端（默认命令）
    Repl(commands::repl::ReplArgs),
    /// 进入全屏 TUI（OpenCode 风格）
    Tui(tui::TuiArgs),
    /// 生成 AGENTS.md 项目规则文件
    Init(commands::serve::InitArgs),
    /// 运行评估套件（内置 demo 或自定义 JSON）
    Eval(commands::eval::EvalArgs),
    /// 产品评测底座（V1-R1）：validate/run/compare（固定任务集 × 重复 × 单/多对照）
    ProductEval(product_eval_cmd::ProductEvalArgs),
    /// 本机 IPC 往返延迟基准
    Bench(commands::bench::BenchArgs),
    /// 云端执行任务（M4a：提交/列表/状态/diff/应用/回滚）
    Cloud(commands::cloud::CloudArgs),
    /// 插件市场治理（M4b：catalog/check/install/update/uninstall/verify；本地离线模式）
    Plugin(commands::plugins::PluginArgs),
    /// MCP 服务器管理（对标 Codex `codex mcp`：list/get/add/remove；有核心则热连接）
    Mcp(commands::mcp::McpArgs),
    /// 审计链校验/导出（R6 audit_chain：verify 检出篡改，export 输出可离线校验的导出文件）
    Audit(commands::audit::AuditArgs),
    /// 数据备份（R9：zip 打包 index.db/settings/notes/skills/workflows，复用服务端备份逻辑）
    Backup(commands::backup::BackupArgs),
    /// 环境诊断（R10：数据目录/凭据/模型/端点/服务健康逐项检查）
    Doctor(commands::doctor::DoctorArgs),
    /// 受控 worker 子进程运维（A1：demo 演示真实子进程闭环）
    Worker(commands::worker::WorkerArgs),
    /// 工作区信任（N1：status/grant/revoke，未信任时不加载 hooks/插件/权限放宽）
    Trust(commands::trust::TrustArgs),
    /// 用户自定义多 Agent 工作流（团队模板的用户/社区通道：校验/CRUD/AI 草稿/社区包）
    Workflow(commands::workflow::WorkflowArgs),
    /// 逐 hunk 审阅工作区改动（G13：列出 hunk，按序号接受/拒绝）
    Review(commands::review::ReviewArgs),
    /// 列出会话（对标 Codex `codex resume` 选择器；标出 --continue 会选中的会话）
    Sessions(commands::sessions::SessionsArgs),
    /// 符号级检索（G14：按名字定位函数/结构体/类等定义；离线只读）
    Symbols(commands::symbols::SymbolsArgs),
    /// 功能目录（§8.3）：列出产品能力、成熟度与用户入口（UI/CLI/诊断页共同来源）
    Capabilities,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    validate_cli_options(&cli)?;

    // A1：受控子进程协议入口最先分流——在任何日志/运行时初始化之前进入协议循环，
    // 保证 stdout 只承载 JSONL（ready/task/pong/result），人类可读输出只走 stderr。
    if cli.owo_worker_child {
        let handler = match cli.handler.as_deref() {
            Some(name) => worker_child::ChildHandler::parse_name(name)?,
            None => worker_child::ChildHandler::Echo,
        };
        eprintln!(
            "owo-worker-child: handler={} pid={}",
            handler.as_str(),
            std::process::id()
        );
        worker_child::run_child(handler);
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    match cli.command {
        None => run_async(commands::repl::Repl::run(
            commands::repl::ReplArgs {
                workspace: PathBuf::from("."),
                model: None,
                agent: "build".to_string(),
                no_approval: false,
                data_dir: None,
                local: false,
            },
            cli.permissions,
        ))?,
        Some(Commands::Turn(args)) => {
            let code = run_async(commands::turn::run_turn(args, cli.output, cli.permissions))?;
            if code != 0 {
                std::process::exit(i32::from(code));
            }
        }
        Some(Commands::Serve(args)) => run_async(commands::serve::run_serve(args))?,
        Some(Commands::Daemon(args)) => run_async(commands::daemon::run_daemon_cmd(args))?,
        Some(Commands::Repl(args)) => run_async(commands::repl::Repl::run(args, cli.permissions))?,
        Some(Commands::Tui(args)) => tui::run(args, cli.permissions)?,
        Some(Commands::Init(args)) => commands::serve::run_init(args)?,
        Some(Commands::Eval(args)) => run_async(commands::eval::run_eval(args))?,
        Some(Commands::ProductEval(args)) => run_async(product_eval_cmd::run(args))?,
        Some(Commands::Bench(args)) => run_async(commands::bench::run_bench(args))?,
        Some(Commands::Cloud(args)) => run_async(commands::cloud::run_cloud(args))?,
        Some(Commands::Plugin(args)) => commands::plugins::run_plugin(args)?,
        Some(Commands::Mcp(args)) => commands::mcp::run_mcp_cmd(args)?,
        Some(Commands::Audit(args)) => commands::audit::run_audit_cmd(args)?,
        Some(Commands::Backup(args)) => commands::backup::run_backup_cmd(args)?,
        Some(Commands::Doctor(args)) => run_async(commands::doctor::run_doctor_cmd(args))?,
        Some(Commands::Capabilities) => commands::capabilities::run_capabilities_cmd(cli.output)?,
        Some(Commands::Worker(args)) => run_async(commands::worker::run_worker_cmd(args))?,
        Some(Commands::Trust(args)) => commands::trust::run_trust_cmd(args, cli.output)?,
        Some(Commands::Workflow(args)) => commands::workflow::run_workflow_cmd(args)?,
        Some(Commands::Review(args)) => commands::review::run_review_cmd(args)?,
        Some(Commands::Sessions(args)) => commands::sessions::run_sessions_cmd(args)?,
        Some(Commands::Symbols(args)) => commands::symbols::run_symbols_cmd(args)?,
    }
    Ok(())
}

fn print_help() {
    println!("{}", "── 命令 ──".bold());
    println!("  直接输入文字        向当前 Agent 发起任务");
    println!("  @explore <问题>     直呼只读探索子代理");
    println!("  @subagent <任务>    直呼通用子代理");
    println!("  /new [模型]         新建会话");
    println!("  /sessions           列出会话");
    println!("  /resume <id>        恢复会话");
    println!("  /fork [消息序号]     在指定消息处创建子会话");
    println!("  /rewind <条数>      回退会话历史（文件改动一并撤销）");
    println!("  /redo               恢复最近一次 rewind");
    println!("  /undo-msg [n]       撤销最近 n 条对话消息");
    println!("  /redo-msg           恢复最近一次消息撤销");
    println!("  /tree               查看会话树");
    println!("  /share [html]       导出会话分享（Markdown/HTML）");
    println!("  /traces | /trace <n>  列出/回放回合轨迹");
    println!("  /settings           查看工作区 settings.json 配置");
    println!("  /plugins            列出已加载插件");
    println!("  /model [名称]       查看/切换模型");
    println!("  /plan | /build      切换只读规划模式 / 执行模式");
    println!("  /agent [build|plan] 查看/切换 Agent 模式（等价 /build、/plan）");
    println!("  /diff               查看本次会话文件改动");
    println!("  /undo               回滚本次会话全部写操作");
    println!("  /status             查看工作区/模型/会话状态");
    println!("  /permissions [set <read_only|workspace|auto_review|full_access|unrestricted|custom>]  查看/切换权限档位（unrestricted=越界+命令执行，高风险）");
    println!("  /approvals [clear]  查看/清除本会话「总是允许」的工具（--local）");
    println!("  /compact            立即压缩会话历史（省 token）");
    println!("  /goal [目标|status|clear]  目标模式：未完成时持续自动推进，模型标记完成才停");
    println!("  /todo               查看会话任务清单（todo 工具维护）");
    println!("  /review [提示]      发起一次「审查工作区改动」的回合");
    println!("  /mention <路径>     展示文件引用信息（字节/行数）");
    println!("  /history [n]        查看最近输入历史");
    println!("  /editor             用 $VISUAL/$EDITOR 写多行输入");
    println!("  /login | /logout    查看凭据来源 / 登出说明");
    println!("  /debug              诊断信息（工作区/模型/MCP/trace）");
    println!("  /audit              查看最近审计记录");
    println!("  /mcp add|list|remove  管理 MCP 服务器");
    println!("  /skills [reload]    列出/重载已加载技能");
    println!("  /whitelist          查看应用白名单（v0.4）");
    println!("  /perception         查看当前情景快照（v0.4）");
    println!("  /learn <start|pause|resume|stop|clear|status>  示范学习录制（v0.4）");
    println!("  /proactive <status|observe|decide>  主动建议（v0.4）");
    println!("  /init               生成 AGENTS.md");
    println!("  /abort              中止当前回合");
    println!("  /clear              清屏");
    println!("  /exit | /quit       退出");
}

#[cfg(test)]
mod tests {
    /// §7.1 冻结：`--version` 行（= identity().oneline()）必须携带 release
    /// manifest 解析所依赖的全部字段键；缺任一字段，发布链交叉核对会瞎。
    #[test]
    fn version_line_carries_full_identity_fields() {
        let line = owo_build_info::identity().oneline();
        for key in ["api=", "commit=", "dirty=", "built_at=", "source="] {
            assert!(line.contains(key), "--version 行缺 {key}：{line}");
        }
        // 版本号打头（release-artifact-manifest.ps1 按空格取第二段）。
        let first = line.split(' ').next().unwrap_or_default();
        assert!(
            first.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "首段应为版本号：{line}"
        );
    }
}

fn validate_cli_options(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    if cli.owo_worker_child {
        return Ok(());
    }
    if !matches!(cli.output, OutputMode::Human)
        && !matches!(
            cli.command.as_ref(),
            Some(Commands::Turn(_)) | Some(Commands::Capabilities) | Some(Commands::Trust(_))
        )
    {
        return Err(
            "plain/jsonl 当前仅支持 turn、capabilities 和 trust；该命令不支持机器输出，已拒绝执行"
                .into(),
        );
    }
    if !matches!(cli.permissions, PermissionsProfile::Default) {
        match cli.command.as_ref() {
            None | Some(Commands::Turn(_)) | Some(Commands::Tui(_)) => {}
            Some(Commands::Repl(args)) if !args.local => {}
            _ => {
                return Err(
                    "该命令不支持请求级 permissions；请使用默认 Daemon REPL、TUI 或 turn".into(),
                )
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod cli_options_contract_tests {
    use super::*;
    #[test]
    fn unsupported_global_modes_are_rejected_before_any_runtime_or_storage_access() {
        for arguments in [
            vec!["owo-agent", "--output", "jsonl", "tui"],
            vec!["owo-agent", "--permissions", "read-only", "serve"],
            vec!["owo-agent", "--permissions", "read-only", "repl", "--local"],
        ] {
            let cli = Cli::try_parse_from(arguments).unwrap();
            assert!(validate_cli_options(&cli).is_err());
        }
        for arguments in [
            vec![
                "owo-agent",
                "--permissions",
                "read-only",
                "turn",
                "--prompt",
                "fixture",
            ],
            vec!["owo-agent", "--permissions", "read-only", "repl"],
            vec!["owo-agent", "--permissions", "read-only", "tui"],
            vec![
                "owo-agent",
                "--output",
                "jsonl",
                "turn",
                "--prompt",
                "fixture",
            ],
            vec!["owo-agent", "--output", "jsonl", "trust", "--status"],
        ] {
            let cli = Cli::try_parse_from(arguments).unwrap();
            assert!(validate_cli_options(&cli).is_ok());
        }
    }

    #[test]
    fn turn_session_flags_are_explicit_and_mutually_exclusive() {
        // 续接必须显式：--session 与 --continue 不能同时给（避免"续哪个"含糊）。
        assert!(Cli::try_parse_from([
            "owo-agent",
            "turn",
            "--prompt",
            "fixture",
            "--session",
            "sess-1",
            "--continue",
        ])
        .is_err());

        // 默认（新建会话）与两种续接方式都能正常解析。
        for arguments in [
            vec!["owo-agent", "turn", "--prompt", "fixture"],
            vec![
                "owo-agent",
                "turn",
                "--prompt",
                "fixture",
                "--session",
                "sess-1",
            ],
            vec!["owo-agent", "turn", "--prompt", "fixture", "--continue"],
            vec!["owo-agent", "turn", "--prompt", "fixture", "--last"],
        ] {
            let cli = Cli::try_parse_from(arguments).unwrap();
            assert!(validate_cli_options(&cli).is_ok());
            assert!(matches!(cli.command, Some(Commands::Turn(_))));
        }
    }
}
