//! 工作区设置：`<workspace>/settings.json`（默认模型/只读/危险命令/MCP 服务器/v0.4 配置组）。

use crate::whitelist::WhitelistEntry;
use owo_agent_plugins::McpServerConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 语音输入配置（v0.4 D20）。
///
/// M14：类型已随它的域（感知内核 `owo_agent_perception::stt::LocalStt`）搬到
/// `owo-agent-perception`；这里用 `pub use` 转出，因此
/// `owo_agent_core::settings::SttSettings` 与 `Settings { stt, .. }` 的字段类型
/// 都保持不变，调用方零改动（§9.2「配置类型随域走」，M7/M11 之后的第三次应用）。
pub use owo_agent_perception::SttSettings;

/// 受限自主探索配置（v0.4 D23，默认 S0 隔离虚拟机层）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreSettings {
    #[serde(default = "default_explore_tier")]
    pub default_tier: String,
    #[serde(default = "default_action_budget")]
    pub action_budget: u32,
    #[serde(default = "default_max_duration")]
    pub max_duration_s: u64,
    #[serde(default = "default_false")]
    pub allow_s1: bool,
}

impl Default for ExploreSettings {
    fn default() -> Self {
        Self {
            default_tier: "S0".to_string(),
            action_budget: 50,
            max_duration_s: 600,
            allow_s1: false,
        }
    }
}

/// 主动建议阈值配置（v0.4 D24）。
///
/// M11：类型已随它的域（主动建议引擎 `owo_agent_memory::learn::ProactiveEngine`）
/// 搬到 `owo-agent-memory`；这里用 `pub use` 转出，因此
/// `owo_agent_core::settings::ProactiveSettings` 与 `Settings { proactive, .. }`
/// 的字段类型都保持不变，调用方零改动（§9.2「配置类型随域走」）。
pub use owo_agent_memory::ProactiveSettings;

/// 技能包分享/导入配置（v0.4 D26）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsSettings {
    #[serde(default = "default_share_format")]
    pub share_format: String,
    #[serde(default = "default_false")]
    pub require_signature: bool,
    /// 禁用的技能名列表（设置页启用/禁用，重启后从 settings.json 恢复）。
    #[serde(default)]
    pub disabled: Vec<String>,
}

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

fn default_explore_tier() -> String {
    "S0".to_string()
}

fn default_action_budget() -> u32 {
    50
}

fn default_max_duration() -> u64 {
    600
}

fn default_share_format() -> String {
    "owskill".to_string()
}

impl Default for SkillsSettings {
    fn default() -> Self {
        Self {
            share_format: "owskill".to_string(),
            require_signature: false,
            disabled: Vec::new(),
        }
    }
}

/// 数据出境开关（v0.3 7.5）：关闭后拒绝云端模型调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressSettings {
    #[serde(default = "default_true")]
    pub cloud_enabled: bool,
}

impl Default for EgressSettings {
    fn default() -> Self {
        Self {
            cloud_enabled: true,
        }
    }
}

/// v0.4.30 模型用量预算配置（持久化到 settings.json，运行时写回环境变量）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageSettings {
    /// 累计 token 上限（None = 不熔断）。
    #[serde(default)]
    pub token_budget: Option<u64>,
    /// 累计成本上限（美元，None = 不熔断）。
    #[serde(default)]
    pub cost_budget_usd: Option<f64>,
    /// 输入单价（美元/百万 token，0 = 不估算成本）。
    #[serde(default)]
    pub input_price_per_mtok: f64,
    /// 输出单价（美元/百万 token）。
    #[serde(default)]
    pub output_price_per_mtok: f64,
}

