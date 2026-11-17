//! `owo-agent playbook`：任务剧本的离线工具（校验 / 顺序预览 / 进度记录）。
//!
//! 纯读取 + 纯计算 + 本地状态文件：不启动 Daemon、不执行剧本里的命令。
//! 执行由宿主完成（Check 步骤经 Tool Host），本命令负责回答：
//! 「这份剧本合法吗」「按什么顺序做」「下一步是什么」「这一步记什么结果」。
//!
//! 运行状态落在剧本旁边的 `<剧本名>.state.json`（可用 `--run` 指定），
//! 格式就是 `PlaybookRun` 的 serde 形态：任何客户端都能读写同一份进度。

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use owo_agent_core::playbook::{
    Playbook, PlaybookIssue, PlaybookRun, PlaybookStep, StepKind, StepState, MAX_PLAYBOOK_STEPS,
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
    /// 查看运行进度（没有状态文件时显示初始进度）
    Status {
        file: PathBuf,
        /// 状态文件路径（默认 `<剧本名>.state.json`）
        #[arg(long)]
        run: Option<PathBuf>,
    },
    /// 显示下一个可执行步骤（不存在状态文件时创建）
    Next {
        file: PathBuf,
        #[arg(long)]
        run: Option<PathBuf>,
    },
    /// 记录一步的结果（pass/fail/skip 三选一）
    Record {
        file: PathBuf,
        /// 步骤 id
        step: String,
        /// 通过（附证据）
        #[arg(long)]
        pass: Option<String>,
        /// 失败（附原因；重复失败会累计尝试次数）
        #[arg(long)]
        fail: Option<String>,
        /// 跳过（附原因；下游会被阻塞）
        #[arg(long)]
        skip: Option<String>,
        #[arg(long)]
        run: Option<PathBuf>,
    },
    /// 删除运行状态（下一次 next 重新开始）
    Reset {
        file: PathBuf,
        #[arg(long)]
        run: Option<PathBuf>,
    },
    /// 列出一个目录下的全部剧本与各自进度（默认 `playbooks/`）
    List {
        /// 剧本目录（默认 `playbooks`）
        #[arg(default_value = "playbooks")]
        dir: PathBuf,
    },
    /// 生成一份剧本骨架（默认写 `playbooks/<id>.json`）
    Init {
        /// 剧本 id（小写字母/数字/连字符）
        id: String,
        /// 输出路径（默认 `playbooks/<id>.json`）
        #[arg(long)]
        out: Option<PathBuf>,
        /// 覆盖已存在的文件
        #[arg(long)]
        force: bool,
    },
    /// 归档当前运行状态（快照到 `<剧本>.runs/<标记>.json`）
    Archive {
        file: PathBuf,
        /// 归档标记（默认当前 epoch 毫秒；只允许字母/数字/.-_）
        #[arg(long)]
        marker: Option<String>,
        /// 覆盖同标记的归档
        #[arg(long)]
        force: bool,
    },
    /// 列出历次归档与各自进度
    History { file: PathBuf },
}

pub(crate) fn run_playbook_cmd(args: PlaybookArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.action {
        PlaybookAction::Validate { file } => validate(&file, args.json),
        PlaybookAction::Plan { file } => plan(&file, args.json),
        PlaybookAction::Status { file, run } => status(&file, run.as_deref(), args.json),
        PlaybookAction::Next { file, run } => next(&file, run.as_deref(), args.json),
        PlaybookAction::Record {
            file,
            step,
            pass,
            fail,
            skip,
            run,
        } => record(&file, &step, pass, fail, skip, run.as_deref(), args.json),
        PlaybookAction::Reset { file, run } => reset(&file, run.as_deref(), args.json),
        PlaybookAction::List { dir } => list(&dir, args.json),
        PlaybookAction::Init { id, out, force } => init(&id, out.as_deref(), force, args.json),
        PlaybookAction::Archive {
            file,
            marker,
            force,
        } => archive(&file, marker.as_deref(), force, args.json),
        PlaybookAction::History { file } => history(&file, args.json),
    }
}

