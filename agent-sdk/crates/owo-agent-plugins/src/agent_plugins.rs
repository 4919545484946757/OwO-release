//! Agent Plugins 1.0.0 兼容层：把「厂商中立插件包」映射进 OwO 的插件宿主。
//!
//! ## 为什么做这一层
//!
//! 行业在 2026-08 发布了[Agent Plugins 1.0.0 规范](https://agent-plugins.org/specification)
//! （TSC：Amazon / Cursor / Microsoft / OpenAI / Vercel / Google）：`plugin.json` 清单 +
//! `skills/` 技能目录 + `mcp.json` MCP 声明，三件固定位置的东西就是一个可移植插件包。
//! 本 crate 原有的 `manifest.json` 是 OwO 自己的历史格式；这一层让**同一个包**既能被
//! 别的宿主加载，也能直接放进 `plugins/` 被我们加载，而不是要求作者为 OwO 再包一层。
//!
//! ## 规范里真正有约束力的几点（本实现逐条落实）
//!
//! 1. **清单先行**：先校验 `plugin.json`，再发现组件；清单非法 = 整个插件拒绝（§5）。
//! 2. **闭模式**：`plugin.json` 只允许 `$schema`/`name`/`version`/`description`/`author`/
//!    `homepage`/`repository`/`license`/`keywords`/`extensions`；未知**顶层**字段报告后
//!    忽略（非致命），其余 schema 违规致命（§5.2）。`extensions` 非对象同样非致命。
//! 3. **固定位置**：技能只在 `skills/` 的**直接子目录**里找 `SKILL.md`，不递归；
//!    MCP 只认根目录 `mcp.json`，不认清单内联（§6/§7）。
//! 4. **窄失败边界**（§4.1）：清单越界 → 拒绝插件；组件目录越界 → 只废掉该组件类型；
//!    单个 SKILL.md 越界 → 只跳过该技能；单个 MCP 条目非法 → 只跳过该服务器。
//! 5. **不联网取 schema**：`$schema` 只用于选择本地已支持的校验规则（§5.2）。
//!
//! 本模块只做「读 + 校验 + 映射」，不执行、不联网、不落盘；执行授权仍归 Tool-Safety
//! 与 MCP 宿主的沙箱门卫。

use crate::McpServerConfig;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// 本实现支持的 Agent Plugins 规范版本。
pub const AGENT_PLUGINS_SPEC_VERSION: &str = "1.0.0";
/// `plugin.json` 的 `$schema` 规范标识（v1.0.0）。
pub const AGENT_PLUGINS_MANIFEST_SCHEMA: &str =
    "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
/// `mcp.json` 的 `$schema` 规范标识（v1.0.0）。
pub const AGENT_PLUGINS_MCP_SCHEMA: &str =
    "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";
/// 插件清单文件名（固定在包根）。
pub const AGENT_PLUGINS_MANIFEST_FILE: &str = "plugin.json";
/// MCP 配置文件名（固定在包根）。
pub const AGENT_PLUGINS_MCP_FILE: &str = "mcp.json";
/// 技能固定目录名。
pub const AGENT_PLUGINS_SKILLS_DIR: &str = "skills";
/// 技能入口文件名。
pub const AGENT_PLUGINS_SKILL_FILE: &str = "SKILL.md";

// ---------------------------------------------------------------------------
// 客户端扩展命名空间（§8）：OwO 自己的 hooks 贡献点
// ---------------------------------------------------------------------------

/// 本客户端在 `extensions` 里使用的反向域名命名空间，也是扩展目录名（§8）。
pub const OWO_EXTENSION_NAMESPACE: &str = "com.owo.agent";
/// 扩展目录里的可选 hooks 文件（与清单内 `hooks` 数组等价）。
pub const OWO_EXTENSION_HOOKS_FILE: &str = "hooks.json";
/// 单包可声明的 hook 上限（防御性上限：hooks 串行执行，数量直接放大回合延迟）。
pub const MAX_PLUGIN_HOOKS: usize = 32;
/// hooks 贡献点认可的事件名（与 core `HookEvent` 同一组；跨 crate 无法共享常量，
/// 由 core 侧的映射测试保证同步）。
pub const PLUGIN_HOOK_EVENTS: &[&str] = &[
    "pre_tool_use",
    "post_tool_use",
    "user_prompt_submit",
    "stop",
    "pre_compact",
];

/// `plugin.json` 允许的顶层字段（闭模式，§5.2）。
const MANIFEST_FIELDS: &[&str] = &[
    "$schema",
    "name",
    "version",
    "description",
    "author",
    "homepage",
    "repository",
    "license",
    "keywords",
    "extensions",
];
/// `mcp.json` 允许的顶层字段（§7.2.1）。
const MCP_FIELDS: &[&str] = &["$schema", "mcpServers"];
/// stdio 服务器允许的字段（闭变体）。
const STDIO_FIELDS: &[&str] = &["type", "command", "args", "env", "cwd"];
/// HTTP 服务器允许的字段（闭变体）。
const HTTP_FIELDS: &[&str] = &["type", "url", "headers"];

// ---------------------------------------------------------------------------
// 诊断与错误
// ---------------------------------------------------------------------------

/// 诊断级别：加载过程**不中断**，但要让调用方看得见发生了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticLevel {
    /// 正常信息（发现了什么组件）。
    Info,
    /// 非致命问题：已按规范降级（忽略字段 / 跳过组件）。
    Warning,
    /// 致命问题的伴生记录（插件已被拒绝时，说明拒绝原因）。
    Error,
}

impl DiagnosticLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for DiagnosticLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 一条加载诊断。`code` 是稳定标识（日志/测试断言用），`message` 面向人。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDiagnostic {
    pub level: DiagnosticLevel,
    pub code: &'static str,
    pub path: Option<PathBuf>,
    pub message: String,
}

impl PluginDiagnostic {
    fn new(
        level: DiagnosticLevel,
        code: &'static str,
        path: Option<&Path>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            level,
            code,
            path: path.map(Path::to_path_buf),
            message: message.into(),
        }
    }

    fn info(code: &'static str, path: Option<&Path>, message: impl Into<String>) -> Self {
        Self::new(DiagnosticLevel::Info, code, path, message)
    }

    fn warning(code: &'static str, path: Option<&Path>, message: impl Into<String>) -> Self {
        Self::new(DiagnosticLevel::Warning, code, path, message)
    }
}

impl fmt::Display for PluginDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(
                formatter,
                "[{}] {} {}：{}",
                self.level,
                self.code,
                path.display(),
                self.message
            ),
            None => write!(
                formatter,
                "[{}] {}：{}",
                self.level, self.code, self.message
            ),
        }
    }
}

/// 插件级致命错误（对应规范里「拒绝整个插件」的那些边界）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPluginError {
    /// 稳定错误码（`manifest_missing` / `manifest_escape` / `manifest_invalid` …）。
    pub code: &'static str,
    pub message: String,
}

impl AgentPluginError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for AgentPluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}（{}）", self.message, self.code)
    }
}

impl std::error::Error for AgentPluginError {}

