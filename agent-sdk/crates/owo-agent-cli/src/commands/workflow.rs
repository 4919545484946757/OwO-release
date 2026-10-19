//! `owo-agent workflow`：用户自定义多 Agent 工作流的命令行入口（团队模板的用户/社区通道）。
//!
//! 子命令覆盖全链路：
//! - `validate <文件>` / `review <文件>`：**本地离线**校验（core 校验器 + 评审器，不必起 Daemon）；
//! - `list` / `show <id>`：用户/社区工作流；
//! - `create <文件>` / `delete <id>`：写入/删除（经 Daemon，服务端先校验再落盘）；
//! - `draft --description ... [--constraints ...] [--refine <草稿id>]`：AI 生成候选；
//! - `drafts` / `adopt <草稿id>` / `discard <草稿id>`：草稿生命周期；
//! - `export <id> [--out 文件]` / `import <文件> [--review-only]`：社区包分享。

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use colored::Colorize;
use owo_agent_core::user_workflow::{
    review_workflow_template, validate_workflow_template, workflow_review_blocked,
};
use owo_agent_protocol::TeamTemplate;

use crate::support::{ensure_daemon_client, ensure_data_root};

#[derive(Debug, Args)]
pub struct WorkflowArgs {
    /// 工作区（默认当前目录）；用于发现/启动共享 Daemon 与本地校验
    #[arg(long, global = true, default_value = ".")]
    pub workspace: PathBuf,
    /// 数据根（缺省用户级数据目录）
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    /// 机器可读输出（JSON）
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub action: WorkflowAction,
}

#[derive(Debug, Subcommand)]
pub enum WorkflowAction {
    /// 列出用户/社区工作流（--origin 可按来源过滤）
    List {
        /// 按来源过滤：user | community | all（缺省 all）
        #[arg(long, default_value = "all")]
        origin: String,
    },
    /// 查看单个工作流（含校验与评审发现）
    Show { id: String },
    /// 本地校验配置文件（离线，不需要 Daemon）
    Validate { file: PathBuf },
    /// 本地评审配置文件：能力面/写范围/MCP 依赖/人工节点（离线）
    Review { file: PathBuf },
    /// 从配置文件创建/更新用户工作流（经 Daemon 校验并落盘）
    Create { file: PathBuf },
    /// 删除用户/社区工作流
    Delete { id: String },
    /// 用自然语言让 AI 生成工作流草稿（需要配置模型）
    Draft {
        #[arg(long)]
        description: String,
        #[arg(long)]
        constraints: Option<String>,
        /// 基于既有草稿迭代
        #[arg(long)]
        refine: Option<String>,
        /// 多轮收敛：>1 时走 /workflows/draft/converge（最多 5 轮，错误反馈驱动）
        #[arg(long)]
        rounds: Option<usize>,
        /// 基于已保存工作流迭代：注入它的真实运行证据（成败/需要修复/最近失败）
        #[arg(long)]
        from: Option<String>,
    },
    /// 列出 AI 草稿
    Drafts,
    /// 采纳草稿 → 用户工作流
    Adopt {
        draft_id: String,
        /// 采纳后沿用基线工作流的目标再跑一次，打印前后对比（改配置是否真的康复）
        #[arg(long)]
        verify: bool,
        /// 验证运行后跟随到终态
        #[arg(long)]
        follow: bool,
        /// 跟随的最大秒数（缺省 120，硬顶 600）
        #[arg(long)]
        follow_timeout: Option<u64>,
    },
    /// 丢弃草稿
    Discard { draft_id: String },
    /// 导出社区包（默认写到 <id>.owowf.json）
    Export {
        id: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// 导入社区包（--review-only 只评审不落盘）
    Import {
        file: PathBuf,
        #[arg(long)]
        review_only: bool,
    },
    /// 工作流使用统计（运行次数/成败/最后运行；便于识别常用与总是失败的工作流）
    Stats {
        /// 按来源过滤：user | community | all（缺省 all）
        #[arg(long, default_value = "all")]
        origin: String,
        /// 排序：runs（默认）| failed | success_rate | last
        #[arg(long)]
        sort: Option<String>,
        /// asc（缺省 desc）
        #[arg(long)]
        order: Option<String>,
        /// 最多显示多少条（缺省全部，硬顶 200）
        #[arg(long)]
        limit: Option<usize>,
        /// 把统计写到文件（JSON，便于团队复盘）
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// 社区来源清单：导出我的来源（或 --verify 对照清单核对本地导入）
    Provenance {
        /// 把清单写到文件（缺省打印摘要）
        #[arg(long)]
        out: Option<PathBuf>,
        /// 对照清单核对本地：本地缺失或 sha256 不一致即为问题
        #[arg(long)]
        verify: Option<PathBuf>,
        /// 核对有问题时以退出码 4 失败（CI/交接门禁）
        #[arg(long)]
        strict: bool,
    },
    /// 用这条工作流创建团队运行（有 warn 级能力面时需 --yes 确认）
    Run {
        id: String,
        #[arg(long)]
        objective: String,
        /// 确认已知情（跳过 warn 级发现的确认门）
        #[arg(long)]
        yes: bool,
        /// 建队后跟随进度直到终态（打印团队事件）
        #[arg(long)]
        follow: bool,
        /// 跟随的最大秒数（缺省 120，硬顶 600）
        #[arg(long)]
        follow_timeout: Option<u64>,
    },
    /// 沿用最近一次运行的目标再跑一次（失败复现；--objective 可覆盖）
    Rerun {
        id: String,
        /// 覆盖上次的目标（缺省沿用最近一次运行的 objective）
        #[arg(long)]
        objective: Option<String>,
        /// 确认已知情（跳过 warn 级发现的确认门）
        #[arg(long)]
        yes: bool,
        /// 建队后跟随进度直到终态（打印团队事件）
        #[arg(long)]
        follow: bool,
        /// 跟随的最大秒数（缺省 120，硬顶 600）
        #[arg(long)]
        follow_timeout: Option<u64>,
    },
    /// 对比两个工作流（社区包 base64 或模板 JSON）：看清角色/工具/写范围/依赖变化
    Diff {
        before: PathBuf,
        after: PathBuf,
        /// 出现权限面风险（新增执行/联网工具、MCP、写范围放宽、角色增加）时以退出码 4 失败
        #[arg(long)]
        fail_on_risk: bool,
        /// 有任何差异时以退出码 5 失败（CI 检测上游更新）
        #[arg(long)]
        fail_on_change: bool,
    },
}

pub(crate) fn run_workflow_cmd(args: WorkflowArgs) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_workflow(args))
}

