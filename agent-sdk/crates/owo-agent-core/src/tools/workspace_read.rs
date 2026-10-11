//! Read-only workspace discovery tools.

use super::{
    decode_process_output, resolve_session_path, tool_sandbox_policy, Tool, ToolContext, ToolSpec,
};
use crate::external_tools;
use async_trait::async_trait;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};

pub(super) struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_dir".into(),
            description: "列出工作区内目录条目".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| ".".to_string());
        let abs = resolve_session_path(ctx, &path)?;
        let mut entries = Vec::new();
        let mut reader = tokio::fs::read_dir(&abs)
            .await
            .map_err(|e| format!("读取目录 {path} 失败：{e}"))?;
        while let Some(entry) = reader.next_entry().await.map_err(|e| e.to_string())? {
            entries.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "is_dir": entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false),
            }));
        }
        Ok(json!({ "path": path, "entries": entries }))
    }
}

/// 无 ripgrep / 沙箱不可用时的纯 Rust 兜底：按文件名关键字（大小写不敏感）递归搜索。
/// 只读、不跟随符号链接、跳过常见噪声目录；用条目预算防止超大仓库拖死回合。
pub(super) fn fallback_search_files(workspace: &std::path::Path, pattern: &str) -> Vec<String> {
    const MAX_ENTRIES: usize = 20_000;
    const MAX_DEPTH: usize = 12;
    const MAX_RESULTS: usize = 200;
    let needle = pattern.to_lowercase();
    let mut results = Vec::new();
    let mut visited = 0usize;
    let mut stack = vec![(workspace.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH || results.len() >= MAX_RESULTS || visited >= MAX_ENTRIES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited >= MAX_ENTRIES || results.len() >= MAX_RESULTS {
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if matches!(
                    name.as_str(),
                    ".git" | "target" | "node_modules" | ".svn" | ".hg"
                ) {
                    continue;
                }
                stack.push((entry.path(), depth + 1));
            } else if file_type.is_file() && name.to_lowercase().contains(&needle) {
                let path = entry.path();
                let relative = path.strip_prefix(workspace).unwrap_or(&path);
                results.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    results.sort();
    results
}

fn fallback_search_result(workspace: &std::path::Path, pattern: &str, reason: &str) -> Value {
    json!({
        "pattern": pattern,
        "matches": fallback_search_files(workspace, pattern),
        "tool": "walk",
        "note": format!("{reason}；已使用内置文件名搜索兜底（结果集上限 200，跳过 .git/target/node_modules）"),
    })
}

pub(super) struct SearchFilesTool;

#[async_trait]
impl Tool for SearchFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "search_files".into(),
            description: "使用随包 ripgrep 按**文件名关键字**递归搜索工作区文件（只读）。\
                          关键字是字面量匹配，不是正则——传 `\\.(txt|md)$` 之类的正则会搜不到东西"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "文件名关键字（字面量，非正则），例如 notes、.md、config"
                    }
                },
                "required": ["pattern"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let pattern = required_string(&args, "pattern")?;
        if pattern.trim().is_empty() {
            return Err("搜索模式不能为空".to_string());
        }
        let Some(rg) = external_tools::resolve_ripgrep() else {
            return Ok(fallback_search_result(
                ctx.workspace,
                &pattern,
                "随包 ripgrep 不可用",
            ));
        };

        let mut policy = tool_sandbox_policy(ctx, "search_files");
        policy.cpu_ms = Some(30_000);
        policy.mem_mb = Some(512);
        let mut sandbox_command =
            crate::sandbox::SandboxCommand::new(rg.to_string_lossy().into_owned(), policy)
                .with_args(vec![
                    "--files".to_string(),
                    "--hidden".to_string(),
                    "--glob".to_string(),
                    "!.git/**".to_string(),
                    "--glob".to_string(),
                    "!target/**".to_string(),
                    "--glob".to_string(),
                    "!node_modules/**".to_string(),
                    "--iglob".to_string(),
                    format!("*{}*", pattern),
                ])
                .with_cwd(ctx.workspace.to_path_buf());
        if let Some(path) = external_tools::path_with_bundled_tools() {
            sandbox_command.env.push(("PATH".to_string(), path));
        }

        let manager = crate::sandbox::default_manager();
        let process = {
            let mut manager = manager
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match manager.spawn(&sandbox_command) {
                Ok(process) => process,
                Err(error) => {
                    return Ok(fallback_search_result(
                        ctx.workspace,
                        &pattern,
                        &format!("搜索沙箱拒绝执行：{error}"),
                    ))
                }
            }
        };
        let output = match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::task::spawn_blocking(move || {
                let mut process = process;
                process.wait_output()
            }),
        )
        .await
        {
            Ok(Ok(Ok(output))) => output,
            Ok(Ok(Err(error))) => {
                return Ok(fallback_search_result(
                    ctx.workspace,
                    &pattern,
                    &format!("ripgrep 执行失败：{error}"),
                ))
            }
            Ok(Err(join_error)) => {
                return Ok(fallback_search_result(
                    ctx.workspace,
                    &pattern,
                    &format!("搜索等待失败：{join_error}"),
                ))
            }
            Err(_) => {
                return Ok(fallback_search_result(
                    ctx.workspace,
                    &pattern,
                    "ripgrep 搜索超时（30s）",
                ))
            }
        };

        if output.exit_code != 0 && output.exit_code != 1 {
            return Ok(fallback_search_result(
                ctx.workspace,
                &pattern,
                &format!(
                    "ripgrep 搜索失败（exit_code={}）：{}",
                    output.exit_code,
                    decode_process_output(&output.stderr).trim()
                ),
            ));
        }
        let matches = decode_process_output(&output.stdout)
            .lines()
            .filter(|line| !line.is_empty())
            .take(200)
            .map(|line| line.replace('\\', "/"))
            .collect::<Vec<_>>();
        Ok(json!({
            "pattern": pattern,
            "matches": matches,
            "tool": "ripgrep",
            "tool_version": external_tools::RIPGREP_VERSION,
        }))
    }
}