/// 模型提供商接入配置（设置页「模型」面板写入，运行时投影为环境变量）。
///
/// **存在理由**：`Settings` 此前没有 provider 段，而工作台（desktop/web）保存模型时
/// 会提交 `{"provider": {"base_url": ..., "api_key": ...}}`。serde 对未知字段**静默忽略**，
/// 于是用户填的端点与密钥从未落盘 —— 表现为「测试连接通过，但发送仍被首启门拦住」
/// （首启门读 runtime.credential_source，而它只看环境变量）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProviderSettings {
    /// OpenAI 兼容端点（如 https://api.deepseek.com/v1）。空 = 用内置默认。
    #[serde(default)]
    pub base_url: Option<String>,
    /// API 密钥。**明文只在本机 settings.json**，仓库红线要求它不得进版本库；
    /// 服务端 `settings_get` 只回 `api_key_set` 布尔，不回明文。
    #[serde(default)]
    pub api_key: Option<String>,
    /// 凭据来源的环境变量名（默认 OPENAI_API_KEY）。
    #[serde(default)]
    pub api_key_env: Option<String>,
}

impl ProviderSettings {
    /// 是否已填端点（首启门与设置页回填共用）。
    pub fn has_base_url(&self) -> bool {
        self.base_url
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    /// 是否已填密钥（明文，不用于回显）。
    pub fn has_api_key(&self) -> bool {
        self.api_key
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    /// 清除明文密钥（保存时留空密钥 = 保留旧值，故需要显式清除路径）。
    pub fn clear_api_key(&mut self) {
        self.api_key = None;
    }
}

/// 可选内置工具能力。默认关闭；改动在下一次 Daemon 启动时装配到 Agent 工具表。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentToolCapabilities {
    /// 屏幕/窗口观察、OCR 与视觉定位。
    pub desktop_observation: bool,
    /// 点击、键盘输入、快捷键、窗口激活/启动等桌面副作用。
    pub desktop_control: bool,
    /// 浏览器导航、搜索、交互与下载。
    pub browser: bool,
}

/// 十一期：团队（`/team`）统一配置——`<workspace>/settings.json` 的 `team` 段。
///
/// **一个文件统一配置 + 支持自定义**：模型、并行路数、并行度、角色编排都写在
/// 同一份设置文件里；建队请求（CLI `--model/--parallel/--role/--write`、UI 表单）
/// 的显式字段优先级更高，配置只在缺省时生效。
///
/// 示例：
/// ```json
/// {
///   "team": {
///     "model": "glm-5.3-flashx",
///     "parallel": 4,
///     "max_parallel": 4,
///     "roles": [
///       { "role": "lead", "assignee": "agent", "worker": "agent" },
///       { "role": "w1", "assignee": "agent", "worker": "agent", "depends_on": ["lead"], "write_paths": ["src/auth"] },
///       { "role": "w2", "assignee": "agent", "worker": "agent", "depends_on": ["lead"], "write_paths": ["src/order"] },
///       { "role": "leader", "assignee": "agent", "worker": "agent", "depends_on": ["w1", "w2"] }
///     ]
///   }
/// }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamSettings {
    /// 团队统一模型（None = 回退 [`Settings::model`] / 环境变量 / 服务端内置缺省）。
    #[serde(default)]
    pub model: Option<String>,
    /// 并行路数：Some(N) = 缺省启用「lead 拆解 → w1..wN 并行 → leader 汇总」。
    #[serde(default)]
    pub parallel: Option<usize>,
    /// 同一 wave 的并行度上限（None = 用 [`TeamSettings::parallel`]；运行期 1..=8 收敛）。
    #[serde(default)]
    pub max_parallel: Option<usize>,
    /// 自定义角色编排（非空 = 建队未显式给角色时使用；字段同 `RoleSpec`，
    /// 支持 `model`/`write_paths`/`depends_on`/`worker`，写范围相对工作区根）。
    #[serde(default)]
    pub roles: Vec<crate::workswarm::RoleSpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 默认模型（低于环境变量与命令行参数）。
    #[serde(default)]
    pub model: Option<String>,
    /// 模型提供商接入（端点 / 密钥）。缺失时 provider_ready 仍可由环境变量满足。
    #[serde(default)]
    pub provider: ProviderSettings,
    /// Exact connection/model declarations; None inherits host environment, Some([]) clears.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_input_capabilities: Option<Vec<owo_agent_protocol::ModelInputCapabilityBinding>>,
    /// 启动默认只读（plan）模式。
    #[serde(default)]
    pub read_only: bool,
    /// §5.3 权限档位（read_only / workspace / auto_review / full_access / custom；
    /// 缺省时由 Agent 默认为 workspace；read_only=true 等效 read_only 档）。
    #[serde(default)]
    pub permission_profile: Option<String>,
    /// §4.5.3 结构化权限配置（权限中心提交；`None` = 只按档位走）。
    ///
    /// 与 `permission_profile` 同时存在时，档位是 spec 的下界投影：
    /// `Policy::set_spec` 会把档位同步为 `nearest_profile()`，两者不会互相矛盾。
    #[serde(default)]
    pub permission_spec: Option<crate::permission_spec::PermissionSpec>,
    /// 额外危险命令片段（deny 优先）。
    #[serde(default)]
    pub deny_commands: Vec<String>,
    /// A2-1 hooks 生命周期扩展点：`hooks` 数组（event / matcher? / command；
    /// 命令经系统 shell 执行、事件 JSON 走 stdin、exit 2 = 阻断）。
    #[serde(default)]
    pub hooks: Vec<crate::hooks::HookConfig>,
    /// 按需暴露的可选 Agent 工具能力；缺字段的旧 settings.json 默认全部关闭。
    #[serde(default)]
    pub tool_capabilities: AgentToolCapabilities,
    /// 启动时自动连接的 MCP 服务器。
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    /// TUI 主题：dark / light。
    #[serde(default)]
    pub theme: Option<String>,
    /// TUI 键位：action → 按键描述（如 "tab"、"ctrl+c"、"f2"）。
    #[serde(default)]
    pub keybinds: HashMap<String, String>,
    /// v0.4 语音输入配置。
    #[serde(default)]
    pub stt: SttSettings,
    /// v0.4 自主探索配置。
    #[serde(default)]
    pub explore: ExploreSettings,
    /// v0.4 主动建议配置。
    #[serde(default)]
    pub proactive: ProactiveSettings,
    /// v0.4 技能包分享/导入配置。
    #[serde(default)]
    pub skills: SkillsSettings,
    /// v0.4 应用白名单（可被默认清单覆盖，用户增删）。
    #[serde(default)]
    pub whitelist: Vec<WhitelistEntry>,
    /// 数据出境开关。
    #[serde(default)]
    pub egress: EgressSettings,
    /// v0.4.30 模型用量预算。
    #[serde(default)]
    pub usage: UsageSettings,
    /// 推理档位（`reasoning_effort`，取优合并自远端 engine）：minimal / low / medium / high。
    /// 留空 = 不发送该参数（用模型自身默认）；只有显式选择时才写入请求体，
    /// 避免不支持该字段的 OpenAI 兼容端点直接 400。
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// §13 批次六：可选遥测开关（默认关；仅聚合功能计数/错误码分布/性能分位，
    /// 不含任何消息/提示词/输出/文件内容——数据字典经 /metrics/telemetry/status 暴露）。
    #[serde(default)]
    pub telemetry_enabled: Option<bool>,
    /// 十一期：团队（`/team`）统一配置（模型/并行/自定义角色；请求显式字段优先）。
    #[serde(default)]
    pub team: TeamSettings,
}