async fn run_workflow(args: WorkflowArgs) -> Result<(), Box<dyn std::error::Error>> {
    // 本地校验/评审：直接读文件，不需要 Daemon（离线可用）。
    if let WorkflowAction::Diff {
        before,
        after,
        fail_on_risk,
        fail_on_change,
    } = &args.action
    {
        let old = read_template_or_package(before)?;
        let new = read_template_or_package(after)?;
        let diff = owo_agent_core::workflow_diff::diff_templates(&old, &new);
        if args.json {
            println!("{}", serde_json::to_string_pretty(&diff)?);
        } else {
            for line in owo_agent_core::workflow_diff::summarize_diff(&diff) {
                if line.starts_with("[风险]") {
                    println!("{}", line.red());
                } else {
                    println!("{line}");
                }
            }
        }
        // CI 门禁：风险优先于"有变化"（风险 => 4；仅变化 => 5）。
        if *fail_on_risk && !diff.risks.is_empty() {
            eprintln!("检测到 {} 条权限面风险（--fail-on-risk）", diff.risks.len());
            std::process::exit(4);
        }
        if *fail_on_change && !diff.is_empty() {
            eprintln!("检测到工作流差异（--fail-on-change）");
            std::process::exit(5);
        }
        return Ok(());
    }

    if let WorkflowAction::Provenance {
        out,
        verify,
        strict,
    } = &args.action
    {
        // 离线可用：清单与本地溯源都在数据目录；读清单文件 + 本地记录。
        let root = ensure_data_root(args.data_dir.clone(), &args.workspace);
        let local = owo_agent_core::community_provenance::load_community_imports(&root);
        if let Some(manifest_path) = verify {
            let text = std::fs::read_to_string(manifest_path).map_err(|read_error| {
                format!("读取清单失败 {}：{read_error}", manifest_path.display())
            })?;
            let manifest = owo_agent_core::community_provenance::SourcesManifest::parse(&text)?;
            let check =
                owo_agent_core::community_provenance::check_against_manifest(&local, &manifest);
            if args.json {
                println!("{}", serde_json::to_string_pretty(&check)?);
            } else {
                println!(
                    "核对结果：一致 {} / 本地缺失 {} / sha256 不一致 {} / 本地额外 {}",
                    check.matched.len(),
                    check.missing.len(),
                    check.mismatched.len(),
                    check.extra.len()
                );
                for template_id in &check.missing {
                    println!("  ! 本地未导入：{template_id}");
                }
                for (template_id, expected, actual) in &check.mismatched {
                    println!(
                        "  ! {template_id} 的 sha256 与清单不同：清单 {}… 本地 {}…",
                        &expected[..expected.len().min(12)],
                        &actual[..actual.len().min(12)]
                    );
                }
                for template_id in &check.extra {
                    println!("  · 本地额外来源（清单未列）：{template_id}");
                }
            }
            if *strict && !check.is_clean() {
                eprintln!("核对未通过（--strict）：存在本地缺失或 sha256 不一致");
                std::process::exit(4);
            }
            return Ok(());
        }

        let manifest = owo_agent_core::community_provenance::SourcesManifest::from_imports(&local);
        match out {
            Some(path) => {
                std::fs::write(path, serde_json::to_string_pretty(&manifest)?)?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&manifest)?);
                } else {
                    println!(
                        "{} {}（{} 条来源）",
                        "已写出社区来源清单：".green(),
                        path.display(),
                        manifest.sources.len()
                    );
                }
            }
            None => {
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&manifest)?);
                } else if manifest.sources.is_empty() {
                    println!("（还没有社区导入来源）");
                } else {
                    println!("社区来源（{} 条）：", manifest.sources.len());
                    for source in &manifest.sources {
                        println!(
                            "  {}  sha256 {}…  导入@{}",
                            source.template_id,
                            &source.sha256[..source.sha256.len().min(12)],
                            source.imported_at
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    match &args.action {
        WorkflowAction::Validate { file } | WorkflowAction::Review { file } => {
            let template = read_template(file)?;
            let errors = validate_workflow_template(&template);
            let findings = review_workflow_template(&template);
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "file": file.display().to_string(),
                        "valid": errors.is_empty(),
                        "errors": errors,
                        "findings": findings,
                        "blocked": workflow_review_blocked(&findings),
                    }))?
                );
            } else {
                print_local_report(file, &template, &errors, &findings);
            }
            // 校验失败用退出码表达（脚本可直接判定）。
            if !errors.is_empty() {
                return Err(format!("校验未通过（{} 条问题）", errors.len()).into());
            }
            return Ok(());
        }
        _ => {}
    }

    let workspace = args
        .workspace
        .canonicalize()
        .unwrap_or_else(|_| args.workspace.clone());
    let root = ensure_data_root(args.data_dir.clone(), &workspace);
    let client = ensure_daemon_client(&root, &workspace).await?;

    if let WorkflowAction::Stats {
        origin,
        sort,
        order,
        limit,
        out,
    } = &args.action
    {
        let filter = origin.trim().to_ascii_lowercase();
        if !matches!(filter.as_str(), "all" | "user" | "community") {
            return Err(format!("--origin 只能是 user/community/all（收到 {origin}）").into());
        }
        let mut uri = String::from("/workflows/stats");
        let mut query: Vec<String> = Vec::new();
        if let Some(sort) = sort
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !matches!(sort, "runs" | "failed" | "success_rate" | "last") {
                return Err(
                    format!("--sort 只能是 runs/failed/success_rate/last（收到 {sort}）").into(),
                );
            }
            query.push(format!("sort={sort}"));
        }
        if let Some(order) = order
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !matches!(order, "asc" | "desc") {
                return Err(format!("--order 只能是 asc/desc（收到 {order}）").into());
            }
            query.push(format!("order={order}"));
        }
        if let Some(limit) = limit {
            query.push(format!("limit={limit}"));
        }
        if !query.is_empty() {
            uri.push('?');
            uri.push_str(&query.join("&"));
        }
        let stats = client.get_json::<serde_json::Value>(&uri).await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&stats)?);
            return Ok(());
        }
        if let Some(path) = out {
            std::fs::write(path, serde_json::to_string_pretty(&stats)?)?;
            println!("{} {}", "已写出统计：".green(), path.display());
        }
        let items: Vec<serde_json::Value> = stats["templates"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|item| {
                filter == "all" || item["origin"].as_str().unwrap_or("") == filter.as_str()
            })
            .collect();
        if items.is_empty() {
            println!("（还没有用这些工作流创建过团队）");
            return Ok(());
        }
        println!("工作流使用统计（{} 条）：", items.len());
        for item in items {
            let rate = match item["success_rate"].as_f64() {
                Some(value) => format!("{:.0}%", value * 100.0),
                None => "—".to_string(),
            };
            println!(
                "  {} [{}] 运行 {} · 成功 {} · 失败 {} · 取消 {} · 成功率 {} · 最后 {}",
                item["template_id"].as_str().unwrap_or("?"),
                item["origin"].as_str().unwrap_or("?"),
                item["runs"].as_u64().unwrap_or(0),
                item["succeeded"].as_u64().unwrap_or(0),
                item["failed"].as_u64().unwrap_or(0),
                item["cancelled"].as_u64().unwrap_or(0),
                rate,
                item["last_run_at"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(16)
                    .collect::<String>()
            );
            let attention: Vec<String> = item["attention"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if !attention.is_empty() {
                println!(
                    "      {}",
                    format!("⚠ 需要修复：{}", attention.join("；")).yellow()
                );
                // 复现入口：沿用最近一次运行的目标（id 就地在同一行给出，省一次查找）。
                if let Some(id) = item["template_id"].as_str() {
                    println!("      复现：owo-agent workflow rerun {id}");
                }
            }
            if let Some(failure) = item["last_failure"].as_object() {
                println!(
                    "      最近失败（本进程审计）：{} · {}",
                    failure
                        .get("event")
                        .and_then(|value| value.as_str())
                        .unwrap_or("?"),
                    failure
                        .get("detail")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .chars()
                        .take(80)
                        .collect::<String>()
                );
            }
            if item["running"].as_u64().unwrap_or(0) > 0
                || item["awaiting_human"].as_u64().unwrap_or(0) > 0
            {
                println!(
                    "      （进行中 {} · 等待人工 {}）",
                    item["running"].as_u64().unwrap_or(0),
                    item["awaiting_human"].as_u64().unwrap_or(0)
                );
            }
        }
        if stats["truncated"] == serde_json::json!(true) {
            println!(
                "（共 {} 条，按 {} {} 只显示前 {} 条）",
                stats["total"].as_u64().unwrap_or(0),
                stats["sort"].as_str().unwrap_or("runs"),
                stats["order"].as_str().unwrap_or("desc"),
                stats["count"].as_u64().unwrap_or(0)
            );
        }
        if let Some(summary) = stats["summary"].as_object() {
            println!(
                "合计：运行 {} · 成功 {} · 失败 {}",
                summary
                    .get("total_runs")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                summary
                    .get("succeeded")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                summary
                    .get("failed")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0)
            );
        }
        return Ok(());
    }

    match args.action {
        WorkflowAction::List { origin } => {
            let filter = origin.trim().to_ascii_lowercase();
            if !matches!(filter.as_str(), "all" | "user" | "community") {
                return Err(format!("--origin 只能是 user/community/all（收到 {origin}）").into());
            }
            let listed = client.list_user_workflows().await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&listed)?);
                return Ok(());
            }
            let items: Vec<serde_json::Value> = listed["templates"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|item| {
                    filter == "all" || item["origin"].as_str().unwrap_or("") == filter.as_str()
                })
                .collect();
            if items.is_empty() {
                println!(
                    "（还没有自定义工作流；用 `owo-agent workflow create <文件>`，\
或 `owo-agent workflow draft --description \"...\"` 让 AI 生成）"
                );
                return Ok(());
            }
            println!(
                "自定义工作流（{} 条，其中 {} 条校验不过，{} 条需要修复）：",
                items.len(),
                listed["invalid"].as_u64().unwrap_or(0),
                listed["needs_attention"].as_u64().unwrap_or(0)
            );
            for item in items {
                let template = &item["template"];
                let risk_label = match item["health"]["risk_level"].as_str().unwrap_or("?") {
                    "low" => "低".green().to_string(),
                    "medium" => "中".yellow().to_string(),
                    _ => "高".red().to_string(),
                };
                let provenance = if item["provenance"].is_null() {
                    String::new()
                } else {
                    format!(
                        " · 导入@{}",
                        item["provenance"]["imported_at"]
                            .as_str()
                            .unwrap_or("?")
                            .chars()
                            .take(16)
                            .collect::<String>()
                    )
                };
                println!(
                    "  {} [{}] {} · {} 个角色 · 风险 {}{}{}{}",
                    template["template_id"].as_str().unwrap_or("?"),
                    item["origin"].as_str().unwrap_or("?"),
                    template["name"].as_str().unwrap_or(""),
                    template["roles"].as_array().map(Vec::len).unwrap_or(0),
                    risk_label,
                    provenance,
                    match item["runs"].as_u64().unwrap_or(0) {
                        0 => String::new(),
                        runs => format!(" · 运行 {runs} 次"),
                    },
                    if item["valid"] == serde_json::json!(true) {
                        String::new()
                    } else {
                        format!(
                            " · {} 条问题",
                            item["validation_errors"]
                                .as_array()
                                .map(Vec::len)
                                .unwrap_or(0)
                        )
                    }
                );
                let attention: Vec<String> = item["attention"]
                    .as_array()
                    .map(|reasons| {
                        reasons
                            .iter()
                            .filter_map(|value| value.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if !attention.is_empty() {
                    println!(
                        "      {}",
                        format!("⚠ 需要修复：{}", attention.join("；")).yellow()
                    );
                }
            }
            Ok(())
        }
        WorkflowAction::Show { id } => {
            let listed = client.list_user_workflows().await?;
            let item = listed["templates"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item["template"]["template_id"].as_str() == Some(id.as_str()))
                })
                .cloned()
                .ok_or_else(|| format!("工作流不存在：{id}"))?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&item)?);
                return Ok(());
            }
            let template: TeamTemplate = serde_json::from_value(item["template"].clone())?;
            let errors = validate_workflow_template(&template);
            let findings = review_workflow_template(&template);
            println!(
                "{} [{}] {}",
                template.template_id,
                item["origin"].as_str().unwrap_or("?"),
                template.name
            );
            if let Some(description) = template.description.as_deref() {
                println!("  描述：{description}");
            }
            println!("  适用：{}", template.applicability);
            for role in &template.roles {
                println!(
                    "  - {}（{}）{}",
                    role.role,
                    role.assignee,
                    if role.depends_on.is_empty() {
                        String::new()
                    } else {
                        format!(" ← {}", role.depends_on.join(", "))
                    }
                );
                if !role.tools.is_empty() {
                    println!("      工具：{}", role.tools.join(", "));
                }
                if !role.write_paths.is_empty() {
                    println!("      写范围：{}", role.write_paths.join(", "));
                }
            }
            if !item["provenance"].is_null() {
                let provenance = &item["provenance"];
                println!(
                    "  溯源：社区导入@{} sha256={}…（当时 {} 条发现）",
                    provenance["imported_at"].as_str().unwrap_or("?"),
                    provenance["sha256"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(12)
                        .collect::<String>(),
                    provenance["findings"].as_u64().unwrap_or(0)
                );
            }
            let health = owo_agent_core::workflow_health(&template);
            println!(
                "  健康：{} 个角色 / {} 个工具 / 依赖深度 {} / 人工节点 {} / 风险 {}",
                health.roles,
                health.tools,
                health.max_depth,
                health.human_roles,
                match health.risk_level.as_str() {
                    "low" => "低".green().to_string(),
                    "medium" => "中".yellow().to_string(),
                    _ => "高".red().to_string(),
                }
            );
            for row in &health.role_rows {
                println!(
                    "  - {}（{}）工具 {} 个{}{}",
                    row.role,
                    row.assignee,
                    row.tools.len(),
                    if row.write_paths.is_empty() {
                        String::new()
                    } else {
                        format!(" / 写 {}", row.write_paths.join(", "))
                    },
                    if row.risks.is_empty() {
                        String::new()
                    } else {
                        format!(" / {}", row.risks.join("；"))
                    }
                );
            }
            print_findings(&findings);
            if !errors.is_empty() {
                println!("{}", "校验问题：".red());
                for error in &errors {
                    println!("  ! {error}");
                }
            }
            // 运行期状态：列表口径的 runs/attention + 最近一次运行（可复用目标）。
            let runs = item["runs"].as_u64().unwrap_or(0);
            println!("  运行：{runs} 次");
            let attention: Vec<String> = item["attention"]
                .as_array()
                .map(|reasons| {
                    reasons
                        .iter()
                        .filter_map(|value| value.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if !attention.is_empty() {
                println!(
                    "  {}",
                    format!("⚠ 需要修复：{}", attention.join("；")).yellow()
                );
            }
            if let Ok(last) = client.workflow_last_run(&template.template_id).await {
                match last["last_run"].as_object() {
                    Some(run) => println!(
                        "  最近一次：{} · {} · {}{}",
                        run.get("status")
                            .and_then(|value| value.as_str())
                            .unwrap_or("?"),
                        run.get("created_at")
                            .and_then(|value| value.as_str())
                            .unwrap_or("?")
                            .chars()
                            .take(16)
                            .collect::<String>(),
                        run.get("objective")
                            .and_then(|value| value.as_str())
                            .unwrap_or("?"),
                        // 有运行记录时给出可复现入口（沿用同一目标）。
                        "   （复用：owo-agent workflow rerun <id>）"
                    ),
                    None => println!("  最近一次：从未运行"),
                }
            }
            Ok(())
        }
        WorkflowAction::Create { file } => {
            let template = read_template(&file)?;
            let value = serde_json::to_value(&template)?;
            match client.upsert_user_workflow(&value).await {
                Ok(saved) => {
                    if args.json {
                        println!("{}", serde_json::to_string_pretty(&saved)?);
                    } else {
                        println!(
                            "{} {}（{} 个角色）",
                            "已保存用户工作流：".green(),
                            saved["template"]["template_id"].as_str().unwrap_or("?"),
                            saved["template"]["roles"]
                                .as_array()
                                .map(Vec::len)
                                .unwrap_or(0)
                        );
                    }
                    Ok(())
                }
                Err(error) => {
                    // 422 的响应体带逐条原因，尽量原样展示，便于照着改文件。
                    Err(format!("保存失败：{error}").into())
                }
            }
        }
        WorkflowAction::Delete { id } => {
            let deleted = client.delete_user_workflow(&id).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&deleted)?);
            } else {
                println!("{} {}", "已删除工作流：".green(), id);
            }
            Ok(())
        }
        WorkflowAction::Draft {
            description,
            constraints,
            refine,
            rounds,
            from,
        } => {
            let converge_rounds = rounds.unwrap_or(1);
            if converge_rounds > 1 && refine.is_some() {
                return Err("--rounds 与 --refine 不能同时使用（收敛已自带多轮迭代）".into());
            }
            if from.is_some() && refine.is_some() {
                return Err(
                    "--from 与 --refine 不能同时使用（一个基于已保存工作流，一个基于草稿）".into(),
                );
            }
            let draft = if converge_rounds > 1 {
                client
                    .converge_workflow_draft(
                        &description,
                        constraints.as_deref(),
                        Some(converge_rounds),
                        from.as_deref(),
                    )
                    .await?
            } else {
                client
                    .create_workflow_draft(
                        &description,
                        constraints.as_deref(),
                        refine.as_deref(),
                        from.as_deref(),
                    )
                    .await?
            };
            if args.json {
                println!("{}", serde_json::to_string_pretty(&draft)?);
                return Ok(());
            }
            let id = draft["draft"]["draft_id"].as_str().unwrap_or("?");
            println!(
                "{} {}（{} 个角色，{}）",
                "已生成草稿：".green(),
                id,
                draft["preview"]["roles"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0),
                if draft["ok"] == serde_json::json!(true) {
                    if draft["repaired"] == serde_json::json!(true) {
                        "校验通过（已按校验错误自动修复 1 轮）".to_string()
                    } else {
                        "校验通过".to_string()
                    }
                } else {
                    format!(
                        "{} 条校验问题（可改描述后 --refine {id} 迭代）",
                        draft["errors"].as_array().map(Vec::len).unwrap_or(0)
                    )
                }
            );
            if let Some(evidence) = draft["run_evidence"]
                .as_str()
                .filter(|value| !value.is_empty())
            {
                let bullets = evidence
                    .lines()
                    .filter(|line| line.starts_with("- "))
                    .collect::<Vec<_>>()
                    .join(" | ");
                println!("  {}（{}）", "已注入真实运行证据".cyan(), bullets);
            } else if from.is_some() {
                println!("  （该工作流还没有运行记录：这次迭代没有运行证据可用）");
            }
            for field in ["name", "description", "applicability"] {
                if let Some(value) = draft["template"][field].as_str() {
                    println!("  {field}: {value}");
                }
            }
            for role in draft["preview"]["roles"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                println!(
                    "  - {}（{}）{}",
                    role["role"].as_str().unwrap_or("?"),
                    role["assignee"].as_str().unwrap_or("?"),
                    if role["depends_on"].as_array().map(Vec::len).unwrap_or(0) > 0 {
                        format!(
                            " ← {}",
                            role["depends_on"]
                                .as_array()
                                .map(|items| items
                                    .iter()
                                    .filter_map(|item| item.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", "))
                                .unwrap_or_default()
                        )
                    } else {
                        String::new()
                    }
                );
            }
            if let Some(errors) = draft["errors"].as_array() {
                for error in errors {
                    println!("  ! {}", error.as_str().unwrap_or(""));
                }
            }
            if let Some(health) = draft["health"].as_object() {
                let elevated: Vec<String> = health
                    .get("elevated_tools")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let risk = match health.get("risk_level").and_then(|value| value.as_str()) {
                    Some("low") => "低".green().to_string(),
                    Some("medium") => "中".yellow().to_string(),
                    _ => "高".red().to_string(),
                };
                println!(
                    "  风险：{risk}{}",
                    if elevated.is_empty() {
                        String::new()
                    } else {
                        format!(" · 执行/联网工具：{}", elevated.join(", "))
                    }
                );
            }
            if let Some(trajectory) = draft["trajectory"].as_array() {
                println!("  收敛轨迹（{} 轮）：", trajectory.len());
                for step in trajectory {
                    println!(
                        "    #{} 角色 {} / 错误 {}{}",
                        step["round"].as_u64().unwrap_or(0),
                        step["roles"].as_u64().unwrap_or(0),
                        step["errors"].as_u64().unwrap_or(0),
                        if step["accepted"] == serde_json::json!(false) {
                            "（未改善，停止）"
                        } else {
                            ""
                        }
                    );
                }
            }
            println!(
                "\n采纳：owo-agent workflow adopt {id}；丢弃：owo-agent workflow discard {id}"
            );
            Ok(())
        }
        WorkflowAction::Drafts => {
            let drafts = client.list_workflow_drafts().await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&drafts)?);
                return Ok(());
            }
            let items = drafts["drafts"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                println!("（没有草稿）");
                return Ok(());
            }
            for item in items {
                println!(
                    "  {} · {} · {} 个角色 · {}{}",
                    item["draft_id"].as_str().unwrap_or("?"),
                    item["name"].as_str().unwrap_or(""),
                    item["roles"].as_u64().unwrap_or(0),
                    item["created_at"].as_str().unwrap_or(""),
                    if item["valid"] == serde_json::json!(true) {
                        ""
                    } else {
                        " · 校验未通过"
                    }
                );
            }
            Ok(())
        }
        WorkflowAction::Adopt {
            draft_id,
            verify,
            follow,
            follow_timeout,
        } => {
            let adopted = client.adopt_workflow_draft(&draft_id).await?;
            let template_id = adopted["template"]["template_id"]
                .as_str()
                .unwrap_or("?")
                .to_string();
            if args.json {
                println!("{}", serde_json::to_string_pretty(&adopted)?);
            } else {
                println!(
                    "{} {}（用户工作流，可用于组队创建）",
                    "已采纳：".green(),
                    template_id
                );
            }
            if !verify {
                return Ok(());
            }
            // 验证运行：必须真的有基线（上一次运行的目标），否则不假装对比。
            let Some(plan) = adopted["verify"].as_object() else {
                println!(
                    "（这版草稿没有运行基线：用 `owo-agent workflow run {template_id} --objective \"...\"` 跑第一次）"
                );
                return Ok(());
            };
            let objective = plan
                .get("objective")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            if objective.is_empty() {
                println!("（基线运行没有记录目标；请显式指定 --objective 后手动跑一次）");
                return Ok(());
            }
            let previous_status = plan
                .get("previous_status")
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            let previous_attention = plan
                .get("previous_attention")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                        .join("；")
                })
                .unwrap_or_default();
            println!(
                "── 验证运行（沿用基线目标）──\n  采纳前：{} {}\n  目标：{}",
                previous_status,
                if previous_attention.is_empty() {
                    String::new()
                } else {
                    format!("（{previous_attention}）")
                },
                objective
            );
            let created = client
                .create_team(&objective, "team", Some(template_id.as_str()), true)
                .await?;
            let team_id = created["team_id"]
                .as_str()
                .or_else(|| created["team"]["team_id"].as_str())
                .unwrap_or("?")
                .to_string();
            println!(
                "  新团队：{team_id}（{} 个成员）",
                created["members"].as_array().map(Vec::len).unwrap_or(0)
            );
            if follow {
                let timeout_secs = follow_timeout.unwrap_or(120).clamp(5, 600);
                follow_team(&client, &team_id, timeout_secs).await?;
            }
            // 对比：新的一次运行是否出现（与采纳前的基线状态并列，不做主观判定）。
            let after = client.workflow_last_run(&template_id).await?;
            if let Some(run) = after["last_run"].as_object() {
                let status = run
                    .get("status")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?");
                println!(
                    "── 对比 ──\n  采纳前：{previous_status}\n  本次：{status}\n  {}",
                    if status == previous_status {
                        "状态未变：按运行证据再改一轮（workflow draft --from）"
                    } else {
                        "状态已变化：用 workflow stats 看累计成功率"
                    }
                );
                // 回写验证结果：下一轮 `draft --from` 会把"上次这样改有没有用"一并交给模型。
                match client
                    .record_workflow_draft_verify(
                        &draft_id,
                        &team_id,
                        Some(&objective),
                        Some(previous_status),
                    )
                    .await
                {
                    Ok(written) => println!(
                        "  已回写验证结果（draft {}）：下一轮 `workflow draft --from {template_id}` 会带上这次反馈",
                        written["draft"]["draft_id"].as_str().unwrap_or(&draft_id)
                    ),
                    Err(write_error) => {
                        println!("  （验证结果回写失败：{write_error}；不影响本次对比）")
                    }
                }
            }
            Ok(())
        }
        WorkflowAction::Discard { draft_id } => {
            client.discard_workflow_draft(&draft_id).await?;
            println!("{} {}", "已丢弃草稿：".green(), draft_id);
            Ok(())
        }
        WorkflowAction::Export { id, out } => {
            let exported = client.export_user_workflow(&id).await?;
            let package = exported["package_b64"].as_str().unwrap_or_default();
            let target = out.unwrap_or_else(|| PathBuf::from(format!("{id}.owowf.json")));
            std::fs::write(&target, package)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&exported)?);
            } else {
                println!(
                    "{} {}（sha256 {}，{} 个角色）\n  预审：{} 条发现（blocked={}）",
                    "已导出社区包：".green(),
                    target.display(),
                    exported["sha256"].as_str().unwrap_or("?"),
                    exported["manifest"]["role_count"].as_u64().unwrap_or(0),
                    exported["review"]["findings"]
                        .as_array()
                        .map(Vec::len)
                        .unwrap_or(0),
                    exported["review"]["blocked"].as_bool().unwrap_or(false)
                );
            }
            Ok(())
        }
        WorkflowAction::Import { file, review_only } => {
            let package = std::fs::read_to_string(&file)?;
            let review = client.review_workflow_import(package.trim()).await?;
            if args.json && review_only {
                println!("{}", serde_json::to_string_pretty(&review)?);
                return Ok(());
            }
            if !args.json {
                println!(
                    "评审：blocked={}，{} 条发现",
                    review["review"]["blocked"].as_bool().unwrap_or(false),
                    review["review"]["findings"]
                        .as_array()
                        .map(Vec::len)
                        .unwrap_or(0)
                );
                for finding in review["review"]["findings"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    println!(
                        "  [{}] {}：{}",
                        finding["level"].as_str().unwrap_or("?"),
                        finding["category"].as_str().unwrap_or("?"),
                        finding["detail"].as_str().unwrap_or("")
                    );
                }
            }
            if review_only {
                return Ok(());
            }
            let imported = client.import_user_workflow(package.trim()).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&imported)?);
            } else {
                println!(
                    "{} {}（origin={}）",
                    "已导入工作流：".green(),
                    imported["template"]["template_id"].as_str().unwrap_or("?"),
                    imported["template"]["origin"].as_str().unwrap_or("?")
                );
            }
            Ok(())
        }
        WorkflowAction::Run {
            id,
            objective,
            yes,
            follow,
            follow_timeout,
        } => {
            create_team_from_workflow(
                &client,
                args.json,
                &id,
                &objective,
                yes,
                follow,
                follow_timeout,
            )
            .await
        }
        WorkflowAction::Rerun {
            id,
            objective,
            yes,
            follow,
            follow_timeout,
        } => {
            let last = client.workflow_last_run(&id).await?;
            let Some(last_run) = last["last_run"].as_object() else {
                return Err(format!(
                    "工作流 {id} 还没有运行记录；用 `owo-agent workflow run {id} --objective \"...\"` 跑第一次"
                )
                .into());
            };
            let objective = objective
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    last_run
                        .get("objective")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                        .filter(|value| !value.trim().is_empty())
                })
                .ok_or("最近一次运行没有可复用的目标；请显式给 --objective")?;
            if !args.json {
                println!(
                    "沿用最近一次运行的目标（{}，{}）：{}",
                    last_run
                        .get("team_id")
                        .and_then(|value| value.as_str())
                        .unwrap_or("?"),
                    last_run
                        .get("status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("?"),
                    objective
                );
            }
            create_team_from_workflow(
                &client,
                args.json,
                &id,
                &objective,
                yes,
                follow,
                follow_timeout,
            )
            .await
        }
        // 以下分支在各自段落内自行处理（离线/诊断）。
        WorkflowAction::Validate { .. }
        | WorkflowAction::Review { .. }
        | WorkflowAction::Diff { .. }
        | WorkflowAction::Provenance { .. }
        | WorkflowAction::Stats { .. } => Ok(()),
    }
}

