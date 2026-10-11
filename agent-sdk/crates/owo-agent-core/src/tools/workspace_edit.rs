//! Workspace file reading and editing tools: snapshots, conflict checks, and patch application.

use super::{resolve_session_path, snapshot_key, Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "读取工作区内的文本文件（支持 offset/limit 分页与行号，默认 400 行）"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "offset": { "type": "integer", "description": "起始行（1-based，默认 1）" },
                    "limit": { "type": "integer", "description": "读取行数（默认 400，上限 2000）" },
                    "number": { "type": "boolean", "description": "是否输出行号（默认 false）" }
                },
                "required": ["path"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = required_string(&args, "path")?;
        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(400)
            .clamp(1, 2000) as usize;
        let number = args.get("number").and_then(Value::as_bool).unwrap_or(false);
        let abs = resolve_session_path(ctx, &path)?;
        let raw_bytes = tokio::fs::read(&abs)
            .await
            .map_err(|e| format!("读取 {path} 失败：{e}"))?;
        let sha256 = crate::CasStore::hash_of(&raw_bytes);
        let raw = String::from_utf8(raw_bytes)
            .map_err(|e| format!("读取 {path} 失败：文件不是有效 UTF-8：{e}"))?;
        let total_lines = raw.lines().count();
        let start = offset.min(total_lines.saturating_add(1));
        let selected: Vec<&str> = raw.lines().skip(start - 1).take(limit).collect();
        let end_line = start + selected.len().saturating_sub(1);
        let truncated = end_line < total_lines;
        let content = if number {
            selected
                .iter()
                .enumerate()
                .map(|(index, line)| format!("{:>5}\t{line}", start + index))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            selected.join("\n")
        };
        Ok(json!({
            "path": path,
            "content": content,
            "start_line": start,
            "end_line": end_line,
            "total_lines": total_lines,
            "truncated": truncated,
            "bytes": content.len(),
            "sha256": format!("sha256:{sha256}"),
        }))
    }
}

pub(super) struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".into(),
            description: "写入工作区内的文件（自动快照，可 diff/revert）".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = required_string(&args, "path")?;
        let content = required_string(&args, "content")?;
        let abs = resolve_session_path(ctx, &path)?;
        write_file_body(ctx, &path, &abs, &content).await
    }
}

/// 写入执行体（[`WriteFileTool`] / [`WhitelistWriteFileTool`] 共享）：
/// 首写快照；后续写入先校验上次 Agent 写入哈希，避免覆盖 Agent 运行期间的外部修改。
pub(super) async fn write_file_body(
    ctx: &mut ToolContext<'_>,
    path: &str,
    abs: &Path,
    content: &str,
) -> Result<Value, String> {
    let key = snapshot_key(abs);
    if let Some(snapshot) = ctx.session.snapshots.get(&key) {
        let current = match tokio::fs::read(abs).await {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("写入前读取 {path} 失败：{error}")),
        };
        let matches_expected = if let Some(expected) = snapshot.expected_after_sha256.as_deref() {
            current
                .as_deref()
                .is_some_and(|bytes| crate::CasStore::hash_of(bytes) == expected)
        } else {
            let original = match snapshot.original_b64.as_deref() {
                Some(encoded) => Some(
                    BASE64
                        .decode(encoded)
                        .map_err(|error| format!("快照解码失败：{error}"))?,
                ),
                None => None,
            };
            current == original
        };
        if !matches_expected {
            return Err(format!(
                "写入冲突：{path} 在 Agent 上次记录的文件状态后再次变化，已拒绝覆盖"
            ));
        }
    } else {
        let original = match tokio::fs::read(abs).await {
            Ok(bytes) => Some(BASE64.encode(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("写入前快照 {path} 失败：{error}")),
        };
        ctx.session.snapshots.insert(
            key.clone(),
            crate::session::SnapshotEntry {
                original_b64: original,
                expected_after_sha256: None,
                turn: ctx.session.messages.len(),
            },
        );
    }
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建目录失败：{e}"))?;
    }
    tokio::fs::write(abs, content.as_bytes())
        .await
        .map_err(|e| format!("写入 {path} 失败：{e}"))?;
    if let Some(snapshot) = ctx.session.snapshots.get_mut(&key) {
        snapshot.expected_after_sha256 = Some(crate::CasStore::hash_of(content.as_bytes()));
    }
    Ok(json!({
        "path": path,
        "written": true,
        "bytes": content.len(),
    }))
}