// ---------------------------------------------------------------------------
// 清单
// ---------------------------------------------------------------------------

/// 清单 `author` 对象（闭字段：name/email/url）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentPluginAuthor {
    pub name: Option<String>,
    pub email: Option<String>,
    pub url: Option<String>,
}

/// Agent Plugins 1.0.0 清单（`plugin.json`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPluginManifest {
    /// 原始 `$schema`（已校验为本地支持的版本）。
    pub schema: String,
    /// 插件名（同时作为宿主侧稳定 id）。
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub author: Option<AgentPluginAuthor>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub license: Option<String>,
    pub keywords: Vec<String>,
    /// 客户端扩展数据：命名空间 → 该命名空间自己的对象（内容不解释，§8）。
    pub extensions: BTreeMap<String, Map<String, Value>>,
}

impl AgentPluginManifest {
    /// 读取并校验包根 `plugin.json`。
    ///
    /// 返回 `(manifest, diagnostics)`：诊断里可能包含「未知顶层字段已忽略」
    /// 与「extensions 非对象已忽略」这两类**非致命**问题。
    pub fn load(plugin_root: &Path) -> Result<(Self, Vec<PluginDiagnostic>), AgentPluginError> {
        // 比较基准统一成规范路径；读文件前先确认解析结果仍在包根内（§4.1）。
        let root = std::fs::canonicalize(plugin_root).map_err(|error| {
            AgentPluginError::new(
                "plugin_root_unavailable",
                format!("插件目录不可用（{}）：{error}", plugin_root.display()),
            )
        })?;
        let manifest_path = root.join(AGENT_PLUGINS_MANIFEST_FILE);
        let metadata = std::fs::symlink_metadata(&manifest_path).map_err(|error| {
            AgentPluginError::new(
                "manifest_missing",
                format!("缺少 {}：{error}", manifest_path.display()),
            )
        })?;
        // 允许包内符号链接，但解析后必须仍落在包根内。
        if !metadata.is_file() && !metadata.file_type().is_symlink() {
            return Err(AgentPluginError::new(
                "manifest_not_file",
                format!("{} 不是普通文件", manifest_path.display()),
            ));
        }
        let canonical = std::fs::canonicalize(&manifest_path).map_err(|error| {
            AgentPluginError::new("manifest_missing", format!("plugin.json 无法解析：{error}"))
        })?;
        if !path_is_within(&root, &canonical) {
            return Err(AgentPluginError::new(
                "manifest_escape",
                "plugin.json 解析到包根之外，拒绝加载",
            ));
        }
        if !canonical.is_file() {
            return Err(AgentPluginError::new(
                "manifest_not_file",
                "plugin.json 解析结果不是普通文件",
            ));
        }
        let content = std::fs::read_to_string(&canonical).map_err(|error| {
            AgentPluginError::new(
                "manifest_unreadable",
                format!("读取 {} 失败：{error}", canonical.display()),
            )
        })?;
        let value: Value = serde_json::from_str(&content).map_err(|error| {
            AgentPluginError::new(
                "manifest_invalid_json",
                format!("{} 不是合法 JSON：{error}", canonical.display()),
            )
        })?;
        Self::from_value(&value)
    }

    /// 从已解析的 JSON 值校验清单（供测试与「先读后判」场景复用）。
    pub fn from_value(value: &Value) -> Result<(Self, Vec<PluginDiagnostic>), AgentPluginError> {
        let object = value.as_object().ok_or_else(|| {
            AgentPluginError::new("manifest_not_object", "plugin.json 顶层必须是 JSON 对象")
        })?;
        let mut diagnostics = Vec::new();

        // 未知顶层字段：报告并忽略（§5.2 非致命）。
        for key in object.keys() {
            if !MANIFEST_FIELDS.contains(&key.as_str()) {
                diagnostics.push(PluginDiagnostic::warning(
                    "manifest_unknown_field",
                    None,
                    format!("未知顶层字段 `{key}` 已忽略（闭模式只允许 10 个字段）"),
                ));
            }
        }

        let schema = required_string(object, "$schema", "manifest_missing_schema")?;
        if schema != AGENT_PLUGINS_MANIFEST_SCHEMA {
            return Err(AgentPluginError::new(
                "manifest_unsupported_schema",
                format!(
                    "不支持的 $schema `{schema}`：本客户端只实现 Agent Plugins {AGENT_PLUGINS_SPEC_VERSION}"
                ),
            ));
        }
        let name = required_string(object, "name", "manifest_missing_name")?;
        validate_plugin_name(&name)?;

        Ok((
            Self {
                schema,
                name,
                version: optional_string(object, "version")?,
                description: optional_string(object, "description")?,
                author: optional_author(object)?,
                homepage: optional_string(object, "homepage")?,
                repository: optional_string(object, "repository")?,
                license: optional_string(object, "license")?,
                keywords: optional_string_array(object, "keywords")?,
                extensions: parse_extensions(object, &mut diagnostics)?,
            },
            diagnostics,
        ))
    }
}

fn required_string(
    object: &Map<String, Value>,
    key: &str,
    code: &'static str,
) -> Result<String, AgentPluginError> {
    match object.get(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
        Some(Value::String(_)) => Err(AgentPluginError::new(
            code,
            format!("字段 `{key}` 不能为空字符串"),
        )),
        Some(_) => Err(AgentPluginError::new(
            code,
            format!("字段 `{key}` 必须是字符串"),
        )),
        None => Err(AgentPluginError::new(code, format!("缺少必填字段 `{key}`"))),
    }
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, AgentPluginError> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(AgentPluginError::new(
            "manifest_invalid_field_type",
            format!("字段 `{key}` 必须是字符串"),
        )),
    }
}

fn optional_string_array(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, AgentPluginError> {
    match object.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(item) = item else {
                    return Err(AgentPluginError::new(
                        "manifest_invalid_field_type",
                        format!("字段 `{key}` 必须是字符串数组"),
                    ));
                };
                out.push(item.clone());
            }
            Ok(out)
        }
        Some(_) => Err(AgentPluginError::new(
            "manifest_invalid_field_type",
            format!("字段 `{key}` 必须是字符串数组"),
        )),
    }
}

fn optional_author(
    object: &Map<String, Value>,
) -> Result<Option<AgentPluginAuthor>, AgentPluginError> {
    let Some(value) = object.get("author") else {
        return Ok(None);
    };
    let Value::Object(author) = value else {
        return Err(AgentPluginError::new(
            "manifest_invalid_field_type",
            "字段 `author` 必须是对象",
        ));
    };
    for key in author.keys() {
        if !matches!(key.as_str(), "name" | "email" | "url") {
            return Err(AgentPluginError::new(
                "manifest_author_unknown_field",
                format!("author 不允许字段 `{key}`（只允许 name/email/url）"),
            ));
        }
    }
    let field = |key: &str| -> Result<Option<String>, AgentPluginError> {
        match author.get(key) {
            None => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => Err(AgentPluginError::new(
                "manifest_author_invalid_field_type",
                format!("author.`{key}` 必须是字符串"),
            )),
        }
    };
    Ok(Some(AgentPluginAuthor {
        name: field("name")?,
        email: field("email")?,
        url: field("url")?,
    }))
}