/// 按工作流建队（`run` 与 `rerun` 共用同一套门禁：block 拒绝、warn 需 --yes、可选跟随）。
async fn create_team_from_workflow(
    client: &owo_agent_client::AgentClient,
    json: bool,
    id: &str,
    objective: &str,
    yes: bool,
    follow: bool,
    follow_timeout: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let objective = objective.trim().to_string();
    if objective.is_empty() {
        return Err("目标不能为空（这条团队运行要达成什么）；用 --objective 指定".into());
    }
    {
        {
            let listed = client.list_user_workflows().await?;
            let item = listed["templates"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item["template"]["template_id"].as_str() == Some(id))
                })
                .cloned()
                .ok_or_else(|| format!("工作流不存在：{id}"))?;
            let template: TeamTemplate = serde_json::from_value(item["template"].clone())?;
            // 与面板同源：建队前做能力面评审（本地确定性规则）。
            let findings = review_workflow_template(&template);
            if workflow_review_blocked(&findings) {
                let reasons = findings
                    .iter()
                    .filter(|finding| finding.level == "block")
                    .map(|finding| format!("  ! {}：{}", finding.category, finding.detail))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Err(format!("工作流存在阻止级问题，不能建队：\n{reasons}").into());
            }
            let warnings: Vec<&owo_agent_core::WorkflowFinding> = findings
                .iter()
                .filter(|finding| finding.level == "warn")
                .collect();
            if !warnings.is_empty() && !yes {
                for finding in &warnings {
                    eprintln!("  [warn] {}：{}", finding.category, finding.detail);
                }
                return Err(format!(
                    "该工作流有 {} 条能力面提示；确认已知情后加 --yes 重试",
                    warnings.len()
                )
                .into());
            }
            let mode = match template.mode {
                owo_agent_protocol::TeamMode::Single => "single",
                owo_agent_protocol::TeamMode::Swarmflow => "swarmflow",
                owo_agent_protocol::TeamMode::Team => "team",
            };
            let created = client.create_team(&objective, mode, Some(id), yes).await?;
            let team_id = created["team_id"]
                .as_str()
                .or_else(|| created["team"]["team_id"].as_str())
                .unwrap_or("?")
                .to_string();
            if json {
                println!("{}", serde_json::to_string_pretty(&created)?);
            } else {
                println!(
                    "{} {}（{}，{} 个成员）\n  跟踪：桌面「团队协作」面板，或 GET /teams/{team_id}",
                    "已按工作流创建团队：".green(),
                    team_id,
                    mode,
                    created["members"].as_array().map(Vec::len).unwrap_or(0)
                );
            }
            if follow {
                let timeout_secs = follow_timeout.unwrap_or(120).clamp(5, 600);
                follow_team(client, &team_id, timeout_secs).await?;
            }
            Ok(())
        }
    }
}