/// 编辑片段匹配：优先精确；未命中时容忍 CRLF/LF 行尾差异（变体必须真实存在）。
/// 返回（实际用于替换的片段，命中次数）；0 = 未找到。
///
/// 真实模型实测：Windows 工作区文件是 CRLF，模型按 LF 提供片段 → 精确匹配失败、
/// 白白消耗一个修复回合。这里只在"换行符差异"上容忍，其余空白仍要求一致。
pub(super) fn match_edit_fragment(original: &str, old_string: &str) -> (String, usize) {
    let exact = original.matches(old_string).count();
    if exact > 0 {
        return (old_string.to_string(), exact);
    }
    for candidate in [
        old_string.replace("\r\n", "\n"),
        old_string.replace('\n', "\r\n"),
    ] {
        if candidate == old_string {
            continue;
        }
        let count = original.matches(candidate.as_str()).count();
        if count > 0 {
            return (candidate, count);
        }
    }
    (old_string.to_string(), 0)
}

/// 让 new_string 沿用匹配片段的行尾风格，避免编辑后文件混合 CRLF/LF。
pub(super) fn align_new_lines(matched_old: &str, new_string: &str) -> String {
    if matched_old.contains("\r\n") && !new_string.contains('\r') {
        new_string.replace('\n', "\r\n")
    } else if !matched_old.contains('\r') && new_string.contains("\r\n") {
        new_string.replace("\r\n", "\n")
    } else {
        new_string.to_string()
    }
}

/// `edit_file`：精确替换（`old_string` 必须唯一命中，除非 `replace_all`）。
/// 复用 [`write_file_body`] 的快照/冲突校验，可 diff/revert。
pub(super) struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit_file".into(),
            description: "精确替换文件片段（old_string → new_string；默认要求唯一命中，replace_all=true 替换全部）".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = required_string(&args, "path")?;
        let old_string = required_string(&args, "old_string")?;
        let new_string = required_string(&args, "new_string")?;
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if old_string.is_empty() {
            return Err("old_string 不能为空（新增内容请用 write_file / apply_patch）".to_string());
        }
        let abs = resolve_session_path(ctx, &path)?;
        let original = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|e| format!("读取 {path} 失败：{e}"))?;
        let (matched_old, occurrences) = match_edit_fragment(&original, &old_string);
        if occurrences == 0 {
            return Err(format!(
                "未找到 old_string（{path}）：请确认空白/缩进与文件一致；文件行尾可能是 CRLF，可先 read_file 复制原文"
            ));
        }
        if occurrences > 1 && !replace_all {
            return Err(format!(
                "old_string 命中 {occurrences} 处（不唯一）：请扩大上下文或设置 replace_all=true"
            ));
        }
        let matched_new = align_new_lines(&matched_old, &new_string);
        let updated = if replace_all {
            original.replace(matched_old.as_str(), matched_new.as_str())
        } else {
            original.replacen(matched_old.as_str(), matched_new.as_str(), 1)
        };
        write_file_body(ctx, &path, &abs, &updated).await?;
        Ok(json!({
            "path": path,
            "replaced": if replace_all { occurrences } else { 1 },
            "replace_all": replace_all,
        }))
    }
}

/// 对同一文件做多处精准替换（取优合并自远端 engine）：一次调用替代多次
/// `edit_file`，省回合。原子性：任一处失败则整批不落盘。
pub(super) struct MultiEditTool;

