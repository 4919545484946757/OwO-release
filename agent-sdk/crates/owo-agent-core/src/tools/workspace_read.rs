//! Read-only workspace discovery tools.

use super::{
    decode_process_output, resolve_session_path, tool_sandbox_policy, Tool, ToolContext, ToolSpec,
};
use crate::external_tools;
use async_trait::async_trait;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};

pub(super) struct ListDirTool;

/// 噪声目录：递归搜索跳过，列目录时只做标记（模型可据此避免往下钻）。
pub(super) fn is_noise_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".svn" | ".hg" | ".venv"
    )
}

#[async_trait]
impl Tool for ListDirTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_dir".into(),
            description: "列出工作区内目录条目（目录在前、按名字排序；默认最多 200 条）".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "max_entries": {
                        "type": "integer",
                        "description": "返回条目上限（默认 200，上限 2000）"
                    }
                }
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
        let max_entries = args
            .get("max_entries")
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .clamp(1, 2000) as usize;
        let abs = resolve_session_path(ctx, &path)?;
        let mut entries = Vec::new();
        let mut truncated = false;
        let mut reader = tokio::fs::read_dir(&abs)
            .await
            .map_err(|e| format!("读取目录 {path} 失败：{e}"))?;
        while let Some(entry) = reader.next_entry().await.map_err(|e| e.to_string())? {
            if entries.len() >= max_entries {
                // 目录可能极大（构建产物/依赖树）：明确报告截断，别让模型以为这就是全部。
                truncated = true;
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            let mut item = json!({ "name": name, "is_dir": is_dir });
            if is_dir && is_noise_dir_name(&name) {
                item["noise"] = json!(true);
            }
            entries.push(item);
        }
        // 文件系统返回顺序不稳定（同一目录多次调用顺序可能不同）：固定"目录在前、名字升序"，
        // 让模型跨回合看到的清单保持一致，便于 diff 自己的观察结果。
        entries.sort_by(|left, right| {
            let left_dir = left["is_dir"].as_bool().unwrap_or(false);
            let right_dir = right["is_dir"].as_bool().unwrap_or(false);
            right_dir.cmp(&left_dir).then_with(|| {
                left["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase()
                    .cmp(&right["name"].as_str().unwrap_or_default().to_lowercase())
            })
        });
        let count = entries.len();
        Ok(json!({
            "path": path,
            "entries": entries,
            "count": count,
            "truncated": truncated,
        }))
    }
}

