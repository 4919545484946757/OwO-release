//! OwO Agent 插件内核（M7）。
//!
//! ## 内容
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`plugin`] | 插件清单、签名校验、风险扫描（含 zip-slip 变体拦截）、安装/启用/回滚生命周期、插件状态存储、市场目录与版本解析、插件入口与 MCP 服务器的沙箱门卫授权 |
//! | `McpServerConfig` | 插件清单里的 `mcp` 字段类型（从 core 的 `mcp` 下沉而来，见下） |
//!
//! ## 本步的依赖倒置方向：配置类型**随插件走**，而不是塞进内核
//!
//! 迁移前 `plugin` 的唯一出边是 `crate::mcp::McpServerConfig`；`mcp`（MCP 运行时客户端）
//! 留在 core，因此这条边必须倒置。但倒置方向不是「把类型下沉内核」，理由很直接：
//!
//! > `McpServerConfig` 是**插件清单里的一个字段**
//! > （`PluginManifest.mcp: Option<McpServerConfig>`），由插件 manifest 解析而来。
//! > 它本来就属于插件域，不属于内核。
//!
//! 所以正确做法是让它随插件一起外迁，再让 core 的 `mcp.rs` 反向引用它。
//! 实测 `mcp.rs` 对 `plugin` **没有**反向依赖，倒置后不成环。
//!
//! 这与 ADR-001 里被否决的「把 `SandboxAuditEvent` 下沉内核」形成对照：
//! 判断依据不是「谁更好拿」，而是**这个类型在概念上属于谁**。
//!
//! ## 依赖方向
//!
//! ```text
//! owo-agent-core ──► owo-agent-plugins ──► owo-agent-tool-safety ──► owo-agent-kernel
//! ```
//!
//! 本 crate 不依赖 core / server / ONNX / Sherpa。

pub mod agent_plugins;
pub mod plugin;

pub use agent_plugins::{
    collect_agent_plugin_code, is_agent_plugin_dir, load_agent_plugin, scan_agent_plugin_for_risks,
    validate_mcp_url, validate_plugin_name, AgentMcpServer, AgentMcpTransport, AgentPluginAuthor,
    AgentPluginError, AgentPluginHook, AgentPluginManifest, AgentSkill, DiagnosticLevel,
    LoadedAgentPlugin, PluginDiagnostic, AGENT_PLUGINS_MANIFEST_SCHEMA, AGENT_PLUGINS_MCP_SCHEMA,
    AGENT_PLUGINS_SPEC_VERSION, MAX_PLUGIN_HOOKS, OWO_EXTENSION_NAMESPACE, PLUGIN_HOOK_EVENTS,
};
pub use plugin::{
    agent_plugin_to_manifest, discover_plugins, discover_plugins_with_report, plugin_mcp_config,
    plugin_mcp_configs, scan_plugin_for_risks, verify_plugin_signature, MarketPluginEntry,
    MarketUpdateManifest, PluginInstallReport, PluginInstallState, PluginManager, PluginManifest,
    PluginReviewState, PluginSignature, PluginStateStore, PluginSubmission, VersionsJson,
};

// ---------------------------------------------------------------------------
// 从 core 的 `mcp` 迁入：插件清单里的 MCP 服务器配置。
// 保留在同一模块路径下由 core 反向引用（core 的 mcp.rs 用 `use owo_agent_plugins::McpServerConfig;`）。
// ---------------------------------------------------------------------------

fn default_transport() -> String {
    "stdio".to_string()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    /// "stdio" 或 "http"
    #[serde(default = "default_transport")]
    pub transport: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// HTTP 传输时的端点 URL
    #[serde(default)]
    pub url: Option<String>,
    /// stdio 单次请求超时（毫秒）；未配置时读 OWO_MCP_STDIO_TIMEOUT_MS，再默认 15s。
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// 网络白名单（R9：HTTP 传输静态扫描 allowlist；非空时 URL host 必须命中，
    /// 空 = 不校验，兼容既有配置）。
    #[serde(default)]
    pub network_allowlist: Vec<String>,
    /// §5.2 宿主可信只读声明：管理员显式列出的「工具名」清单。连接注册时按
    /// `server + tool + 当前 schema hash` 校验；schema/版本变化后自动失效
    /// （hash 不一致即退回 Execute 询问）。空 = 无任何工具获可信只读。
    #[serde(default)]
    pub trusted_readonly: Vec<String>,
    /// stdio：子进程环境变量（Agent Plugins 1.0.0 §7.2.1）。插件把凭据放在这里
    /// 而不是 args，避免出现在进程命令行与审计明文里。空 = 继承宿主环境。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env: std::collections::BTreeMap<String, String>,
    /// stdio：子进程工作目录（绝对路径；相对路径按宿主当前目录解析）。
    /// Agent Plugins 包加载后此处固定写成解析好的插件根目录（§7.2.1 缺省 cwd）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// HTTP：连接时附加的固定请求头（Agent Plugins 1.0.0 §7.2.1）。
    /// 仅随请求发送到配置的 origin；不得含凭据（规范把凭据留给客户端管理）。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub headers: std::collections::BTreeMap<String, String>,
}

impl McpServerConfig {
    pub fn stdio(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            transport: "stdio".to_string(),
            command: command.into(),
            args,
            url: None,
            timeout_ms: None,
            network_allowlist: Vec::new(),
            trusted_readonly: Vec::new(),
            env: std::collections::BTreeMap::new(),
            cwd: None,
            headers: std::collections::BTreeMap::new(),
        }
    }

    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: "http".to_string(),
            command: String::new(),
            args: Vec::new(),
            url: Some(url.into()),
            timeout_ms: None,
            network_allowlist: Vec::new(),
            trusted_readonly: Vec::new(),
            env: std::collections::BTreeMap::new(),
            cwd: None,
            headers: std::collections::BTreeMap::new(),
        }
    }

    /// 是否把该工具声明为宿主可信只读（按工具名）。
    pub fn is_trusted_readonly_tool(&self, tool_name: &str) -> bool {
        self.trusted_readonly.iter().any(|name| name == tool_name)
    }
}