#[async_trait]
impl Tool for MultiEditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "multi_edit".into(),
            description: "对同一文件做多处精准替换（一次调用替代多次 edit_file，省回合）。edits 按顺序应用——后面替换的 old_string 要匹配前面替换后的内容。原子性：任一处失败则整批不生效。old_string 必须先 read_file 确认且唯一（或设 replace_all）。自动快照，可 diff/revert。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "文件路径" },
                    "edits": {
                        "type": "array",
                        "description": "替换列表（按顺序应用，上限 20 个）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string": { "type": "string", "description": "要替换的原文（必须唯一、含缩进）" },
                                "new_string": { "type": "string", "description": "替换后的内容" },
                                "replace_all": { "type": "boolean", "description": "替换该片段的全部出现位置（默认 false）" }
                            },
                            "required": ["old_string", "new_string"]
                        }
                    }
                },
                "required": ["path", "edits"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = required_string(&args, "path")?;
        let raw_edits = args
            .get("edits")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if raw_edits.is_empty() {
            return Err("edits 不能为空".to_string());
        }
        if raw_edits.len() > 20 {
            return Err(format!(
                "edits 过多（{} 个 > 20）。请拆分为多次 multi_edit 调用",
                raw_edits.len()
            ));
        }
        // 预解析全部替换项（先整体校验参数，再动文件）。
        let mut planned: Vec<(String, String, bool)> = Vec::with_capacity(raw_edits.len());
        for (index, edit) in raw_edits.iter().enumerate() {
            let old_string = edit
                .get("old_string")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edits[{index}] 缺少 old_string"))?
                .to_string();
            let new_string = edit
                .get("new_string")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edits[{index}] 缺少 new_string"))?
                .to_string();
            if old_string.is_empty() {
                return Err(format!("edits[{index}] 的 old_string 不能为空"));
            }
            let replace_all = edit
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            planned.push((old_string, new_string, replace_all));
        }

        let abs = resolve_session_path(ctx, &path)?;
        if !abs.is_file() {
            return Err(format!("{path} 不存在（新建文件请用 write_file）"));
        }
        let mut content = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|e| format!("读取 {path} 失败（仅支持 UTF-8 文本）：{e}"))?;

        // 内存中顺序应用；任一处失败立即返回，原文件不受影响。
        for (index, (old_string, new_string, replace_all)) in planned.iter().enumerate() {
            let (matched_old, occurrences) = match_edit_fragment(&content, old_string);
            if occurrences == 0 {
                return Err(format!(
                    "第 {}/{} 处替换失败：未找到 old_string（前面的替换可能已改变上下文）。整批未应用，请 read_file 后重试",
                    index + 1,
                    planned.len()
                ));
            }
            if occurrences > 1 && !replace_all {
                return Err(format!(
                    "第 {}/{} 处替换失败：old_string 出现 {occurrences} 次。请扩大上下文使其唯一或设 replace_all。整批未应用",
                    index + 1,
                    planned.len()
                ));
            }
            let matched_new = align_new_lines(&matched_old, new_string);
            content = if *replace_all {
                content.replace(matched_old.as_str(), matched_new.as_str())
            } else {
                content.replacen(matched_old.as_str(), matched_new.as_str(), 1)
            };
        }

        write_file_body(ctx, &path, &abs, &content).await?;
        Ok(json!({
            "path": path,
            "applied": planned.len(),
            "bytes_after": content.len(),
        }))
    }
}

/// 补丁段落：`old_lines`（上下文 + `-`）→ `new_lines`（上下文 + `+`）。
pub(super) struct PatchHunk {
    pub(super) old_lines: Vec<String>,
    pub(super) new_lines: Vec<String>,
}