fn parse_extensions(
    object: &Map<String, Value>,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> Result<BTreeMap<String, Map<String, Value>>, AgentPluginError> {
    let Some(value) = object.get("extensions") else {
        return Ok(BTreeMap::new());
    };
    let Value::Object(extensions) = value else {
        // §5.2：extensions 非对象是**非致命**的，报告后继续。
        diagnostics.push(PluginDiagnostic::warning(
            "manifest_extensions_not_object",
            None,
            "`extensions` 不是对象，已忽略该字段",
        ));
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (namespace, value) in extensions {
        let Value::Object(value) = value else {
            return Err(AgentPluginError::new(
                "manifest_extension_not_object",
                format!("extensions.`{namespace}` 必须是对象"),
            ));
        };
        out.insert(namespace.clone(), value.clone());
    }
    Ok(out)
}

/// 插件名约束（§5.5）：1-64 字符、`[a-z0-9.-]`、首尾字母数字、禁止 `--` 与 `..`。
pub fn validate_plugin_name(name: &str) -> Result<(), AgentPluginError> {
    let characters: Vec<char> = name.chars().collect();
    if characters.is_empty() {
        return Err(AgentPluginError::new(
            "plugin_name_invalid",
            "插件名不能为空",
        ));
    }
    if characters.len() > 64 {
        return Err(AgentPluginError::new(
            "plugin_name_invalid",
            format!("插件名超过 64 字符（{}）", characters.len()),
        ));
    }
    if !characters.iter().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '.')
    }) {
        return Err(AgentPluginError::new(
            "plugin_name_invalid",
            format!("插件名 `{name}` 含非法字符（只允许 a-z 0-9 - .）"),
        ));
    }
    let first = characters.first().copied().unwrap_or(' ');
    let last = characters.last().copied().unwrap_or(' ');
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return Err(AgentPluginError::new(
            "plugin_name_invalid",
            format!("插件名 `{name}` 必须以字母或数字开头/结尾"),
        ));
    }
    if name.contains("--") || name.contains("..") {
        return Err(AgentPluginError::new(
            "plugin_name_invalid",
            format!("插件名 `{name}` 不得含连续 `-` 或 `.`"),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 静态风险扫描（只扫可执行内容，不把清单元数据当网络请求）
// ---------------------------------------------------------------------------

/// 会被静态扫描的代码后缀（纯文本脚本；不碰二进制）。
const CODE_EXTENSIONS: &[&str] = &[
    "py", "js", "mjs", "cjs", "ts", "ps1", "sh", "bash", "bat", "cmd", "rb", "pl", "lua", "go",
];

/// 单文件与总量上限：扫描是启发式的，不值得为超大包付出 IO 与内存。
const SCAN_FILE_LIMIT_BYTES: usize = 256 * 1024;
const SCAN_TOTAL_LIMIT_BYTES: usize = 1024 * 1024;

/// 收集包内**可执行内容**（技能 `scripts/`、`servers/`、常见脚本后缀）。
///
/// 刻意排除 `plugin.json` / `mcp.json` / `SKILL.md`：清单里的 `$schema`、`homepage`、
/// `repository` 是元数据，不是网络访问；把它们交给 URL 提取器只会产生假阳性。
pub fn collect_agent_plugin_code(root: &Path) -> String {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut collected = String::new();
    let mut total = 0usize;
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                // 解析后仍在包内才进入（与组件发现的边界一致）。
                if let Ok(canonical) = std::fs::canonicalize(&path) {
                    if path_is_within(&root, &canonical) {
                        pending.push(canonical);
                    }
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !CODE_EXTENSIONS.contains(&extension.as_str()) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.len() > SCAN_FILE_LIMIT_BYTES as u64 {
                continue;
            }
            if total + metadata.len() as usize > SCAN_TOTAL_LIMIT_BYTES {
                break;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                total += content.len();
                collected.push_str(&content);
                collected.push('\n');
            }
        }
    }
    collected
}

/// 规范包静态扫描：可执行内容 + 网络域白名单（规范包没有 allowlist，默认全不信任）。
pub fn scan_agent_plugin_for_risks(root: &Path, allowlist_domains: &[String]) -> Vec<String> {
    let code = collect_agent_plugin_code(root);
    crate::plugin::scan_plugin_for_risks("", Some(&code), allowlist_domains)
}

// ---------------------------------------------------------------------------
// 可选签名层（OwO 扩展，非规范强制）：`plugin.sig.json`
// ---------------------------------------------------------------------------

/// 规范包签名文件名（包根）。
pub const AGENT_PLUGIN_SIGNATURE_FILE: &str = "plugin.sig.json";

/// 规范包的可选签名（detached，覆盖整个包内容）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentPluginPackageSignature {
    /// 当前仅 `ed25519`。
    pub algorithm: String,
    /// 公钥（base64，32 字节）。
    pub public_key: String,
    /// 签名（base64，64 字节）。
    pub signature: String,
}

/// 包摘要：按相对路径排序，对「相对路径 \0 文件 sha256 \n」整体再做 sha256。
///
/// 排除签名文件本身、隐藏目录（如 `.git`）与符号链接，保证"签名覆盖可移植内容"。
pub fn agent_plugin_package_digest(root: &Path) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    let root = std::fs::canonicalize(root)
        .map_err(|error| format!("包目录不可用（{}）：{error}", root.display()))?;
    let mut files: Vec<String> = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| format!("读取目录失败（{}）：{error}", directory.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            let name = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            if metadata.is_dir() {
                if name.starts_with('.') {
                    continue;
                }
                pending.push(path);
            } else if name != AGENT_PLUGIN_SIGNATURE_FILE {
                let relative = path
                    .strip_prefix(&root)
                    .map(|value| value.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                files.push(relative);
            }
        }
    }
    files.sort();
    let mut outer = Sha256::new();
    for relative in files {
        let path = root.join(&relative);
        let content = std::fs::read(&path)
            .map_err(|error| format!("读取文件失败（{}）：{error}", path.display()))?;
        let mut inner = Sha256::new();
        inner.update(&content);
        outer.update(relative.as_bytes());
        outer.update(b"\0");
        outer.update(inner.finalize());
        outer.update(b"\n");
    }
    Ok(outer.finalize().into())
}

/// 读取包根的可选签名文件；不存在返回 `Ok(None)`。
pub fn load_agent_plugin_signature(
    root: &Path,
) -> Result<Option<AgentPluginPackageSignature>, String> {
    let path = root.join(AGENT_PLUGIN_SIGNATURE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&path).map_err(|error| format!("读取签名文件失败：{error}"))?;
    let signature: AgentPluginPackageSignature = serde_json::from_str(&content)
        .map_err(|error| format!("签名文件不是合法 JSON：{error}"))?;
    Ok(Some(signature))
}