/// `grep`：用随包 ripgrep 做**内容**检索（只读，正则）。
pub(super) struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".into(),
            description: "使用随包 ripgrep 做内容检索（正则；只读，返回 path/line/text）".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "正则表达式或字面量" },
                    "path": { "type": "string", "description": "可选：检索子路径（默认工作区根）" },
                    "glob": { "type": "string", "description": "可选：文件名过滤，如 *.rs" },
                    "max_results": { "type": "integer", "description": "最多返回条数（默认 100，上限 500）" }
                },
                "required": ["pattern"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let pattern = required_string(&args, "pattern")?;
        if pattern.trim().is_empty() {
            return Err("检索模式不能为空".to_string());
        }
        let rg = external_tools::resolve_ripgrep().ok_or_else(|| {
            "随包 ripgrep 不可用：请重新安装 OwO Agent，或仅在测试时设置 OWO_EXTERNAL_TOOLS_DIR"
                .to_string()
        })?;
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 500) as usize;
        let search_path = match args.get("path").and_then(Value::as_str) {
            Some(path) if !path.trim().is_empty() => resolve_session_path(ctx, path)?,
            _ => ctx.workspace.to_path_buf(),
        };

        let mut rg_args = vec![
            "--json".to_string(),
            "--glob".to_string(),
            "!.git/**".to_string(),
            "--glob".to_string(),
            "!target/**".to_string(),
            "--glob".to_string(),
            "!node_modules/**".to_string(),
        ];
        if let Some(glob) = args.get("glob").and_then(Value::as_str) {
            if !glob.trim().is_empty() {
                rg_args.push("--glob".to_string());
                rg_args.push(glob.to_string());
            }
        }
        rg_args.push("-e".to_string());
        rg_args.push(pattern.clone());
        rg_args.push(search_path.to_string_lossy().into_owned());

        let mut policy = tool_sandbox_policy(ctx, "grep");
        policy.cpu_ms = Some(30_000);
        policy.mem_mb = Some(512);
        let mut sandbox_command =
            crate::sandbox::SandboxCommand::new(rg.to_string_lossy().into_owned(), policy)
                .with_args(rg_args)
                .with_cwd(ctx.workspace.to_path_buf());
        if let Some(path) = external_tools::path_with_bundled_tools() {
            sandbox_command.env.push(("PATH".to_string(), path));
        }

        let manager = crate::sandbox::default_manager();
        let process = {
            let mut manager = manager
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            manager
                .spawn(&sandbox_command)
                .map_err(|error| format!("检索沙箱拒绝执行：{error}"))?
        };
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::task::spawn_blocking(move || {
                let mut process = process;
                process.wait_output()
            }),
        )
        .await
        .map_err(|_| "ripgrep 检索超时（30s，进程仍在受限 Job 内）".to_string())?
        .map_err(|join_error| format!("检索等待失败：{join_error}"))?
        .map_err(|error| format!("ripgrep 执行失败：{error}"))?;

        if output.exit_code != 0 && output.exit_code != 1 {
            return Err(format!(
                "ripgrep 检索失败（exit_code={}）：{}",
                output.exit_code,
                decode_process_output(&output.stderr).trim()
            ));
        }
        // `--json` 输出逐行解析，避免 Windows 盘符冒号破坏 `path:line:text` 切分。
        let mut matches: Vec<Value> = decode_process_output(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|value| value.get("type").and_then(Value::as_str) == Some("match"))
            .filter_map(|value| {
                let data = value.get("data")?;
                let path = data.get("path")?.get("text")?.as_str()?.replace('\\', "/");
                let line_no = data.get("line_number")?.as_u64()?;
                let raw = data.get("lines")?.get("text")?.as_str()?.trim_end();
                let text: String = raw.chars().take(300).collect();
                Some(json!({ "path": path, "line": line_no, "text": text }))
            })
            .take(max_results + 1)
            .collect();
        let truncated = matches.len() > max_results;
        matches.truncate(max_results);
        Ok(json!({
            "pattern": pattern,
            "matches": matches,
            "truncated": truncated,
            "tool": "ripgrep",
            "tool_version": external_tools::RIPGREP_VERSION,
        }))
    }
}
