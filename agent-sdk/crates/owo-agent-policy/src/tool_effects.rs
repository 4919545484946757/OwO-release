//! §7.2 工具副作用元数据（ToolEffect）：工具声明副作用，Policy 只做统一判定。
//!
//! - 内置工具：以 builtin 注册表固化原 `level_for` 矩阵（单一事实源）；
//! - MCP 工具：按 MCP annotations（`readOnlyHint` / `destructiveHint`）推导等级；
//!   未声明 annotations 时落 `Execute`（deny-by-default）并给出统一风险提示，
//!   审批卡据此展示「该工具未声明风险信息」。
//!
//! 注册表为进程级全局：MCP 工具按服务器前缀注册/移除，重连时覆盖旧条目。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Once;
use std::sync::RwLock;

/// 工具未声明风险信息时的统一提示（审批卡直接展示给用户）。
pub const UNDECLARED_RISK_NOTE: &str = "该工具未声明风险信息（未提供 MCP annotations）";

/// 副作用等级（与 `permissions::Level` 对齐；独立定义避免策略模块反向依赖元数据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    Read,
    Write,
    Execute,
    Inject,
}

impl EffectClass {
    pub fn label(&self) -> &'static str {
        match self {
            EffectClass::Read => "read",
            EffectClass::Write => "write",
            EffectClass::Execute => "execute",
            EffectClass::Inject => "inject",
        }
    }
}

impl From<EffectClass> for crate::permissions::Level {
    fn from(class: EffectClass) -> Self {
        match class {
            EffectClass::Read => crate::permissions::Level::Read,
            EffectClass::Write => crate::permissions::Level::Write,
            EffectClass::Execute => crate::permissions::Level::Execute,
            EffectClass::Inject => crate::permissions::Level::Inject,
        }
    }
}

/// 单个工具的副作用元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEffect {
    /// 模型可见的完整工具名（MCP 工具为 `server_tool` 前缀名）。
    pub tool: String,
    pub class: EffectClass,
    /// 来源：`builtin` 或 `mcp:<server>`。
    pub source: String,
    /// 风险说明（审批卡展示）；None = 已知内置工具，未额外声明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_note: Option<String>,
    /// MCP annotations 原文（审计/诊断用；内置工具为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
    /// §5.2 宿主验证：仅当管理员按「server+tool+schema hash」显式声明可信只读时
    /// 才为 true。MCP 自报 `readOnlyHint` 不得单独视为免审批凭据。
    #[serde(default)]
    pub host_verified_readonly: bool,
}

static REGISTRY: RwLock<BTreeMap<String, ToolEffect>> = RwLock::new(BTreeMap::new());
static SEED: Once = Once::new();

/// 内置矩阵只播种一次；MCP 注册发生在 connect 之后的运行期。
fn ensure_builtin_seeded() {
    SEED.call_once(|| {
        let mut guard = REGISTRY
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (tool, class) in builtin_effects() {
            guard.insert(
                tool.to_string(),
                ToolEffect {
                    tool: tool.to_string(),
                    class,
                    source: "builtin".to_string(),
                    risk_note: None,
                    annotations: None,
                    host_verified_readonly: true,
                },
            );
        }
    });
}

fn write_effect(effect: ToolEffect) {
    REGISTRY
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(effect.tool.clone(), effect);
}

/// 注册 MCP 工具副作用：annotations 缺失/未声明只读 → `Execute` + 风险提示。
/// `host_verified_readonly` 仅当宿主按「server+tool+schema hash」显式声明可信时
/// 传 true——MCP 自报 `readOnlyHint` 不能单独把工具降到免审批（§5.2）。
/// 返回注册后的元数据（供日志与测试断言）。
pub fn register_mcp_effect(
    server_name: &str,
    tool_name: &str,
    annotations: Option<&Value>,
    host_verified_readonly: bool,
) -> ToolEffect {
    ensure_builtin_seeded();
    let full_name = crate::tool_names::mcp_tool_full_name(server_name, tool_name);
    let (class, risk_note) = classify_mcp_annotations(annotations, host_verified_readonly);
    let effect = ToolEffect {
        tool: full_name,
        class,
        source: format!("mcp:{server_name}"),
        risk_note,
        annotations: annotations.cloned(),
        host_verified_readonly,
    };
    write_effect(effect.clone());
    effect
}