/// 校验规范包签名：内容摘要 + Ed25519 验签。
pub fn verify_agent_plugin_signature(
    root: &Path,
    signature: &AgentPluginPackageSignature,
) -> Result<(), String> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use std::convert::TryInto;

    if signature.algorithm.to_lowercase() != "ed25519" {
        return Err(format!("不支持的签名算法：{}", signature.algorithm));
    }
    let public_key = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &signature.public_key,
    )
    .map_err(|error| format!("公钥 base64 非法：{error}"))?;
    let signature_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &signature.signature,
    )
    .map_err(|error| format!("签名 base64 非法：{error}"))?;
    let key: VerifyingKey = public_key
        .as_slice()
        .try_into()
        .map_err(|_| "公钥长度非法（需 32 字节）".to_string())?;
    let sig: Signature = signature_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "签名长度非法（需 64 字节）".to_string())?;
    let digest = agent_plugin_package_digest(root)?;
    key.verify(&digest, &sig)
        .map_err(|_| "规范包签名校验失败（内容可能被篡改）".to_string())
}

// ---------------------------------------------------------------------------
// 技能
// ---------------------------------------------------------------------------

/// 一个通过规范校验的技能。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSkill {
    /// frontmatter 的 `name`（Agent Skills 规范要求）。
    pub name: String,
    /// frontmatter 的 `description`。
    pub description: String,
    /// 技能目录（`skills/<dir>` 的规范路径）。
    pub dir: PathBuf,
    /// `SKILL.md` 的规范路径。
    pub skill_md: PathBuf,
}

/// 插件贡献的 hooks（`com.owo.agent` 扩展命名空间）。
///
/// 与宿主 `hooks` 同一个执行器，但来源可追溯（`plugin:<name>`），便于审计与排障。
/// 脚本必须是包内文件（`./...` 或 `${PLUGIN_ROOT}/...`）：不接受裸命令，避免借
/// PATH 执行包外程序；扩展名决定解释器（见 core 的 `plugin_hook_configs`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPluginHook {
    /// 事件名（[`PLUGIN_HOOK_EVENTS`] 之一）。
    pub event: String,
    /// 工具名通配（`*` / `desktop_*`；缺省 = 全部）。
    pub matcher: Option<String>,
    /// 包内脚本的规范路径。
    pub script: PathBuf,
    /// 传给脚本的参数（占位符已展开；不含引号/换行）。
    pub args: Vec<String>,
    /// `open`（缺省）/ `closed`。
    pub fail_mode: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------

/// 本客户端支持的服务器传输类型。
///
/// 规范里还有已废弃的 `sse`（MCP 2024-11-05 的 GET /sse 长连接）：本客户端
/// 的 HTTP 传输是 Streamable HTTP，遇到 `sse` 按 §7.2.2 第 4 条跳过该服务器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMcpTransport {
    Stdio,
    /// MCP Streamable HTTP（当前 MCP 传输）。
    StreamableHttp,
}

impl AgentMcpTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::StreamableHttp => "streamable-http",
        }
    }
}

/// 一个通过校验、已映射到宿主 `McpServerConfig` 的服务器。
#[derive(Debug, Clone)]
pub struct AgentMcpServer {
    /// `mcpServers` 里的成员名（工具名前缀来源）。
    pub key: String,
    /// 声明类型（保留原始值，便于报告与审计）。
    pub transport: AgentMcpTransport,
    /// 宿主运行配置（相对路径已解析、占位符已展开）。
    pub config: McpServerConfig,
}

// ---------------------------------------------------------------------------
// 加载结果
// ---------------------------------------------------------------------------

/// 一个加载完成的 Agent Plugins 包。
#[derive(Debug, Clone)]
pub struct LoadedAgentPlugin {
    /// 规范化的包根目录（所有组件路径都在它之内）。
    pub root: PathBuf,
    pub manifest: AgentPluginManifest,
    pub skills: Vec<AgentSkill>,
    pub mcp_servers: Vec<AgentMcpServer>,
    /// 客户端扩展命名空间贡献的 hooks。
    pub hooks: Vec<AgentPluginHook>,
    /// 加载过程中的全部诊断（信息 + 非致命问题）。
    pub diagnostics: Vec<PluginDiagnostic>,
}

impl LoadedAgentPlugin {
    /// 宿主侧稳定 id（就是规范里的插件名）。
    pub fn id(&self) -> &str {
        &self.manifest.name
    }
}

/// 加载并校验一个 Agent Plugins 1.0.0 包。
///
/// `plugin_data_root` 是客户端管理的插件数据目录（用于 `${PLUGIN_DATA}` 展开与
/// 越界校验）。传 `None` 时使用 `<root>/.plugin-data` 作为**校验用**基准（不会创建）。
pub fn load_agent_plugin(
    plugin_root: &Path,
    plugin_data_root: Option<&Path>,
) -> Result<LoadedAgentPlugin, AgentPluginError> {
    let root = std::fs::canonicalize(plugin_root).map_err(|error| {
        AgentPluginError::new(
            "plugin_root_unavailable",
            format!("插件目录不可用（{}）：{error}", plugin_root.display()),
        )
    })?;
    if !root.is_dir() {
        return Err(AgentPluginError::new(
            "plugin_root_unavailable",
            format!("插件根 {} 不是目录", root.display()),
        ));
    }
    let data_root = plugin_data_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(".plugin-data"));

    let (manifest, mut diagnostics) = AgentPluginManifest::load(&root)?;

    // 清单必须解析在包根之内（§4.1 边界 1；symlink 已在 load 内拦截，这里再兜底）。
    let manifest_canonical = std::fs::canonicalize(root.join(AGENT_PLUGINS_MANIFEST_FILE))
        .map_err(|error| {
            AgentPluginError::new(
                "manifest_missing",
                format!("plugin.json 无法规范化：{error}"),
            )
        })?;
    if !path_is_within(&root, &manifest_canonical) {
        return Err(AgentPluginError::new(
            "manifest_escape",
            "plugin.json 解析到包根之外，拒绝加载",
        ));
    }

    let skills = discover_skills(&root, &mut diagnostics);
    let mcp_servers = load_mcp_servers(&root, &data_root, &manifest, &mut diagnostics);
    let hooks = load_plugin_hooks(&root, &data_root, &manifest, &mut diagnostics);

    diagnostics.push(PluginDiagnostic::info(
        "plugin_loaded",
        Some(&root),
        format!(
            "Agent Plugins {} 已加载：skills={} mcp_servers={} hooks={}",
            AGENT_PLUGINS_SPEC_VERSION,
            skills.len(),
            mcp_servers.len(),
            hooks.len()
        ),
    ));

    Ok(LoadedAgentPlugin {
        root,
        manifest,
        skills,
        mcp_servers,
        hooks,
        diagnostics,
    })
}

/// 目录是否像一个 Agent Plugins 包（只做存在性判断，不做校验）。
pub fn is_agent_plugin_dir(directory: &Path) -> bool {
    directory.join(AGENT_PLUGINS_MANIFEST_FILE).is_file()
}

// ---------------------------------------------------------------------------
// 客户端扩展：hooks 贡献点
// ---------------------------------------------------------------------------