fn load(file: &Path) -> Result<Playbook, Box<dyn std::error::Error>> {
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

fn validate(file: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
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

fn plan(file: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
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
                    "command": step.command,
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
            if !step.command.is_empty() {
                println!("     命令：{}", step.command);
            }
        }
        if let Some(next) = run.next_ready(&playbook) {
            println!("下一步：{}（{}）", next.id, next.title);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 运行进度：状态文件读写（格式 = PlaybookRun 的 serde 形态）
// ---------------------------------------------------------------------------

/// 默认状态文件：与剧本同目录的 `<stem>.state.json`。
fn default_run_path(file: &Path) -> PathBuf {
    file.with_extension("state.json")
}

#[derive(Debug)]
struct LoadedRun {
    playbook: Playbook,
    run: PlaybookRun,
    path: PathBuf,
    existed: bool,
}

fn ensure_valid(playbook: &Playbook) -> Result<(), Box<dyn std::error::Error>> {
    let issues = playbook.validate();
    if issues.is_empty() {
        return Ok(());
    }
    Err(format!(
        "剧本不合法（{} 个问题），先运行 `owo-agent playbook validate <文件>` 查看",
        issues.len()
    )
    .into())
}

fn load_run(file: &Path, run_path: Option<&Path>) -> Result<LoadedRun, Box<dyn std::error::Error>> {
    let playbook = load(file)?;
    ensure_valid(&playbook)?;
    let path = run_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_run_path(file));
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .map_err(|error| format!("读取状态文件失败（{}）：{error}", path.display()))?;
        let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
        let run: PlaybookRun = serde_json::from_str(content)
            .map_err(|error| format!("状态文件不是合法 JSON（{}）：{error}", path.display()))?;
        if run.playbook_id != playbook.id {
            return Err(format!(
                "状态文件属于另一个剧本（{} ≠ {}）：换 --run 或先删除 {}",
                run.playbook_id,
                playbook.id,
                path.display()
            )
            .into());
        }
        Ok(LoadedRun {
            playbook,
            run,
            path,
            existed: true,
        })
    } else {
        Ok(LoadedRun {
            run: PlaybookRun::new(&playbook),
            playbook,
            path,
            existed: false,
        })
    }
}

/// 原子写：先写临时文件再 rename，避免半截 JSON 让进度不可读。
fn save_run(path: &Path, run: &PlaybookRun) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("创建状态目录失败（{}）：{error}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(run)?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "playbook.state.json".to_string());
    let temporary = path.with_file_name(format!("{file_name}.tmp"));
    std::fs::write(&temporary, json)
        .map_err(|error| format!("写状态文件失败（{}）：{error}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("替换状态文件失败（{}）：{error}", path.display()))?;
    Ok(())
}

fn state_label(run: &PlaybookRun, playbook: &Playbook, step_id: &str) -> (&'static str, String) {
    match run.states.get(step_id).unwrap_or(&StepState::Pending) {
        StepState::Passed { evidence } => ("✔", format!("通过：{evidence}")),
        StepState::Failed { reason, attempts } => ("✘", format!("失败 {attempts} 次：{reason}")),
        StepState::Skipped { reason } => ("⊘", format!("跳过：{reason}")),
        StepState::Pending => {
            if run.is_blocked(playbook, step_id) {
                ("○", "阻塞（前置未通过）".to_string())
            } else {
                ("○", "待执行".to_string())
            }
        }
    }
}

fn progress_json(playbook: &Playbook, run: &PlaybookRun) -> serde_json::Value {
    let progress = run.progress(playbook);
    serde_json::json!({
        "total": progress.total,
        "passed": progress.passed,
        "failed": progress.failed,
        "skipped": progress.skipped,
        "pending": progress.pending,
        "blocked": progress.blocked,
        "percent": progress.percent(),
        "summary": run.summary(playbook),
        "complete": run.is_complete(playbook),
    })
}

fn status(
    file: &Path,
    run_path: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_run(file, run_path)?;
    let next = loaded
        .run
        .next_ready(&loaded.playbook)
        .map(|step| step.id.clone());
    if json {
        let steps: Vec<serde_json::Value> = loaded
            .playbook
            .steps
            .iter()
            .map(|step| {
                let (mark, note) = state_label(&loaded.run, &loaded.playbook, &step.id);
                serde_json::json!({
                    "id": step.id,
                    "title": step.title,
                    "kind": kind_label(step.kind),
                    "mark": mark,
                    "note": note,
                    "attempts": loaded.run.attempts(&step.id),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "id": loaded.playbook.id,
                "name": loaded.playbook.name,
                "state_file": loaded.path.to_string_lossy(),
                "exists": loaded.existed,
                "progress": progress_json(&loaded.playbook, &loaded.run),
                "next": next,
                "steps": steps,
            })
        );
    } else {
        println!(
            "剧本：{}（{}）· 进度 {}",
            loaded.playbook.name,
            loaded.playbook.id,
            loaded.run.summary(&loaded.playbook)
        );
        println!(
            "状态文件：{}（{}）",
            loaded.path.display(),
            if loaded.existed {
                "已存在"
            } else {
                "尚未创建"
            }
        );
        for step in &loaded.playbook.steps {
            let (mark, note) = state_label(&loaded.run, &loaded.playbook, &step.id);
            println!(
                "  {mark} {:<16} {:<6} {}（{}）",
                step.id,
                kind_label(step.kind),
                step.title,
                note
            );
        }
        match next {
            Some(id) => println!("下一步：{id}"),
            None if loaded.run.is_complete(&loaded.playbook) => println!("已全部完成"),
            None => println!("没有可执行步骤：前置失败/跳过导致阻塞"),
        }
    }
    Ok(())
}

fn next(
    file: &Path,
    run_path: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut loaded = load_run(file, run_path)?;
    if !loaded.existed {
        // 首次 next 即落盘：让后续 record 有稳定的状态文件。
        save_run(&loaded.path, &loaded.run)?;
        loaded.existed = true;
    }
    let next_step = loaded.run.next_ready(&loaded.playbook).cloned();
    let blocked: Vec<String> = loaded
        .playbook
        .steps
        .iter()
        .filter(|step| loaded.run.is_blocked(&loaded.playbook, &step.id))
        .map(|step| step.id.clone())
        .collect();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "id": loaded.playbook.id,
                "state_file": loaded.path.to_string_lossy(),
                "next": next_step.as_ref().map(|step| serde_json::json!({
                    "id": step.id,
                    "title": step.title,
                    "kind": kind_label(step.kind),
                    "detail": step.detail,
                    "acceptance": step.acceptance,
                    "depends_on": step.depends_on,
                })),
                "blocked": blocked,
                "progress": progress_json(&loaded.playbook, &loaded.run),
            })
        );
        return Ok(());
    }
    match next_step {
        Some(step) => {
            println!(
                "下一步：{}（{}）· {}",
                step.id,
                step.title,
                kind_label(step.kind)
            );
            if !step.detail.is_empty() {
                println!("  说明：{}", step.detail);
            }
            if !step.acceptance.is_empty() {
                println!("  判据：{}", step.acceptance);
            }
            println!("进度：{}", loaded.run.summary(&loaded.playbook));
        }
        None if loaded.run.is_complete(&loaded.playbook) => {
            println!("已全部完成（{}）", loaded.run.summary(&loaded.playbook));
        }
        None => {
            println!(
                "没有可执行步骤：阻塞 {}（{}）",
                blocked.join("、"),
                loaded.run.summary(&loaded.playbook)
            );
        }
    }
    Ok(())
}