/// 按 MCP 服务器前缀移除副作用条目（工具卸载/服务器下线时调用）。
pub fn remove_prefix(prefix: &str) -> usize {
    revoke_trusted_readonly_prefix(prefix);
    let mut guard = REGISTRY
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = guard.len();
    guard.retain(|name, _| !name.starts_with(prefix));
    before - guard.len()
}

/// Immutable declaration for a host-constructed builtin. Registering a foreign tool
/// with the same name does not make it a trusted builtin.
pub fn builtin_effect_for(tool: &str) -> Option<ToolEffect> {
    builtin_effects()
        .into_iter()
        .find(|(name, _)| *name == tool)
        .map(|(name, class)| ToolEffect {
            tool: name.to_string(),
            class,
            source: "builtin".to_string(),
            risk_note: None,
            annotations: None,
            host_verified_readonly: class == EffectClass::Read && name != "kill_shell",
        })
}

/// 查询单个工具的副作用元数据（自动播种内置矩阵）。
pub fn effect_for(tool: &str) -> Option<ToolEffect> {
    ensure_builtin_seeded();
    REGISTRY
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(tool)
        .cloned()
}

/// 判定工具副作用等级：注册表 → 内置矩阵 → 兜底 `Execute`（deny-by-default）。
pub fn effect_class_for(tool: &str) -> EffectClass {
    match effect_for(tool) {
        Some(effect) => effect.class,
        None => builtin_class_for(tool).unwrap_or(EffectClass::Execute),
    }
}

/// 全部已注册副作用（诊断面板/审计导出用只读快照）。
pub fn all_effects() -> Vec<ToolEffect> {
    ensure_builtin_seeded();
    REGISTRY
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .cloned()
        .collect()
}

/// §5.2 宿主可信只读声明：`{server}_{tool}` → 注册时校验的 schema hash。
/// 管理员按「server + tool + schema hash」显式声明可信只读；schema 或版本
/// 变化（hash 不一致）后自动失效（`is_trusted_readonly` 返回 false），
/// 退回升级执行询问。进程内持久（与 MCP 客户端同生命周期）。
static TRUSTED_READONLY: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// 声明某 MCP 工具为宿主验证的可信只读（并记录当时的 schema hash 指纹）。
pub fn declare_trusted_readonly(server_name: &str, tool_name: &str, schema_hash: &str) {
    let full_name = crate::tool_names::mcp_tool_full_name(server_name, tool_name);
    TRUSTED_READONLY
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(full_name, schema_hash.to_string());
}

/// schema hash 指纹：`input_schema`（压缩前完整 JSON）的 sha256 十六进制。
pub fn schema_fingerprint(input_schema: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input_schema.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 宿主是否已声明该工具可信只读，且声明时的 schema hash 与当前一致
/// （schema/版本变化 → 失效）。未声明或 hash 不一致 → false。
pub fn is_trusted_readonly(server_name: &str, tool_name: &str, schema_hash: &str) -> bool {
    let full_name = crate::tool_names::mcp_tool_full_name(server_name, tool_name);
    TRUSTED_READONLY
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&full_name)
        .map(|recorded| recorded == schema_hash)
        .unwrap_or(false)
}

/// 撤销某 MCP 服务器的可信只读声明（下线/卸载时调用；同步副作用注册清理）。
pub fn revoke_trusted_readonly_prefix(prefix: &str) {
    let mut guard = TRUSTED_READONLY
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|name, _| !name.starts_with(prefix));
}