/// 读取 `com.owo.agent` 命名空间贡献的 hooks。
///
/// 两个来源（§8 允许清单内数据与客户端目录并存）：
/// 1. `plugin.json` 的 `extensions["com.owo.agent"].hooks` 数组；
/// 2. 包根 `com.owo.agent/hooks.json`（同样的数组）。
///
/// 单条非法只跳过自己（与组件级窄边界一致），并在诊断里说明原因。
fn load_plugin_hooks(
    root: &Path,
    data_root: &Path,
    manifest: &AgentPluginManifest,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> Vec<AgentPluginHook> {
    let mut raw_entries: Vec<serde_json::Value> = Vec::new();
    if let Some(namespace) = manifest.extensions.get(OWO_EXTENSION_NAMESPACE) {
        if let Some(hooks) = namespace.get("hooks") {
            match hooks.as_array() {
                Some(entries) => raw_entries.extend(entries.iter().cloned()),
                None => diagnostics.push(PluginDiagnostic::warning(
                    "plugin_hooks_not_array",
                    None,
                    "extensions.com.owo.agent.hooks 不是数组，已忽略",
                )),
            }
        }
    }
    let hooks_file = root
        .join(OWO_EXTENSION_NAMESPACE)
        .join(OWO_EXTENSION_HOOKS_FILE);
    if std::fs::symlink_metadata(&hooks_file).is_ok() {
        match std::fs::canonicalize(&hooks_file) {
            Ok(canonical) if path_is_within(root, &canonical) && canonical.is_file() => {
                match std::fs::read_to_string(&canonical)
                    .ok()
                    .and_then(|content| serde_json::from_str::<Value>(&content).ok())
                {
                    Some(Value::Array(entries)) => raw_entries.extend(entries),
                    _ => diagnostics.push(PluginDiagnostic::warning(
                        "plugin_hooks_file_invalid",
                        Some(&canonical),
                        "扩展目录 hooks.json 必须是数组，已忽略",
                    )),
                }
            }
            _ => diagnostics.push(PluginDiagnostic::warning(
                "plugin_hooks_file_invalid",
                Some(&hooks_file),
                "扩展目录 hooks.json 解析到包外或不是普通文件，已忽略",
            )),
        }
    }

    let mut hooks = Vec::new();
    for entry in raw_entries {
        if hooks.len() >= MAX_PLUGIN_HOOKS {
            diagnostics.push(PluginDiagnostic::warning(
                "plugin_hooks_truncated",
                Some(root),
                format!("hooks 超过 {MAX_PLUGIN_HOOKS} 条，其余已忽略"),
            ));
            break;
        }
        match parse_plugin_hook(&entry, root, data_root) {
            Ok(hook) => hooks.push(hook),
            Err(reason) => diagnostics.push(PluginDiagnostic::warning(
                "plugin_hook_skipped",
                Some(root),
                format!("hook 已跳过：{reason}"),
            )),
        }
    }
    hooks
}

fn parse_plugin_hook(
    value: &Value,
    root: &Path,
    data_root: &Path,
) -> Result<AgentPluginHook, String> {
    const FIELDS: &[&str] = &["event", "matcher", "command", "args", "fail_mode"];
    let object = value
        .as_object()
        .ok_or_else(|| "hook 必须是 JSON 对象".to_string())?;
    if let Some(unknown) = object
        .keys()
        .find(|field| !FIELDS.contains(&field.as_str()))
    {
        return Err(format!("hook 不允许字段 `{unknown}`"));
    }
    let event = match object.get("event") {
        Some(Value::String(event)) => event.clone(),
        Some(_) => return Err("`event` 必须是字符串".to_string()),
        None => return Err("缺少必填字段 `event`".to_string()),
    };
    if !PLUGIN_HOOK_EVENTS.contains(&event.as_str()) {
        return Err(format!(
            "未知事件 `{event}`（允许：{}）",
            PLUGIN_HOOK_EVENTS.join("/")
        ));
    }
    let command = match object.get("command") {
        Some(Value::String(command)) if !command.is_empty() => command.clone(),
        Some(_) => return Err("`command` 必须是非空字符串".to_string()),
        None => return Err("缺少必填字段 `command`".to_string()),
    };
    // 只接受包内脚本：裸命令可借 PATH 执行包外程序，与「能力随包分发」不符。
    let relative = if is_plugin_relative(&command) {
        PathBuf::from(&command)
    } else if let Some(rest) = command.strip_prefix("${PLUGIN_ROOT}/") {
        PathBuf::from(rest)
    } else {
        return Err(format!(
            "`command` 必须是包内脚本（./x 或 ${{PLUGIN_ROOT}}/x）：{command}"
        ));
    };
    let resolved = resolve_within(root, &relative)
        .ok_or_else(|| format!("`command` 越出插件根：{command}"))?;
    let canonical = std::fs::canonicalize(&resolved)
        .map_err(|error| format!("`command` 脚本不存在（{}）：{error}", resolved.display()))?;
    if !path_is_within(root, &canonical) || !canonical.is_file() {
        return Err(format!("`command` 解析到包外或不是普通文件：{command}"));
    }
    let args = match object.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            let mut args = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(item) = item else {
                    return Err("`args` 必须是字符串数组".to_string());
                };
                if item.contains('"') || item.contains(['\n', '\r']) {
                    return Err(format!("参数含引号或换行，拒绝拼进 shell：{item}"));
                }
                args.push(expand_placeholders(item, root, data_root));
            }
            args
        }
        Some(_) => return Err("`args` 必须是字符串数组".to_string()),
    };
    let matcher = match object.get("matcher") {
        None => None,
        Some(Value::String(matcher)) => Some(matcher.clone()),
        Some(_) => return Err("`matcher` 必须是字符串".to_string()),
    };
    let fail_mode = match object.get("fail_mode") {
        None => None,
        Some(Value::String(mode))
            if mode.eq_ignore_ascii_case("open") || mode.eq_ignore_ascii_case("closed") =>
        {
            Some(mode.to_ascii_lowercase())
        }
        Some(_) => return Err("`fail_mode` 只能是 open / closed".to_string()),
    };
    Ok(AgentPluginHook {
        event,
        matcher,
        script: canonical,
        args,
        fail_mode,
    })
}

// ---------------------------------------------------------------------------
// 路径工具
// ---------------------------------------------------------------------------

