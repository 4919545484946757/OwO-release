//! `owo-agent playbook`：任务剧本的离线工具（校验 / 执行顺序预览）。
//!
//! 纯读取 + 纯计算：不启动 Daemon、不执行剧本里的命令、不写工作区。
//! 执行由后续的宿主驱动完成（Check 步骤经 Tool Host），本命令只回答两件事：
//! 「这份剧本合法吗」「按什么顺序做」。

use std::path::PathBuf;

use clap::{Args, Subcommand};
use owo_agent_core::playbook::{
    Playbook, PlaybookIssue, PlaybookRun, StepKind, MAX_PLAYBOOK_STEPS,
};

#[derive(Args)]
pub(crate) struct PlaybookArgs {
    #[command(subcommand)]
    action: PlaybookAction,
    /// 机器可读输出（JSON）
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
pub(crate) enum PlaybookAction {
    /// 校验剧本文件（结构/依赖/环/大小）
    Validate { file: PathBuf },
    /// 打印执行顺序与依赖（不执行）
    Plan { file: PathBuf },
}

pub(crate) fn run_playbook_cmd(args: PlaybookArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.action {
        PlaybookAction::Validate { file } => validate(&file, args.json),
        PlaybookAction::Plan { file } => plan(&file, args.json),
    }
}

fn load(file: &PathBuf) -> Result<Playbook, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(file)
        .map_err(|error| format!("读取剧本失败（{}）：{error}", file.display()))?;
    // Windows 编辑器常带 UTF-8 BOM：它是文件头，不是 JSON 内容。
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let playbook: Playbook = serde_json::from_str(content)
        .map_err(|error| format!("剧本不是合法 JSON（{}）：{error}", file.display()))?;
    Ok(playbook)
}

fn kind_label(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Model => "model",
        StepKind::Check => "check",
        StepKind::Human => "human",
    }
}

fn issues_json(issues: &[PlaybookIssue]) -> Vec<serde_json::Value> {
    issues
        .iter()
        .map(|issue| {
            serde_json::json!({
                "code": issue.code,
                "step": issue.step,
                "message": issue.message,
            })
        })
        .collect()
}

fn validate(file: &PathBuf, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let playbook = load(file)?;
    let issues = playbook.validate();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": issues.is_empty(),
                "id": playbook.id,
                "name": playbook.name,
                "steps": playbook.steps.len(),
                "max_steps": MAX_PLAYBOOK_STEPS,
                "issues": issues_json(&issues),
            })
        );
    } else if issues.is_empty() {
        println!(
            "剧本通过：{}（{}）· {} 步",
            playbook.name,
            playbook.id,
            playbook.steps.len()
        );
        for step in &playbook.steps {
            println!(
                "  - {:<16} {:<6} {}",
                step.id,
                kind_label(step.kind),
                step.title
            );
        }
    } else {
        println!(
            "剧本不通过：{}（{}）· {} 个问题",
            playbook.name,
            playbook.id,
            issues.len()
        );
        for issue in &issues {
            match &issue.step {
                Some(step) => println!("  [{}] {}：{}", issue.code, step, issue.message),
                None => println!("  [{}] {}", issue.code, issue.message),
            }
        }
        std::process::exit(1);
    }
    Ok(())
}

fn plan(file: &PathBuf, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let playbook = load(file)?;
    let issues = playbook.validate();
    if !issues.is_empty() {
        // 不合法的剧本没有可信顺序：先报问题，再退出。
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "ok": false,
                    "issues": issues_json(&issues),
                })
            );
        } else {
            println!("剧本不合法，无法给出执行顺序：");
            for issue in &issues {
                println!("  [{}] {}", issue.code, issue.message);
            }
        }
        std::process::exit(1);
    }
    let order = playbook
        .topological_order()
        .map_err(|cycle| format!("依赖成环：{}", cycle.join(" → ")))?;
    let run = PlaybookRun::new(&playbook);
    if json {
        let steps: Vec<serde_json::Value> = order
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let step = playbook
                    .steps
                    .iter()
                    .find(|step| step.id == *id)
                    .expect("order 来自同一剧本");
                serde_json::json!({
                    "order": index + 1,
                    "id": step.id,
                    "kind": kind_label(step.kind),
                    "title": step.title,
                    "depends_on": step.depends_on,
                    "acceptance": step.acceptance,
                    "detail": step.detail,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "id": playbook.id,
                "name": playbook.name,
                "progress": run.summary(&playbook),
                "steps": steps,
            })
        );
    } else {
        println!(
            "执行计划：{}（{}）· {} 步 · 进度 {}",
            playbook.name,
            playbook.id,
            order.len(),
            run.summary(&playbook)
        );
        for (index, id) in order.iter().enumerate() {
            let step = playbook
                .steps
                .iter()
                .find(|step| step.id == *id)
                .expect("order 来自同一剧本");
            let deps = if step.depends_on.is_empty() {
                "无前置".to_string()
            } else {
                format!("前置 {}", step.depends_on.join("、"))
            };
            println!(
                "  {}. {:<16} {:<6} {}（{}）",
                index + 1,
                step.id,
                kind_label(step.kind),
                step.title,
                deps
            );
            if !step.acceptance.is_empty() {
                println!("     判据：{}", step.acceptance);
            }
        }
        if let Some(next) = run.next_ready(&playbook) {
            println!("下一步：{}（{}）", next.id, next.title);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_labels_are_stable_for_scripts() {
        assert_eq!(kind_label(StepKind::Model), "model");
        assert_eq!(kind_label(StepKind::Check), "check");
        assert_eq!(kind_label(StepKind::Human), "human");
    }
}
