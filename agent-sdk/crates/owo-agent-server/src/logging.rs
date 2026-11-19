// R11:logging 质量收尾完成。
// R12:logging 复核完成（trace_id 贯穿/脱敏，无需改动）。
//! 结构化日志与 trace_id（R8 + R9 + R10 文件日志/轮转）：JSON 单行日志 + 统一脱敏 + 请求贯穿 ID。
//! R10:logging 已接线（§13 批次八：`init_file_logging` serve 启动落盘
//! `logs/server.jsonl` + 8MB×5 轮转，优雅关闭 `close_file_logging` 配对；
//! `set_current_trace_id` 请求入口设置/清除；emit 未显式传 trace_id 时自动继承
//! 全局上下文；`audit_event` 已由自动化触发与服务生命周期三站点消费）。
//!
//! - `emit`：分级（trace/debug/info/warn/error）单行 JSON（ts/level/target/trace_id/msg/fields），
//!   同时写 stderr 与（可选）轮转文件。
//! - `Redactor`：统一脱敏器——apiKey/消息内容默认不落详文（保留前缀 + 哈希指纹）。
//! - `TraceId`：`X-Trace-Id` 头继承或生成（uuid 短格式），可序列化进日志与 SSE 帧。
//! - R9 全局 trace 上下文：`set_current_trace_id`/`current_trace_id`，供 SSE 事件、
//!   /metrics 关联字段与后台任务继承当前请求的 trace_id。
//! - R10 文件日志：`init_file_logging(path, max_bytes, backups)` 追加落盘，达到
//!   max_bytes 时按 `.1/.2/…` 轮转，保留 `backups` 份。
//!
//! 本模块不引用 `crate::`/`super::`，可被测试以 `#[path] mod` 独立编译。

// 主控接线现状（§13 批次八更新）：lib 目标引用 TraceId/emit/Level 及全部
// R10 面（init/close_file_logging、audit_event、safe_field、sanitize_json、
// Redactor）；无 logging 专用 #[path] 测试目标（历史注释"测试面符号"已失实，
// 批次七纠偏、批次八接线消化）。Level::Trace/Debug 为预留诊断级别，保留 allow。

use serde_json::{json, Value};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 日志级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 预留诊断级别（当前无调用方；详细诊断开关接线时启用，届时摘除 allow）。
    #[allow(dead_code)]
    Trace,
    /// 预留诊断级别（同上）。
    #[allow(dead_code)]
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Trace => "trace",
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// 统一脱敏器：凭据/消息内容默认不落详文。
/// 已接线（§13 批次八）：经 safe_field/sanitize_json 于 audit_event 与
/// emit 文件落盘路径消费。
pub struct Redactor;

impl Redactor {
    /// apiKey 类：保留前 4 后 4，中间 `***`；短值整体掩码。按字符切片（避免多字节 UTF-8 panic）。
    pub fn redact_api_key(value: &str) -> String {
        let chars: Vec<char> = value.chars().collect();
        if chars.len() <= 8 {
            return "***".to_string();
        }
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}***{tail}")
    }

    /// 消息内容：只落长度 + 哈希指纹（DefaultHasher 前 8 位十六进制），不落详文。
    pub fn redact_message(value: &str) -> String {
        let fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        };
        format!("len={} hash={}", value.chars().count(), &fingerprint[..8])
    }

    /// 通用脱敏：非敏感字段可保留原文；此处默认同消息策略（保守）。
    pub fn redact(value: &str) -> String {
        Self::redact_message(value)
    }

    /// 按字段名选择策略（含 "key"/"token"/"secret"/"password"/"auth" 的字段名走密钥策略；
    /// "message"/"content"/"prompt"/"text" 走消息策略）。
    pub fn redact_field(name: &str, value: &str) -> String {
        let lower = name.to_ascii_lowercase();
        if [
            "key", "token", "secret", "password", "api_key", "apikey", "auth", "bearer",
        ]
        .iter()
        .any(|k| lower.contains(k))
        {
            Self::redact_api_key(value)
        } else if ["message", "content", "prompt", "text", "body"]
            .iter()
            .any(|k| lower.contains(k))
        {
            Self::redact_message(value)
        } else {
            Self::redact(value)
        }
    }
}

/// 请求贯穿 ID：从 `X-Trace-Id` 头继承或生成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceId(pub String);

