//! Agent 工具定义与注册表。
//!
//! 原始句柄查找与直接执行仅供 core 内部运行时使用；下游 crate 必须通过
//! Agent 的受策略控制回合入口，不得自行取出工具句柄运行。
//!
//! ```compile_fail
//! let _ = owo_agent_core::tools::ToolRegistry::get;
//! ```
//!
//! ```compile_fail
//! let _ = owo_agent_core::tools::ToolRegistry::execute;
//! ```

use crate::audit::AuditLog;
#[cfg(test)]
use crate::external_tools;
use crate::mcp::{McpClient, McpPrompt, McpResource, McpTool};
use crate::permissions::Policy;
#[cfg(test)]
use crate::permissions::{Decision, PermissionRequest};
use crate::session::Session;
use crate::skill::SkillRegistry;
use crate::subagent::{FanOutRunner, SubagentRunner};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::RwLock;
use std::sync::{Arc, Mutex};

#[path = "tools/command.rs"]
mod command_tools;
#[path = "tools/mcp_adapters.rs"]
mod mcp_adapters;
use command_tools::{KillShellTool, RunCommandTool, ShellOutputTool};
use mcp_adapters::{McpPromptAdapter, McpResourceAdapter, McpToolAdapter};

#[path = "tools/web.rs"]
mod web;
#[cfg(test)]
use web::{decode_search_href, html_to_text};
use web::{WebFetchTool, WebSearchTool};

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// §5.1 工具副作用元数据：注册表导出的 spec 携带 effect（唯一事实源）。
    /// 内置工具由 registry 填充；MCP 工具在注册时内嵌（断开随实例消失，
    /// 不再依赖进程级全局名字查询）。None = 未登记（approval 按高风险处理）。
    pub effect: Option<crate::tool_effects::ToolEffect>,
}

impl ToolSpec {
    /// 附带 effect 的构造（MCP 注册与 registry 导出填充用）。
    pub fn with_effect(
        name: impl Into<String>,
        description: String,
        input_schema: Value,
        effect: Option<crate::tool_effects::ToolEffect>,
    ) -> Self {
        Self {
            name: name.into(),
            description,
            input_schema,
            effect,
        }
    }
}

pub struct ToolContext<'a> {
    pub workspace: &'a Path,
    pub policy: &'a Policy,
    pub session: &'a mut Session,
    pub audit: &'a Arc<Mutex<AuditLog>>,
    pub subagent: Option<SubagentRunner<'a>>,
    pub skills: &'a SkillRegistry,
    /// 窗口元素注册表（感知多源融合的稳定元素 ID 空间）。
    pub elements: &'a Arc<Mutex<crate::ElementRegistry>>,
    /// A5-1：fan-out 只读子代理的注入通道（owned；子代理内/无通道场景为 None）。
    pub fanout: Option<FanOutRunner>,
    /// 回合取消标志：工具层桥接（fan-out 停止调度新子任务并 abort 在飞者）。
    pub abort: Option<&'a AtomicBool>,
    /// 用户提问通道（ask_user 工具）：None 表示当前环境没有 UI 通道（CLI/子代理），
    /// 工具会明确报错并提示模型改为书面提问。
    pub questioner: Option<&'a dyn crate::question::Questioner>,
}

/// Stable tool contract shared by the registry, Agent loop, and domain modules.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String>;
}

#[path = "tools/tool_host.rs"]
mod tool_host;
pub(crate) use tool_host::{ToolApprovalGrant, ToolCapabilityContext, ToolHostService};

/// Metadata stays attached to the actual builtin instance through lookup and execution.
struct DeclaredBuiltin<T> {
    inner: T,
    effect: Option<crate::tool_effects::ToolEffect>,
}
#[async_trait]
impl<T: Tool> Tool for DeclaredBuiltin<T> {
    fn spec(&self) -> ToolSpec {
        let mut spec = self.inner.spec();
        if spec.effect.is_none() {
            spec.effect = self.effect.clone();
        }
        spec
    }
    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        self.inner.run(ctx, args).await
    }
}

pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
    /// MCP 大 schema 的完整副本（注册时超预算被压缩为骨架；此处保留原始 schema 供按需查询）。
    full_schemas: HashMap<String, Value>,
}

impl ToolRegistry {
    /// 默认最小工具集：文件读写/搜索、受控命令与委派。桌面、视觉、浏览器能力
    /// 必须由工作区设置或显式调用方按场景加入，避免每个 Agent 默认暴露完整工具面。
    pub fn new() -> Self {
        let mut registry = Self::empty();
        // 保留历史基础工具顺序，避免不相关的模型提示变化。
        registry.register_builtin(ReadFileTool);
        registry.register_builtin(WriteFileTool);
        registry.register_builtin(EditFileTool);
        registry.register_builtin(MultiEditTool);
        registry.register_builtin(ApplyPatchTool);
        registry.register_builtin(ListDirTool);
        registry.register_builtin(SearchFilesTool);
        registry.register_builtin(GrepTool);
        // 超大工具结果落盘后的 ref 必须有工具可取回（否则 artifact 指针是死路）。
        registry.register_builtin(ReadArtifactTool);
        registry.register_builtin(FindSymbolTool);
        registry.register_builtin(crate::git_tools::GitStatusTool);
        registry.register_builtin(crate::git_tools::GitDiffTool);
        registry.register_builtin(crate::git_tools::GitLogTool);
        registry.register_run_command();
        registry.register_builtin(ShellOutputTool);
        registry.register_builtin(KillShellTool);
        registry.register_builtin(TodoTool);
        registry.register_builtin(GoalRequestTool);
        registry.register_builtin(SingleVerificationPlanTool);
        registry.register_builtin(WebFetchTool);
        registry.register_builtin(WebSearchTool);
        registry.register_builtin(ReadImageTool);
        registry.register_delegation_tools();
        registry
    }

    /// 显式构造所有内置能力；主要用于兼容性/全能力集成测试，不作为生产默认值。
    pub fn with_all_builtin_capabilities() -> Self {
        let mut registry = Self::new();
        registry.register_desktop_observation_tools();
        registry.register_desktop_control_tools();
        registry.register_browser_tools();
        registry
    }

    /// 空注册表：产品评测等需要最小受控工具集的场景，由调用方自行注册工具。
    pub fn empty() -> Self {
        Self {
            tools: Vec::new(),
            full_schemas: HashMap::new(),
        }
    }

    /// 只读工具表（子代理 explore 使用）：不含写/执行/委派工具。
    pub fn read_only() -> Self {
        let mut registry = Self {
            tools: Vec::new(),
            full_schemas: HashMap::new(),
        };
        registry.register_builtin(ReadFileTool);
        registry.register_builtin(ListDirTool);
        registry.register_builtin(SearchFilesTool);
        registry.register_builtin(GrepTool);
        registry.register_builtin(ReadArtifactTool);
        registry.register_builtin(FindSymbolTool);
        registry.register_builtin(crate::git_tools::GitStatusTool);
        registry.register_builtin(crate::git_tools::GitDiffTool);
        registry.register_builtin(crate::git_tools::GitLogTool);
        registry.register_builtin(ReadImageTool);
        registry
    }

    // -------------------------------------------------------------------
    // 按角色分组注册（七期 · 二路）：角色画像按「组」装配工具面，
    // 注册表只含该角色允许的工具——权限在工具面层生效，而非仅靠审批拒绝。
    // -------------------------------------------------------------------

    /// 文件读取组：`read_file` / `list_dir` / `search_files`（只读角色基线）。
    pub fn register_file_read_tools(&mut self) {
        self.register_builtin(ReadFileTool);
        self.register_builtin(ListDirTool);
        self.register_builtin(SearchFilesTool);
    }

    /// 文件写入（无白名单限制；受 `Policy` 审批约束）。
    pub fn register_write_file(&mut self) {
        self.register_builtin(WriteFileTool);
    }

    /// 白名单受限写入：写目标必须落在 `allowed` 绝对路径前缀内（见
    /// [`WhitelistWriteFileTool`]）；`allowed` 为空 = 工作区内可写。
    pub fn register_whitelist_write_file(&mut self, allowed: Vec<PathBuf>) {
        self.register_builtin(WhitelistWriteFileTool { allowed });
    }

    /// 白名单受限精确补丁：每个补丁目标都必须落入同一授权路径前缀内。
    pub fn register_whitelist_apply_patch(&mut self, allowed: Vec<PathBuf>) {
        self.register_builtin(WhitelistApplyPatchTool { allowed });
    }

    /// 受控命令执行：`run_command`（实现族角色专用；沙箱 + 审批约束不变）。
    pub fn register_run_command(&mut self) {
        self.register_builtin(RunCommandTool);
    }

    /// 桌面观察/视觉组：截图 OCR、窗口信息与视觉 grounding/verify，不包含输入操作。
    pub fn register_desktop_observation_tools(&mut self) {
        self.register_builtin(crate::computer_use::ScreenOcrTool);
        self.register_builtin(crate::computer_use::OcrRegionTool);
        self.register_builtin(crate::computer_use::DesktopWindowOcrTool);
        self.register_builtin(crate::computer_use::DesktopForegroundTool);
        self.register_builtin(crate::computer_use::DesktopWindowListTool);
        self.register_builtin(crate::computer_use::ScreenVisionTool);
        self.register_builtin(crate::computer_use::VisionVerifyTool);
        self.register_builtin(crate::computer_use::VisionGroundTool);
    }