/// 当前可信只读声明总数（诊断/测试用）。
pub fn trusted_readonly_count() -> usize {
    TRUSTED_READONLY
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len()
}

/// §5.2 按服务器配置批量声明可信只读：config 的 `trusted_readonly` 列出工具名，
/// 仅当服务器实际暴露同名工具时登记（并记录当前 schema hash 指纹）。
/// 连接注册前调用；未列出的工具不受影响。
pub fn declare_trusted_from_config(
    config: &owo_agent_plugins::McpServerConfig,
    tools: &[owo_agent_mcp::McpTool],
) -> usize {
    if config.trusted_readonly.is_empty() {
        return 0;
    }
    let mut declared = 0;
    for tool in tools {
        if config.is_trusted_readonly_tool(&tool.name) {
            declare_trusted_readonly(
                &config.name,
                &tool.name,
                &schema_fingerprint(&tool.input_schema),
            );
            declared += 1;
        }
    }
    declared
}

/// MCP annotations → 副作用等级 + 风险说明。
/// 规则（重构方案 5.2）：
/// - `destructiveHint=true` 永远覆盖 `readOnlyHint=true`；
/// - `readOnlyHint=true` 仅在 `host_verified_readonly`（宿主按 server+tool+schema
///   hash 显式声明可信）时才落到 Read；未受信的 MCP 自报只读只能提高风险说明，
///   等级仍按 Execute（deny-by-default）——annotations 是服务端提示，不是
///   宿主验证的安全证明；
/// - 未声明 annotations → Execute + 统一风险提示。
pub fn classify_mcp_annotations(
    annotations: Option<&Value>,
    host_verified_readonly: bool,
) -> (EffectClass, Option<String>) {
    match annotations {
        None => (EffectClass::Execute, Some(UNDECLARED_RISK_NOTE.to_string())),
        Some(value) => {
            let read_only = value.get("readOnlyHint").and_then(Value::as_bool);
            let destructive = value.get("destructiveHint").and_then(Value::as_bool);
            match destructive {
                Some(true) => (
                    EffectClass::Execute,
                    Some("MCP 标注 destructiveHint：可能产生不可逆副作用".to_string()),
                ),
                _ => match read_only {
                    Some(true) if host_verified_readonly => (
                        EffectClass::Read,
                        Some("宿主已验证只读（server+tool+schema hash 可信声明）".to_string()),
                    ),
                    Some(true) => (
                        EffectClass::Execute,
                        Some(
                            "MCP 声明只读但未经宿主验证，按最高风险等级（execute）处理".to_string(),
                        ),
                    ),
                    _ => (
                        EffectClass::Execute,
                        Some("MCP 未声明只读标注，按最高风险等级（execute）处理".to_string()),
                    ),
                },
            }
        }
    }
}

/// 内置工具矩阵（与原 `Policy::level_for` 完全一致；未知工具返回 None）。
pub fn builtin_class_for(tool: &str) -> Option<EffectClass> {
    let class = match tool {
        "read_file" | "list_dir" | "search_files" | "grep" | "read_artifact" | "read_image"
        | "find_symbol" | "shell_output" | "todo" | "verification_plan" | "git_status"
        | "git_diff" | "git_log" | "ask_user" => EffectClass::Read,
        "write_file" | "edit_file" | "multi_edit" | "apply_patch" => EffectClass::Write,
        // kill_shell 终止进程（不可逆副作用），不得归为 Read：只读档在旧矩阵里会把
        // 它当宿主验证只读自动放行（审查记录的低危项）。
        "run_command" | "kill_shell" | "web_fetch" | "web_search" => EffectClass::Execute,
        "text.inject" | "clipboard" => EffectClass::Inject,
        "screen_ocr"
        | "desktop_window_ocr"
        | "ocr_region"
        | "desktop_foreground"
        | "desktop_window_list"
        | "desktop_wait"
        | "desktop_wait_until"
        | "browser_snapshot"
        | "screen_vision"
        | "vision_verify"
        | "vision_ground" => EffectClass::Read,
        "desktop_click" | "desktop_type" | "desktop_key" | "desktop_shortcut"
        | "desktop_activate" | "desktop_launch" | "desktop_scroll" => EffectClass::Inject,
        "browser_navigate" | "browser_search" | "browser_click" | "browser_type"
        | "browser_press" | "browser_close" => EffectClass::Execute,
        "browser_screenshot" | "browser_download_image" => EffectClass::Write,
        _ => return None,
    };
    Some(class)
}

