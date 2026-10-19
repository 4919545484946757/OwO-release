//! `owo-agent trust`：查看/授予/撤销工作区信任（审查 N1）。
//!
//! 信任记录写在数据根 `trust.json`（`OWO_AGENT_DATA` 或 `%LOCALAPPDATA%\OwO\Agent`），
//! 与桌面端 `GET/POST /workspace/trust` 共用同一份数据。
//!
//! 用法：
//! - `owo-agent trust`：授予信任（记录当前配置指纹）；
//! - `owo-agent trust --status`：只查看状态与将加载的危险配置；
//! - `owo-agent trust --revoke`：撤销信任（hooks 等下次启动不再加载）。

use clap::Args;
use std::path::PathBuf;

use crate::ui_output::OutputMode;

#[derive(Args)]
pub(crate) struct TrustArgs {
    /// 工作区目录（缺省当前目录）
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    /// 只查看状态，不修改
    #[arg(long)]
    status: bool,
    /// 撤销信任
    #[arg(long)]
    revoke: bool,
    /// 数据根覆盖（缺省 OWO_AGENT_DATA / %LOCALAPPDATA%\OwO\Agent）
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

pub(crate) fn run_trust_cmd(
    args: TrustArgs,
    output: OutputMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = args.workspace.canonicalize()?;
    let root = args
        .data_dir
        .unwrap_or_else(owo_agent_core::trust::default_data_root);
    let store = owo_agent_core::trust::TrustStore::new(root);
    let status = if args.status {
        store.status(&workspace)
    } else if args.revoke {
        store.revoke(&workspace)?
    } else {
        store.trust(&workspace)?
    };
    // `--output plain|jsonl`：stdout 只放机器可读状态（与 turn/capabilities 同口径），
    // 便于脚本与编排消费，例如 CI 里 `owo-agent --output jsonl trust --status`。
    match output {
        OutputMode::Jsonl => {
            println!("{}", serde_json::to_string(&status)?);
            return Ok(());
        }
        OutputMode::Plain => {
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(());
        }
        OutputMode::Human => {}
    }
    print_status(&status);
    if status.trusted {
        println!("已信任：hooks / 权限配置即时生效；工作区插件 MCP 在核心下次启动时加载。");
    } else if status.gated {
        println!("未信任（受限模式）：工作区 hooks / 插件 MCP / 权限放宽 / 白名单不会加载。");
    }
    Ok(())
}

fn print_status(status: &owo_agent_core::trust::TrustStatus) {
    println!("工作区：{}", status.workspace);
    println!(
        "信任：{}",
        match (status.trusted, status.gated) {
            (true, _) => "已信任",
            (false, true) => "未信任（受限模式）",
            (false, false) => "未信任（无自动执行配置，不受限）",
        }
    );
    if status.changed {
        println!("注意：配置内容已变化，需要重新确认（owo-agent trust）。");
    }
    if status.reasons.is_empty() {
        println!("危险配置：无");
    } else {
        println!("危险配置：");
        for reason in &status.reasons {
            println!("  - {reason}");
        }
    }
    println!("配置指纹：{}", status.fingerprint);
}
