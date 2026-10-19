//! Git 只读工具（取优合并自远端 engine）：`git_status` / `git_diff` / `git_log`。
//!
//! 设计：
//! - **只读**：固定子命令白名单，不接受模型拼任意 git 参数；不提供 commit/push
//!   （写操作继续走 run_command 的 Execute 审批，保持权限模型单一出口）；
//! - 免审批：三个工具在效应矩阵映射为 `EffectClass::Read`（宿主验证只读），
//!   可进入只读并发组；
//! - 输出截断（diff/log 上限），超限给出续读提示；
//! - `git` 不存在 / 非 git 仓库 → 可读错误（引导改用 run_command 或换工作区）。

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::{Tool, ToolContext, ToolSpec};

const DIFF_MAX_CHARS: usize = 20_000;
const LOG_MAX_CHARS: usize = 10_000;
const STATUS_MAX_CHARS: usize = 10_000;

fn truncate_chars(text: &str, max: usize) -> String {
    bounded_text(text, max).0
}

/// 截断并返回 `(文本, 是否截断, 原字符数)`。
///
/// 机器可读的 `truncated` 很关键：只把提示写进正文时，模型（和评测的完成度判定）
/// 无法可靠区分"改动就这么大"和"被截断了"——本仓库其它读取类工具都返回该字段。
fn bounded_text(text: &str, max: usize) -> (String, bool, usize) {
    let total = text.chars().count();
    if total <= max {
        return (text.to_string(), false, total);
    }
    let head: String = text.chars().take(max).collect();
    (
        format!("{head}\n\n[输出已截断（原 {total} 字符）。可用更细的过滤参数分次查看]"),
        true,
        total,
    )
}

/// 执行只读 git 子命令：`git -C <workspace> -c core.quotepath=false <args...>`。
async fn run_git(workspace: &Path, args: &[&str], timeout_secs: u64) -> Result<String, String> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(workspace)
        .arg("-c")
        .arg("core.quotepath=false") // 中文文件名不转义
        .args(args)
        .stdin(std::process::Stdio::null());
    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), command.output())
        .await
        .map_err(|_| {
            format!(
                "git {0} 超时（{timeout_secs}s）",
                args.first().unwrap_or(&"")
            )
        })?
        .map_err(|e| format!("git 启动失败（未安装或不在 PATH？）：{e}"))?;
    if !output.status.success() {
        let stderr = crate::tools::decode_process_output(&output.stderr);
        if stderr.contains("not a git repository") {
            return Err("当前工作区不是 git 仓库（可在会话里切换到仓库目录，或用 run_command 执行 git init）".to_string());
        }
        return Err(format!("git 失败：{}", truncate_chars(stderr.trim(), 500)));
    }
    Ok(crate::tools::decode_process_output(&output.stdout).to_string())
}

async fn git_status_impl(workspace: &Path, args: Value) -> Result<Value, String> {
    let include_untracked = args
        .get("include_untracked")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut command = vec!["status", "--porcelain=v1", "-b"];
    if include_untracked {
        command.push("-unormal");
    }
    let stdout = run_git(workspace, &command, 20).await?;
    let lines: Vec<&str> = stdout.lines().collect();
    let (status, truncated, total_chars) = bounded_text(&stdout, STATUS_MAX_CHARS);
    Ok(json!({
        "branch": lines.first().copied().unwrap_or_default().strip_prefix("## ").unwrap_or_default(),
        "changes": lines.len().saturating_sub(1),
        "status": status,
        "truncated": truncated,
        "total_chars": total_chars,
    }))
}

