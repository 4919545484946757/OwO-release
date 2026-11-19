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
#[path = "tools/playbook.rs"]
mod playbook;
use command_tools::{KillShellTool, RunCommandTool, ShellOutputTool};
use mcp_adapters::{McpPromptAdapter, McpResourceAdapter, McpToolAdapter};
use playbook::{PlaybookCheckTool, PlaybookNextTool, PlaybookRecordTool, PlaybookRunTool};

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

    /// 任务剧本组（**可选能力**，默认不装配）：`playbook_next` / `playbook_record` /
    /// `playbook_check`。
    ///
    /// 开启后模型可以按剧本推进；`playbook_next` 只读，`playbook_check` 执行步骤声明的
    /// 命令（复用 run_command 的沙箱与审计），二者一起把"确定性校验"压成一次工具调用。
    pub fn register_playbook_tools(&mut self) {
        self.register_builtin(PlaybookNextTool);
        self.register_builtin(PlaybookRecordTool);
        self.register_builtin(PlaybookCheckTool);
        self.register_builtin(PlaybookRunTool);
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
#[path = "tools/registry_tests.rs"]
mod tests;