/// 跟随团队进度：订阅 `/teams/{id}/events`，打印可读事件，直到终态/超时/事件上限。
///
/// 事件文案复用 Daemon REPL `/team` 的同一套解析（`team_audit_summary`），不另写一份。
async fn follow_team(
    client: &owo_agent_client::AgentClient,
    team_id: &str,
    timeout_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    const MAX_EVENTS: usize = 300;
    println!("── 跟随团队 {team_id}（最多 {timeout_secs}s） ──");
    let mut stream = client
        .open_event_stream(&format!("/teams/{team_id}/events"))
        .await?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut seen = 0usize;
    let mut terminal_by_event = false;
    while seen < MAX_EVENTS {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            println!("（已到跟随时限 {timeout_secs}s，停止跟随；团队仍在后台运行）");
            break;
        }
        match tokio::time::timeout(remaining, stream.next_event()).await {
            Err(_) => {
                println!("（已到跟随时限 {timeout_secs}s，停止跟随；团队仍在后台运行）");
                break;
            }
            Ok(None) => break,
            Ok(Some(Err(error))) => {
                println!("（事件流中断：{error}）");
                break;
            }
            Ok(Some(Ok(event))) => {
                seen += 1;
                let name = event
                    .get("event")
                    .and_then(|value| value.as_str())
                    .unwrap_or("team.event");
                match crate::commands::repl_daemon::team_audit_summary(&event) {
                    Some(summary) => println!("  · {summary}"),
                    None => println!("  · {name}"),
                }
                if matches!(
                    name,
                    "team.succeeded"
                        | "team.failed"
                        | "team.cancelled"
                        | "team.budget_exhausted"
                        | "team.interrupted"
                ) {
                    terminal_by_event = true;
                    break;
                }
            }
        }
    }
    if seen >= MAX_EVENTS {
        println!("（事件数达上限 {MAX_EVENTS}，停止跟随；团队仍在后台运行）");
    }
    // 终态以服务端查询为准（事件流可能断开或漏报）。
    let detail = client.get_team(team_id).await?;
    let status = crate::commands::repl_daemon::team_status(&detail);
    let is_terminal = crate::commands::repl_daemon::team_status_is_terminal(&status);
    println!(
        "── 团队状态：{status}（{}） ──",
        if is_terminal {
            "终态"
        } else {
            "仍在运行"
        }
    );
    if !terminal_by_event && !is_terminal {
        println!("提示：团队仍在运行；可再次 `--follow`，或到桌面「团队协作」面板继续跟踪。");
    }
    Ok(())
}