/// 规范化路径用于**比较**：统一分隔符、去掉 `\\?\` 扩展前缀、折叠末尾分隔符，
/// Windows 上再折叠大小写（其余平台保持大小写敏感，避免把 `/A` 当成 `/a`）。
fn normalize_for_compare(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string();
    let mut normalized = raw.replace('/', "\\");
    while normalized.len() > 3 && normalized.ends_with('\\') {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

/// `candidate` 是否等于 `base` 或位于其子树内（Windows 大小写不敏感）。
pub fn path_is_within(base: &Path, candidate: &Path) -> bool {
    let base = normalize_for_compare(base);
    let candidate = normalize_for_compare(candidate);
    if candidate == base {
        return true;
    }
    let prefix = if base.ends_with('\\') {
        base
    } else {
        format!("{base}\\")
    };
    candidate.starts_with(&prefix)
}

/// 词法解析「相对 base 的路径」，遇到越界的 `..` 返回 None（不触盘）。
fn resolve_within(base: &Path, relative: &Path) -> Option<PathBuf> {
    if relative.is_absolute() {
        return None;
    }
    let mut resolved = base.to_path_buf();
    let mut depth = 0usize;
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => {
                resolved.push(segment);
                depth += 1;
            }
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }
                resolved.pop();
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(resolved)
}

/// 展开 `${PLUGIN_ROOT}` / `${PLUGIN_DATA}`（只在这两种占位符上生效）。
fn expand_placeholders(value: &str, root: &Path, data_root: &Path) -> String {
    value
        .replace("${PLUGIN_ROOT}", &root.to_string_lossy())
        .replace("${PLUGIN_DATA}", &data_root.to_string_lossy())
}

/// 判断字符串是否以插件相对路径前缀开头（`./` 或 Windows 习惯的 `.\`）。
fn is_plugin_relative(raw: &str) -> bool {
    raw.starts_with("./") || raw.starts_with(".\\")
}

// ---------------------------------------------------------------------------
// 技能发现
// ---------------------------------------------------------------------------

/// 只在 `skills/` 的**直接子目录**里发现技能（§7.1：不递归）。
fn discover_skills(root: &Path, diagnostics: &mut Vec<PluginDiagnostic>) -> Vec<AgentSkill> {
    let skills_dir = root.join(AGENT_PLUGINS_SKILLS_DIR);
    let Ok(metadata) = std::fs::symlink_metadata(&skills_dir) else {
        // 固定位置缺失不是错误（§6.2）。
        return Vec::new();
    };
    let canonical_dir = match std::fs::canonicalize(&skills_dir) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(PluginDiagnostic::warning(
                "skills_unreadable",
                Some(&skills_dir),
                format!("skills 目录无法解析，跳过全部技能：{error}"),
            ));
            return Vec::new();
        }
    };
    if !path_is_within(root, &canonical_dir) {
        diagnostics.push(PluginDiagnostic::warning(
            "skills_escape",
            Some(&skills_dir),
            "skills 目录解析到包根之外，跳过该组件类型",
        ));
        return Vec::new();
    }
    if !canonical_dir.is_dir() {
        diagnostics.push(PluginDiagnostic::warning(
            "skills_not_directory",
            Some(&skills_dir),
            "skills 不是目录，跳过该组件类型",
        ));
        return Vec::new();
    }
    let _ = metadata;
    let Ok(entries) = std::fs::read_dir(&canonical_dir) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let directory = entry.path();
        // 只认直接子目录（符号链接目录允许，但解析后必须仍在包内）。
        if !directory.is_dir() {
            continue;
        }
        let skill_md = directory.join(AGENT_PLUGINS_SKILL_FILE);
        let canonical_md = match std::fs::canonicalize(&skill_md) {
            Ok(path) => path,
            Err(_) => continue, // 没有 SKILL.md 的子目录不是技能。
        };
        if !canonical_md.is_file() {
            continue;
        }
        if !path_is_within(root, &canonical_md) {
            diagnostics.push(PluginDiagnostic::warning(
                "skill_escape",
                Some(&canonical_md),
                "SKILL.md 解析到包根之外，跳过该技能",
            ));
            continue;
        }
        let content = match std::fs::read_to_string(&canonical_md) {
            Ok(content) => content,
            Err(error) => {
                diagnostics.push(PluginDiagnostic::warning(
                    "skill_unreadable",
                    Some(&canonical_md),
                    format!("SKILL.md 读取失败，跳过该技能：{error}"),
                ));
                continue;
            }
        };
        match parse_skill_frontmatter(&content) {
            Ok((name, description)) => skills.push(AgentSkill {
                name,
                description,
                dir: std::fs::canonicalize(&directory).unwrap_or(directory),
                skill_md: canonical_md,
            }),
            Err(reason) => diagnostics.push(PluginDiagnostic::warning(
                "skill_invalid",
                Some(&canonical_md),
                format!("不符合 Agent Skills 规范，跳过该技能：{reason}"),
            )),
        }
    }
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    skills
}

/// 解析 `SKILL.md` frontmatter 的 `name` / `description`（Agent Skills 规范要求）。
fn parse_skill_frontmatter(content: &str) -> Result<(String, String), &'static str> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.lines();
    let first = lines.next().ok_or("文件为空")?;
    if first.trim() != "---" {
        return Err("缺少 YAML frontmatter 起始行 `---`");
    }
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut pending_key: Option<&'static str> = None;
    let mut pending_block = String::new();
    let mut closed = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            closed = true;
            break;
        }
        // 块标量（`>` / `|`）的续行：缩进行并入上一个字段。
        if pending_key.is_some() {
            if line.starts_with(' ') || line.starts_with('\t') {
                if !pending_block.is_empty() {
                    pending_block.push(' ');
                }
                pending_block.push_str(trimmed);
                continue;
            }
            flush_block(&mut name, &mut description, pending_key, &pending_block);
            pending_key = None;
            pending_block.clear();
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            let value = rest.trim();
            if value == ">" || value == "|" || value == ">-" || value == "|-" {
                pending_key = Some("name");
            } else {
                name = Some(unquote_scalar(value));
            }
        } else if let Some(rest) = trimmed.strip_prefix("description:") {
            let value = rest.trim();
            if value == ">" || value == "|" || value == ">-" || value == "|-" {
                pending_key = Some("description");
            } else {
                description = Some(unquote_scalar(value));
            }
        }
    }
    if pending_key.is_some() {
        flush_block(&mut name, &mut description, pending_key, &pending_block);
    }
    if !closed {
        return Err("frontmatter 缺少结束行 `---`");
    }
    let name = name.ok_or("frontmatter 缺少 `name`")?;
    let description = description.ok_or("frontmatter 缺少 `description`")?;
    validate_skill_name(&name)?;
    if description.trim().is_empty() {
        return Err("`description` 为空");
    }
    if description.chars().count() > 1024 {
        return Err("`description` 超过 1024 字符");
    }
    Ok((name, description))
}

fn flush_block(
    name: &mut Option<String>,
    description: &mut Option<String>,
    key: Option<&'static str>,
    block: &str,
) {
    let block = block.trim();
    if block.is_empty() {
        return;
    }
    match key {
        Some("name") => *name = Some(block.to_string()),
        Some("description") => *description = Some(block.to_string()),
        _ => {}
    }
}

fn unquote_scalar(raw: &str) -> String {
    let raw = raw.trim();
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        if (bytes[0] == b'"' && bytes[raw.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[raw.len() - 1] == b'\'')
        {
            return raw[1..raw.len() - 1].to_string();
        }
    }
    raw.to_string()
}

/// Agent Skills：≤64 字符、小写字母/数字/连字符、不以连字符开头或结尾。
fn validate_skill_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("`name` 为空");
    }
    if name.chars().count() > 64 {
        return Err("`name` 超过 64 字符");
    }
    if !name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        return Err("`name` 只允许小写字母、数字与连字符");
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("`name` 不得以连字符开头或结尾");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// MCP 配置
// ---------------------------------------------------------------------------