impl Settings {
    pub fn load(workspace: &Path) -> Self {
        let path = workspace.join("settings.json");
        std::fs::read_to_string(path)
            .ok()
            // Windows 编辑器常写 UTF-8 BOM，serde 不识别，先剥离。
            .map(|content| content.trim_start_matches('\u{feff}').to_string())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, workspace: &Path) -> Result<(), String> {
        if let Some(bindings) = &self.model_input_capabilities {
            crate::gateway::input_capabilities::validate_input_capability_bindings(bindings)?;
        }
        let path = workspace.join("settings.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(self).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    }

    /// 加密落盘（R9）：`settings.json.owo-crypt` 信封加密；settings 仍零明文密钥
    /// （api_key_ref 引用模型不变，加密的是配置整体）。非 Windows 显式失败。
    pub fn save_encrypted(&self, workspace: &Path) -> Result<(), String> {
        if let Some(bindings) = &self.model_input_capabilities {
            crate::gateway::input_capabilities::validate_input_capability_bindings(bindings)?;
        }
        let path = workspace.join("settings.json.owo-crypt");
        let content = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        crate::storage_crypto::encrypt_file_envelope(&path, &content)
            .map_err(|error| error.to_string())
    }

    /// 加密读取（R9）：优先加密文件（解密读取，损坏显式报错不静默回退），
    /// 无加密文件时回退明文 `settings.json`（兼容既有安装）。
    pub fn load_encrypted(workspace: &Path) -> Result<Self, String> {
        let encrypted = workspace.join("settings.json.owo-crypt");
        if encrypted.exists() {
            let content = crate::storage_crypto::decrypt_file_envelope(&encrypted)
                .map_err(|error| format!("settings 解密失败：{error}"))?;
            return serde_json::from_slice(&content).map_err(|error| error.to_string());
        }
        Ok(Self::load(workspace))
    }

    /// Apply host declarations only. Per-turn connections never mutate this environment.
    pub fn apply_model_input_capabilities_env(&self) {
        if let Some(bindings) = &self.model_input_capabilities {
            // Malformed persisted declarations remain fail-closed at the request boundary.
            let raw =
                crate::gateway::input_capabilities::validate_input_capability_bindings(bindings)
                    .and_then(|()| {
                        serde_json::to_string(bindings).map_err(|error| error.to_string())
                    })
                    // Explicit invalid marker: do not silently retain stale support or overflow Windows.
                    .unwrap_or_else(|_| "null".to_string());
            std::env::set_var(
                crate::gateway::input_capabilities::INPUT_CAPABILITIES_ENV,
                raw,
            );
        }
    }

    /// 把用量预算配置写回环境变量（provider 每次调用前读取，即时生效）。
    /// None 字段清除对应环境变量，避免旧值残留。
    pub fn apply_usage_env(&self) {
        match self.usage.token_budget {
            Some(value) => std::env::set_var("OWO_USAGE_TOKEN_BUDGET", value.to_string()),
            None => std::env::remove_var("OWO_USAGE_TOKEN_BUDGET"),
        }
        match self.usage.cost_budget_usd {
            Some(value) => std::env::set_var("OWO_USAGE_COST_BUDGET_USD", value.to_string()),
            None => std::env::remove_var("OWO_USAGE_COST_BUDGET_USD"),
        }
        std::env::set_var(
            "OWO_MODEL_INPUT_PRICE_PER_MTOK",
            self.usage.input_price_per_mtok.to_string(),
        );
        std::env::set_var(
            "OWO_MODEL_OUTPUT_PRICE_PER_MTOK",
            self.usage.output_price_per_mtok.to_string(),
        );
    }

    /// 把推理档位写回环境变量（provider 每次请求前读取，设置页保存后即时生效）。
    /// 仅接受 minimal/low/medium/high：其余取值（含空串）一律清除变量 = 不下发该参数。
    pub fn apply_reasoning_env(&self) {
        let normalized = self
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .filter(|value| matches!(value.as_str(), "minimal" | "low" | "medium" | "high"));
        match normalized {
            Some(value) => std::env::set_var("OWO_REASONING_EFFORT", value),
            None => std::env::remove_var("OWO_REASONING_EFFORT"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_settings_from_workspace() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-workspace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("settings.json"),
            r#"{
                "model": "deepseek-v4-flash",
                "read_only": true,
                "deny_commands": ["git push"],
                "mcp_servers": [
                    { "name": "files", "transport": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] }
                ],
                "theme": "light",
                "keybinds": { "toggle_mode": "f2" },
                "stt": { "model": "SenseVoice-Small", "hotwords": ["VSCode", "提交"] },
                "explore": { "default_tier": "S0", "action_budget": 20 },
                "proactive": { "enabled": true, "weekly_threshold": 3 },
                "skills": { "share_format": "owskill" },
                "whitelist": [
                    { "app_id": "code", "name": "VSCode", "tier": "productivity", "learn_allowed": true, "auto_ops_allowed": true }
                ],
                "usage": { "token_budget": 5000, "cost_budget_usd": 1.25, "input_price_per_mtok": 0.3, "output_price_per_mtok": 1.2 }
            }"#,
        )
        .unwrap();
        let settings = Settings::load(&workspace);
        assert_eq!(settings.model.as_deref(), Some("deepseek-v4-flash"));
        assert!(settings.read_only);
        assert_eq!(settings.deny_commands, vec!["git push"]);
        assert_eq!(settings.mcp_servers.len(), 1);
        assert_eq!(settings.mcp_servers[0].name, "files");
        assert_eq!(settings.theme.as_deref(), Some("light"));
        assert_eq!(
            settings.keybinds.get("toggle_mode").map(String::as_str),
            Some("f2")
        );
        assert_eq!(settings.stt.model, "SenseVoice-Small");
        assert_eq!(settings.stt.hotwords, vec!["VSCode", "提交"]);
        assert_eq!(settings.explore.default_tier, "S0");
        assert_eq!(settings.explore.action_budget, 20);
        assert_eq!(settings.proactive.weekly_threshold, 3);
        assert_eq!(settings.proactive.daily_threshold, 3);
        assert_eq!(settings.skills.share_format, "owskill");
        assert_eq!(settings.whitelist.len(), 1);
        assert_eq!(settings.whitelist[0].app_id, "code");
        assert_eq!(settings.usage.token_budget, Some(5000));
        assert_eq!(settings.usage.cost_budget_usd, Some(1.25));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn loads_settings_with_utf8_bom() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-bom-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("settings.json"),
            "\u{feff}{\"model\":\"bom-model\",\"usage\":{\"token_budget\":9000}}",
        )
        .unwrap();
        let settings = Settings::load(&workspace);
        assert_eq!(settings.model.as_deref(), Some("bom-model"));
        assert_eq!(settings.usage.token_budget, Some(9000));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn missing_settings_returns_defaults() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-missing-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let settings = Settings::load(&workspace);
        assert!(settings.model.is_none());
        assert!(!settings.read_only);
        assert!(settings.deny_commands.is_empty());
        assert!(settings.theme.is_none());
        assert!(settings.keybinds.is_empty());
        assert_eq!(settings.stt.model, "SenseVoice-Small");
        assert!(!settings.stt.enable_high_accuracy);
        assert_eq!(settings.explore.default_tier, "S0");
        assert_eq!(settings.explore.action_budget, 50);
        assert!(!settings.explore.allow_s1);
        assert!(settings.proactive.enabled);
        assert_eq!(settings.proactive.weekly_threshold, 5);
        assert_eq!(settings.proactive.similarity, 0.9);
        assert_eq!(settings.skills.share_format, "owskill");
        assert!(!settings.skills.require_signature);
        assert!(settings.whitelist.is_empty());
        assert!(settings.egress.cloud_enabled);
        assert_eq!(settings.tool_capabilities, AgentToolCapabilities::default());
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn save_and_load_round_trip_preserves_all_groups() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let settings = Settings {
            model: Some("deepseek-v4-flash".to_string()),
            read_only: true,
            stt: SttSettings {
                model: "Other-Model".to_string(),
                language: "zh".to_string(),
                itn: false,
                ..SttSettings::default()
            },
            proactive: ProactiveSettings {
                enabled: false,
                ..ProactiveSettings::default()
            },
            skills: SkillsSettings {
                disabled: vec!["demo".to_string()],
                ..SkillsSettings::default()
            },
            egress: EgressSettings {
                cloud_enabled: false,
            },
            usage: UsageSettings {
                token_budget: Some(100_000),
                cost_budget_usd: Some(5.0),
                input_price_per_mtok: 0.5,
                output_price_per_mtok: 2.0,
            },
            tool_capabilities: AgentToolCapabilities {
                desktop_observation: true,
                desktop_control: false,
                browser: true,
            },
            ..Settings::default()
        };
        settings.save(&workspace).unwrap();
        let loaded = Settings::load(&workspace);
        assert_eq!(loaded.model.as_deref(), Some("deepseek-v4-flash"));
        assert!(loaded.read_only);
        assert_eq!(loaded.stt.model, "Other-Model");
        assert_eq!(loaded.stt.language, "zh");
        assert!(!loaded.stt.itn);
        assert!(!loaded.proactive.enabled);
        assert_eq!(loaded.skills.disabled, vec!["demo"]);
        assert!(!loaded.egress.cloud_enabled);
        assert_eq!(loaded.usage.token_budget, Some(100_000));
        assert_eq!(loaded.usage.cost_budget_usd, Some(5.0));
        assert!((loaded.usage.input_price_per_mtok - 0.5).abs() < 1e-9);
        assert_eq!(
            loaded.tool_capabilities,
            AgentToolCapabilities {
                desktop_observation: true,
                desktop_control: false,
                browser: true,
            }
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn apply_usage_env_syncs_budget_and_prices() {
        static ENV_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
            std::sync::LazyLock::new(|| std::sync::Mutex::new(()));
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let settings = Settings {
            usage: UsageSettings {
                token_budget: Some(42_000),
                cost_budget_usd: Some(3.25),
                input_price_per_mtok: 1.0,
                output_price_per_mtok: 4.0,
            },
            ..Settings::default()
        };
        settings.apply_usage_env();
        assert_eq!(
            std::env::var("OWO_USAGE_TOKEN_BUDGET").as_deref(),
            Ok("42000")
        );
        assert_eq!(
            std::env::var("OWO_USAGE_COST_BUDGET_USD").as_deref(),
            Ok("3.25")
        );
        assert_eq!(
            std::env::var("OWO_MODEL_INPUT_PRICE_PER_MTOK").as_deref(),
            Ok("1")
        );
        assert_eq!(
            std::env::var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK").as_deref(),
            Ok("4")
        );

        // None 清除预算变量（价格始终写回）。
        Settings::default().apply_usage_env();
        assert!(std::env::var("OWO_USAGE_TOKEN_BUDGET").is_err());
        assert!(std::env::var("OWO_USAGE_COST_BUDGET_USD").is_err());
        std::env::remove_var("OWO_MODEL_INPUT_PRICE_PER_MTOK");
        std::env::remove_var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK");
    }

    /// 取优合并（远端 engine）：推理档位只接受 minimal/low/medium/high；
    /// 大小写与空白归一，非法值/缺省一律清除变量（= 不下发该参数）。
    #[test]
    fn apply_reasoning_env_only_accepts_known_levels() {
        static ENV_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
            std::sync::LazyLock::new(|| std::sync::Mutex::new(()));
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let settings = Settings {
            reasoning_effort: Some(" HIGH ".to_string()),
            ..Settings::default()
        };
        settings.apply_reasoning_env();
        assert_eq!(std::env::var("OWO_REASONING_EFFORT").as_deref(), Ok("high"));

        let invalid = Settings {
            reasoning_effort: Some("unsupported".to_string()),
            ..Settings::default()
        };
        invalid.apply_reasoning_env();
        assert!(std::env::var("OWO_REASONING_EFFORT").is_err());

        Settings::default().apply_reasoning_env();
        assert!(std::env::var("OWO_REASONING_EFFORT").is_err());
    }

    #[test]
    fn round_trips_provider_segment_from_workspace_json() {
        let workspace = std::env::temp_dir().join(format!("owo-provider-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("settings.json"),
            r#"{
                "model": "deepseek-chat",
                "provider": {
                    "base_url": "https://api.deepseek.com/v1",
                    "api_key": "sk-test-123",
                    "api_key_env": "DEEPSEEK_API_KEY"
                }
            }"#,
        )
        .unwrap();
        let settings = Settings::load(&workspace);
        assert_eq!(
            settings.provider.base_url.as_deref(),
            Some("https://api.deepseek.com/v1")
        );
        assert!(settings.provider.has_api_key());
        assert!(settings.provider.has_base_url());
        assert_eq!(
            settings.provider.api_key_env.as_deref(),
            Some("DEEPSEEK_API_KEY")
        );
        let blank = ProviderSettings {
            base_url: Some("   ".to_string()),
            api_key: Some("".to_string()),
            api_key_env: None,
        };
        assert!(!blank.has_base_url());
        assert!(!blank.has_api_key());

        // 默认值：缺 provider 段的旧 settings.json 必须能正常加载。
        std::fs::write(
            workspace.join("settings.json"),
            r#"{"model":"glm-5.3-flash"}"#,
        )
        .unwrap();
        let old = Settings::load(&workspace);
        assert!(!old.provider.has_base_url());
        assert!(!old.provider.has_api_key());
        std::fs::remove_dir_all(&workspace).ok();
    }

    #[test]
    fn provider_clear_api_key_drops_plaintext() {
        let mut provider = ProviderSettings {
            base_url: Some("https://api.deepseek.com/v1".to_string()),
            api_key: Some("sk-secret".to_string()),
            api_key_env: None,
        };
        assert!(provider.has_api_key());
        provider.clear_api_key();
        assert!(!provider.has_api_key());
        assert!(provider.has_base_url());
    }

    #[test]
    fn team_settings_load_roundtrip_and_legacy_default() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-team-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("settings.json"),
            r#"{
              "model": "glm-5.3-flash",
              "team": {
                "model": "glm-5.3-flashx",
                "parallel": 3,
                "max_parallel": 2,
                "roles": [
                  {"role": "w1", "assignee": "agent", "depends_on": ["lead"], "write_paths": ["src/a"], "model": "glm-5.3-flashx"},
                  {"role": "w2", "assignee": "agent", "depends_on": ["lead"], "write_paths": ["src/b"]}
                ]
              }
            }"#,
        ).unwrap();
        let settings = Settings::load(&workspace);
        assert_eq!(settings.team.model.as_deref(), Some("glm-5.3-flashx"));
        assert_eq!(settings.team.parallel, Some(3));
        assert_eq!(settings.team.max_parallel, Some(2));
        assert_eq!(settings.team.roles.len(), 2);
        assert_eq!(settings.team.roles[0].role, "w1");
        assert_eq!(settings.team.roles[0].write_paths, vec!["src/a"]);
        assert_eq!(
            settings.team.roles[0].model.as_deref(),
            Some("glm-5.3-flashx")
        );
        settings.save(&workspace).unwrap();
        let reloaded = Settings::load(&workspace);
        assert_eq!(reloaded.team.roles[1].role, "w2");
        assert_eq!(reloaded.team.parallel, Some(3));
        std::fs::write(
            workspace.join("settings.json"),
            r#"{"model":"glm-5.3-flash","read_only":false}"#,
        )
        .unwrap();
        let legacy = Settings::load(&workspace);
        assert!(legacy.team.model.is_none());
        assert!(legacy.team.parallel.is_none());
        assert!(legacy.team.roles.is_empty());
        let _ = std::fs::remove_dir_all(&workspace);
    }
}