fn record(
    file: &Path,
    step_id: &str,
    pass: Option<String>,
    fail: Option<String>,
    skip: Option<String>,
    run_path: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let outcomes = [pass.is_some(), fail.is_some(), skip.is_some()];
    if outcomes.iter().filter(|present| **present).count() != 1 {
        return Err("必须且只能指定 --pass / --fail / --skip 之一".into());
    }
    let mut loaded = load_run(file, run_path)?;
    if !loaded.playbook.steps.iter().any(|step| step.id == step_id) {
        return Err(format!("剧本里没有步骤 `{step_id}`").into());
    }
    let (state_text, attempts) = if let Some(evidence) = pass {
        loaded.run.record_pass(step_id, evidence);
        ("pass", loaded.run.attempts(step_id))
    } else if let Some(reason) = fail {
        loaded.run.record_fail(step_id, reason);
        ("fail", loaded.run.attempts(step_id))
    } else {
        loaded.run.record_skip(step_id, skip.unwrap_or_default());
        ("skip", 0)
    };
    save_run(&loaded.path, &loaded.run)?;
    let next_id = loaded
        .run
        .next_ready(&loaded.playbook)
        .map(|step| step.id.clone());
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "id": loaded.playbook.id,
                "step": step_id,
                "state": state_text,
                "attempts": attempts,
                "progress": progress_json(&loaded.playbook, &loaded.run),
                "next": next_id,
                "state_file": loaded.path.to_string_lossy(),
            })
        );
        return Ok(());
    }
    println!(
        "已记录：{} → {}（{}）",
        step_id,
        match state_text {
            "pass" => "通过",
            "fail" => "失败",
            _ => "跳过",
        },
        loaded.run.summary(&loaded.playbook)
    );
    match next_id {
        Some(id) => println!("下一步：{id}"),
        None if loaded.run.is_complete(&loaded.playbook) => println!("已全部完成"),
        None => println!("没有可执行步骤：前置失败/跳过导致阻塞"),
    }
    Ok(())
}