/// 单个文件补丁操作。
pub(super) enum PatchOp {
    Add { path: String, content: String },
    Delete { path: String },
    Update { path: String, hunks: Vec<PatchHunk> },
}

pub(super) fn patch_op_path(op: &PatchOp) -> &str {
    match op {
        PatchOp::Add { path, .. } | PatchOp::Delete { path } | PatchOp::Update { path, .. } => path,
    }
}

/// 解析 Codex 风格补丁：`*** Add/Update/Delete File:` + `@@` 段落 + `+`/`-`/空格 行。
pub(super) fn parse_patch(patch: &str) -> Result<Vec<PatchOp>, String> {
    fn close(
        current: &mut Option<PatchOp>,
        hunk: &mut Option<(Vec<String>, Vec<String>)>,
        ops: &mut Vec<PatchOp>,
    ) -> Result<(), String> {
        if let Some((old, new)) = hunk.take() {
            if old == new {
                return Err("补丁段落没有实际变化（- / + 内容相同）".to_string());
            }
            match current.as_mut() {
                Some(PatchOp::Update { hunks, .. }) => {
                    hunks.push(PatchHunk {
                        old_lines: old,
                        new_lines: new,
                    });
                }
                _ => return Err("@@ 段落只能出现在 Update File 下".to_string()),
            }
        }
        if let Some(op) = current.take() {
            if let PatchOp::Update { hunks, .. } = &op {
                if hunks.is_empty() {
                    return Err("Update File 缺少 @@ 段落".to_string());
                }
            }
            ops.push(op);
        }
        Ok(())
    }

    let mut ops: Vec<PatchOp> = Vec::new();
    let mut current: Option<PatchOp> = None;
    let mut hunk: Option<(Vec<String>, Vec<String>)> = None;
    for line in patch.lines() {
        if line.starts_with("*** Begin Patch") || line.starts_with("*** End Patch") {
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            close(&mut current, &mut hunk, &mut ops)?;
            current = Some(PatchOp::Add {
                path: path.trim().to_string(),
                content: String::new(),
            });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            close(&mut current, &mut hunk, &mut ops)?;
            current = Some(PatchOp::Delete {
                path: path.trim().to_string(),
            });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            close(&mut current, &mut hunk, &mut ops)?;
            current = Some(PatchOp::Update {
                path: path.trim().to_string(),
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(op) = current.as_mut() else {
            if line.trim().is_empty() {
                continue;
            }
            return Err(format!("补丁格式错误：行不在任何文件操作下：{line}"));
        };
        match op {
            PatchOp::Add { content, .. } => {
                if let Some(rest) = line.strip_prefix('+') {
                    content.push_str(rest);
                    content.push('\n');
                } else if !line.trim().is_empty() {
                    return Err(format!("Add File 内容行必须以 + 开头：{line}"));
                }
            }
            PatchOp::Delete { .. } => {
                if !line.trim().is_empty() {
                    return Err(format!("Delete File 后不应再有内容行：{line}"));
                }
            }
            PatchOp::Update { hunks, .. } => {
                if line.starts_with("@@") {
                    if let Some((old, new)) = hunk.take() {
                        if old == new {
                            return Err("补丁段落没有实际变化（- / + 内容相同）".to_string());
                        }
                        hunks.push(PatchHunk {
                            old_lines: old,
                            new_lines: new,
                        });
                    }
                    hunk = Some((Vec::new(), Vec::new()));
                    continue;
                }
                let Some((old, new)) = hunk.as_mut() else {
                    if line.trim().is_empty() {
                        continue;
                    }
                    return Err(format!("Update File 内容必须位于 @@ 段落内：{line}"));
                };
                if let Some(rest) = line.strip_prefix('-') {
                    old.push(rest.to_string());
                } else if let Some(rest) = line.strip_prefix('+') {
                    new.push(rest.to_string());
                } else if let Some(rest) = line.strip_prefix(' ') {
                    old.push(rest.to_string());
                    new.push(rest.to_string());
                } else if !line.trim().is_empty() {
                    return Err(format!("补丁行必须以 + / - / 空格 开头：{line}"));
                }
            }
        }
    }
    close(&mut current, &mut hunk, &mut ops)?;
    if ops.is_empty() {
        return Err("补丁为空".to_string());
    }
    Ok(ops)
}

/// 逐段应用补丁：每段上下文必须**唯一命中**，否则报错（不改文件）。
pub(super) fn apply_hunks(original: &str, hunks: &[PatchHunk]) -> Result<String, String> {
    let mut lines: Vec<String> = original.lines().map(str::to_string).collect();
    let trailing_newline = original.ends_with('\n');
    for hunk in hunks {
        if hunk.old_lines.is_empty() {
            return Err("补丁段落缺少上下文（无法定位）".to_string());
        }
        let mut found: Option<usize> = None;
        let mut matches = 0usize;
        for index in 0..=lines.len().saturating_sub(hunk.old_lines.len()) {
            if lines[index..index + hunk.old_lines.len()] == hunk.old_lines[..] {
                matches += 1;
                if found.is_none() {
                    found = Some(index);
                }
            }
        }
        let Some(index) = found else {
            return Err(format!(
                "补丁上下文未命中（首个上下文行：{:?}）",
                hunk.old_lines.first()
            ));
        };
        if matches > 1 {
            return Err("补丁上下文命中多处（不唯一）：请扩大上下文".to_string());
        }
        lines.splice(
            index..index + hunk.old_lines.len(),
            hunk.new_lines.iter().cloned(),
        );
    }
    let mut result = lines.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    Ok(result)
}

/// `apply_patch`：先校验整组补丁，再逐文件写入；写入走快照可 diff/revert。
pub(super) struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "apply_patch".into(),
            description:
                "应用多文件补丁；可传 expected_hashes 做并发修改保护，成功后返回每个文件的哈希收据"
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "patch": { "type": "string" },
                    "expected_hashes": {
                        "type": "object",
                        "description": "可选并发保护；按补丁中的相对路径提供全部文件的基线，Add 用 absent，Update/Delete 用 sha256:<hex>",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["patch"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let patch = required_string(&args, "patch")?;
        let ops = parse_patch(&patch)?;
        let expected_hashes = match args.get("expected_hashes") {
            None => None,
            Some(Value::Object(values)) => Some(values),
            Some(_) => return Err("expected_hashes 必须是对象".to_string()),
        };
        let mut seen = std::collections::HashSet::new();
        let mut prepared: Vec<(String, PathBuf, String, Option<Vec<u8>>, String)> =
            Vec::with_capacity(ops.len());
        for op in &ops {
            let path = patch_op_path(op).to_string();
            let abs = resolve_session_path(ctx, &path)?;
            let key = snapshot_key(&abs);
            if !seen.insert(key) {
                return Err(format!("补丁重复操作同一文件：{path}"));
            }
            let (content, base, kind) = match op {
                PatchOp::Add { content, .. } => {
                    let base = match tokio::fs::read(&abs).await {
                        Ok(_) => return Err(format!("Add File 目标已存在：{path}")),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                        Err(error) => return Err(format!("Add File 检查 {path} 失败：{error}")),
                    };
                    (content.clone(), base, "add")
                }
                PatchOp::Delete { .. } => {
                    let base = tokio::fs::read(&abs)
                        .await
                        .map_err(|e| format!("Delete File 读取 {path} 失败：{e}"))?;
                    let content = String::from_utf8(base.clone())
                        .map_err(|e| format!("Delete File {path} 不是有效 UTF-8：{e}"))?;
                    (content, Some(base), "delete")
                }
                PatchOp::Update { hunks, .. } => {
                    let base = tokio::fs::read(&abs)
                        .await
                        .map_err(|e| format!("Update File 读取 {path} 失败：{e}"))?;
                    let original = String::from_utf8(base.clone())
                        .map_err(|e| format!("Update File {path} 不是有效 UTF-8：{e}"))?;
                    (apply_hunks(&original, hunks)?, Some(base), "update")
                }
            };
            if let Some(expected) = expected_hashes {
                let expected = expected
                    .get(&path)
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("expected_hashes 缺少文件基线：{path}"))?;
                let actual = base
                    .as_deref()
                    .map(|bytes| format!("sha256:{}", crate::CasStore::hash_of(bytes)))
                    .unwrap_or_else(|| "absent".to_string());
                if expected != actual {
                    return Err(format!(
                        "补丁基线冲突：{path} 当前为 {actual}，期望 {expected}"
                    ));
                }
            }
            prepared.push((path, abs, content, base, kind.to_string()));
        }
        if let Some(expected) = expected_hashes {
            if expected.len() != ops.len()
                || expected
                    .keys()
                    .any(|path| !ops.iter().any(|op| patch_op_path(op) == path))
            {
                return Err("expected_hashes 必须且只能包含补丁涉及的文件路径".to_string());
            }
        }

        let mut applied = Vec::with_capacity(ops.len());
        for (path, abs, content, base, kind) in prepared {
            let current = match tokio::fs::read(&abs).await {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("写入前重新读取 {path} 失败：{error}")),
            };
            if current != base {
                let actual = current
                    .as_deref()
                    .map(|bytes| format!("sha256:{}", crate::CasStore::hash_of(bytes)))
                    .unwrap_or_else(|| "absent".to_string());
                return Err(format!(
                    "补丁基线冲突：{path} 在准备期间已变化，当前为 {actual}"
                ));
            }
            let base_hash = base
                .as_deref()
                .map(|bytes| format!("sha256:{}", crate::CasStore::hash_of(bytes)));
            let result_hash = if kind == "delete" {
                let key = snapshot_key(&abs);
                if let Some(snapshot) = ctx.session.snapshots.get(&key) {
                    let matches_snapshot = if let Some(expected_after) =
                        snapshot.expected_after_sha256.as_deref()
                    {
                        base.as_deref()
                            .is_some_and(|bytes| crate::CasStore::hash_of(bytes) == expected_after)
                    } else {
                        let original = snapshot
                            .original_b64
                            .as_deref()
                            .map(|encoded| BASE64.decode(encoded))
                            .transpose()
                            .map_err(|error| format!("快照解码失败：{error}"))?;
                        original == base
                    };
                    if !matches_snapshot {
                        return Err(format!(
                            "删除冲突：{path} 已在 Agent 记录的文件状态后再次变化"
                        ));
                    }
                } else {
                    ctx.session.snapshots.insert(
                        key,
                        crate::session::SnapshotEntry {
                            original_b64: base.as_deref().map(|bytes| BASE64.encode(bytes)),
                            expected_after_sha256: None,
                            turn: ctx.session.messages.len(),
                        },
                    );
                }
                tokio::fs::remove_file(&abs)
                    .await
                    .map_err(|e| format!("删除 {path} 失败：{e}"))?;
                Value::Null
            } else {
                let key = snapshot_key(&abs);
                ctx.session
                    .snapshots
                    .entry(key)
                    .or_insert_with(|| crate::session::SnapshotEntry {
                        original_b64: base.as_deref().map(|bytes| BASE64.encode(bytes)),
                        expected_after_sha256: None,
                        turn: ctx.session.messages.len(),
                    });
                write_file_body(ctx, &path, &abs, &content).await?;
                json!(format!(
                    "sha256:{}",
                    crate::CasStore::hash_of(content.as_bytes())
                ))
            };
            applied.push(json!({
                "path": path,
                "op": kind,
                "base_sha256": base_hash,
                "result_sha256": result_hash,
            }));
        }
        Ok(json!({ "ok": true, "files": applied }))
    }
}

/// 白名单受限写入工具（七期 · 二路）：与 [`WriteFileTool`] 同语义（快照可
/// diff/revert），但写目标必须落在 `allowed` 绝对路径前缀内——工具面层强制，
/// 叠加在审批策略之上（权限三道闸：注册表面 → 白名单前缀 → 审批）。
pub(super) struct WhitelistWriteFileTool {
    /// 允许写入的绝对路径前缀（canonicalize 口径；空 = 仅限工作区根内）。
    pub(super) allowed: Vec<PathBuf>,
}

/// 白名单前缀判定（纯路径版，便于测试）：candidate 是否落在任一 allowed 前缀内。
/// 空白名单在此返回 false——「空 = 未约束」的放行语义由调用方
/// （[`WhitelistWriteFileTool::resolve_whitelisted`]）短路处理。
pub(super) fn path_in_whitelist(candidate: &Path, allowed: &[PathBuf]) -> bool {
    allowed.iter().any(|base| candidate.starts_with(base))
}

/// 去掉 Windows verbatim 前缀（`\\?\C:\...` → `C:\...`）：canonicalize 语义不变。
/// 与服务端绑定侧（root/allowed 存储前去前缀）的路径口径对齐——否则
/// `starts_with` 前缀比对在 verbatim × 非 verbatim 混用时恒为 false，
/// 白名单内写入会被误拒。
pub(super) fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped.to_string()),
        None => path.to_path_buf(),
    }
}