    /// 桌面控制组：会激活窗口、注入键鼠或启动程序，默认不注册。
    pub fn register_desktop_control_tools(&mut self) {
        self.register_builtin(crate::computer_use::DesktopActivateTool);
        self.register_builtin(crate::computer_use::DesktopClickTool);
        self.register_builtin(crate::computer_use::DesktopTypeTool);
        self.register_builtin(crate::computer_use::DesktopKeyTool);
        self.register_builtin(crate::computer_use::DesktopShortcutTool);
        self.register_builtin(crate::computer_use::DesktopLaunchTool);
        self.register_builtin(crate::computer_use::DesktopScrollTool);
        self.register_builtin(crate::computer_use::DesktopWaitTool);
        self.register_builtin(crate::computer_use::DesktopWaitUntilTool);
    }

    /// 委派组：`explore` / `subagent` / `use_skill`。
    pub fn register_delegation_tools(&mut self) {
        self.register_builtin(ExploreTool);
        self.register_builtin(SubagentTool);
        self.register_builtin(FanOutSubagentsTool);
        self.register_builtin(AskUserTool);
        self.register_builtin(UseSkillTool);
    }

    /// 浏览器组：导航/搜索/快照 + 交互与写工作区变体（含 `browser_screenshot` /
    /// `browser_download_image` 写文件、`browser_close`）。只读角色的可见面
    /// 应经 `retain_names` 裁掉写工作区变体。
    pub fn register_browser_tools(&mut self) {
        let browser = crate::computer_use::BrowserTools::new();
        self.register_builtin(crate::computer_use::BrowserNavigateTool {
            tools: browser.clone(),
        });
        self.register_builtin(crate::computer_use::BrowserSearchTool {
            tools: browser.clone(),
        });
        self.register_builtin(crate::computer_use::BrowserSnapshotTool {
            tools: browser.clone(),
        });
        self.register_builtin(crate::computer_use::BrowserClickTool {
            tools: browser.clone(),
        });
        self.register_builtin(crate::computer_use::BrowserTypeTool {
            tools: browser.clone(),
        });
        self.register_builtin(crate::computer_use::BrowserPressTool {
            tools: browser.clone(),
        });
        self.register_builtin(crate::computer_use::BrowserScreenshotWriteTool {
            tools: browser.clone(),
        });
        self.register_builtin(crate::computer_use::BrowserDownloadImageWriteTool {
            tools: browser.clone(),
        });
        self.register_builtin(crate::computer_use::BrowserCloseTool { tools: browser });
    }

    fn register_builtin(&mut self, tool: impl Tool + 'static) {
        let effect = crate::tool_effects::builtin_effect_for(&tool.spec().name);
        self.register(DeclaredBuiltin {
            inner: tool,
            effect,
        });
    }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.register_arc(Arc::new(tool));
    }

    /// 注册由宿主构造的共享工具实例；同名时替换原实例，保证模型清单与
    /// 按名称查找的执行器始终指向同一个定义（尤其是 MCP 热重连）。
    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.spec().name;
        if let Some(index) = self
            .tools
            .iter()
            .position(|registered| registered.spec().name == name)
        {
            self.tools[index] = tool;
        } else {
            self.tools.push(tool);
        }
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|tool| tool.spec()).collect()
    }

    /// 按前缀撤销工具（插件热卸载：`owo_plugin_<id>_` 前缀）。
    /// 返回被移除的工具数。
    pub fn remove_prefix(&mut self, prefix: &str) -> usize {
        self.remove_prefix_inner(prefix)
    }

    /// 按名单保留工具（角色画像可见工具面，七期 · 二路）：名单为空 = 全部保留
    /// （未声明可见面时按注册顺序全量可用）；返回被移除的工具数。
    /// MCP 完整 schema 副本随名单同步清理。
    pub fn retain_names(&mut self, names: &[String]) -> usize {
        if names.is_empty() {
            return 0;
        }
        let before = self.tools.len();
        self.tools
            .retain(|tool| names.iter().any(|name| tool.spec().name == *name));
        self.full_schemas
            .retain(|name, _| names.iter().any(|keep| keep == name));
        before - self.tools.len()
    }

    /// 取工具句柄（Arc 克隆，锁外可跨 await 执行）。
    pub(crate) fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|tool| tool.spec().name == name)
            .cloned()
    }

    /// 把 MCP 服务器暴露的工具注册为 Agent 工具（命名：`{server}_{tool}`）。
    ///
    /// 延迟加载（M2）：单工具 schema 超过 `schema_budget_bytes` 时，从模型可见
    /// schema 中只剥离说明性元数据（description/examples/default/title/$schema），
    /// 保留 properties、items、required、enum、additionalProperties 等完整约束。
    /// 完整原始 schema 仍保存在 `full_schemas` 供 `full_schema()` 查询。安全压缩后
    /// 若仍超过预算，优先保证模型获得可执行的参数契约，不再删除校验语义。
    pub fn register_mcp_tools(
        &mut self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<McpClient>>,
        tools: Vec<McpTool>,
    ) {
        self.register_mcp_tools_with_health(server_name, client, tools, None);
    }

    /// §10：带 per-server 健康跟踪（熔断/限流/幂等重试）的 MCP 工具注册。
    pub fn register_mcp_tools_with_health(
        &mut self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<McpClient>>,
        tools: Vec<McpTool>,
        health: Option<Arc<crate::mcp_health::McpHealthTracker>>,
    ) {
        let budget = schema_budget_bytes();
        for tool in tools {
            let full_name = mcp_tool_full_name(server_name, &tool.name);
            let (input_schema, full_schema) = if schema_bytes(&tool.input_schema) > budget {
                let full = tool.input_schema.clone();
                (compact_schema(&tool.input_schema), Some(full))
            } else {
                (tool.input_schema.clone(), None)
            };
            let mut description = tool.description;
            if full_schema.is_some() {
                description.push_str("（schema 已压缩，完整参数见 /mcp/schema 接口）");
            }
            // §7.2：MCP 工具副作用按 annotations 推导并注册（审批卡/只读判定消费）。
            // §5.1：effect 内嵌进 ToolSpec——断开/卸载时随工具实例消失，不遗留全局条目。
            // §5.2：宿主只读可信声明按「server+tool+schema hash」校验，hash 匹配才
            // 允许 readOnlyHint 降级为 Read；否则按 Execute（deny-by-default）。
            let host_verified_readonly = crate::tool_effects::is_trusted_readonly(
                server_name,
                &tool.name,
                &crate::tool_effects::schema_fingerprint(&tool.input_schema),
            );
            let effect = crate::tool_effects::register_mcp_effect(
                server_name,
                &tool.name,
                tool.annotations.as_ref(),
                host_verified_readonly,
            );
            let spec = ToolSpec {
                name: full_name.clone(),
                description,
                input_schema,
                effect: Some(effect),
            };
            self.full_schemas.remove(&full_name);
            if let Some(full) = full_schema {
                self.full_schemas.insert(full_name.clone(), full);
            }
            self.register_arc(Arc::new(McpToolAdapter {
                full_name,
                server_name: server_name.to_string(),
                tool_name: tool.name,
                spec,
                client: Arc::clone(&client),
                health: health.clone(),
            }));
        }
    }

    /// A2-2：把 MCP 服务器的 resources/prompts 注册为**泛化工具**——
    /// 每服务器至多 2 个（`mcp__{server}__read_resource` / `mcp__{server}__get_prompt`），
    /// 逐资源/逐模板开工具会撑爆工具表；目录摘要放在描述里供模型选 URI/模板名。
    /// 副作用按「未声明注解的 MCP 工具」登记（Execute，deny-by-default），
    /// 不因名字里带 read 就自动降级。
    pub fn register_mcp_extras(
        &mut self,
        server_name: &str,
        client: Arc<tokio::sync::Mutex<McpClient>>,
        resources: Vec<McpResource>,
        prompts: Vec<McpPrompt>,
    ) {
        let prefix = mcp_tool_prefix(server_name);
        if !resources.is_empty() {
            let catalog: Vec<String> = resources
                .iter()
                .take(12)
                .map(|resource| {
                    if resource.name.is_empty() {
                        resource.uri.clone()
                    } else {
                        format!("{}（{}）", resource.uri, resource.name)
                    }
                })
                .collect();
            let full_name = format!("{prefix}read_resource");
            let effect =
                crate::tool_effects::register_mcp_effect(server_name, "read_resource", None, false);
            self.register_arc(Arc::new(McpResourceAdapter {
                full_name: full_name.clone(),
                server_name: server_name.to_string(),
                spec: ToolSpec {
                    name: full_name,
                    description: format!(
                        "读取 MCP 服务器 {server_name} 的资源内容（resources/read）。可用资源：{}",
                        catalog.join("；")
                    ),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "uri": { "type": "string", "description": "资源 URI（见工具描述里的目录）" }
                        },
                        "required": ["uri"]
                    }),
                    effect: Some(effect),
                },
                client: Arc::clone(&client),
            }));
        }
        if !prompts.is_empty() {
            let catalog: Vec<String> = prompts
                .iter()
                .take(12)
                .map(|prompt| prompt.name.clone())
                .collect();
            let full_name = format!("{prefix}get_prompt");
            let effect =
                crate::tool_effects::register_mcp_effect(server_name, "get_prompt", None, false);
            self.register_arc(Arc::new(McpPromptAdapter {
                full_name: full_name.clone(),
                server_name: server_name.to_string(),
                spec: ToolSpec {
                    name: full_name,
                    description: format!(
                        "获取 MCP 服务器 {server_name} 的提示模板（prompts/get）。可用模板：{}",
                        catalog.join("；")
                    ),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "模板名" },
                            "arguments": { "type": "object", "description": "模板参数（可选，键值对）" }
                        },
                        "required": ["name"]
                    }),
                    effect: Some(effect),
                },
                client,
            }));
        }
    }

    /// 按需取 MCP 工具的完整 schema（压缩注册时保留；小 schema 工具不重复存储）。
    pub fn full_schema(&self, name: &str) -> Option<Value> {
        self.full_schemas.get(name).cloned()
    }
    /// 移除工具时同步清理完整 schema 副本与副作用注册。
    fn remove_prefix_inner(&mut self, prefix: &str) -> usize {
        let before = self.tools.len();
        self.tools
            .retain(|tool| !tool.spec().name.starts_with(prefix));
        self.full_schemas
            .retain(|name, _| !name.starts_with(prefix));
        crate::tool_effects::remove_prefix(prefix);
        before - self.tools.len()
    }
}