fn builtin_effects() -> Vec<(&'static str, EffectClass)> {
    [
        ("read_file", EffectClass::Read),
        ("list_dir", EffectClass::Read),
        ("search_files", EffectClass::Read),
        ("grep", EffectClass::Read),
        ("read_artifact", EffectClass::Read),
        ("read_image", EffectClass::Read),
        ("find_symbol", EffectClass::Read),
        ("shell_output", EffectClass::Read),
        ("kill_shell", EffectClass::Execute),
        ("todo", EffectClass::Read),
        ("verification_plan", EffectClass::Read),
        ("edit_file", EffectClass::Write),
        ("multi_edit", EffectClass::Write),
        ("apply_patch", EffectClass::Write),
        ("git_status", EffectClass::Read),
        ("git_diff", EffectClass::Read),
        ("git_log", EffectClass::Read),
        ("ask_user", EffectClass::Read),
        ("web_fetch", EffectClass::Execute),
        ("web_search", EffectClass::Execute),
        ("screen_ocr", EffectClass::Read),
        ("desktop_window_ocr", EffectClass::Read),
        ("ocr_region", EffectClass::Read),
        ("desktop_foreground", EffectClass::Read),
        ("desktop_window_list", EffectClass::Read),
        ("desktop_wait", EffectClass::Read),
        ("desktop_wait_until", EffectClass::Read),
        ("browser_snapshot", EffectClass::Read),
        ("screen_vision", EffectClass::Read),
        ("vision_verify", EffectClass::Read),
        ("vision_ground", EffectClass::Read),
        ("write_file", EffectClass::Write),
        ("browser_screenshot", EffectClass::Write),
        ("browser_download_image", EffectClass::Write),
        ("run_command", EffectClass::Execute),
        ("browser_navigate", EffectClass::Execute),
        ("browser_search", EffectClass::Execute),
        ("browser_click", EffectClass::Execute),
        ("browser_type", EffectClass::Execute),
        ("browser_press", EffectClass::Execute),
        ("browser_close", EffectClass::Execute),
        ("text.inject", EffectClass::Inject),
        ("clipboard", EffectClass::Inject),
        ("desktop_click", EffectClass::Inject),
        ("desktop_type", EffectClass::Inject),
        ("desktop_key", EffectClass::Inject),
        ("desktop_shortcut", EffectClass::Inject),
        ("desktop_activate", EffectClass::Inject),
        ("desktop_launch", EffectClass::Inject),
        ("desktop_scroll", EffectClass::Inject),
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builtin_matrix_is_seeded_and_classified() {
        assert_eq!(effect_class_for("read_file"), EffectClass::Read);
        assert_eq!(effect_class_for("write_file"), EffectClass::Write);
        assert_eq!(effect_class_for("run_command"), EffectClass::Execute);
        assert_eq!(effect_class_for("desktop_click"), EffectClass::Inject);
        assert_eq!(effect_class_for("browser_snapshot"), EffectClass::Read);
        assert_eq!(effect_class_for("text.inject"), EffectClass::Inject);
        assert_eq!(effect_class_for("grep"), EffectClass::Read);
        assert_eq!(effect_class_for("read_artifact"), EffectClass::Read);
        assert_eq!(effect_class_for("apply_patch"), EffectClass::Write);
        assert_eq!(effect_class_for("multi_edit"), EffectClass::Write);
        assert_eq!(effect_class_for("git_status"), EffectClass::Read);
        assert_eq!(effect_class_for("git_diff"), EffectClass::Read);
        assert_eq!(effect_class_for("git_log"), EffectClass::Read);
        assert_eq!(effect_class_for("ask_user"), EffectClass::Read);
        assert_eq!(effect_class_for("verification_plan"), EffectClass::Read);
        assert_eq!(effect_class_for("web_fetch"), EffectClass::Execute);
        let builtin = all_effects()
            .into_iter()
            .filter(|effect| effect.source == "builtin")
            .count();
        assert_eq!(
            builtin, 50,
            "内置矩阵应完整播种 50 个工具（含 multi_edit / read_artifact / find_symbol / git 只读 / ask_user / verification_plan）"
        );
    }

    #[test]
    fn unknown_tool_falls_back_to_execute() {
        assert_eq!(
            effect_class_for("definitely_not_a_tool_owo"),
            EffectClass::Execute,
            "未知工具必须按最高风险等级兜底"
        );
    }

    #[test]
    fn mcp_annotations_read_only_hint_requires_host_verification() {
        // 重构方案 5.2：未受信的 MCP 自报只读不能降到免审批。
        let (class, note) = classify_mcp_annotations(
            Some(&json!({
                "title": "查天气",
                "readOnlyHint": true
            })),
            false,
        );
        assert_eq!(
            class,
            EffectClass::Execute,
            "readOnlyHint 未经宿主验证不得降级为 Read"
        );
        assert!(note.unwrap().contains("未经宿主验证"));

        // 宿主按 server+tool+schema hash 声明可信后才落到 Read。
        let (class, note) = classify_mcp_annotations(
            Some(&json!({
                "title": "查天气",
                "readOnlyHint": true
            })),
            true,
        );
        assert_eq!(class, EffectClass::Read);
        assert!(note.unwrap().contains("宿主已验证"));
    }

    #[test]
    fn mcp_destructive_hint_maps_to_execute_with_note() {
        for host_verified in [false, true] {
            let (class, note) = classify_mcp_annotations(
                Some(&json!({
                    "readOnlyHint": false,
                    "destructiveHint": true
                })),
                host_verified,
            );
            assert_eq!(
                class,
                EffectClass::Execute,
                "destructiveHint 恒为 Execute，不受宿主声明影响"
            );
            assert!(note.unwrap().contains("destructiveHint"));
        }
    }

    #[test]
    fn mcp_destructive_hint_overrides_read_only_hint() {
        // 重构方案 5.2：destructiveHint=true 永远覆盖 readOnlyHint=true。
        for host_verified in [false, true] {
            let (class, _) = classify_mcp_annotations(
                Some(&json!({
                    "readOnlyHint": true,
                    "destructiveHint": true
                })),
                host_verified,
            );
            assert_eq!(
                class,
                EffectClass::Execute,
                "同时声明时破坏性标注优先，不得降级为 Read"
            );
        }
    }

    #[test]
    fn mcp_without_annotations_gets_undeclared_risk_note() {
        let (class, note) = classify_mcp_annotations(None, false);
        assert_eq!(class, EffectClass::Execute);
        assert_eq!(note.as_deref(), Some(UNDECLARED_RISK_NOTE));
    }

    #[test]
    fn trusted_readonly_declaration_gates_read_hint() {
        let server = format!("tst-trust-{}", uuid::Uuid::new_v4().simple());
        let tool = "lookup";
        let schema = json!({ "type": "object", "properties": { "q": { "type": "string" } } });
        let hash = schema_fingerprint(&schema);
        assert!(!is_trusted_readonly(&server, tool, &hash), "未声明前不可信");

        // 声明后：同 hash 可信。注册时 readOnlyHint 才能落到 Read。
        declare_trusted_readonly(&server, tool, &hash);
        assert!(is_trusted_readonly(&server, tool, &hash));
        let effect = register_mcp_effect(
            &server,
            tool,
            Some(&json!({ "readOnlyHint": true })),
            is_trusted_readonly(&server, tool, &hash),
        );
        assert_eq!(effect.class, EffectClass::Read);
        assert!(effect.host_verified_readonly);
        assert_eq!(
            effect_for(&crate::tool_names::mcp_tool_full_name(&server, tool))
                .unwrap()
                .class,
            EffectClass::Read
        );

        // schema 变化（hash 不一致）→ 声明失效，退回 Execute。
        let changed = json!({ "type": "object", "properties": { "q": { "type": "string" }, "n": { "type": "integer" } } });
        let changed_hash = schema_fingerprint(&changed);
        assert_ne!(hash, changed_hash);
        assert!(!is_trusted_readonly(&server, tool, &changed_hash));
        let effect = register_mcp_effect(
            &server,
            tool,
            Some(&json!({ "readOnlyHint": true })),
            is_trusted_readonly(&server, tool, &changed_hash),
        );
        assert_eq!(
            effect.class,
            EffectClass::Execute,
            "schema 变化后可信失效，不得再降级 Read"
        );
        assert!(!effect.host_verified_readonly);

        // 撤销声明（前缀）：count 归零。
        revoke_trusted_readonly_prefix(&crate::tool_names::mcp_tool_prefix(&server));
        assert_eq!(trusted_readonly_count(), 0);
        assert!(!is_trusted_readonly(&server, tool, &hash));
    }

    #[test]
    fn register_and_remove_prefix_roundtrip() {
        let server = format!("tst-{}", uuid::Uuid::new_v4().simple());
        let effect = register_mcp_effect(
            &server,
            "query",
            Some(&json!({ "readOnlyHint": true })),
            false,
        );
        // sanitize_tool_name 保留连字符（模型 API 允许 `-`），前缀名与注册名同源。
        let full = crate::tool_names::mcp_tool_full_name(&server, "query");
        assert_eq!(effect.tool, full);
        assert_eq!(effect.class, EffectClass::Execute);
        assert_eq!(effect.source, format!("mcp:{server}"));
        assert!(!effect.host_verified_readonly);
        assert!(effect_for(&full).is_some());
        let removed = remove_prefix(&crate::tool_names::mcp_tool_prefix(&server));
        assert_eq!(removed, 1);
        assert!(effect_for(&full).is_none());
        assert_eq!(trusted_readonly_count(), 0);
    }

    #[test]
    fn re_register_overwrites_previous_class() {
        let server = format!("tst-{}", uuid::Uuid::new_v4().simple());
        let tool = "flip";
        let first = register_mcp_effect(&server, tool, None, false);
        assert_eq!(first.class, EffectClass::Execute);
        // 无宿主验证：readOnlyHint 也保持 Execute（覆盖旧类）。
        let second =
            register_mcp_effect(&server, tool, Some(&json!({ "readOnlyHint": true })), false);
        assert_eq!(second.class, EffectClass::Execute);
        let full = crate::tool_names::mcp_tool_full_name(&server, tool);
        assert_eq!(effect_for(&full).unwrap().class, EffectClass::Execute);
        remove_prefix(&crate::tool_names::mcp_tool_prefix(&server));
    }

    #[test]
    fn effect_class_converts_into_permission_level() {
        let levels: Vec<crate::permissions::Level> = [
            EffectClass::Read,
            EffectClass::Write,
            EffectClass::Execute,
            EffectClass::Inject,
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        assert_eq!(
            levels,
            vec![
                crate::permissions::Level::Read,
                crate::permissions::Level::Write,
                crate::permissions::Level::Execute,
                crate::permissions::Level::Inject,
            ]
        );
    }
}