impl WhitelistWriteFileTool {
    /// 解析 + 白名单校验：与 [`resolve_session_path`] 同口径解析目标路径
    /// （canonicalize；不存在的目标按「父目录 canonicalize + 文件名」解析），
    /// 比对前去 verbatim 前缀（两侧同口径），不在白名单内 → Err。
    fn resolve_whitelisted(&self, ctx: &ToolContext<'_>, path: &str) -> Result<PathBuf, String> {
        let abs = resolve_session_path(ctx, path)?;
        if self.allowed.is_empty() {
            return Ok(abs);
        }
        let candidate = abs.canonicalize().unwrap_or_else(|_| {
            abs.parent()
                .and_then(|parent| parent.canonicalize().ok())
                .map(|parent| parent.join(abs.file_name().unwrap_or_default()))
                .unwrap_or_else(|| abs.clone())
        });
        let candidate = strip_verbatim_prefix(&candidate);
        let allowed: Vec<PathBuf> = self
            .allowed
            .iter()
            .map(|base| strip_verbatim_prefix(base))
            .collect();
        if path_in_whitelist(&candidate, &allowed) {
            return Ok(abs);
        }
        let list = self
            .allowed
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!("写入目标不在写白名单内：{path}（白名单：{list}）"))
    }
}

#[async_trait]
impl Tool for WhitelistWriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".into(),
            description: "写入工作区内的文件（仅限白名单路径；自动快照，可 diff/revert）".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let path = required_string(&args, "path")?;
        let content = required_string(&args, "content")?;
        let abs = self.resolve_whitelisted(ctx, &path)?;
        write_file_body(ctx, &path, &abs, &content).await
    }
}

/// 白名单受限补丁工具：在交给补丁执行器前校验每个文件目标。
pub(super) struct WhitelistApplyPatchTool {
    pub(super) allowed: Vec<PathBuf>,
}

#[async_trait]
impl Tool for WhitelistApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = ApplyPatchTool.spec();
        spec.description =
            "应用多文件精确补丁（仅限白名单路径；支持 expected_hashes 与哈希收据）".into();
        spec
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let patch = required_string(&args, "patch")?;
        let guard = WhitelistWriteFileTool {
            allowed: self.allowed.clone(),
        };
        for op in parse_patch(&patch)? {
            guard.resolve_whitelisted(ctx, patch_op_path(&op))?;
        }
        ApplyPatchTool.run(ctx, args).await
    }
}
