//! `owo-agent review`：逐 hunk 审阅工作区改动（G13：接受/拒绝单个 hunk）。
//!
//! 数据来源是**工作树 vs HEAD**（不需要 Daemon/模型）：
//! - 原始内容 = `git show HEAD:<path>`；当前内容 = 磁盘文件；
//! - 切分用 `owo_agent_core::diff_hunks`（纯函数），列表里看到的 hunk 序号
//!   就是 `--accept/--reject` 的序号；
//! - `--accept/--reject` 会**真实写盘**（原子替换），默认不写（只列清单）。
//!
//! 边界与诚实性：只处理"已跟踪且已修改"的文件；新增/删除文件不在此命令范围；
//! 写盘前校验路径必须落在工作区内；写入后会话快照不受影响（会话 `revert` 仍会
//! 整体回滚到会话开始前的原内容——命令输出会把这点讲清楚）。

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;
use colored::Colorize;
use owo_agent_core::{apply_hunks, diff_hunks, DiffHunk};
use serde_json::json;

/// 单文件审阅上限（超过则拒绝，避免把大文件整份读进内存再切分）。
const MAX_REVIEW_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Args)]
pub struct ReviewArgs {
    /// 工作区（缺省当前目录；与 --session 互斥，会话模式以会话工作区为准）
    #[arg(long)]
    pub workspace: Option<PathBuf>,
    /// 审阅指定会话的 Agent 改动（经 Daemon；与 --workspace 互斥）
    #[arg(long)]
    pub session: Option<String>,
    /// 只审阅该文件（相对工作区；--accept/--reject 必填）
    #[arg(long)]
    pub path: Option<String>,
    /// 接受这些 hunk 序号（1 起，逗号/空格分隔，或 all）
    #[arg(long)]
    pub accept: Option<String>,
    /// 拒绝这些 hunk 序号（1 起，逗号/空格分隔，或 all）
    #[arg(long)]
    pub reject: Option<String>,
    /// 机器可读输出（JSON）
    #[arg(long)]
    pub json: bool,
    /// 数据根（缺省用户级数据目录）；`--session` 时用于发现/启动共享 Daemon
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

pub(crate) fn run_review_cmd(args: ReviewArgs) -> Result<(), Box<dyn std::error::Error>> {
    let wants_apply = args.accept.is_some() || args.reject.is_some();
    if wants_apply && args.path.is_none() {
        return Err("--accept/--reject 需要同时指定 --path（一次审阅一个文件，避免误写）".into());
    }
    if let Some(session_id) = args.session.clone() {
        // 会话模式：走 Daemon 的两条路由（快照护栏与审计都在服务端，不在本地算/写）。
        return review_session(&args, &session_id);
    }
    let workspace_arg = args.workspace.clone().unwrap_or_else(|| PathBuf::from("."));
    let workspace = workspace_arg
        .canonicalize()
        .map_err(|error| format!("工作区不可用：{}（{error}）", workspace_arg.display()))?;

    let files = match args.path.as_deref() {
        Some(relative) => {
            let path = resolve_in_workspace(&workspace, relative)?;
            if !is_tracked_modified(&workspace, relative) {
                return Err(format!(
                    "该文件不是本工作区「已跟踪且已修改」的文件：{relative}（新增/删除/未跟踪文件不参与逐 hunk 审阅）"
                )
                .into());
            }
            vec![(relative.to_string(), path)]
        }
        None => list_modified_files(&workspace)?
            .into_iter()
            .map(|relative| {
                let path = resolve_in_workspace(&workspace, &relative)
                    .unwrap_or_else(|_| workspace.join(&relative));
                (relative, path)
            })
            .collect(),
    };

    let wants_apply = args.accept.is_some() || args.reject.is_some();

    let mut report = Vec::new();
    for (relative, absolute) in files {
        let original = git_show(&workspace, &relative)?;
        let modified = read_text(&absolute)?;
        let hunks = diff_hunks(&original, &modified);
        let selection = if wants_apply {
            Some(resolve_selection(&args, hunks.len())?)
        } else {
            None
        };
        if let Some(selection) = selection {
            let next = apply_hunks(&original, &modified, &selection.accepted);
            write_atomically(&absolute, &next)?;
            let remaining = diff_hunks(&original, &next);
            report.push(FileReview {
                path: relative.clone(),
                hunks: hunks.clone(),
                applied: true,
                accepted: selection.accepted.clone(),
                rejected: selection.rejected.clone(),
                remaining: remaining.len(),
            });
            if !args.json {
                println!(
                    "{} {}：接收 {} 个 hunk（{}），还原 {} 个；写盘后剩余改动 {} 处",
                    "已应用".green(),
                    relative,
                    selection.accepted.len(),
                    numbers(&selection.accepted),
                    selection.rejected.len(),
                    remaining.len()
                );
            }
        } else {
            report.push(FileReview {
                path: relative.clone(),
                hunks: hunks.clone(),
                applied: false,
                accepted: Vec::new(),
                rejected: Vec::new(),
                remaining: hunks.len(),
            });
            if !args.json {
                print_file_review(&relative, &original, &modified, &hunks);
            }
        }
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if !wants_apply {
        let total: usize = report.iter().map(|file| file.hunks.len()).sum();
        if total == 0 {
            println!("（工作区相对 HEAD 没有改动）");
        } else {
            println!(
                "\n共 {} 个文件 / {} 个 hunk。应用示例：owo-agent review --path <文件> --accept 1,3（序号从 1 起）",
                report.iter().filter(|file| !file.hunks.is_empty()).count(),
                total
            );
        }
    } else {
        println!("注意：会话快照未随之更新，会话内 /revert 仍会整体回到会话开始前的内容。");
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
struct FileReview {
    path: String,
    hunks: Vec<DiffHunk>,
    applied: bool,
    /// 已接受的 hunk 序号（1 起）。
    accepted: Vec<usize>,
    /// 被拒绝（保持原样）的 hunk 序号（1 起）。
    rejected: Vec<usize>,
    /// 写盘后相对 HEAD 仍存在的改动处数。
    remaining: usize,
}

struct Selection {
    accepted: Vec<usize>,
    rejected: Vec<usize>,
}

/// 把 `--accept/--reject` 解析为 0 起的接受集合；两者同时给视为冲突。
fn resolve_selection(
    args: &ReviewArgs,
    total: usize,
) -> Result<Selection, Box<dyn std::error::Error>> {
    if args.accept.is_some() && args.reject.is_some() {
        return Err("--accept 与 --reject 不能同时使用（语义会互相冲突）".into());
    }
    if let Some(spec) = args.accept.as_deref() {
        let accepted = parse_selection(spec, total)?;
        let rejected = (0..total)
            .filter(|index| !accepted.contains(index))
            .collect();
        return Ok(Selection { accepted, rejected });
    }
    let spec = args.reject.as_deref().unwrap_or_default();
    let rejected = parse_selection(spec, total)?;
    let accepted = (0..total)
        .filter(|index| !rejected.contains(index))
        .collect();
    Ok(Selection { accepted, rejected })
}

/// `--session <id>`：审阅该会话里 Agent 的改动（经 Daemon 的 `/hunks` 与 `/hunks/apply`）。
///
/// 本地不重新计算 hunk：序号、冲突判定与写盘护栏都以服务端视图为准，
/// 避免"CLI 显示一套、服务端执行另一套"。
fn review_session(args: &ReviewArgs, session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let workspace_hint = args
        .workspace
        .clone()
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."));
    let root = crate::support::ensure_data_root(args.data_dir.clone(), &workspace_hint);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let client = crate::support::ensure_daemon_client(&root, &workspace_hint).await?;
        let view = client
            .get_json::<serde_json::Value>(&format!("/session/{session_id}/hunks"))
            .await?;
        let empty = Vec::new();
        let files = view
            .get("files")
            .and_then(serde_json::Value::as_array)
            .unwrap_or(&empty);
        let conflicts: Vec<String> = view
            .get("conflicts")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        if let Some(relative) = args.path.as_deref() {
            let file = files
                .iter()
                .find(|file| file.get("path").and_then(|value| value.as_str()) == Some(relative))
                .ok_or_else(|| format!("会话改动里没有该文件：{relative}"))?;
            let hunks: Vec<owo_agent_core::DiffHunk> =
                serde_json::from_value(file.get("hunks").cloned().unwrap_or_else(|| json!([])))
                    .map_err(|error| format!("解析 hunk 视图失败：{error}"))?;
            if hunks.is_empty() {
                let reason = file
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("该文件不可逐 hunk 审阅");
                println!("（{relative}：{reason}）");
                return Ok(());
            }
            let selection = resolve_selection(args, hunks.len())?;
            // 只发用户指定的那一侧：服务端按对侧补全。两侧都发会被判为语义冲突（422）。
            let object = match (args.accept.as_deref(), args.reject.as_deref()) {
                (Some(spec), None) => json!({
                    "path": relative,
                    "accept": parse_selection(spec, hunks.len())?,
                }),
                (None, Some(spec)) => json!({
                    "path": relative,
                    "reject": parse_selection(spec, hunks.len())?,
                }),
                _ => return Err("--accept 与 --reject 不能同时使用".into()),
            };
            let applied = client
                .post_json::<_, serde_json::Value>(
                    &format!("/session/{session_id}/hunks/apply"),
                    &object,
                )
                .await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&applied)?);
            } else {
                let _ = selection;
                println!("{}", crate::support::render_hunks_apply(&applied));
            }
            return Ok(());
        }

        // 列表模式：人类输出走共享渲染器（Daemon REPL `/hunks` 用同一份，不各写一套）。
        if args.json {
            let rows: Vec<serde_json::Value> = files
                .iter()
                .map(|file| {
                    let path = file
                        .get("path")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let mut row = file.clone();
                    if let Some(object) = row.as_object_mut() {
                        object.insert(
                            "conflict".to_string(),
                            serde_json::json!(conflicts.contains(&path)),
                        );
                    }
                    row
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
            return Ok(());
        }
        println!(
            "{}",
            crate::support::render_hunks_view(
                session_id,
                &serde_json::json!({ "files": files, "conflicts": conflicts }),
            )
        );
        Ok(())
    })
}

fn parse_selection(spec: &str, total: usize) -> Result<Vec<usize>, String> {
    let trimmed = spec.trim();
    if trimmed.eq_ignore_ascii_case("all") {
        return Ok((0..total).collect());
    }
    if trimmed.is_empty() {
        return Err("序号为空：请给 1 起的序号（如 --accept 1,3）或 all".to_string());
    }
    let mut indices = Vec::new();
    for token in trimmed
        .split([',', ' ', ';'])
        .filter(|token| !token.is_empty())
    {
        let number: usize = token
            .parse()
            .map_err(|_| format!("无法解析 hunk 序号：{token}（应为 1 起的整数或 all）"))?;
        if number == 0 || number > total {
            return Err(format!(
                "hunk 序号越界：{number}（本文件共 {total} 个 hunk，序号从 1 起）"
            ));
        }
        let index = number - 1;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    indices.sort_unstable();
    if indices.is_empty() {
        return Err("没有有效序号".to_string());
    }
    Ok(indices)
}

/// 工作区内的相对路径：拒绝绝对路径、`..` 与盘符，避免写到工作区外。
fn resolve_in_workspace(workspace: &Path, relative: &str) -> Result<PathBuf, String> {
    let trimmed = relative.trim();
    if trimmed.is_empty() {
        return Err("路径为空".to_string());
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err(format!("只接受工作区内相对路径：{trimmed}"));
    }
    for component in candidate.components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            other => {
                return Err(format!(
                    "路径不得包含 {:?}（只能写工作区内文件）：{trimmed}",
                    other
                ))
            }
        }
    }
    let joined = workspace.join(candidate);
    if let Ok(canonical) = joined.canonicalize() {
        if !canonical.starts_with(workspace) {
            return Err(format!("路径越出工作区：{trimmed}"));
        }
    }
    Ok(joined)
}