/// 贯穿头名（wire 与 CORS 放行清单的唯一来源）。
pub const TRACE_HEADER: &str = "x-trace-id";

impl TraceId {
    /// 生成新 trace_id（uuid v4 短格式）。
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string()[..24].to_string())
    }

    /// 从请求头继承（非空合法）或生成。
    pub fn from_header(header: Option<&str>) -> Self {
        match header {
            Some(v)
                if !v.trim().is_empty()
                    && v.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') =>
            {
                Self(v.trim().to_string())
            }
            _ => Self::generate(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 回填响应头 `X-Trace-Id` 的值。
    pub fn to_header_value(&self) -> String {
        self.0.clone()
    }
}

/// 全局 trace 上下文（R9）：请求入口设置，后台任务/SSE/指标可继承。
/// 单例语义为“当前活跃请求”；接线方在 middleware 中 set，请求结束清除。
static CURRENT_TRACE: Mutex<Option<String>> = Mutex::new(None);

/// 设置当前 trace 上下文（None 清除）。
pub fn set_current_trace_id(trace_id: Option<&str>) {
    let mut slot = CURRENT_TRACE.lock().unwrap_or_else(|e| e.into_inner());
    *slot = trace_id.map(str::to_string);
}

/// 读取当前 trace 上下文（无则 None）。
pub fn current_trace_id() -> Option<String> {
    CURRENT_TRACE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 输出一条 JSON 结构化日志（单行，stderr + 可选文件；无 trace_id 时继承全局上下文，仍无则省略字段）。
pub fn emit(
    level: Level,
    target: &str,
    trace_id: Option<&str>,
    message: &str,
    fields: &[(&str, Value)],
) {
    let effective_trace = trace_id.map(str::to_string).or_else(current_trace_id);
    let mut entry = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "level": level.as_str(),
        "target": target,
        "msg": message,
    });
    if let Some(trace_id) = effective_trace {
        entry["trace_id"] = json!(trace_id);
    }
    for (name, value) in fields {
        entry[name] = value.clone();
    }
    let line = entry.to_string();
    eprintln!("{}", line);
    // R10 文件日志（§13 批次八接线）：落盘前对用户供给字段统一脱敏。
    // 信任边界：ts/level/target/trace_id 为结构字段、msg 契约为第一方静态描述，
    // 均保持原样；动态内容必须经 fields 传递（按字段名走 Redactor 策略，
    // 未知字段名保守哈希）。stderr 保持完整原文供现场诊断。
    let field_map: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(name, value)| ((*name).to_string(), value.clone()))
        .collect();
    let mut wrapped = json!({ "fields": field_map });
    sanitize_json(&mut wrapped);
    let mut file_entry = entry;
    if let Some(map) = wrapped.get_mut("fields").and_then(|v| v.as_object_mut()) {
        for (name, value) in map {
            file_entry[name] = value.clone();
        }
    }
    write_file_log(&file_entry.to_string());
}

// ==================== R10：文件日志与轮转 ====================

/// 文件日志配置（append + 大小轮转）。
struct FileLog {
    file: std::fs::File,
    path: PathBuf,
    max_bytes: u64,
    backups: u32,
    current: u64,
}

static FILE_LOG: Mutex<Option<FileLog>> = Mutex::new(None);

/// 初始化文件日志（R10）：追加写入 `path`，达到 `max_bytes` 时轮转
/// （`.1/.2/…` 移位），保留 `backups` 份（上限 9）。失败返回 Err，不 panic；
/// 未初始化时 emit 仅落 stderr。
/// 已接线（§13 批次八）：cli serve 启动时落盘 `logs/server.jsonl`（8MB×5 轮转）。
pub fn init_file_logging(path: &Path, max_bytes: u64, backups: u32) -> std::io::Result<()> {
    // 首次启动时父目录（`<data_root>/logs`）还不存在：核心不在初始化前预建它，
    // 直接 open 只会拿到 ERROR_PATH_NOT_FOUND（Windows os error 3），文件日志
    // 于是静默降级成「仅落 stderr」——正是这条长期没被发现的失效路径。
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let current = file.metadata().map(|m| m.len()).unwrap_or(0);
    *FILE_LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(FileLog {
        file,
        path: path.to_path_buf(),
        max_bytes: max_bytes.max(1024),
        backups: backups.clamp(0, 9),
        current,
    });
    Ok(())
}

