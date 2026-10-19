//! 工具注册辅助：schema 预算/压缩、路径解析、进程输出解码与沙箱策略（从 tools.rs 拆出）。

use super::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) fn snapshot_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// MCP 工具 schema 延迟加载（M2）：单工具 schema 序列化字节数预算，默认 2048 字节。
pub fn schema_budget_bytes() -> usize {
    std::env::var("OWO_MCP_SCHEMA_BUDGET_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2048)
}

/// 估算 JSON schema 的序列化体积（字节）。
pub fn schema_bytes(schema: &Value) -> usize {
    serde_json::to_string(schema)
        .map(|text| text.len())
        .unwrap_or(usize::MAX)
}

/// 安全压缩 JSON Schema：递归删除说明性元数据，完整保留类型、必填项、
/// enum、数组/对象嵌套、additionalProperties 和其他校验关键字。
/// 若校验语义本身超过预算，允许结果继续超预算，不能把错误契约发给模型。
pub fn compact_schema(schema: &Value) -> Value {
    fn strip_annotations(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut compact = serde_json::Map::new();
                for (key, child) in map {
                    if matches!(
                        key.as_str(),
                        "description" | "examples" | "default" | "title" | "$schema"
                    ) {
                        continue;
                    }
                    compact.insert(key.clone(), strip_annotations(child));
                }
                Value::Object(compact)
            }
            Value::Array(items) => Value::Array(items.iter().map(strip_annotations).collect()),
            other => other.clone(),
        }
    }

    strip_annotations(schema)
}