/// 目录里的一份剧本 + 其当前进度（`None` = 状态文件不存在）。
struct ListedPlaybook {
    path: PathBuf,
    playbook: Option<Playbook>,
    issues: Vec<PlaybookIssue>,
    run: Option<PlaybookRun>,
}

/// 扫描目录：只认 `*.json`，排除 `*.state.json`；按 id（或文件名）排序。
fn collect_playbooks(dir: &Path) -> Result<Vec<ListedPlaybook>, Box<dyn std::error::Error>> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("读取剧本目录失败（{}）：{error}", dir.display()))?;
    let mut listed: Vec<ListedPlaybook> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_json = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("json"));
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned());
        if !is_json
            || name
                .as_deref()
                .is_some_and(|value| value.ends_with(".state.json"))
        {
            continue;
        }
        match load(&path) {
            Ok(playbook) => {
                let issues = playbook.validate();
                let run = if issues.is_empty() {
                    load_run(&path, None).ok().map(|loaded| loaded.run)
                } else {
                    None
                };
                listed.push(ListedPlaybook {
                    path,
                    playbook: Some(playbook),
                    issues,
                    run,
                });
            }
            Err(error) => listed.push(ListedPlaybook {
                path,
                playbook: None,
                issues: vec![PlaybookIssue {
                    code: "playbook_unreadable",
                    step: None,
                    message: error.to_string(),
                }],
                run: None,
            }),
        }
    }
    listed.sort_by(|left, right| {
        let left_id = left
            .playbook
            .as_ref()
            .map(|playbook| playbook.id.clone())
            .unwrap_or_else(|| left.path.to_string_lossy().into_owned());
        let right_id = right
            .playbook
            .as_ref()
            .map(|playbook| playbook.id.clone())
            .unwrap_or_else(|| right.path.to_string_lossy().into_owned());
        left_id.cmp(&right_id)
    });
    Ok(listed)
}