async fn git_diff_impl(workspace: &Path, args: Value) -> Result<Value, String> {
    let staged = args.get("staged").and_then(Value::as_bool).unwrap_or(false);
    // `stat_only`：改动很大时先看"改了哪些文件、各多少行"，再决定是否读补丁。
    // 只读一次 diff 全文在很多仓库里就是几十万字符，先看 --stat 是标准工作流。
    let stat_only = args
        .get("stat_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let context_lines = args
        .get("context_lines")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .clamp(0, 10);
    let path = args.get("path").and_then(Value::as_str);
    let mut command = vec!["diff".to_string()];
    if stat_only {
        command.push("--stat".to_string());
    } else {
        command.push(format!("-U{context_lines}"));
    }
    if staged {
        command.push("--cached".to_string());
    }
    if let Some(path) = path {
        command.push("--".to_string());
        command.push(path.to_string());
    }
    let refs: Vec<&str> = command.iter().map(String::as_str).collect();
    let stdout = run_git(workspace, &refs, 30).await?;
    if stdout.trim().is_empty() {
        return Ok(json!({
            "empty": true,
            "staged": staged,
            "diff": "",
            "stat": "",
            "truncated": false,
            "total_chars": 0,
            "hint": if staged { "暂存区无改动（staged）。工作区改动用 staged=false 查看" } else { "工作区相对 HEAD 无未暂存改动" },
        }));
    }
    let (body, truncated, total_chars) = bounded_text(&stdout, DIFF_MAX_CHARS);
    if stat_only {
        let files = stdout.lines().filter(|line| line.contains('|')).count();
        return Ok(json!({
            "empty": false,
            "staged": staged,
            "stat": body,
            "files": files,
            "patch_omitted": true,
            "truncated": truncated,
            "total_chars": total_chars,
            "hint": "这是 --stat 概览；要看具体补丁用 stat_only=false，并可用 path 过滤单文件",
        }));
    }
    Ok(json!({
        "empty": false,
        "staged": staged,
        "diff": body,
        "truncated": truncated,
        "total_chars": total_chars,
        "hint": if truncated {
            "补丁被截断：用 path 过滤单文件，或调小 context_lines 分次查看"
        } else {
            ""
        },
    }))
}

async fn git_log_impl(workspace: &Path, args: Value) -> Result<Value, String> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100);
    let limit_text = limit.to_string();
    let path = args.get("path").and_then(Value::as_str);
    let mut command = vec!["log", "--oneline", "--decorate", "-n", limit_text.as_str()];
    let path_owned;
    if let Some(path) = path {
        command.push("--");
        path_owned = path.to_string();
        command.push(&path_owned);
    }
    let stdout = run_git(workspace, &command, 20).await?;
    let (log, truncated, total_chars) = bounded_text(&stdout, LOG_MAX_CHARS);
    Ok(json!({
        "commits": stdout.lines().count(),
        "log": log,
        "truncated": truncated,
        "total_chars": total_chars,
    }))
}

/// 宿主验证只读效应：免审批 + 可进入只读并发组（本地并发判定读 spec.effect）。
fn readonly_effect(name: &str) -> Option<crate::tool_effects::ToolEffect> {
    Some(crate::tool_effects::ToolEffect {
        tool: name.to_string(),
        class: crate::tool_effects::EffectClass::Read,
        source: "builtin".to_string(),
        risk_note: None,
        annotations: None,
        host_verified_readonly: true,
    })
}

macro_rules! readonly_git_tool {
    ($tool:ident, $name:literal, $desc:literal, $impl:ident, $schema:expr) => {
        pub struct $tool;

        #[async_trait]
        impl Tool for $tool {
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: $name.into(),
                    description: $desc.into(),
                    input_schema: $schema,
                    effect: readonly_effect($name),
                }
            }

            async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
                $impl(ctx.workspace, args).await
            }
        }
    };
}

readonly_git_tool!(
    GitStatusTool,
    "git_status",
    "查看 git 仓库状态（当前分支 + 变更文件列表，porcelain 格式）。只读、免审批。提交/暂存等写操作请用 run_command。",
    git_status_impl,
    json!({
        "type": "object",
        "properties": {
            "include_untracked": { "type": "boolean", "description": "包含未跟踪文件（默认 true）" }
        }
    })
);

readonly_git_tool!(
    GitDiffTool,
    "git_diff",
    "查看未提交改动的 diff（工作区 vs HEAD；staged=true 看暂存区 vs HEAD）。可按文件过滤；\
     改动大时先用 stat_only=true 看 --stat 概览。只读、免审批。",
    git_diff_impl,
    json!({
        "type": "object",
        "properties": {
            "staged": { "type": "boolean", "description": "查看暂存区改动（默认 false = 工作区）" },
            "path": { "type": "string", "description": "仅查看该文件的改动" },
            "context_lines": { "type": "integer", "description": "上下文行数（默认 3，最大 10）" },
            "stat_only": { "type": "boolean", "description": "只看 --stat 概览（改了哪些文件各多少行），默认 false 返回完整补丁" }
        }
    })
);

readonly_git_tool!(
    GitLogTool,
    "git_log",
    "查看提交历史（oneline + 分支装饰）。可按文件过滤。只读、免审批。",
    git_log_impl,
    json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "description": "条数（默认 20，最大 100）" },
            "path": { "type": "string", "description": "仅查看涉及该文件的提交" }
        }
    })
);

#[cfg(test)]
mod tests {
    use super::*;

    /// git 不可用时跳过（提前返回）——测试环境无 git 不应误报失败。
    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    async fn init_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("owo-git-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} 失败：{}",
                crate::tools::decode_process_output(&output.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@owo.local"]);
        run(&["config", "user.name", "owo-test"]);
        std::fs::write(dir.join("a.txt"), "line-1\nline-2\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init: 基线"]);
        dir
    }