/// 读取并校验 `mcp.json`，逐条映射为运行配置；非法条目只跳过自己（§7.2.2）。
fn load_mcp_servers(
    root: &Path,
    data_root: &Path,
    manifest: &AgentPluginManifest,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> Vec<AgentMcpServer> {
    let mcp_path = root.join(AGENT_PLUGINS_MCP_FILE);
    if std::fs::symlink_metadata(&mcp_path).is_err() {
        return Vec::new(); // 固定位置缺失不是错误。
    }
    let canonical = match std::fs::canonicalize(&mcp_path) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(PluginDiagnostic::warning(
                "mcp_disabled",
                Some(&mcp_path),
                format!("mcp.json 无法解析，禁用该插件的 MCP：{error}"),
            ));
            return Vec::new();
        }
    };
    if !path_is_within(root, &canonical) {
        diagnostics.push(PluginDiagnostic::warning(
            "mcp_disabled",
            Some(&mcp_path),
            "mcp.json 解析到包根之外，禁用该插件的 MCP",
        ));
        return Vec::new();
    }
    if !canonical.is_file() {
        diagnostics.push(PluginDiagnostic::warning(
            "mcp_disabled",
            Some(&mcp_path),
            "mcp.json 不是普通文件，禁用该插件的 MCP",
        ));
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&canonical) {
        Ok(content) => content,
        Err(error) => {
            diagnostics.push(PluginDiagnostic::warning(
                "mcp_disabled",
                Some(&mcp_path),
                format!("mcp.json 读取失败，禁用该插件的 MCP：{error}"),
            ));
            return Vec::new();
        }
    };
    let value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(PluginDiagnostic::warning(
                "mcp_disabled",
                Some(&mcp_path),
                format!("mcp.json 不是合法 JSON，禁用该插件的 MCP：{error}"),
            ));
            return Vec::new();
        }
    };
    let Some(object) = value.as_object() else {
        diagnostics.push(PluginDiagnostic::warning(
            "mcp_disabled",
            Some(&mcp_path),
            "mcp.json 顶层不是对象，禁用该插件的 MCP",
        ));
        return Vec::new();
    };
    // 顶层闭模式：出现规范外字段即整份配置无效（§7.2.1）。
    if let Some(key) = object
        .keys()
        .find(|key| !MCP_FIELDS.contains(&key.as_str()))
    {
        diagnostics.push(PluginDiagnostic::warning(
            "mcp_disabled",
            Some(&mcp_path),
            format!("mcp.json 含规范外顶层字段 `{key}`，禁用该插件的 MCP"),
        ));
        return Vec::new();
    }
    let Some(Value::String(mcp_schema)) = object.get("$schema") else {
        diagnostics.push(PluginDiagnostic::warning(
            "mcp_disabled",
            Some(&mcp_path),
            "mcp.json 缺少字符串 `$schema`，禁用该插件的 MCP",
        ));
        return Vec::new();
    };
    if mcp_schema != AGENT_PLUGINS_MCP_SCHEMA {
        diagnostics.push(PluginDiagnostic::warning(
            "mcp_disabled",
            Some(&mcp_path),
            format!("mcp.json 使用了不支持的 $schema `{mcp_schema}`，禁用该插件的 MCP"),
        ));
        return Vec::new();
    }
    // 两份清单必须指向同一个规范版本（此处两侧都只接受 1.0.0，天然一致；保留显式判断）。
    if manifest.schema != AGENT_PLUGINS_MANIFEST_SCHEMA || mcp_schema != AGENT_PLUGINS_MCP_SCHEMA {
        diagnostics.push(PluginDiagnostic::warning(
            "mcp_disabled",
            Some(&mcp_path),
            "mcp.json 与 plugin.json 的规范版本不一致，禁用该插件的 MCP",
        ));
        return Vec::new();
    }
    let Some(Value::Object(servers)) = object.get("mcpServers") else {
        diagnostics.push(PluginDiagnostic::warning(
            "mcp_disabled",
            Some(&mcp_path),
            "mcp.json 缺少对象 `mcpServers`，禁用该插件的 MCP",
        ));
        return Vec::new();
    };

    let mut out = Vec::new();
    for (key, server_value) in servers {
        match parse_mcp_server(key, server_value, root, data_root) {
            Ok(server) => out.push(server),
            Err(reason) => diagnostics.push(PluginDiagnostic::warning(
                "mcp_server_skipped",
                Some(&mcp_path),
                format!("服务器 `{key}` 已跳过：{reason}"),
            )),
        }
    }
    out.sort_by(|left, right| left.key.cmp(&right.key));
    out
}

fn parse_mcp_server(
    key: &str,
    value: &Value,
    root: &Path,
    data_root: &Path,
) -> Result<AgentMcpServer, String> {
    if key.trim().is_empty() {
        return Err("服务器名不能为空".to_string());
    }
    let object = value
        .as_object()
        .ok_or_else(|| "服务器配置必须是 JSON 对象".to_string())?;
    let transport = match object.get("type") {
        Some(Value::String(kind)) => kind.as_str(),
        Some(_) => return Err("`type` 必须是字符串".to_string()),
        None => return Err("缺少必填字段 `type`".to_string()),
    };
    match transport {
        "stdio" => parse_stdio_server(key, object, root, data_root),
        "streamable-http" => parse_http_server(key, object),
        // §7.2.2 第 4 条：不支持声明的传输时跳过该服务器（不是让插件失败）。
        // 本客户端的 HTTP 传输是 MCP Streamable HTTP（POST + 事件流），
        // 不是 2024-11-05 的 GET /sse 长连接，因此 sse 明确不支持。
        "sse" => Err("传输类型 `sse` 未实现（本客户端支持 stdio 与 streamable-http）".to_string()),
        other => Err(format!("未知传输类型 `{other}`")),
    }
}