fn list(dir: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let listed = collect_playbooks(dir)?;
    if json {
        let items: Vec<serde_json::Value> = listed
            .iter()
            .map(|item| {
                let mut value = serde_json::json!({
                    "path": item.path.to_string_lossy(),
                    "ok": item.playbook.is_some() && item.issues.is_empty(),
                    "issues": issues_json(&item.issues),
                });
                if let Some(playbook) = &item.playbook {
                    value["id"] = serde_json::json!(playbook.id);
                    value["name"] = serde_json::json!(playbook.name);
                    value["steps"] = serde_json::json!(playbook.steps.len());
                }
                if let Some(run) = &item.run {
                    let playbook = item.playbook.as_ref().expect("run 只在有剧本时读取");
                    value["progress"] = progress_json(playbook, run);
                    value["next"] =
                        serde_json::json!(run.next_ready(playbook).map(|step| step.id.clone()));
                }
                value
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({ "ok": true, "dir": dir.to_string_lossy(), "playbooks": items })
        );
        return Ok(());
    }
    println!("剧本目录：{}（{} 个）", dir.display(), listed.len());
    if listed.is_empty() {
        println!("  （没有剧本；用 `owo-agent playbook validate <文件>` 开始）");
        return Ok(());
    }
    for item in &listed {
        match &item.playbook {
            Some(playbook) if item.issues.is_empty() => {
                let progress = item
                    .run
                    .as_ref()
                    .map(|run| run.summary(playbook))
                    .unwrap_or_else(|| format!("0/{}（未开始）", playbook.steps.len()));
                println!(
                    "  {:<20} {:<12} {} · {}",
                    playbook.id,
                    playbook.name,
                    playbook.steps.len().to_string() + " 步",
                    progress
                );
            }
            _ => {
                println!(
                    "  {:<20} 不合法（{} 个问题）",
                    item.path
                        .file_stem()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    item.issues.len()
                );
            }
        }
    }
    Ok(())
}

fn reset(
    file: &Path,
    run_path: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // 重置不解析既有状态（状态文件可能已损坏，正好用它清掉）。
    let playbook = load(file)?;
    ensure_valid(&playbook)?;
    let path = run_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_run_path(file));
    let removed = if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("删除状态文件失败（{}）：{error}", path.display()))?;
        true
    } else {
        false
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "id": playbook.id,
                "state_file": path.to_string_lossy(),
                "removed": removed,
            })
        );
    } else if removed {
        println!("已重置：{}", path.display());
    } else {
        println!("没有可重置的状态文件：{}", path.display());
    }
    Ok(())
}

/// 剧本 id 约束：1–64 字符、小写字母/数字/连字符、首尾字母数字。
fn validate_playbook_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.chars().count() > 64 {
        return Err("剧本 id 长度必须在 1–64 之间".to_string());
    }
    if !id.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        return Err("剧本 id 只允许小写字母、数字与连字符".to_string());
    }
    let first = id.chars().next().unwrap_or(' ');
    let last = id.chars().last().unwrap_or(' ');
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return Err("剧本 id 必须以字母或数字开头/结尾".to_string());
    }
    Ok(())
}

/// 骨架：plan(model) → check(check，命令留空=人工判定) → deliver(human)。
fn skeleton(id: &str) -> Playbook {
    let step =
        |id: &str, title: &str, kind: StepKind, detail: &str, acceptance: &str, dep: &[&str]| {
            PlaybookStep {
                id: id.to_string(),
                title: title.to_string(),
                detail: detail.to_string(),
                acceptance: acceptance.to_string(),
                command: String::new(),
                depends_on: dep.iter().map(|value| value.to_string()).collect(),
                kind,
            }
        };
    Playbook {
        id: id.to_string(),
        name: id.to_string(),
        description: "TODO：说明这类任务的输入、产出与边界".to_string(),
        steps: vec![
            step(
                "plan",
                "确认任务范围",
                StepKind::Model,
                "读输入、确认产出与不做的事。",
                "列出输入、产出与不做的事",
                &[],
            ),
            step(
                "check",
                "可自动校验的检查",
                StepKind::Check,
                "能写成一条「退出码即结论」的命令就填 command；留空则人工判定。",
                "TODO：写可判定的判据",
                &["plan"],
            ),
            step(
                "deliver",
                "评审与交付",
                StepKind::Human,
                "汇总证据，交给人工确认。",
                "人工确认结果",
                &["check"],
            ),
        ],
    }
}