/// 关闭文件日志（优雅关闭时调用）。
/// 已接线（§13 批次八）：cli serve 关闭序列与 init 配对调用。
pub fn close_file_logging() {
    *FILE_LOG.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// 追加一行到轮转文件（失败静默——日志不得拖垮业务）。
fn write_file_log(line: &str) {
    let mut guard = FILE_LOG.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(file_log) = guard.as_mut() {
        let bytes = line.len() as u64 + 1;
        if file_log.current + bytes > file_log.max_bytes {
            rotate(file_log);
        }
        if file_log.current + bytes <= file_log.max_bytes {
            let _ = file_log.file.write_all(line.as_bytes());
            let _ = file_log.file.write_all(b"\n");
            file_log.current += bytes;
        }
    }
}

/// 轮转：`.n` 文件依次后移，当前文件移为 `.1`，重新打开新文件。
fn rotate(file_log: &mut FileLog) {
    let path = file_log.path.clone();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let _ = file_log.file.flush();
    for i in (1..file_log.backups).rev() {
        let src = path.with_file_name(format!("{name}.{i}"));
        let dst = path.with_file_name(format!("{name}.{}", i + 1));
        if dst.exists() {
            let _ = std::fs::remove_file(&dst);
        }
        if src.exists() {
            let _ = std::fs::rename(&src, &dst);
        }
    }
    let first = path.with_file_name(format!("{name}.1"));
    if first.exists() {
        let _ = std::fs::remove_file(&first);
    }
    let _ = std::fs::rename(&path, &first);
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(file) => {
            file_log.file = file;
            file_log.current = 0;
        }
        Err(_) => {
            // 打开失败：标记已满，避免反复轮转。
            file_log.current = file_log.max_bytes;
        }
    }
}

/// 审计事件便捷（R10）：结构化 + trace_id 贯穿 + 详情强制脱敏。
/// 注：HMAC 审计链在 core `audit_chain`；此处为可观测面审计日志事件。
/// 已接线（§13 批次八）：自动化触发、服务启动/关闭生命周期三站点调用。
pub fn audit_event(action: &str, trace_id: Option<&str>, detail: &str) {
    emit(
        Level::Info,
        "audit",
        trace_id,
        "audit_event",
        &[
            ("action", json!(action)),
            ("detail", safe_field("detail", detail)),
        ],
    );
}

/// 便捷：info 级。
pub fn info(target: &str, trace_id: Option<&str>, message: &str) {
    emit(Level::Info, target, trace_id, message, &[]);
}

/// 便捷：warn 级。
pub fn warn(target: &str, trace_id: Option<&str>, message: &str, fields: &[(&str, Value)]) {
    emit(Level::Warn, target, trace_id, message, fields);
}

/// 便捷：error 级。
pub fn error(target: &str, trace_id: Option<&str>, message: &str, fields: &[(&str, Value)]) {
    emit(Level::Error, target, trace_id, message, fields);
}

/// 把字段值按脱敏策略转换（供调用方落日志前使用）。
/// 已接线（§13 批次八）：audit_event 详情字段经由本函数脱敏。
pub fn safe_field(name: &str, value: &str) -> Value {
    json!(Redactor::redact_field(name, value))
}

/// 把任意 JSON 值中的敏感字段原地脱敏（遍历一层，递归两层）。
/// 已接线（§13 批次八）：emit 文件落盘路径对用户供给字段包装脱敏。
pub fn sanitize_json(value: &mut Value) {
    fn walk(value: &mut Value, depth: usize) {
        if depth > 2 {
            return;
        }
        if let Value::Object(map) = value {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(field) = map.get_mut(&key) {
                    if field.is_string() {
                        let original = field.as_str().unwrap_or_default();
                        *field = json!(Redactor::redact_field(&key, original));
                    } else {
                        walk(field, depth + 1);
                    }
                }
            }
        } else if let Value::Array(items) = value {
            for item in items {
                walk(item, depth + 1);
            }
        }
    }
    walk(value, 0);
}

/// 占位：确保 std::fmt::Write 路径类型完整（供测试独立编译）。
#[allow(dead_code)]
fn _probe_format() -> String {
    let mut s = String::new();
    let _ = write!(s, "{}", TraceId::generate().as_str());
    s
}