fn parse_stdio_server(
    key: &str,
    object: &Map<String, Value>,
    root: &Path,
    data_root: &Path,
) -> Result<AgentMcpServer, String> {
    if let Some(unknown) = object
        .keys()
        .find(|field| !STDIO_FIELDS.contains(&field.as_str()))
    {
        return Err(format!("stdio 服务器不允许字段 `{unknown}`"));
    }
    let command = match object.get("command") {
        Some(Value::String(command)) if !command.is_empty() => command.clone(),
        Some(_) => return Err("`command` 必须是非空字符串".to_string()),
        None => return Err("缺少必填字段 `command`".to_string()),
    };
    // command 必须是单个可执行 token：裸名，或以 `./` 开头的包内相对路径。
    let command = if is_plugin_relative(&command) {
        let relative = Path::new(&command);
        let resolved = resolve_within(root, relative)
            .ok_or_else(|| format!("`command` 越出插件根：{command}"))?;
        resolved.to_string_lossy().into_owned()
    } else {
        if command.contains('/') || command.contains('\\') {
            return Err(format!(
                "`command` 必须是裸可执行名或以 `./` 开头的包内路径：{command}"
            ));
        }
        command
    };

    let arguments = match object.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            let mut arguments = Vec::with_capacity(items.len());
            for item in items {
                let Value::String(item) = item else {
                    return Err("`args` 必须是字符串数组".to_string());
                };
                arguments.push(expand_placeholders(item, root, data_root));
            }
            arguments
        }
        Some(_) => return Err("`args` 必须是字符串数组".to_string()),
    };

    let environment = match object.get("env") {
        None => BTreeMap::new(),
        Some(Value::Object(entries)) => {
            let mut environment = BTreeMap::new();
            for (name, value) in entries {
                if matches!(name.as_str(), "PLUGIN_ROOT" | "PLUGIN_DATA") {
                    return Err(format!("`env` 不得声明保留变量 `{name}`"));
                }
                let Value::String(value) = value else {
                    return Err(format!("`env.{name}` 必须是字符串"));
                };
                environment.insert(name.clone(), expand_placeholders(value, root, data_root));
            }
            environment
        }
        Some(_) => return Err("`env` 必须是字符串映射".to_string()),
    };

    let cwd = match object.get("cwd") {
        None => Some(root.to_string_lossy().into_owned()),
        Some(Value::String(raw)) => {
            let resolved = resolve_cwd(raw, root, data_root)?;
            Some(resolved.to_string_lossy().into_owned())
        }
        Some(_) => return Err("`cwd` 必须是字符串".to_string()),
    };

    Ok(AgentMcpServer {
        key: key.to_string(),
        transport: AgentMcpTransport::Stdio,
        config: McpServerConfig {
            name: key.to_string(),
            transport: "stdio".to_string(),
            command,
            args: arguments,
            url: None,
            timeout_ms: None,
            network_allowlist: Vec::new(),
            trusted_readonly: Vec::new(),
            env: environment,
            cwd,
            headers: BTreeMap::new(),
        },
    })
}

/// `cwd` 三种合法形态 + 展开 + 越界校验（§7.2.1）。
///
/// 合法形态只有：`./x`、`${PLUGIN_ROOT}` 或 `${PLUGIN_ROOT}/x`、
/// `${PLUGIN_DATA}` 或 `${PLUGIN_DATA}/x`；其它写法（如 `${PLUGIN_ROOT}foo`）判为非法。
fn resolve_cwd(raw: &str, root: &Path, data_root: &Path) -> Result<PathBuf, String> {
    let (base, relative): (&Path, PathBuf) = if is_plugin_relative(raw) {
        (root, PathBuf::from(raw))
    } else if raw == "${PLUGIN_ROOT}" {
        (root, PathBuf::new())
    } else if let Some(rest) = raw.strip_prefix("${PLUGIN_ROOT}/") {
        (root, PathBuf::from(rest))
    } else if raw == "${PLUGIN_DATA}" {
        (data_root, PathBuf::new())
    } else if let Some(rest) = raw.strip_prefix("${PLUGIN_DATA}/") {
        (data_root, PathBuf::from(rest))
    } else {
        return Err(format!(
            "`cwd` 必须形如 ./x、${{PLUGIN_ROOT}}/x 或 ${{PLUGIN_DATA}}/x：{raw}"
        ));
    };
    let resolved =
        resolve_within(base, &relative).ok_or_else(|| format!("`cwd` 越出允许的根目录：{raw}"))?;
    Ok(resolved)
}

fn parse_http_server(key: &str, object: &Map<String, Value>) -> Result<AgentMcpServer, String> {
    if let Some(unknown) = object
        .keys()
        .find(|field| !HTTP_FIELDS.contains(&field.as_str()))
    {
        return Err(format!("HTTP 服务器不允许字段 `{unknown}`"));
    }
    let url = match object.get("url") {
        Some(Value::String(url)) if !url.is_empty() => url.clone(),
        Some(_) => return Err("`url` 必须是非空字符串".to_string()),
        None => return Err("缺少必填字段 `url`".to_string()),
    };
    let host = validate_mcp_url(&url)?;
    let headers = match object.get("headers") {
        None => BTreeMap::new(),
        Some(Value::Object(entries)) => {
            let mut headers: BTreeMap<String, String> = BTreeMap::new();
            let mut seen_lowercase = std::collections::HashSet::new();
            for (name, value) in entries {
                let Value::String(value) = value else {
                    return Err(format!("`headers.{name}` 必须是字符串"));
                };
                validate_header(name, value)?;
                let lowercase = name.to_ascii_lowercase();
                if !seen_lowercase.insert(lowercase) {
                    return Err(format!("`headers` 里 `{name}` 重复（大小写不敏感）"));
                }
                headers.insert(name.clone(), value.clone());
            }
            headers
        }
        Some(_) => return Err("`headers` 必须是字符串映射".to_string()),
    };
    Ok(AgentMcpServer {
        key: key.to_string(),
        transport: AgentMcpTransport::StreamableHttp,
        config: McpServerConfig {
            name: key.to_string(),
            transport: "http".to_string(),
            command: String::new(),
            args: Vec::new(),
            url: Some(url),
            timeout_ms: None,
            // R9：把声明的 host 写进白名单，让既有 HTTP allowlist 检查有据可依。
            network_allowlist: vec![host],
            trusted_readonly: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
            headers,
        },
    })
}

/// URL 约束：绝对 http(s)、无 userinfo、无 fragment、非回环必须 HTTPS。返回 host。
pub fn validate_mcp_url(url: &str) -> Result<String, String> {
    let lowercase = url.to_ascii_lowercase();
    let (scheme, rest) = if let Some(rest) = lowercase.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = lowercase.strip_prefix("http://") {
        ("http", rest)
    } else {
        return Err(format!("`url` 必须是绝对 http(s) URL：{url}"));
    };
    if rest.is_empty() {
        return Err("`url` 缺少主机".to_string());
    }
    if rest.contains('#') {
        return Err("`url` 不得含 fragment".to_string());
    }
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() {
        return Err("`url` 缺少主机".to_string());
    }
    if authority.contains('@') {
        return Err("`url` 不得含用户信息".to_string());
    }
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        // IPv6 字面量：[::1]:8080
        let end = stripped
            .find(']')
            .ok_or_else(|| "IPv6 主机缺少 `]`".to_string())?;
        stripped[..end].to_string()
    } else {
        authority.split(':').next().unwrap_or(authority).to_string()
    };
    if host.is_empty() {
        return Err("`url` 缺少主机".to_string());
    }
    if scheme == "http" && !is_loopback_host(&host) {
        return Err(format!("非回环主机必须使用 HTTPS：{host}"));
    }
    Ok(host)
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if host == "::1" {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

fn validate_header(name: &str, value: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("`headers` 含空名称".to_string());
    }
    if !name.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            )
    }) {
        return Err(format!("非法 HTTP 头名称 `{name}`"));
    }
    if value.bytes().any(|byte| {
        byte == b'\r' || byte == b'\n' || (byte < 0x20 && byte != b'\t') || byte == 0x7f
    }) {
        return Err(format!("HTTP 头 `{name}` 的值含控制字符"));
    }
    Ok(())
}