fn init(
    id: &str,
    out: Option<&Path>,
    force: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_playbook_id(id).map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let path = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("playbooks").join(format!("{id}.json")));
    if path.exists() && !force {
        return Err(format!("{} 已存在（用 --force 覆盖）", path.display()).into());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("创建目录失败（{}）：{error}", parent.display()))?;
        }
    }
    let playbook = skeleton(id);
    let content = serde_json::to_string_pretty(&playbook)?;
    std::fs::write(&path, content)
        .map_err(|error| format!("写入失败（{}）：{error}", path.display()))?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "id": playbook.id,
                "path": path.to_string_lossy(),
                "steps": playbook.steps.len(),
            })
        );
    } else {
        println!("已生成剧本骨架：{}", path.display());
        println!("下一步：用 `owo-agent playbook validate` 校验，再按需补 command 与判据");
    }
    Ok(())
}

/// 归档目录：与剧本同级的 `<stem>.runs/`。
fn runs_dir(file: &Path) -> PathBuf {
    file.with_extension("runs")
}

fn validate_marker(marker: &str) -> Result<(), String> {
    if marker.is_empty() || marker.chars().count() > 64 {
        return Err("归档标记长度必须在 1–64 之间".to_string());
    }
    if !marker
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
    {
        return Err("归档标记只允许字母、数字与 . - _".to_string());
    }
    if marker.starts_with('.') || marker.contains("..") {
        return Err("归档标记不能以 . 开头或包含 ..".to_string());
    }
    Ok(())
}

/// 把当前状态快照到 `<剧本>.runs/<标记>.json`（不改动当前状态）。
fn archive(
    file: &Path,
    marker: Option<&str>,
    force: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let playbook = load(file)?;
    ensure_valid(&playbook)?;
    let state = default_run_path(file);
    if !state.exists() {
        return Err("还没有运行状态：先用 playbook next/record 产生进度，再归档".into());
    }
    let marker = marker.map(str::to_string).unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis().to_string())
            .unwrap_or_else(|_| "0".to_string())
    });
    validate_marker(&marker).map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let dir = runs_dir(file);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("创建归档目录失败（{}）：{error}", dir.display()))?;
    let target = dir.join(format!("{marker}.json"));
    if target.exists() && !force {
        return Err(format!("归档 {} 已存在（用 --force 覆盖）", target.display()).into());
    }
    std::fs::copy(&state, &target)
        .map_err(|error| format!("写入归档失败（{}）：{error}", target.display()))?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "id": playbook.id,
                "marker": marker,
                "archive": target.to_string_lossy(),
            })
        );
    } else {
        println!("已归档：{}（{}）", target.display(), marker);
    }
    Ok(())
}