/// 读模板或社区包：优先按 base64 社区包解析（kind=owo-workflow），否则按模板 JSON。
fn read_template_or_package(path: &Path) -> Result<TeamTemplate, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("读取失败 {}：{error}", path.display()))?;
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        return read_template(path);
    }
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .map_err(|error| format!("既不是模板 JSON 也不是合法 base64 社区包：{error}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("社区包不是合法 JSON：{error}"))?;
    if value.get("kind").and_then(|kind| kind.as_str()) != Some("owo-workflow") {
        return Err("社区包 kind 不是 owo-workflow".into());
    }
    serde_json::from_value(value.get("template").cloned().unwrap_or_default())
        .map_err(|error| format!("社区包缺少 template：{error}").into())
}

fn read_template(file: &Path) -> Result<TeamTemplate, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(file)
        .map_err(|error| format!("读取失败 {}：{error}", file.display()))?;
    let template: TeamTemplate = serde_json::from_str(&raw).map_err(|error| {
        format!(
            "解析失败 {}（应为 TeamTemplate JSON）：{error}",
            file.display()
        )
    })?;
    Ok(template)
}

fn print_local_report(
    file: &Path,
    template: &TeamTemplate,
    errors: &[String],
    findings: &[owo_agent_core::WorkflowFinding],
) {
    println!(
        "{} {}（{} 个角色）",
        if errors.is_empty() {
            "校验通过：".green()
        } else {
            "校验未通过：".red()
        },
        file.display(),
        template.roles.len()
    );
    for error in errors {
        println!("  ! {error}");
    }
    print_findings(findings);
}

fn print_findings(findings: &[owo_agent_core::WorkflowFinding]) {
    for finding in findings {
        let line = format!(
            "  [{}] {}：{}",
            finding.level, finding.category, finding.detail
        );
        match finding.level.as_str() {
            "block" => println!("{}", line.red()),
            "warn" => println!("{}", line.yellow()),
            _ => println!("{line}"),
        }
    }
}