/// 解析候选路径：存在的路径直接 canonicalize（跟随重解析点）；不存在的路径对
/// **最近已存在祖先** canonicalize，再把剩余段逐段接上，`..` 在剩余段中就地消解。
///
/// 安全动机（审查 N6）：旧实现 `candidate.canonicalize().unwrap_or(candidate)`
/// 对不存在的路径保留未规范化文本，`C:\ws\..\evil.txt` 会被 `starts_with(C:\ws)`
/// 判成"工作区内"（组件比较只看前缀），而 Win32 打开文件时把 `..` 消解到工作区外，
/// 于是 `apply_patch`/`write_file` 能在工作区外落盘。必须先消解 `..` 再比较。
pub(crate) fn canonical_candidate(path: &Path) -> PathBuf {
    let components: Vec<std::path::Component> = path.components().collect();
    for split in (1..=components.len()).rev() {
        let mut base = PathBuf::new();
        for component in &components[..split] {
            base.push(component.as_os_str());
        }
        if !base.exists() {
            continue;
        }
        if let Ok(canonical) = base.canonicalize() {
            let mut result = strip_verbatim_prefix(&canonical);
            for component in &components[split..] {
                match component {
                    std::path::Component::ParentDir => {
                        result.pop();
                    }
                    std::path::Component::CurDir => {}
                    other => result.push(other.as_os_str()),
                }
            }
            return result;
        }
    }
    // 连一个存在前缀都没有（如工作区尚不存在）：纯词法消解 `..` 兜底。
    lexical_normalize(path)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// 以会话工作区为基座解析相对路径，并做策略工作区越界检查。
pub(crate) fn resolve_session_path(ctx: &ToolContext, path: &str) -> Result<PathBuf, String> {
    let base = ctx
        .workspace
        .canonicalize()
        .unwrap_or_else(|_| ctx.workspace.to_path_buf());
    // 绝对路径直接采用（`Path::join` 对绝对路径会整体替换，且 canonicalize 可能不可用）。
    let raw = Path::new(path);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base.join(path)
    };
    let candidate = canonical_candidate(&candidate);
    // 越界边界取**本会话的工作区**，不是 `Policy` 的工作区。
    //
    // 两者的来源不同：`ctx.workspace` 来自 `session.workspace`（每个会话都可以在 UI
    // 里选不同目录），而 `Policy` 的工作区来自服务启动参数——**全局唯一**。只要两者
    // 不相等，选过其它目录的会话就会**所有文件类工具一律"路径越界"**：`list_dir` /
    // `read_file` / `search_files` 全废，而 `run_command` 照常能用（命令工具不走这条
    // 判定），表象极像"Agent 整体坏了"。
    //
    // 实测成因：桌面壳在 `workspace.json` 未配置时 `workspacePath()` 回落
    // `process.cwd()`（= electron 安装目录），核心便以那个目录为工作区启动；用户在
    // 会话里选 `D:\OwO-master`，于是每一次读文件都被判越界。
    //
    // 安全口径不变：read_only 档位、deny 命令、审批链都由 `Policy` 独立把关，这里只
    // 决定"文件类工具的活动范围 = 本会话工作区"。
    let boundary = if ctx.workspace.as_os_str().is_empty() {
        ctx.policy.workspace()
    } else {
        ctx.workspace
    };
    let policy_workspace = boundary
        .canonicalize()
        .unwrap_or_else(|_| boundary.to_path_buf());
    // 两侧统一去 Windows verbatim 前缀再比对（`\\?\C:\x` vs `C:\x` 否则恒不匹配）。
    let candidate_cmp = strip_verbatim_prefix(&candidate);
    let workspace_cmp = strip_verbatim_prefix(&policy_workspace);
    let unrestricted = ctx.policy.profile() == crate::permissions::PermissionProfile::Unrestricted;
    if !candidate_cmp.starts_with(&workspace_cmp) && !unrestricted {
        return Err(format!("路径越界：{path}"));
    }
    Ok(candidate)
}
/// Windows 控制台程序按系统代码页输出（简中为 OEM 936/GBK）；一律 `from_utf8_lossy`
/// 会让中文命令输出乱码。顺序：UTF-8 严格 → 系统代码页 → lossy 兜底（UTF-8 优先保证
/// git/rg/python 等现代工具的中文输出不被二次误解）。
pub(crate) fn decode_process_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    #[cfg(windows)]
    if let Some(text) = decode_with_system_codepage(bytes) {
        return text;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// 按 Windows 系统 OEM 代码页（`GetOEMCP`）把字节转成 UTF-8。
///
/// 用 OEM 而非 ANSI 代码页：控制台程序（cmd / 批处理 / 多数 CLI）输出走 OEM 代码页，
/// 简中环境下是 936（GBK）。
#[cfg(windows)]
fn decode_with_system_codepage(bytes: &[u8]) -> Option<String> {
    use windows::Win32::Globalization::{
        GetOEMCP, MultiByteToWideChar, MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
    };
    if bytes.is_empty() {
        return Some(String::new());
    }
    unsafe {
        let codepage = GetOEMCP();
        let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
        let len = MultiByteToWideChar(codepage, flags, bytes, None);
        if len <= 0 {
            return None;
        }
        let mut wide = vec![0u16; len as usize];
        let written = MultiByteToWideChar(codepage, flags, bytes, Some(&mut wide));
        if written <= 0 {
            return None;
        }
        wide.truncate(written as usize);
        Some(String::from_utf16_lossy(&wide))
    }
}

/// 工具沙箱策略：默认工作区范围 + Job 隔离（允许显式降级，审计记录）。
/// `unrestricted` 档位显式放开文件/网络范围（`SandboxPolicy::validate` 要求显式开关）。
pub(super) fn tool_sandbox_policy(
    ctx: &ToolContext<'_>,
    name: &str,
) -> crate::sandbox::SandboxPolicy {
    let mut policy = crate::sandbox::SandboxPolicy::for_workspace(name, ctx.workspace);
    policy.require_isolation = crate::sandbox::IsolationLevel::JobOnly;
    policy.allow_degraded = true;
    if ctx.policy.profile() == crate::permissions::PermissionProfile::Unrestricted {
        policy.file_scope = crate::FileScope::Unrestricted;
        policy.allow_unrestricted_file = true;
        policy.network_policy = crate::NetworkPolicy::Unrestricted;
        policy.allow_unrestricted_network = true;
    }
    policy
}