fn numbers(indices: &[usize]) -> String {
    indices
        .iter()
        .map(|index| (index + 1).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn git(workspace: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .map_err(|error| format!("无法执行 git（逐 hunk 审阅依赖 git）：{error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {:?} 失败：{}", args, stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 已跟踪且已修改（`git diff --name-only HEAD`）的相对路径列表。
fn list_modified_files(workspace: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let stdout = git(workspace, &["diff", "--name-only", "HEAD", "--"])?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn is_tracked_modified(workspace: &Path, relative: &str) -> bool {
    list_modified_files(workspace)
        .map(|files| files.iter().any(|file| file == relative))
        .unwrap_or(false)
}

fn git_show(workspace: &Path, relative: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(git(workspace, &["show", &format!("HEAD:{relative}")])?)
}

fn read_text(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("读取失败 {}：{error}", path.display()))?;
    if metadata.len() > MAX_REVIEW_BYTES {
        return Err(format!(
            "文件过大（{} 字节 > 上限 {} 字节）：逐 hunk 审阅只处理小文件",
            metadata.len(),
            MAX_REVIEW_BYTES
        )
        .into());
    }
    Ok(std::fs::read_to_string(path)?)
}

/// 原子替换：先写同目录临时文件再 rename（同卷 rename 不会被中断成半截文件）。
fn write_atomically(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法定位目录：{}", path.display()))?;
    let temp = parent.join(format!(
        ".{}.owo-review-tmp",
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string())
    ));
    std::fs::write(&temp, content)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

fn print_file_review(relative: &str, original: &str, modified: &str, hunks: &[DiffHunk]) {
    if hunks.is_empty() {
        return;
    }
    println!("{} {}", "文件".bold(), relative);
    for hunk in hunks {
        let removed: Vec<&str> = owo_agent_core::hunk_original_lines(original, hunk);
        let added: Vec<&str> = owo_agent_core::hunk_modified_lines(modified, hunk);
        println!(
            "  #{} {} @{} → {}（-{} +{}）",
            hunk.index + 1,
            hunk.kind(),
            hunk.old_start + 1,
            hunk.new_start + 1,
            hunk.old_lines,
            hunk.new_lines
        );
        for line in removed.iter().take(3) {
            println!("    - {}", line.trim_end());
        }
        if removed.len() > 3 {
            println!("    - …（共 {} 行）", removed.len());
        }
        for line in added.iter().take(3) {
            println!("    + {}", line.trim_end());
        }
        if added.len() > 3 {
            println!("    + …（共 {} 行）", added.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(accept: Option<&str>, reject: Option<&str>) -> ReviewArgs {
        ReviewArgs {
            workspace: Some(PathBuf::from(".")),
            session: None,
            path: None,
            accept: accept.map(str::to_string),
            reject: reject.map(str::to_string),
            json: false,
            data_dir: None,
        }
    }

    #[test]
    fn selection_accepts_one_based_lists_and_all() {
        assert_eq!(parse_selection("1,3", 3).unwrap(), vec![0, 2]);
        assert_eq!(parse_selection(" 2 1 ", 3).unwrap(), vec![0, 1]);
        assert_eq!(parse_selection("all", 2).unwrap(), vec![0, 1]);
        // 重复序号去重。
        assert_eq!(parse_selection("1,1,2", 2).unwrap(), vec![0, 1]);
    }

    #[test]
    fn selection_rejects_out_of_range_and_garbage() {
        assert!(parse_selection("0", 3).is_err());
        assert!(parse_selection("4", 3).is_err());
        assert!(parse_selection("x", 3).is_err());
        assert!(parse_selection("", 3).is_err());
        assert!(parse_selection("all", 0).unwrap().is_empty());
    }

    #[test]
    fn accept_and_reject_are_mutually_exclusive_with_complement_fill() {
        let selection = resolve_selection(&args(Some("1"), None), 3).unwrap();
        assert_eq!(selection.accepted, vec![0]);
        assert_eq!(selection.rejected, vec![1, 2]);

        let selection = resolve_selection(&args(None, Some("2")), 3).unwrap();
        assert_eq!(selection.rejected, vec![1]);
        assert_eq!(selection.accepted, vec![0, 2]);

        assert!(resolve_selection(&args(Some("1"), Some("2")), 3).is_err());
    }

    #[test]
    fn workspace_containment_rejects_escapes_and_absolute_paths() {
        let workspace = std::env::temp_dir();
        let workspace = workspace.canonicalize().unwrap();
        assert!(resolve_in_workspace(&workspace, "sub/file.rs").is_ok());
        assert!(resolve_in_workspace(&workspace, r"..\escape.rs").is_err());
        assert!(resolve_in_workspace(&workspace, "../escape.rs").is_err());
        assert!(resolve_in_workspace(&workspace, r"C:\Windows\system.ini").is_err());
        assert!(resolve_in_workspace(&workspace, "").is_err());
    }

    #[test]
    fn atomic_write_replaces_content_without_leaving_temp_files() {
        let dir = std::env::temp_dir().join(format!("owo-review-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "old\n").unwrap();
        write_atomically(&file, "new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("owo-review-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