/// 工具名规范化函数已随策略内核下沉到 `owo-agent-policy`（M12）：
/// 它同时是权限/效应判定的输入（`effect_class_for` 按名字查表、MCP 前缀按名字生成），
/// 两条消费链必须共用同一份命名规则，否则会出现"登记名"与"执行名"漂移。
/// 命名空间助手 [`mcp_tool_prefix`] / [`mcp_tool_full_name`] 也统一走这里。
#[cfg(test)]
pub(crate) use owo_agent_policy::tool_names::sanitize_tool_name;

/// MCP 工具注册前缀（审查 N5）：`mcp__{server}__`（如 `mcp__owo_plugin_translate__`），
/// 命名空间保证内置工具名不可被外部 server 顶替。
pub fn mcp_tool_prefix(server_name: &str) -> String {
    owo_agent_policy::tool_names::mcp_tool_prefix(server_name)
}

/// MCP 工具完整注册名（注册表与效应表同源）。
pub fn mcp_tool_full_name(server_name: &str, tool_name: &str) -> String {
    owo_agent_policy::tool_names::mcp_tool_full_name(server_name, tool_name)
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

mod registry_support;

pub(crate) use registry_support::decode_process_output;
pub(crate) use registry_support::resolve_session_path;
pub use registry_support::{compact_schema, schema_budget_bytes, schema_bytes};
use registry_support::{snapshot_key, tool_sandbox_policy};

#[path = "tools/workspace_edit.rs"]
mod workspace_edit;
use workspace_edit::{
    strip_verbatim_prefix, ApplyPatchTool, EditFileTool, MultiEditTool, ReadFileTool,
    WhitelistApplyPatchTool, WhitelistWriteFileTool, WriteFileTool,
};

#[path = "tools/workspace_read.rs"]
mod workspace_read;
use workspace_read::{GrepTool, ListDirTool, ReadArtifactTool, SearchFilesTool};

#[path = "tools/find_symbol.rs"]
mod find_symbol;
use find_symbol::FindSymbolTool;

#[path = "tools/delegation.rs"]
mod delegation;
use delegation::{ExploreTool, FanOutSubagentsTool, SubagentTool};

#[path = "tools/image.rs"]
mod image;
use image::ReadImageTool;

#[path = "tools/verification_tool.rs"]
mod verification_tool;
pub(crate) use verification_tool::validate_single_verification_plan;
use verification_tool::SingleVerificationPlanTool;

#[path = "tools/todo.rs"]
mod todo;
use todo::TodoTool;

#[path = "tools/goal_request.rs"]
mod goal_request;
use goal_request::GoalRequestTool;

#[path = "tools/interaction.rs"]
mod interaction;
use interaction::{AskUserTool, UseSkillTool};

/// 解码来路不明的字节（子进程输出 / 未声明 charset 的响应体等）。
///
/// 向用户提问并等待回答（信息不足/需求含糊/关键分歧时使用；取优合并自远端 engine）。
/// 回合会挂起直到用户答复或超时；无 UI 通道时明确报错，让模型改为书面提问。
#[cfg(test)]
mod tests {
    use super::verification_tool::validate_single_request_coverage;
    use super::web::append_capped;
    use super::workspace_edit::{
        align_new_lines, apply_hunks, match_edit_fragment, parse_patch, path_in_whitelist,
        write_file_body, PatchHunk, PatchOp,
    };
    use super::workspace_read::fallback_search_files;
    use super::*;

    fn sample_single_plan(validator_id: &str, arguments: Value) -> crate::plan::VerificationPlanV1 {
        crate::plan::VerificationPlanV1 {
            plan_id: "single-plan".to_string(),
            requirements: vec![crate::plan::VerificationRequirementV1 {
                requirement_id: "req-user-visible".to_string(),
                covers_requirement_ids: vec!["user-request:用户要求功能正常运行".to_string()],
                validator_id: validator_id.to_string(),
                validator_version: Some("1".to_string()),
                scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                    relative_paths: vec!["src/lib.rs".to_string()],
                },
                arguments,
                required: true,
                resources: crate::plan::VerificationResourcesV1 {
                    cpu_slots: 1,
                    memory_mb: 16,
                    exclusive_workspace: false,
                    timeout_ms: 10_000,
                },
            }],
        }
    }

    #[test]
    fn single_verification_coverage_must_quote_the_current_user_request() {
        let plan = sample_single_plan(
            "workspace-command-success-v1",
            json!({"command":"cargo test -p owo-agent-core"}),
        );
        assert!(validate_single_request_coverage(
            &plan,
            "请实现并确保用户要求功能正常运行\n## 验收标准\n- 用户要求功能正常运行"
        )
        .is_ok());
        assert!(validate_single_request_coverage(
            &plan,
            "请实现并确保用户要求功能正常运行\n## 验收标准\n- 用户要求功能正常运行\n- 保留错误码"
        )
        .is_err());
        assert!(validate_single_request_coverage(&plan, "请更新文档并运行检查").is_err());
    }

    #[test]
    fn single_verification_quote_uses_shared_whitespace_normalization() {
        let mut plan = sample_single_plan(
            "workspace-command-success-v1",
            json!({"command":"cargo test"}),
        );
        plan.requirements[0].covers_requirement_ids = vec!["user-request:默认页码为 1".to_string()];

        assert!(validate_single_request_coverage(&plan, "验收标准：\n- 默认页码为   1").is_ok());
        assert!(validate_single_request_coverage(&plan, "验收标准：\n- 默认页码为 2").is_err());
    }

    #[test]
    fn single_verification_plan_accepts_only_registered_scoped_checks() {
        let command_plan = sample_single_plan(
            "workspace-command-success-v1",
            json!({"command":"cargo test -p owo-agent-core"}),
        );
        assert!(validate_single_verification_plan(&command_plan).is_ok());

        let mut unmapped = command_plan.clone();
        unmapped.requirements[0].covers_requirement_ids.clear();
        assert!(validate_single_verification_plan(&unmapped).is_err());

        let unsupported = sample_single_plan("custom-shell-validator", json!({}));
        assert!(validate_single_verification_plan(&unsupported).is_err());

        let bypass = sample_single_plan(
            "workspace-command-success-v1",
            json!({"command":"cargo test -p owo-agent-core --no-run"}),
        );
        assert!(validate_single_verification_plan(&bypass).is_err());

        let mut manual = sample_single_plan("workspace-command-success-v1", json!({}));
        manual.requirements[0].validator_id =
            crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID.to_string();
        manual.requirements[0].scope = crate::plan::VerificationScopeV1::Manual;
        assert!(validate_single_verification_plan(&manual).is_ok());
        manual.requirements[0].arguments = json!({"question":"accept?"});
        assert!(validate_single_verification_plan(&manual).is_err());
    }

    /// 回归（真实模型实测）：首次写入后允许"追加/加强"验收要求（补行为命令），
    /// 但删除/改写既有要求仍被拒绝——既解开冻结死锁，又保持防作弊语义。
    #[tokio::test]
    async fn verification_plan_amendment_after_write_is_append_only() {
        let workspace =
            std::env::temp_dir().join(format!("owo-plan-amend-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let turn_id = "turn-amend-1".to_string();
        let request = "用户要求功能正常运行：创建 src/a.js 后运行 npm test。";
        session.active_task_context = Some(crate::task_context::ResolvedTaskContext {
            origin: crate::task_context::TaskContextOrigin::SingleTurn,
            attempt_id: Some(turn_id.clone()),
            objective: Some(request.to_string()),
            ..Default::default()
        });
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        let plan_static = sample_single_plan("workspace-file-exists-v1", json!({}));
        SingleVerificationPlanTool
            .run(&mut context, json!({ "plan": plan_static }))
            .await
            .expect("写入前登记静态验收应成功");

        // 模拟首次写入：当前回合出现执行收据。
        context
            .session
            .execution_receipts
            .push(crate::session::ExecutionReceipt {
                receipt_id: "exec-amend-1".to_string(),
                tool: "write_file".to_string(),
                turn_id: turn_id.clone(),
                changed_files: vec!["src/lib.rs".to_string()],
                snapshot_keys: Default::default(),
                before_hashes: Default::default(),
                after_hashes: Default::default(),
                diff_sha256: "diff".to_string(),
                created_at: "2026-10-10T00:00:00Z".to_string(),
                status: "executed".to_string(),
                validation_receipt_id: None,
            });

        // 追加行为命令（逐字保留既有要求）→ 允许。
        let mut plan_amended = plan_static.clone();
        plan_amended
            .requirements
            .push(crate::plan::VerificationRequirementV1 {
                requirement_id: "req-tests".to_string(),
                covers_requirement_ids: vec!["user-request:用户要求功能正常运行".to_string()],
                validator_id: "workspace-command-success-v1".to_string(),
                validator_version: Some("1".to_string()),
                scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                    relative_paths: vec!["src/lib.rs".to_string()],
                },
                arguments: json!({"command":"npm test"}),
                required: true,
                resources: crate::plan::VerificationResourcesV1 {
                    cpu_slots: 1,
                    memory_mb: 16,
                    exclusive_workspace: false,
                    timeout_ms: 10_000,
                },
            });
        SingleVerificationPlanTool
            .run(&mut context, json!({ "plan": plan_amended }))
            .await
            .expect("首次写入后追加行为命令应被允许（不能死锁在冻结计划上）");

        // 删掉既有静态要求 → 属于降低验收，拒绝。
        let mut plan_weaker = plan_amended.clone();
        plan_weaker
            .requirements
            .retain(|requirement| requirement.requirement_id != "req-user-visible");
        let error = SingleVerificationPlanTool
            .run(&mut context, json!({ "plan": plan_weaker }))
            .await
            .unwrap_err();
        assert!(
            error.contains("只能追加/加强"),
            "删除既有要求必须被拒并说明只能追加/加强：{error}"
        );

        // 原样重复登记（幂等）→ 允许。
        SingleVerificationPlanTool
            .run(&mut context, json!({ "plan": plan_amended }))
            .await
            .expect("幂等重登记应成功");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// 回归：UTF-8 输出不得被二次误解，OEM 代码页（GBK）输出不得变成替换字符。
    #[test]
    fn decode_process_output_prefers_utf8_then_oem_codepage() {
        // 现代工具（git / rg / python）输出 UTF-8：原样保留。
        assert_eq!(decode_process_output("中文输出".as_bytes()), "中文输出");
        assert_eq!(decode_process_output(b"plain ascii\n"), "plain ascii\n");
        assert_eq!(decode_process_output(b""), "");
        // 「中文」的 GBK 字节（D6D0 CEC4）：严格 UTF-8 解不了，须按系统代码页还原。
        #[cfg(windows)]
        unsafe {
            // GBK 固定字节只适用于 OEM 936；部分 Windows 已把 OEM 页设成 UTF-8。
            // 在这类机器上，此测试仍覆盖 UTF-8 优先分支，不把环境配置差异判成缺陷。
            if windows::Win32::Globalization::GetOEMCP() == 936 {
                assert_eq!(
                    decode_process_output(&[0xD6, 0xD0, 0xCE, 0xC4]),
                    "中文",
                    "GBK 字节未按系统代码页解码（中文命令输出会乱码）"
                );
            }
        }
    }

    #[tokio::test]
    async fn search_files_uses_bundled_ripgrep_with_workspace_scope() {
        let workspace =
            std::env::temp_dir().join(format!("owo-search-ripgrep-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(workspace.join("nested")).unwrap();
        std::fs::write(workspace.join("nested").join("AlphaMarker.TXT"), b"ok").unwrap();

        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        let result = SearchFilesTool
            .run(&mut context, json!({ "pattern": "alphamarker" }))
            .await
            .unwrap();
        assert_eq!(result["tool"], "ripgrep");
        assert_eq!(result["tool_version"], external_tools::RIPGREP_VERSION);
        assert_eq!(result["matches"][0], "nested/AlphaMarker.TXT");
        drop(context);
        let _ = std::fs::remove_dir_all(workspace);
    }

    /// 回归：文件类工具的越界边界必须是**本会话的工作区**，而不是服务启动时的全局工作区。
    ///
    /// 线上事故：桌面壳在 `workspace.json` 未配置时回落 `process.cwd()`（= electron
    /// 安装目录）来启动核心，用户在会话里选 `D:\OwO-master`，于是 `list_dir` /
    /// `read_file` / `search_files` 全部报"路径越界"；`run_command` 不受影响（命令
    /// 工具不走这条判定），所以整件事看起来像"Agent 只坏了一半"。
    #[tokio::test]
    async fn session_workspace_is_the_boundary_when_it_differs_from_policy_workspace() {
        let root = std::env::temp_dir().join(format!("owo-scope-{}", uuid::Uuid::new_v4()));
        let session_ws = root.join("chosen-by-user");
        let service_ws = root.join("service-launch-cwd");
        std::fs::create_dir_all(session_ws.join("nested")).unwrap();
        std::fs::create_dir_all(&service_ws).unwrap();
        std::fs::write(session_ws.join("nested").join("note.txt"), b"hello").unwrap();

        // Policy 用服务级工作区（模拟 `--workspace` 是壳的 cwd，与会话不同的目录）。
        let policy = crate::Policy::new(&service_ws);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&session_ws, "mock", None);
        let context = ToolContext {
            workspace: &session_ws,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        // 1) 会话工作区自身（"."）不得越界——事故现场就是这里恒失败。
        assert!(
            resolve_session_path(&context, ".").is_ok(),
            "会话工作区内的 '.' 被误判越界"
        );
        // 2) 会话工作区内的相对路径可用。
        assert!(resolve_session_path(&context, "nested/note.txt").is_ok());
        // 3) 真正跳出会话工作区的路径仍须拒绝（安全口径不放松）。
        let escaped = resolve_session_path(&context, "../service-launch-cwd");
        if context.policy.profile() != crate::permissions::PermissionProfile::Unrestricted {
            assert!(escaped.is_err(), "跳出会话工作区的路径必须仍被拒绝");
        }
        // 4) 审查 N6：不存在的路径也必须先消解 `..` 再比对——
        //    `nested/../../escape.txt` 的词法落点在工作区外，不得因"文件不存在"
        //    而绕过（旧实现 starts_with 会误判为工作区内）。
        let hidden_escape = resolve_session_path(&context, "nested/../../owo-hidden-escape.txt");
        if context.policy.profile() != crate::permissions::PermissionProfile::Unrestricted {
            assert!(
                hidden_escape.is_err(),
                "含 `..` 的不存在路径必须被拒绝：{hidden_escape:?}"
            );
        }
        // 5) 工作区内的 `..` 归位（`nested/../note-in-root.txt`）仍应放行。
        assert!(
            resolve_session_path(&context, "nested/../note-in-root.txt").is_ok(),
            "工作区内的 `..` 归位不得误杀"
        );

        drop(context);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn repeated_agent_write_refuses_to_overwrite_external_edit() {
        let workspace =
            std::env::temp_dir().join(format!("owo-write-conflict-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let path = workspace.join("shared.txt");
        std::fs::write(&path, "original").unwrap();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        write_file_body(&mut context, "shared.txt", &path, "agent version")
            .await
            .unwrap();
        std::fs::write(&path, "user edit").unwrap();
        let error = write_file_body(&mut context, "shared.txt", &path, "agent overwrite")
            .await
            .expect_err("外部编辑必须阻止后续 Agent 写入");

        assert!(error.contains("写入冲突"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user edit");
        assert_eq!(
            context.session.snapshots[&snapshot_key(&path)]
                .expected_after_sha256
                .as_deref(),
            Some(crate::CasStore::hash_of(b"agent version").as_str()),
            "被拒绝的写入不得推进快照中的 Agent 版本"
        );
        drop(context);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn read_file_returns_content_hash_and_apply_patch_checks_base_receipt() {
        let workspace =
            std::env::temp_dir().join(format!("owo-patch-receipt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let path = workspace.join("shared.txt");
        std::fs::write(&path, "old\n").unwrap();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        let before_hash = format!("sha256:{}", crate::CasStore::hash_of(b"old\n"));
        let read = ReadFileTool
            .run(&mut context, json!({ "path": "shared.txt" }))
            .await
            .unwrap();
        assert_eq!(read["sha256"], before_hash);

        let patch = "*** Begin Patch\n*** Update File: shared.txt\n@@\n-old\n+new\n*** End Patch";
        let stale = ApplyPatchTool
            .run(
                &mut context,
                json!({ "patch": patch, "expected_hashes": { "shared.txt": "sha256:stale" } }),
            )
            .await
            .expect_err("旧基线必须被拒绝");
        assert!(stale.contains("补丁基线冲突"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");

        let applied = ApplyPatchTool
            .run(
                &mut context,
                json!({ "patch": patch, "expected_hashes": { "shared.txt": before_hash } }),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(applied["files"][0]["base_sha256"], before_hash);
        assert_eq!(
            applied["files"][0]["result_sha256"],
            format!("sha256:{}", crate::CasStore::hash_of(b"new\n"))
        );

        let add = ApplyPatchTool
            .run(
                &mut context,
                json!({
                    "patch": "*** Begin Patch\n*** Add File: created.txt\n+hello\n*** End Patch",
                    "expected_hashes": { "created.txt": "absent" }
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join("created.txt")).unwrap(),
            "hello\n"
        );
        assert_eq!(add["files"][0]["base_sha256"], Value::Null);
        let created_hash = format!("sha256:{}", crate::CasStore::hash_of(b"hello\n"));
        assert_eq!(add["files"][0]["result_sha256"], created_hash);

        let delete = ApplyPatchTool
            .run(
                &mut context,
                json!({
                    "patch": "*** Begin Patch\n*** Delete File: created.txt\n*** End Patch",
                    "expected_hashes": { "created.txt": created_hash }
                }),
            )
            .await
            .unwrap();
        assert!(!workspace.join("created.txt").exists());
        assert_eq!(delete["files"][0]["base_sha256"], created_hash);
        assert_eq!(delete["files"][0]["result_sha256"], Value::Null);
        drop(context);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn whitelist_apply_patch_rejects_targets_outside_allowed_paths() {
        let workspace =
            std::env::temp_dir().join(format!("owo-patch-whitelist-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(workspace.join("allowed")).unwrap();
        let outside = workspace.join("outside.txt");
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        let tool = WhitelistApplyPatchTool {
            allowed: vec![workspace.join("allowed")],
        };
        let error = tool
            .run(
                &mut context,
                json!({ "patch": "*** Begin Patch\n*** Add File: outside.txt\n+denied\n*** End Patch" }),
            )
            .await
            .expect_err("白名单外补丁必须拒绝");
        assert!(error.contains("写入目标不在写白名单内"));
        assert!(!outside.exists());
        drop(context);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn apply_patch_rejects_duplicate_file_operations_before_writing() {
        let workspace =
            std::env::temp_dir().join(format!("owo-patch-duplicate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let path = workspace.join("shared.txt");
        std::fs::write(&path, "old\n").unwrap();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        let patch = "*** Begin Patch\n*** Update File: shared.txt\n@@\n-old\n+new\n*** Update File: shared.txt\n@@\n-old\n+later\n*** End Patch";
        let error = ApplyPatchTool
            .run(&mut context, json!({ "patch": patch }))
            .await
            .expect_err("同一文件重复操作应拒绝");
        assert!(error.contains("重复操作"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
        drop(context);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn sanitizes_tool_names_for_model_api() {
        assert_eq!(
            sanitize_tool_name("owo.plugin.example-hello"),
            "owo_plugin_example-hello"
        );
        assert_eq!(sanitize_tool_name("echo"), "echo");
        assert_eq!(sanitize_tool_name("a b/c"), "a_b_c");
    }

    #[test]
    fn mcp_tool_prefix_sanitizes_plugin_id() {
        // 审查 N5：`mcp__{server}__` 命名空间，内置名不可能被外部 server 顶替。
        assert_eq!(
            mcp_tool_prefix("owo.plugin.translate"),
            "mcp__owo_plugin_translate__"
        );
        assert_eq!(
            mcp_tool_prefix("owo-plugin-clipboard"),
            "mcp__owo-plugin-clipboard__"
        );
        assert_eq!(
            mcp_tool_full_name("read", "file"),
            "mcp__read__file",
            "server=read + tool=file 不得再注册成内置 read_file"
        );
        assert_eq!(
            mcp_tool_full_name("git", "status"),
            "mcp__git__status",
            "server=git 卸载时不得连带删除内置 git_status"
        );
    }

    #[test]
    fn trusted_builtin_read_metadata_is_owned_by_the_registered_instance() {
        let registry = ToolRegistry::read_only();
        for spec in registry.specs() {
            let effect = spec
                .effect
                .as_ref()
                .expect("builtin readonly metadata must be exported");
            assert_eq!(
                effect.class,
                crate::tool_effects::EffectClass::Read,
                "{}",
                spec.name
            );
            assert!(effect.host_verified_readonly, "{}", spec.name);
            let lookup = registry.get(&spec.name).unwrap().spec();
            assert!(lookup.effect.as_ref().unwrap().host_verified_readonly);
        }
    }

    #[test]
    fn a_foreign_tool_named_read_file_does_not_inherit_builtin_trust() {
        let mut registry = ToolRegistry::read_only();
        registry.register(NamedTool {
            name: "read_file".into(),
            description: "foreign fixture".into(),
        });
        let spec = registry
            .specs()
            .into_iter()
            .find(|spec| spec.name == "read_file")
            .unwrap();
        assert!(spec.effect.is_none());
        assert!(registry.get("read_file").unwrap().spec().effect.is_none());
    }

    struct NamedTool {
        name: String,
        description: String,
    }

    #[async_trait]
    impl Tool for NamedTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.clone(),
                description: self.description.clone(),
                input_schema: serde_json::json!({}),
                effect: None,
            }
        }

        async fn run(
            &self,
            _ctx: &mut ToolContext<'_>,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::Value::Null)
        }
    }

    #[tokio::test]
    async fn tool_host_requires_approval_and_emits_receipt() {
        let workspace =
            std::env::temp_dir().join(format!("owo-tool-host-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let registry = Arc::new(RwLock::new(ToolRegistry::empty()));
        registry.write().unwrap().register(NamedTool {
            name: "contract_probe".to_string(),
            description: String::new(),
        });
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let host = ToolHostService::new(Arc::clone(&registry), Arc::clone(&audit));
        let denied_request = PermissionRequest::new(
            "contract_probe",
            json!({"value": 1}),
            crate::permissions::Level::Write,
            "test denial",
        );
        assert!(ToolApprovalGrant::from_decision(&denied_request, Decision::Deny).is_err());
        let approval_for = |args: Value| {
            ToolApprovalGrant::from_decision(
                &PermissionRequest::new(
                    "contract_probe",
                    args,
                    crate::permissions::Level::Write,
                    "test approval",
                ),
                Decision::Allow,
            )
            .unwrap()
        };

        let policy = crate::Policy::new(&workspace);
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let session_id = session.id.clone();
        let capability_context =
            ToolCapabilityContext::for_workspace(&workspace, session.id.clone(), "contract-turn");
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        let capability = host
            .issue(
                "contract_probe",
                json!({"value": 1}),
                approval_for(json!({"value": 1})),
                capability_context,
            )
            .unwrap();
        assert!(!capability.nonce.is_empty());
        assert!(capability.expires_at_unix > 0);
        assert_eq!(
            host.execute(capability, &mut context).await.unwrap(),
            Value::Null
        );
        let wrong_session = host
            .issue(
                "contract_probe",
                json!({"value": 3}),
                approval_for(json!({"value": 3})),
                ToolCapabilityContext::for_workspace(
                    &workspace,
                    "different-session",
                    "contract-turn-3",
                ),
            )
            .unwrap();
        let error = host.execute(wrong_session, &mut context).await.unwrap_err();
        assert!(error.contains("capability session mismatch"));
        let mut stale_schema = host
            .issue(
                "contract_probe",
                json!({"value": 4}),
                approval_for(json!({"value": 4})),
                ToolCapabilityContext::for_workspace(
                    &workspace,
                    session_id.clone(),
                    "contract-turn-4",
                ),
            )
            .unwrap();
        stale_schema.tool_version = "stale-tool-version".to_string();
        let error = host.execute(stale_schema, &mut context).await.unwrap_err();
        assert!(error.contains("capability tool version mismatch"));
        let mut expired = host
            .issue(
                "contract_probe",
                json!({"value": 2}),
                approval_for(json!({"value": 2})),
                ToolCapabilityContext::for_workspace(&workspace, session_id, "contract-turn-2"),
            )
            .unwrap();
        expired.expires_at_unix = 0;
        let error = host
            .execute(expired, &mut context)
            .await
            .expect_err("过期 capability 不得执行工具");
        assert!(error.contains("approval expired"));
        drop(context);

        let entries = audit.lock().unwrap().entries.clone();
        let receipt = entries
            .iter()
            .find(|entry| entry.event == "tool_receipt")
            .expect("受信执行必须产生 receipt");
        assert_eq!(receipt.tool.as_deref(), Some("contract_probe"));
        assert_eq!(receipt.approved, Some(true));
        assert!(receipt.detail.contains("args_sha256"));
        assert!(receipt.detail.contains("expires_at_unix"));
        assert!(receipt.detail.contains("nonce"));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn write_toolhost_emits_execution_receipt_and_scoped_revert() {
        let workspace = std::env::temp_dir().join(format!(
            "owo-tool-host-write-receipt-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let registry = Arc::new(RwLock::new(ToolRegistry::empty()));
        registry.write().unwrap().register_write_file();
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let host = ToolHostService::new(Arc::clone(&registry), Arc::clone(&audit));
        let policy = crate::Policy::new(&workspace);
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let session_id = session.id.clone();
        let args = json!({"path": "receipt.txt", "content": "agent content"});
        let request = PermissionRequest::new(
            "write_file",
            args.clone(),
            crate::permissions::Level::Write,
            "test write",
        );
        let approval = ToolApprovalGrant::from_decision(&request, Decision::Allow).unwrap();
        let context = ToolCapabilityContext::for_workspace(&workspace, session_id, "turn-write");
        let mut tool_context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        let capability = host.issue("write_file", args, approval, context).unwrap();
        let result = host.execute(capability, &mut tool_context).await.unwrap();
        let receipt_id = result
            .get("execution_receipt_id")
            .and_then(Value::as_str)
            .expect("写入结果必须返回执行收据 ID")
            .to_string();
        assert_eq!(tool_context.session.execution_receipts.len(), 1);
        assert_eq!(
            tool_context.session.execution_receipts[0].receipt_id,
            receipt_id
        );
        drop(tool_context);
        let restored = session.revert_receipt(Some(&receipt_id)).await.unwrap();
        assert_eq!(restored, vec!["receipt.txt"]);
        assert!(!workspace.join("receipt.txt").exists());
        let audit_guard = audit.lock().unwrap();
        let receipt = audit_guard
            .entries
            .iter()
            .find(|entry| entry.event == "tool_receipt")
            .expect("写入必须产生 tool receipt");
        assert!(receipt.detail.contains("execution_receipt_id"));
        assert!(receipt.detail.contains("diff_sha256"));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// multi_edit（远端 engine 取优）：按顺序应用多处替换，第 2 处的 old_string
    /// 匹配第 1 处替换后的内容。
    #[tokio::test]
    async fn multi_edit_applies_all_edits_in_order() {
        let workspace =
            std::env::temp_dir().join(format!("owo-multi-edit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let original = "fn main() {\n    let version = \"1.0\";\n    println!(\"v1.0\");\n}\n";
        std::fs::write(workspace.join("app.rs"), original).unwrap();

        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        let result = MultiEditTool
            .run(
                &mut context,
                json!({
                    "path": "app.rs",
                    "edits": [
                        { "old_string": "let version = \"1.0\";", "new_string": "let version = \"2.0\";" },
                        { "old_string": "println!(\"v1.0\");", "new_string": "println!(\"v{version}\");" }
                    ]
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["applied"], 2);
        let updated = std::fs::read_to_string(workspace.join("app.rs")).unwrap();
        assert!(updated.contains("let version = \"2.0\";"), "{updated}");
        assert!(updated.contains("println!(\"v{version}\");"), "{updated}");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// multi_edit 原子性：任一处失败（未命中/歧义）整批不落盘。
    #[tokio::test]
    async fn multi_edit_is_atomic_on_failure() {
        let workspace =
            std::env::temp_dir().join(format!("owo-multi-edit-atomic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let original = "alpha\nbeta\ngamma\n";
        std::fs::write(workspace.join("data.txt"), original).unwrap();

        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        // 第 2 处失败（old_string 不存在）：整批不落盘。
        let error = MultiEditTool
            .run(
                &mut context,
                json!({
                    "path": "data.txt",
                    "edits": [
                        { "old_string": "alpha", "new_string": "ALPHA" },
                        { "old_string": "不存在的片段", "new_string": "x" }
                    ]
                }),
            )
            .await
            .unwrap_err();
        assert!(
            error.contains("2/2") && error.contains("整批未应用"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("data.txt")).unwrap(),
            original,
            "失败时原文件必须保持不变"
        );

        // 多处歧义（未设 replace_all）同样整体失败。
        std::fs::write(workspace.join("data.txt"), "same\nsame\n").unwrap();
        let error = MultiEditTool
            .run(
                &mut context,
                json!({
                    "path": "data.txt",
                    "edits": [{ "old_string": "same", "new_string": "x" }]
                }),
            )
            .await
            .unwrap_err();
        assert!(error.contains("出现 2 次"), "{error}");
        assert_eq!(
            std::fs::read_to_string(workspace.join("data.txt")).unwrap(),
            "same\nsame\n"
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// ask_user（取优合并自远端 engine）：无 UI 通道时明确报错，模型改为书面提问。
    #[tokio::test]
    async fn ask_user_without_channel_reports_error() {
        let workspace =
            std::env::temp_dir().join(format!("owo-ask-user-none-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        let error = AskUserTool
            .run(&mut context, json!({ "question": "先做 A 还是 B？" }))
            .await
            .unwrap_err();
        assert!(error.contains("没有可用的用户问答通道"), "{error}");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// 回显提问通道：直接返回答案，模拟用户在提问卡上作答。
    struct EchoQuestioner;

    #[async_trait]
    impl crate::question::Questioner for EchoQuestioner {
        async fn ask(
            &self,
            question: &crate::question::UserQuestion,
        ) -> Option<crate::question::QuestionAnswer> {
            Some(crate::question::QuestionAnswer {
                question_id: question.question_id.clone(),
                answer: format!("已收到：{}", question.question),
            })
        }
    }

    /// ask_user：有通道时提问经 Questioner 拿回用户答案（answered=true）。
    #[tokio::test]
    async fn ask_user_returns_answer_through_channel() {
        let workspace =
            std::env::temp_dir().join(format!("owo-ask-user-ok-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let questioner = EchoQuestioner;
        let result = {
            let mut context = ToolContext {
                workspace: &workspace,
                policy: &policy,
                session: &mut session,
                audit: &audit,
                subagent: None,
                skills: &skills,
                elements: &elements,
                fanout: None,
                abort: None,
                questioner: Some(&questioner),
            };
            AskUserTool
                .run(
                    &mut context,
                    json!({ "question": "先做 A 还是 B？", "options": ["A", "B"] }),
                )
                .await
                .unwrap()
        };
        assert_eq!(result["answered"], true);
        assert_eq!(result["answer"], "已收到：先做 A 还是 B？");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn same_name_registration_replaces_definition_without_duplicate_model_tools() {
        let mut registry = ToolRegistry::empty();
        registry.register(NamedTool {
            name: "reloadable_tool".to_string(),
            description: "old definition".to_string(),
        });
        registry.register(NamedTool {
            name: "reloadable_tool".to_string(),
            description: "new definition".to_string(),
        });

        let specs = registry.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "reloadable_tool");
        assert_eq!(specs[0].description, "new definition");
        assert_eq!(
            registry.get("reloadable_tool").unwrap().spec().description,
            specs[0].description,
            "模型看到的定义和执行器按名取到的定义必须一致"
        );
    }

    #[test]
    fn remove_prefix_unregisters_only_matching_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(NamedTool {
            name: "owo_plugin_demo_translate".to_string(),
            description: String::new(),
        });
        registry.register(NamedTool {
            name: "owo_plugin_demo_clipboard".to_string(),
            description: String::new(),
        });
        registry.register(NamedTool {
            name: "builtin_tool".to_string(),
            description: String::new(),
        });
        let removed = registry.remove_prefix("owo_plugin_demo_");
        assert_eq!(removed, 2);
        let names: Vec<String> = registry
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        assert!(!names
            .iter()
            .any(|name| name.starts_with("owo_plugin_demo_")));
        assert!(names.iter().any(|name| name == "builtin_tool"));
    }

    #[test]
    fn retain_names_keeps_only_listed_tools() {
        // 空名单 = 全部保留。
        let mut registry = ToolRegistry::new();
        assert_eq!(registry.retain_names(&[]), 0);
        // 非空名单 = 只留名单内工具（含 MCP 完整 schema 副本同步清理）。
        let mut registry = ToolRegistry::new();
        let before = registry.specs().len();
        let removed = registry.retain_names(&["read_file".to_string()]);
        assert_eq!(removed, before - 1);
        let names: Vec<String> = registry
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        assert_eq!(names, vec!["read_file".to_string()]);
    }

    #[test]
    fn grouped_role_constructors_assemble_expected_surface() {
        // 只读组：读三件套（与 read_only() 等价）。
        let mut registry = ToolRegistry::empty();
        registry.register_file_read_tools();
        let names: Vec<String> = registry
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        assert_eq!(
            names,
            vec![
                "read_file".to_string(),
                "list_dir".to_string(),
                "search_files".to_string()
            ]
        );
        // 浏览器组：含读变体与写工作区变体（写变体靠 retain_names 按角色裁剪）。
        let mut registry = ToolRegistry::empty();
        registry.register_browser_tools();
        let names: Vec<String> = registry
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        assert!(names.contains(&"browser_navigate".to_string()));
        assert!(names.contains(&"browser_screenshot".to_string()));
    }

    #[test]
    fn default_registry_is_minimal_and_optional_groups_are_explicit() {
        let mut registry = ToolRegistry::new();
        let names: Vec<String> = registry.specs().into_iter().map(|spec| spec.name).collect();
        assert_eq!(
            names,
            vec![
                "read_file",
                "write_file",
                "edit_file",
                "multi_edit",
                "apply_patch",
                "list_dir",
                "search_files",
                "grep",
                "read_artifact",
                "find_symbol",
                "git_status",
                "git_diff",
                "git_log",
                "run_command",
                "shell_output",
                "kill_shell",
                "todo",
                "goal_request",
                "verification_plan",
                "web_fetch",
                "web_search",
                "read_image",
                "explore",
                "subagent",
                "fan_out_subagents",
                "ask_user",
                "use_skill",
            ]
        );
        assert!(!names.iter().any(|name| name.starts_with("desktop_")));
        assert!(!names.iter().any(|name| name.starts_with("browser_")));
        assert!(!names.iter().any(|name| name == "screen_ocr"));

        registry.register_desktop_observation_tools();
        registry.register_desktop_control_tools();
        registry.register_browser_tools();
        let enabled: Vec<String> = registry.specs().into_iter().map(|spec| spec.name).collect();
        for optional in ["screen_ocr", "desktop_click", "browser_navigate"] {
            assert!(enabled.iter().any(|name| name == optional));
        }
    }

    #[test]
    fn path_in_whitelist_prefix_match() {
        let base = PathBuf::from("T:/ws/src");
        // 四路集成微修：clippy 冗余 clone（单元素切片用 from_ref 借用即可）。
        assert!(path_in_whitelist(
            &PathBuf::from("T:/ws/src/lib/a.rs"),
            std::slice::from_ref(&base)
        ));
        assert!(!path_in_whitelist(
            &PathBuf::from("T:/ws/docs/b.md"),
            std::slice::from_ref(&base)
        ));
        // 空白名单在此判 false（无前缀可匹配）——「空 = 未约束」由
        // resolve_whitelisted 短路放行，纯函数只做前缀匹配。
        assert!(!path_in_whitelist(
            &PathBuf::from("T:/ws/anything.txt"),
            &[]
        ));
    }

    #[test]
    fn strip_verbatim_prefix_normalizes_windows_canonical_paths() {
        // 回归（七期二路冒烟发现）：canonicalize 产物带 `\\?\` 前缀，与绑定侧
        // simplify 后的 allowed 混用比对会恒 false。
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\C:\ws\src\a.rs")),
            PathBuf::from(r"C:\ws\src\a.rs")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"C:\ws\src\a.rs")),
            PathBuf::from(r"C:\ws\src\a.rs")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new("/home/ws/src/a.rs")),
            PathBuf::from("/home/ws/src/a.rs")
        );
    }

    #[test]
    fn edit_fragment_tolerates_crlf_lf_mismatch_and_keeps_line_style() {
        // 文件 CRLF、模型给 LF 片段 → 唯一命中，new 沿用 CRLF。
        let original = "alpha\r\nbeta\r\ngamma\r\n";
        let (matched, count) = match_edit_fragment(original, "beta\ngamma");
        assert_eq!(count, 1);
        assert_eq!(matched, "beta\r\ngamma");
        assert_eq!(
            align_new_lines(&matched, "beta2\ngamma2"),
            "beta2\r\ngamma2"
        );

        // 文件 LF、模型给 CRLF 片段 → 命中且 new 归一为 LF。
        let original_lf = "alpha\nbeta\ngamma\n";
        let (matched_lf, count_lf) = match_edit_fragment(original_lf, "beta\r\ngamma");
        assert_eq!(count_lf, 1);
        assert_eq!(matched_lf, "beta\ngamma");
        assert_eq!(align_new_lines(&matched_lf, "b\r\ng"), "b\ng");

        // 精确命中保持原样；完全不存在仍为 0。
        assert_eq!(match_edit_fragment(original, "beta\r\ngamma").1, 1);
        assert_eq!(match_edit_fragment(original, "not-there").1, 0);
    }

    #[test]
    fn fallback_search_files_matches_names_case_insensitively_and_skips_noise_dirs() {
        let workspace =
            std::env::temp_dir().join(format!("owo-fallback-search-{}", uuid::Uuid::new_v4()));
        for dir in ["a", ".git", "node_modules", "target"] {
            std::fs::create_dir_all(workspace.join(dir)).unwrap();
        }
        std::fs::write(workspace.join("a").join("Note.md"), "x").unwrap();
        std::fs::write(workspace.join("b_notes.txt"), "x").unwrap();
        std::fs::write(workspace.join(".git").join("notes.md"), "x").unwrap();
        std::fs::write(workspace.join("node_modules").join("notes.js"), "x").unwrap();
        std::fs::write(workspace.join("target").join("notes.rs"), "x").unwrap();

        let mut matches = fallback_search_files(&workspace, "note", 100);
        matches.sort();
        assert_eq!(
            matches,
            vec!["a/Note.md".to_string(), "b_notes.txt".to_string()],
            "兜底搜索应大小写不敏感且跳过 .git/node_modules/target"
        );
        assert!(fallback_search_files(&workspace, "zzz-not-there", 100).is_empty());
        // 上限参数必须真实生效（不再硬编码 200）。
        std::fs::write(workspace.join("c_note.txt"), "x").unwrap();
        assert_eq!(fallback_search_files(&workspace, "note", 1).len(), 1);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn list_dir_sorts_marks_noise_and_reports_truncation() {
        let workspace = std::env::temp_dir().join(format!("owo-list-dir-{}", uuid::Uuid::new_v4()));
        for dir in ["src", ".git", "target"] {
            std::fs::create_dir_all(workspace.join(dir)).unwrap();
        }
        std::fs::write(workspace.join("README.md"), "x").unwrap();
        std::fs::write(workspace.join("zeta.txt"), "x").unwrap();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        let listing = ListDirTool
            .run(&mut context, json!({ "path": "." }))
            .await
            .unwrap();
        let entries = listing["entries"].as_array().unwrap();
        let names: Vec<&str> = entries
            .iter()
            .map(|entry| entry["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            names,
            vec![".git", "src", "target", "README.md", "zeta.txt"],
            "目录在前、名字大小写不敏感升序（文件系统原始顺序不稳定）：{listing}"
        );
        let git = entries
            .iter()
            .find(|entry| entry["name"] == ".git")
            .expect(".git 条目应存在");
        assert_eq!(
            git["noise"],
            json!(true),
            "噪声目录要标记，模型可据此不再下钻"
        );
        assert_eq!(listing["count"], json!(5));
        assert_eq!(listing["truncated"], json!(false));

        let capped = ListDirTool
            .run(&mut context, json!({ "path": ".", "max_entries": 2 }))
            .await
            .unwrap();
        assert_eq!(capped["count"], json!(2), "上限必须生效");
        assert_eq!(capped["truncated"], json!(true), "超大目录必须显式报告截断");
        drop(context);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn read_artifact_pages_by_ref_and_rejects_unsafe_refs() {
        let workspace =
            std::env::temp_dir().join(format!("owo-read-artifact-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let dir = workspace.join(".owo").join("artifacts");
        let store = crate::CasStore::new(dir.clone()).expect("CAS 初始化");
        let content = (0..5_000)
            .map(|index| format!("line-{index}\n"))
            .collect::<String>();
        let hash = store.put(content.as_bytes()).expect("artifact 落盘");
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        // 带 `sha256:` 前缀也应被接受，并按字符分页返回 next_offset。
        let first = ReadArtifactTool
            .run(
                &mut context,
                json!({ "ref": format!("sha256:{hash}"), "max_chars": 100 }),
            )
            .await
            .unwrap();
        assert_eq!(first["ref"], json!(format!("sha256:{hash}")));
        assert_eq!(first["returned_chars"], json!(100));
        assert_eq!(first["offset"], json!(0));
        assert_eq!(first["truncated"], json!(true));
        let next_offset = first["next_offset"].as_u64().expect("应有续读偏移");
        assert_eq!(next_offset, 100);
        assert!(
            first["text"].as_str().unwrap().starts_with("line-0\n"),
            "分页必须从 offset 开始：{first}"
        );
        assert_eq!(
            first["total_chars"].as_u64().unwrap(),
            content.chars().count() as u64
        );

        let second = ReadArtifactTool
            .run(
                &mut context,
                json!({ "ref": hash, "offset": next_offset, "max_chars": 50 }),
            )
            .await
            .unwrap();
        assert_eq!(second["offset"], json!(100));
        assert_eq!(second["returned_chars"], json!(50));

        // 越界/非法 ref 必须被拒绝：CasStore 直接用 ref 拼文件名，`../` 会越出目录。
        for bad in ["../../secret", "abc", "zzzz", ""] {
            let error = ReadArtifactTool
                .run(&mut context, json!({ "ref": bad }))
                .await
                .expect_err("非法 ref 必须拒绝");
            assert!(
                error.contains("64 位十六进制") || error.contains("ref"),
                "错误信息要指明 ref 形态：{error}"
            );
        }
        let missing = ReadArtifactTool
            .run(&mut context, json!({ "ref": "0".repeat(64) }))
            .await
            .expect_err("不存在的 ref 必须报错");
        assert!(missing.contains("不存在"), "{missing}");
        drop(context);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn todo_rejects_oversized_plans_and_overlong_items() {
        let workspace = std::env::temp_dir().join(format!("owo-todo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        let ok = TodoTool
            .run(
                &mut context,
                json!({ "todos": [{ "content": "只列关键步骤", "status": "pending" }] }),
            )
            .await
            .unwrap();
        assert_eq!(ok["todos"].as_array().unwrap().len(), 1);

        // 超量整表：必须拒绝并给出上限，避免一次写入把上下文/会话库撑爆。
        let too_many: Vec<Value> = (0..51)
            .map(|index| json!({ "content": format!("步骤 {index}"), "status": "pending" }))
            .collect();
        let error = TodoTool
            .run(&mut context, json!({ "todos": too_many }))
            .await
            .expect_err("超过 50 条必须拒绝");
        assert!(error.contains("50"), "{error}");
        assert_eq!(
            context.session.todos.len(),
            1,
            "被拒绝的写入不得改动会话里的清单"
        );

        let overlong = "很长的内容".repeat(60);
        let error = TodoTool
            .run(
                &mut context,
                json!({ "todos": [{ "content": overlong, "status": "pending" }] }),
            )
            .await
            .expect_err("单条过长必须拒绝");
        assert!(error.contains("200"), "{error}");
        assert_eq!(context.session.todos.len(), 1);
        drop(context);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn use_skill_bounds_long_instructions_and_points_to_the_file() {
        let workspace = std::env::temp_dir().join(format!("owo-skill-{}", uuid::Uuid::new_v4()));
        let skill_dir = workspace.join(".agents").join("skills").join("huge");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let long_body = "步骤说明\n".repeat(6_000); // ~24k+ 字符
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: huge\ndescription: 超长技能\n---\n{long_body}"),
        )
        .unwrap();
        let skills = crate::SkillRegistry::discover(&workspace, &workspace.join("data-root"));
        assert!(skills.get_enabled("huge").is_some(), "技能应被发现");

        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        let result = UseSkillTool
            .run(&mut context, json!({ "name": "huge" }))
            .await
            .unwrap();
        assert_eq!(
            result["truncated"],
            json!(true),
            "超长技能指令必须标记截断：{}",
            result["total_chars"]
        );
        assert!(
            result["total_chars"].as_u64().unwrap_or(0) > 24_000,
            "total_chars 给出真实长度：{result}"
        );
        assert_eq!(
            result["instructions"].as_str().unwrap().chars().count(),
            24_000,
            "instructions 必须截断到上限"
        );
        let note = result["note"].as_str().unwrap_or_default();
        assert!(note.contains("read_file"), "note 要给续读方式：{note}");
        assert_eq!(
            result["instructions_path"],
            json!(".agents/skills/huge/SKILL.md"),
            "路径应是工作区相对路径，供 read_file 续读"
        );
        drop(context);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn web_fetch_byte_cap_never_exceeds_the_limit() {
        let mut buffer = Vec::new();
        assert!(append_capped(&mut buffer, b"12345", 4));
        assert_eq!(buffer, b"1234");
        assert!(append_capped(&mut buffer, b"6", 4));
        assert_eq!(buffer, b"1234");

        let mut exact = Vec::new();
        assert!(!append_capped(&mut exact, b"abc", 3));
        assert_eq!(exact, b"abc");
        assert!(append_capped(&mut exact, b"d", 3));
    }

    #[test]
    fn patch_parser_supports_add_update_delete() {
        let patch = "*** Begin Patch\n*** Add File: a.txt\n+hello\n*** Update File: b.txt\n@@ fn main\n-let x = 1;\n+let x = 2;\n*** Delete File: c.txt\n*** End Patch\n";
        let ops = parse_patch(patch).expect("patch should parse");
        assert_eq!(ops.len(), 3);
        match &ops[0] {
            PatchOp::Add { path, content } => {
                assert_eq!(path, "a.txt");
                assert_eq!(content, "hello\n");
            }
            _ => panic!("first op should be add"),
        }
        match &ops[1] {
            PatchOp::Update { path, hunks } => {
                assert_eq!(path, "b.txt");
                assert_eq!(hunks.len(), 1);
                // `@@` 行是段落标题提示，不计入上下文。
                assert_eq!(hunks[0].old_lines, vec!["let x = 1;"]);
                assert_eq!(hunks[0].new_lines, vec!["let x = 2;"]);
            }
            _ => panic!("second op should be update"),
        }
        assert!(matches!(&ops[2], PatchOp::Delete { path } if path == "c.txt"));
    }

    #[test]
    fn patch_apply_requires_unique_context() {
        let original = "alpha\nbeta\ngamma\n";
        let ok = apply_hunks(
            original,
            &[PatchHunk {
                old_lines: vec!["beta".to_string()],
                new_lines: vec!["BETA".to_string()],
            }],
        )
        .expect("unique context should apply");
        assert_eq!(ok, "alpha\nBETA\ngamma\n");

        let ambiguous = apply_hunks(
            "x\ny\nx\ny\n",
            &[PatchHunk {
                old_lines: vec!["x".to_string(), "y".to_string()],
                new_lines: vec!["z".to_string(), "y".to_string()],
            }],
        );
        assert!(ambiguous.is_err(), "重复上下文必须拒绝");

        let missing = apply_hunks(
            original,
            &[PatchHunk {
                old_lines: vec!["nope".to_string()],
                new_lines: vec!["yes".to_string()],
            }],
        );
        assert!(missing.is_err(), "未命中必须报错");
    }

    #[test]
    fn compact_schema_preserves_nested_parameter_contracts() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["query"],
            "additionalProperties": false,
            "properties": {
                "query": {
                    "type": "object",
                    "required": ["mode"],
                    "additionalProperties": false,
                    "properties": {
                        "mode": {
                            "type": "string",
                            "enum": ["exact", "fuzzy"],
                            "description": "remove annotation"
                        },
                        "filters": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["field"],
                                "properties": {
                                    "field": {"type": "string", "minLength": 1}
                                }
                            }
                        }
                    }
                }
            }
        });
        let compact = compact_schema(&schema);
        assert_eq!(compact["required"], serde_json::json!(["query"]));
        assert_eq!(compact["additionalProperties"], false);
        assert_eq!(
            compact["properties"]["query"]["required"],
            serde_json::json!(["mode"])
        );
        assert_eq!(
            compact["properties"]["query"]["properties"]["mode"]["enum"],
            serde_json::json!(["exact", "fuzzy"])
        );
        assert_eq!(
            compact["properties"]["query"]["properties"]["filters"]["items"]["required"],
            serde_json::json!(["field"])
        );
        assert_eq!(
            compact["properties"]["query"]["properties"]["filters"]["items"]["properties"]["field"]
                ["minLength"],
            1
        );
        assert!(compact["properties"]["query"]["properties"]["mode"]
            .get("description")
            .is_none());
    }

    #[test]
    fn html_to_text_strips_tags_and_scripts() {
        let html = "<html><head><style>body{color:red}</style></head><body><h1>Hello</h1><script>var x=1;</script><p>World&nbsp;!</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"), "{text}");
        assert!(text.contains("World !"), "{text}");
        assert!(!text.contains("var x"), "script 内容应剔除：{text}");
        assert!(!text.contains("color:red"), "style 内容应剔除：{text}");
    }

    #[test]
    fn html_to_text_drops_comments_and_hidden_nodes() {
        let html = "<body>可见\
            <!-- 注释里的注入 -->\
            <div style=\"display:none\">隐藏注入 do-not-read</div>\
            <span hidden>也隐藏</span>\
            <p visibility:hidden>隐形</p>\
            <b aria-hidden=\"true\">无障碍隐藏</b>\
            <p>正文</p></body>";
        let text = html_to_text(html);
        assert!(text.contains("可见"), "{text}");
        assert!(text.contains("正文"), "{text}");
        assert!(!text.contains("注入"), "注释/隐藏节点应剔除：{text}");
        assert!(!text.contains("也隐藏"), "hidden 属性应剔除：{text}");
        assert!(!text.contains("隐形"), "visibility:hidden 应剔除：{text}");
        assert!(!text.contains("无障碍隐藏"), "aria-hidden 应剔除：{text}");
    }

    #[test]
    fn ssrf_guard_blocks_private_and_reserved_addresses() {
        use std::net::IpAddr;
        for blocked in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            let ip: IpAddr = blocked.parse().expect("测试地址应可解析");
            assert!(super::web::ip_is_forbidden(ip), "{blocked} 应被 SSRF 拦截");
        }
        for allowed in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            let ip: IpAddr = allowed.parse().expect("测试地址应可解析");
            assert!(!super::web::ip_is_forbidden(ip), "{allowed} 不应被拦截");
        }
        let local = reqwest::Url::parse("http://127.0.0.1:8080/admin").unwrap();
        assert!(super::web::validate_fetch_target(&local).is_err());
        let public = reqwest::Url::parse("https://example.com/").unwrap();
        assert!(super::web::validate_fetch_target(&public).is_ok());
        let bad_scheme = reqwest::Url::parse("ftp://example.com/x").unwrap();
        assert!(super::web::validate_fetch_target(&bad_scheme).is_err());
    }

    #[test]
    fn search_href_decodes_ddg_redirect() {
        assert_eq!(
            decode_search_href("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&rut=1"),
            "https://example.com/a"
        );
        assert_eq!(
            decode_search_href("https://direct.example"),
            "https://direct.example"
        );
    }
}