/// 无 ripgrep / 沙箱不可用时的纯 Rust 兜底：按文件名关键字（大小写不敏感）递归搜索。
/// 只读、不跟随符号链接、跳过常见噪声目录；用条目预算防止超大仓库拖死回合。
pub(super) fn fallback_search_files(
    workspace: &std::path::Path,
    pattern: &str,
    limit: usize,
) -> Vec<String> {
    const MAX_ENTRIES: usize = 20_000;
    const MAX_DEPTH: usize = 12;
    let needle = pattern.to_lowercase();
    let mut results = Vec::new();
    let mut visited = 0usize;
    let mut stack = vec![(workspace.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH || results.len() >= limit || visited >= MAX_ENTRIES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited >= MAX_ENTRIES || results.len() >= limit {
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if is_noise_dir_name(&name) {
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

fn fallback_search_result(
    workspace: &std::path::Path,
    pattern: &str,
    reason: &str,
    limit: usize,
) -> Value {
    // 多取一条用于判断"是不是被上限截断"，与 ripgrep 路径同一口径。
    let mut matches = fallback_search_files(workspace, pattern, limit + 1);
    let truncated = matches.len() > limit;
    matches.truncate(limit);
    json!({
        "pattern": pattern,
        "matches": matches,
        "truncated": truncated,
        "tool": "walk",
        "note": format!("{reason}；已使用内置文件名搜索兜底（跳过 .git/target/node_modules，上限 {limit} 条）"),
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
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "最多返回条数（默认 100，上限 500）"
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
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 500) as usize;
        let Some(rg) = external_tools::resolve_ripgrep() else {
            return Ok(fallback_search_result(
                ctx.workspace,
                &pattern,
                "随包 ripgrep 不可用",
                max_results,
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
                        max_results,
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
                    max_results,
                ))
            }
            Ok(Err(join_error)) => {
                return Ok(fallback_search_result(
                    ctx.workspace,
                    &pattern,
                    &format!("搜索等待失败：{join_error}"),
                    max_results,
                ))
            }
            Err(_) => {
                return Ok(fallback_search_result(
                    ctx.workspace,
                    &pattern,
                    "ripgrep 搜索超时（30s）",
                    max_results,
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
                max_results,
            ));
        }
        let mut matches = decode_process_output(&output.stdout)
            .lines()
            .filter(|line| !line.is_empty())
            .take(max_results + 1)
            .map(|line| line.replace('\\', "/"))
            .collect::<Vec<_>>();
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

/// `read_artifact`：读取超大工具结果落盘的 CAS artifact（按 ref 分页）。
///
/// 背景：回合里超过 `MAX_TOOL_RESULT_CHARS` 的工具结果会被截断、完整内容写进
/// `<workspace>/.owo/artifacts`，模型拿到的是 `{artifact: {ref, preview, ...}}`。
/// 但此前**没有任何工具能按 ref 取回**，指针是死路——模型只能靠 50k 预览猜。
pub(super) struct ReadArtifactTool;

impl ReadArtifactTool {
    /// artifact 目录与宿主一致（见 CLI `build_agent` 的 `with_artifact_store`）。
    fn artifact_dir(workspace: &std::path::Path) -> std::path::PathBuf {
        workspace.join(".owo").join("artifacts")
    }

    /// 归一化 ref：接受 `sha256:<hex>` 或裸 hex；必须 64 位十六进制。
    ///
    /// `CasStore` 直接用 ref 拼文件名，放任 `../` 之类会越出 artifact 目录，所以这里
    /// 先做白名单校验，不接受任何非十六进制字符。
    fn normalize_ref(raw: &str) -> Result<String, String> {
        let trimmed = raw.trim();
        let hex = trimmed.strip_prefix("sha256:").unwrap_or(trimmed);
        if hex.len() != 64 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
            return Err(
                "ref 必须是 64 位十六进制内容哈希（可带 sha256: 前缀）；请直接用工具结果里 artifact.ref 的值"
                    .to_string(),
            );
        }
        Ok(hex.to_ascii_lowercase())
    }
}

#[async_trait]
impl Tool for ReadArtifactTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_artifact".into(),
            description: "按 ref 读取超大工具结果落盘的 artifact（工具结果里 artifact.ref）；\
                          按字符分页，默认 8000 字符，返回 next_offset 供继续读取"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "内容哈希（artifact.ref，sha256: 前缀可选）" },
                    "offset": { "type": "integer", "description": "起始字符偏移（默认 0）" },
                    "max_chars": { "type": "integer", "description": "本次返回字符上限（默认 8000，上限 40000）" }
                },
                "required": ["ref"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let reference = required_string(&args, "ref")?;
        let hash = Self::normalize_ref(&reference)?;
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(8_000)
            .clamp(1, 40_000) as usize;
        let dir = Self::artifact_dir(ctx.workspace);
        if !dir.is_dir() {
            return Err(format!(
                "artifact 存储尚未创建（{}）：本工作区还没有超大工具结果落盘",
                dir.display()
            ));
        }
        let store =
            crate::CasStore::new(dir).map_err(|error| format!("artifact 存储不可用：{error}"))?;
        let bytes = store
            .get(&hash)
            .ok_or_else(|| format!("artifact 不存在或已被回收：sha256:{hash}"))?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let total_chars = text.chars().count();
        let page: String = text.chars().skip(offset).take(max_chars).collect();
        let next_offset = offset + page.chars().count();
        let truncated = next_offset < total_chars;
        Ok(json!({
            "ref": format!("sha256:{hash}"),
            "offset": offset,
            "returned_chars": page.chars().count(),
            "total_chars": total_chars,
            "truncated": truncated,
            "next_offset": if truncated { json!(next_offset) } else { Value::Null },
            "text": page,
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