/// 列出历次归档与各自进度（按标记倒序）。
fn history(file: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let playbook = load(file)?;
    ensure_valid(&playbook)?;
    let dir = runs_dir(file);
    if !dir.exists() {
        if json {
            println!(
                "{}",
                serde_json::json!({ "ok": true, "id": playbook.id, "runs": [] })
            );
        } else {
            println!("还没有归档（用 playbook archive 保存一次运行）");
        }
        return Ok(());
    }
    let entries = std::fs::read_dir(&dir)
        .map_err(|error| format!("读取归档目录失败（{}）：{error}", dir.display()))?;
    let mut runs: Vec<(String, Option<PlaybookRun>)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let marker = path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let run = std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str::<PlaybookRun>(&content).ok())
            .filter(|run| run.playbook_id == playbook.id);
        runs.push((marker, run));
    }
    runs.sort_by(|left, right| right.0.cmp(&left.0));
    if json {
        let items: Vec<serde_json::Value> = runs
            .iter()
            .map(|(marker, run)| match run {
                Some(run) => serde_json::json!({
                    "marker": marker,
                    "progress": progress_json(&playbook, run),
                    "complete": run.is_complete(&playbook),
                }),
                None => serde_json::json!({ "marker": marker, "invalid": true }),
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({ "ok": true, "id": playbook.id, "runs": items })
        );
        return Ok(());
    }
    println!("剧本 {} 的归档（{} 次）：", playbook.id, runs.len());
    for (marker, run) in &runs {
        match run {
            Some(run) => println!(
                "  {marker:<24} {} {}",
                run.summary(&playbook),
                if run.is_complete(&playbook) {
                    "已完成"
                } else {
                    ""
                }
            ),
            None => println!("  {marker:<24} （无法解析或属于其它剧本）"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("owo-playbook-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_playbook(dir: &Path) -> PathBuf {
        let file = dir.join("demo.json");
        std::fs::write(
            &file,
            r#"{"id":"demo","name":"演示","steps":[
                {"id":"a","title":"A"},
                {"id":"b","title":"B","depends_on":["a"]},
                {"id":"c","title":"C","depends_on":["b"]}
            ]}"#,
        )
        .unwrap();
        file
    }

    #[test]
    fn archive_requires_state_and_validates_markers() {
        let dir = scratch("archive");
        let file = write_playbook(&dir);
        // 没有状态文件不能归档。
        assert!(archive(&file, Some("run-1"), false, false).is_err());

        let mut run = load_run(&file, None).unwrap().run;
        run.record_pass("a", "证据");
        save_run(&default_run_path(&file), &run).unwrap();

        archive(&file, Some("run-1"), false, false).unwrap();
        let archived = runs_dir(&file).join("run-1.json");
        assert!(archived.exists());
        // 重复标记需 --force；非法标记拒绝。
        assert!(archive(&file, Some("run-1"), false, false).is_err());
        archive(&file, Some("run-1"), true, false).unwrap();
        assert!(archive(&file, Some("../escape"), false, false).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_lists_archives_newest_first() {
        let dir = scratch("history");
        let file = write_playbook(&dir);
        let mut run = load_run(&file, None).unwrap().run;
        run.record_pass("a", "证据");
        save_run(&default_run_path(&file), &run).unwrap();
        archive(&file, Some("run-1"), false, false).unwrap();
        archive(&file, Some("run-2"), false, false).unwrap();

        let listed = {
            let dir = runs_dir(&file);
            let mut markers: Vec<String> = std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            markers.sort();
            markers
        };
        assert_eq!(listed, vec!["run-1.json", "run-2.json"]);
        // history 只读：再次载入进度仍然只来自当前状态文件。
        let reloaded = load_run(&file, None).unwrap();
        assert_eq!(reloaded.run.progress(&reloaded.playbook).passed, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_writes_valid_skeleton_and_refuses_overwrite() {
        let dir = scratch("init");
        let path = dir.join("playbooks").join("demo.json");
        init("demo", Some(&path), false, false).unwrap();
        let playbook = load(&path).unwrap();
        assert_eq!(playbook.id, "demo");
        assert!(playbook.validate().is_empty(), "{:?}", playbook.validate());
        assert_eq!(playbook.steps.len(), 3);

        // 默认不覆盖；--force 可以。
        assert!(init("demo", Some(&path), false, false).is_err());
        init("demo", Some(&path), true, false).unwrap();

        // id 约束。
        assert!(validate_playbook_id("Bad").is_err());
        assert!(validate_playbook_id("-lead").is_err());
        assert!(validate_playbook_id("a b").is_err());
        assert!(validate_playbook_id("ok-2").is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_scans_json_playbooks_and_reports_progress() {
        let dir = scratch("list");
        let file = write_playbook(&dir);
        let mut run = load_run(&file, None).unwrap().run;
        run.record_pass("a", "证据");
        save_run(&default_run_path(&file), &run).unwrap();

        std::fs::write(
            dir.join("broken.json"),
            r#"{"id":"broken","name":"坏","steps":[]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "忽略").unwrap();

        let listed = collect_playbooks(&dir).unwrap();
        // 只认 .json 且排除 *.state.json：broken + demo 两个。
        assert_eq!(
            listed.len(),
            2,
            "{:?}",
            listed.iter().map(|i| &i.path).collect::<Vec<_>>()
        );
        let demo = listed
            .iter()
            .find(|item| item.path.file_name().unwrap() == "demo.json")
            .expect("demo 应在列表里");
        assert!(demo.issues.is_empty());
        let run = demo.run.as_ref().expect("demo 有状态文件");
        assert_eq!(run.progress(demo.playbook.as_ref().unwrap()).passed, 1);

        let broken = listed
            .iter()
            .find(|item| item.path.file_name().unwrap() == "broken.json")
            .expect("broken 应在列表里");
        assert!(!broken.issues.is_empty());
        assert!(broken.run.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kind_labels_are_stable_for_scripts() {
        assert_eq!(kind_label(StepKind::Model), "model");
        assert_eq!(kind_label(StepKind::Check), "check");
        assert_eq!(kind_label(StepKind::Human), "human");
    }

    #[test]
    fn default_state_file_lives_next_to_the_playbook() {
        assert_eq!(
            default_run_path(Path::new("playbooks/code-review.json")),
            PathBuf::from("playbooks/code-review.state.json")
        );
    }

    #[test]
    fn record_persists_and_advances_the_progress() {
        let dir = scratch("record");
        let file = write_playbook(&dir);

        let mut loaded = load_run(&file, None).unwrap();
        assert!(!loaded.existed);
        assert_eq!(loaded.run.next_ready(&loaded.playbook).unwrap().id, "a");

        loaded.run.record_pass("a", "证据 A");
        save_run(&loaded.path, &loaded.run).unwrap();
        assert!(loaded.path.exists());

        // 重新载入：状态与尝试次数都在。
        let mut reloaded = load_run(&file, None).unwrap();
        assert!(reloaded.existed);
        assert_eq!(reloaded.run.next_ready(&reloaded.playbook).unwrap().id, "b");
        reloaded.run.record_fail("b", "第一次失败");
        reloaded.run.record_fail("b", "第二次失败");
        save_run(&reloaded.path, &reloaded.run).unwrap();
        let again = load_run(&file, None).unwrap();
        assert_eq!(again.run.attempts("b"), 2);
        assert!(again.run.is_blocked(&again.playbook, "c"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_file_for_another_playbook_is_rejected() {
        let dir = scratch("mismatch");
        let file = write_playbook(&dir);
        let run_path = dir.join("other.state.json");
        let mut foreign = PlaybookRun::new(&load(&file).unwrap());
        foreign.playbook_id = "another".to_string();
        save_run(&run_path, &foreign).unwrap();
        let error = load_run(&file, Some(&run_path)).unwrap_err();
        assert!(error.to_string().contains("另一个剧本"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_save_leaves_valid_json_and_no_temp_file() {
        let dir = scratch("atomic");
        let file = write_playbook(&dir);
        let loaded = load_run(&file, None).unwrap();
        save_run(&loaded.path, &loaded.run).unwrap();
        let content = std::fs::read_to_string(&loaded.path).unwrap();
        let parsed: PlaybookRun = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.playbook_id, "demo");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "不应残留临时文件");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