    #[tokio::test]
    async fn git_status_reports_clean_and_dirty() {
        if !git_available() {
            println!("跳过：环境无 git");
            return;
        }
        let workspace = init_repo("status").await;
        let clean = git_status_impl(&workspace, json!({})).await.unwrap();
        assert_eq!(clean["changes"], 0, "{clean}");
        let branch = clean["branch"].as_str().unwrap_or_default();
        assert!(
            branch == "master" || branch == "main",
            "初始分支应为 master/main：{branch}"
        );

        std::fs::write(workspace.join("a.txt"), "line-1\nline-2 改\n").unwrap();
        std::fs::write(workspace.join("b.txt"), "新文件\n").unwrap();
        let dirty = git_status_impl(&workspace, json!({})).await.unwrap();
        assert_eq!(dirty["changes"], 2, "{dirty}");
        let status = dirty["status"].as_str().unwrap();
        assert!(status.contains(" M a.txt"), "{status}");
        assert!(status.contains("?? b.txt"), "{status}");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn git_diff_shows_and_hints_empty() {
        if !git_available() {
            println!("跳过：环境无 git");
            return;
        }
        let workspace = init_repo("diff").await;
        let empty = git_diff_impl(&workspace, json!({})).await.unwrap();
        assert_eq!(empty["empty"], true);

        std::fs::write(workspace.join("a.txt"), "line-1\nline-2 改\n").unwrap();
        let diff = git_diff_impl(&workspace, json!({ "path": "a.txt" }))
            .await
            .unwrap();
        assert_eq!(diff["empty"], false);
        assert_eq!(diff["truncated"], false, "小改动不应报截断：{diff}");
        let body = diff["diff"].as_str().unwrap();
        assert!(body.contains("+line-2 改"), "{body}");
        assert!(body.contains("-line-2"), "{body}");

        // stat_only：只给 --stat 概览，不返回补丁正文。
        let stat = git_diff_impl(&workspace, json!({ "stat_only": true }))
            .await
            .unwrap();
        assert_eq!(stat["patch_omitted"], true, "{stat}");
        assert_eq!(stat["files"], 1, "应统计到 1 个改动文件：{stat}");
        let stat_text = stat["stat"].as_str().unwrap();
        assert!(stat_text.contains("a.txt"), "{stat_text}");
        assert!(
            !stat_text.contains("+line-2 改"),
            "stat 模式不应包含补丁正文：{stat_text}"
        );

        // staged=true：未暂存时提示。
        let staged = git_diff_impl(&workspace, json!({ "staged": true }))
            .await
            .unwrap();
        assert_eq!(staged["empty"], true);
        assert!(staged["hint"].as_str().unwrap().contains("暂存"));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn git_diff_reports_truncation_with_continuation_hint() {
        if !git_available() {
            println!("跳过：环境无 git");
            return;
        }
        let workspace = init_repo("diff-truncate").await;
        // 制造超过 DIFF_MAX_CHARS(20k) 的补丁：单文件 1200 行改写。
        let big: String = (0..1200)
            .map(|index| format!("line-{index}-changed-long-content\n"))
            .collect();
        std::fs::write(workspace.join("a.txt"), big).unwrap();
        let diff = git_diff_impl(&workspace, json!({ "path": "a.txt" }))
            .await
            .unwrap();
        assert_eq!(
            diff["truncated"], true,
            "超过 20k 字符的补丁必须机器可读地标记截断：{}",
            diff["total_chars"]
        );
        assert!(
            diff["total_chars"].as_u64().unwrap_or(0) > 20_000,
            "total_chars 应给出真实原始长度：{diff}"
        );
        let hint = diff["hint"].as_str().unwrap_or_default();
        assert!(
            hint.contains("path") || hint.contains("context_lines"),
            "{hint}"
        );
        // 续读路径：按文件过滤 + 更小上下文后可以拿到未截断的补丁头部。
        let scoped = git_diff_impl(&workspace, json!({ "path": "a.txt", "context_lines": 0 }))
            .await
            .unwrap();
        assert!(
            scoped["diff"].as_str().unwrap().contains("line-0"),
            "{scoped}"
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn git_log_lists_commits_and_filters_by_path() {
        if !git_available() {
            println!("跳过：环境无 git");
            return;
        }
        let workspace = init_repo("log").await;
        let log = git_log_impl(&workspace, json!({ "limit": 5 }))
            .await
            .unwrap();
        assert_eq!(log["commits"], 1);
        let text = log["log"].as_str().unwrap();
        assert!(text.contains("init: 基线"), "{text}");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn git_tools_report_non_repo_clearly() {
        if !git_available() {
            println!("跳过：环境无 git");
            return;
        }
        let dir = std::env::temp_dir().join(format!("owo-git-none-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let error = git_status_impl(&dir, json!({})).await.unwrap_err();
        assert!(error.contains("不是 git 仓库"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
