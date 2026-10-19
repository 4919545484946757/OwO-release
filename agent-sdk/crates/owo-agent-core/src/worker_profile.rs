//! 角色画像（WorkerProfile，七期 · 二路）：把模板声明的角色权限真正作用到 Worker。
//!
//! - [`WorkerProfile::for_role`]：内置角色 → 工具面 / 只读 / 写白名单 / 回合上限 /
//!   浏览器 / 受控命令的映射（分析·审查·校验·抽取族 = 只读；实现族 = 读写 +
//!   受控命令；研究族 = 只读 + 浏览器；**未知角色默认只读**——权限默认 deny，
//!   显式匹配才有写面）；
//! - [`WorkerProfile::build_registry`]：按画像装配 [`ToolRegistry`]——**注册表面即
//!   权限边界**：读角色的注册表里根本没有写/执行工具，而非注册后靠审批拒绝；
//! - [`intersect_paths`]：角色写白名单 ∩ 团队绑定写白名单（任一侧为空 = 取非空一侧；
//!   两侧都空 = 工作区内可写，仍受审批约束）；
//! - `ProfileSubagentRunner` 与 `ContractSubagentRunner` 是 capability-resolution adapters；
//!   两者均把已解析的策略/工具/预算注入 core `WorkerRuntime`。Runtime 统一 Agent 回合、
//!   任务会话、取消、WorkerOutputV1 修复和逐请求用量，不推断角色或授予工具权限。

use crate::agent::TurnEvent;
use crate::tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 可选 Worker 回合事件回调；事件使用方应只记录安全元数据。
pub type TurnEventSink = Arc<dyn Fn(&TurnEvent) + Send + Sync>;

/// 画像回合上限（与 `SubagentRunner` 子代理口径一致：max_turns 硬上限 16）。
pub const PROFILE_MAX_TURNS_CAP: usize = 16;

/// 模板未声明该角色预算（`budget_calls == 0`）时的缺省回合上限。
pub const DEFAULT_PROFILE_MAX_TURNS: usize = 12;

/// 角色族（工具面装配依据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleFamily {
    /// 分析/审查/校验/抽取族：只读文件面（read_file / list_dir / search_files）。
    Read,
    /// 实现族：读写文件 + 搜索 + 受控命令。
    Implementer,
    /// 研究族：只读文件面 + 浏览器（搜索/导航/快照）。
    Researcher,
}

/// Whether a role name is one of the reserved finite TaskGraph writer slots.
pub fn is_parallel_writer_name(role: &str) -> bool {
    role.strip_prefix('w')
        .map(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false)
}

/// 角色权限画像：单角色的实际工具面与执行上限（七期 · 二路）。
///
/// 由模板角色名（`budget_calls_per_role[].role`）派生；服务端在每个 TeamRun 阶段
/// 重建注册表时逐角色实例化，最终写面 = 「团队绑定写白名单 ∩ 角色白名单」。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerProfile {
    /// 模型可见工具名（注册表按它裁剪；空 = 不额外过滤）。
    pub visible_tools: Vec<String>,
    /// 只读角色：注册表不含任何写/执行工具，`Policy::read_only` 再兜底一层。
    pub read_only: bool,
    /// 角色级写白名单（相对工作区根；空 = 交由团队绑定白名单决定）。
    pub write_allowed_paths: Vec<String>,
    /// 回合上限（模板 `budget_calls_per_role[].budget_calls`；硬上限 16）。
    pub max_turns: usize,
    /// 允许浏览器（搜索/导航/快照；写工作区变体不在可见面）。
    pub can_use_browser: bool,
    /// 允许受控命令（run_command；仍经沙箱 + 审批策略约束）。
    pub can_run_command: bool,
    /// Host-enforced timeout for the task's registered behavior command.
    #[serde(default)]
    pub verification_timeout_ms: Option<u64>,
    /// 模板声明了但被画像能力面剔除的工具名（诊断：声明与实际工具面不一致时可见）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_tools_dropped: Vec<String>,
}

impl WorkerProfile {
    /// Assemble the base profile for a Team role before any task is claimed.
    /// Parallel writer slots start with a writable tool ceiling so a later
    /// host-validated TaskGraph scope can narrow it; the task input must still
    /// explicitly grant write_file/apply_patch before those tools are exposed.
    pub fn for_team_role(
        role: &str,
        capabilities: &[String],
        budget_calls: usize,
        has_declared_write_scope: bool,
        is_parallel_writer_slot: bool,
    ) -> Self {
        if crate::workswarm::is_review_role(role, capabilities) {
            Self::for_role("reviewer", budget_calls)
        } else if has_declared_write_scope || is_parallel_writer_slot {
            Self::explicit_writer(budget_calls)
        } else {
            Self::for_role_with_capabilities(role, capabilities, budget_calls)
        }
    }

    /// Resolve the base profile for a persisted Team role in one shared location.
    pub fn for_team_role_spec(
        spec: &crate::workswarm::RoleSpec,
        budget_calls: usize,
        parallel: bool,
        template_id: Option<&str>,
    ) -> Self {
        let mut profile = Self::for_team_role(
            &spec.role,
            &spec.capabilities,
            budget_calls,
            !spec.write_paths.is_empty(),
            parallel && is_parallel_writer_name(&spec.role),
        );
        if template_id == Some(crate::builtin_team_templates::FULLSTACK_WEB_V1)
            && matches!(spec.role.as_str(), "w1" | "w2")
        {
            profile = profile.without_commands();
        }
        profile.write_allowed_paths = spec
            .write_paths
            .iter()
            .map(|path| path.trim().replace('\\', "/"))
            .collect();
        // 用户自定义工作流：模板声明的角色工具面作为**上界**收窄（空 = 不声明）。
        // 被剔除的声明不静默消失：记在画像里供诊断/测试断言。
        profile.declared_tools_dropped = profile.apply_declared_tools(&spec.tools);
        profile
    }

    /// Materialize validated role paths for this workspace. Writers without an explicit
    /// role path inherit the workspace root; reviewers and read-only roles get no write path.
    pub fn team_role_write_allowed_paths(
        spec: &crate::workswarm::RoleSpec,
        workspace_root: &Path,
        is_writer: bool,
    ) -> Result<Vec<PathBuf>, String> {
        crate::workswarm::validate_role_write_paths_with_capabilities(
            &spec.role,
            &spec.capabilities,
            &spec.write_paths,
        )?;
        if spec.write_paths.is_empty() && !is_writer {
            return Ok(Vec::new());
        }
        let workspace_root = canonicalize_scope_path(workspace_root)?;
        let mut paths = Vec::with_capacity(spec.write_paths.len());
        for raw in &spec.write_paths {
            let path = canonicalize_scope_path(&workspace_root.join(Path::new(raw.trim())))?;
            if !path.starts_with(&workspace_root) {
                return Err(format!("角色 {} 的写路径越出工作区：{raw}", spec.role));
            }
            paths.push(path);
        }
        if paths.is_empty() && is_writer {
            paths.push(workspace_root);
        }
        Ok(paths)
    }

    /// 内置角色 → 画像。`budget_calls` = 模板每角色调用预算（0 = 未声明 → 缺省 12）。
    ///
    /// 映射口径（与四类内置模板对齐）：
    /// - 实现族（名字含 `implementer` / `builder` / `finalizer` / `drafter`，如
    ///   code-change-v1 的 implementer、document-delivery-v1 的 drafter/finalizer）：
    ///   读写文件 + 搜索 + `run_command`；
    /// - 研究族（`researcher*` / 含 `research` / `brief_writer`，如 research-brief-v1
    ///   全角色）：只读文件 + 浏览器三件套（browser_search/navigate/snapshot）；
    /// - 其余（code_analyzer / reviewer / evidence_verifier / schema_validator /
    ///   extractor / artifact_formatter / critic 等分析·审查·校验·抽取族）：
    ///   只读文件面；未知角色同样落这里（默认 deny）。
    pub fn for_role_with_capabilities(
        role: &str,
        capabilities: &[String],
        budget_calls: usize,
    ) -> Self {
        if crate::workswarm::is_review_role(role, capabilities) {
            return Self::for_role("reviewer", budget_calls);
        }
        Self::for_role(role, budget_calls)
    }

    pub fn for_role(role: &str, budget_calls: usize) -> Self {
        let name = role.trim().to_ascii_lowercase();
        // 写角色关键词：实现/交付族。**必须包含通用 producer/writer/leader**——
        // 否则 `producer_role_name` 对未知分类返回的 "producer"（以及 forced team
        // 的 "leader"）会落进只读分支：团队"成功"却什么都没写（"草草了事"的根因）。
        // 注意判定顺序：researcher 分支在前，"brief_writer" 仍是只读研究角色。
        const WRITER_KEYWORDS: [&str; 7] = [
            "implementer",
            "builder",
            "finalizer",
            "drafter",
            "producer",
            "writer",
            "leader",
        ];
        let is_implementer = WRITER_KEYWORDS.iter().any(|k| name.contains(k));
        let is_researcher = name.starts_with("researcher")
            || name.contains("research")
            || name.contains("brief_writer");
        let max_turns = (if budget_calls == 0 {
            DEFAULT_PROFILE_MAX_TURNS
        } else {
            budget_calls
        })
        .clamp(1, PROFILE_MAX_TURNS_CAP);
        if is_researcher {
            // 研究族优先于写角色判定（brief_writer 含 "writer" 但只读）。
            Self {
                visible_tools: [
                    "read_file",
                    "list_dir",
                    "search_files",
                    "browser_search",
                    "browser_navigate",
                    "browser_snapshot",
                ]
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
                read_only: true,
                write_allowed_paths: Vec::new(),
                max_turns,
                can_use_browser: true,
                can_run_command: false,
                verification_timeout_ms: None,
                declared_tools_dropped: Vec::new(),
            }
        } else if is_implementer {
            Self {
                // 写角色的工具面：读写 + 搜索 + 执行（白名单写工具由
                // `build_registry` 装配；注册表面即权限边界）。
                visible_tools: [
                    "read_file",
                    "write_file",
                    "apply_patch",
                    "list_dir",
                    "search_files",
                    "run_command",
                ]
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
                read_only: false,
                write_allowed_paths: Vec::new(),
                max_turns,
                can_use_browser: false,
                can_run_command: true,
                verification_timeout_ms: None,
                declared_tools_dropped: Vec::new(),
            }
        } else {
            Self {
                visible_tools: ["read_file", "list_dir", "search_files"]
                    .iter()
                    .map(|tool| (*tool).to_string())
                    .collect(),
                read_only: true,
                write_allowed_paths: Vec::new(),
                max_turns,
                can_use_browser: false,
                can_run_command: false,
                verification_timeout_ms: None,
                declared_tools_dropped: Vec::new(),
            }
        }
    }

    /// 角色族。
    pub fn family(&self) -> RoleFamily {
        if !self.read_only {
            RoleFamily::Implementer
        } else if self.can_use_browser {
            RoleFamily::Researcher
        } else {
            RoleFamily::Read
        }
    }

    /// 是否写角色（范围写租约与变更追踪只作用于写角色）。
    pub fn is_writer(&self) -> bool {
        !self.read_only
    }

    /// 显式写角色画像（十一期 · 二路）：角色声明了写范围（`RoleSpec.write_paths`）
    /// 时使用——即使角色名未命中实现族关键词（如自定义 `w1`），也按实现族装配
    /// 工具面（读写 + 搜索 + 受控命令；注册表面即权限边界）；最终写面仍由
    /// 「角色 ∩ 团队绑定」白名单收窄。避免自定义角色名落进只读分支导致团队
    /// "成功"却零产出。
    pub fn explicit_writer(budget_calls: usize) -> Self {
        Self::for_role("implementer", budget_calls)
    }

    /// Convert a total TaskGraph request ceiling into Agent turns, keeping one
    /// request available for the WorkerOutputV1 correction path.
    pub fn task_agent_turn_cap(total_calls: usize) -> Result<usize, String> {
        if !(4..=PROFILE_MAX_TURNS_CAP).contains(&total_calls) {
            return Err("TaskGraph 单次尝试预算必须在 4..=16 次模型请求之间".to_string());
        }
        Ok(total_calls - 1)
    }

    /// Limit a TaskGraph attempt to its host-assigned total request budget. The final
    /// request remains available to the single WorkerOutputV1 correction path.
    pub fn with_task_model_call_budget(mut self, total_calls: usize) -> Result<Self, String> {
        self.max_turns = self.max_turns.min(Self::task_agent_turn_cap(total_calls)?);
        Ok(self)
    }

    /// Remove command execution while preserving the profile's file capabilities.
    pub fn without_commands(mut self) -> Self {
        self.can_run_command = false;
        self.visible_tools.retain(|tool| tool != "run_command");
        self
    }

    /// Apply a workspace/host read-only ceiling to the role and task profile.
    /// Keep explicitly allowed read tools only; an empty capability set remains empty.
    pub fn restricted_to_read_only(mut self) -> Self {
        self.read_only = true;
        self.can_run_command = false;
        self.write_allowed_paths.clear();
        self.visible_tools.retain(|tool| {
            matches!(
                tool.as_str(),
                "read_file"
                    | "list_dir"
                    | "search_files"
                    | "browser_search"
                    | "browser_navigate"
                    | "browser_snapshot"
                    | "__owo_no_task_tool__"
            )
        });
        if self.visible_tools.is_empty() {
            self.visible_tools.push("__owo_no_task_tool__".to_string());
        }
        self
    }

    /// Narrow a role profile to the host-resolved capabilities of one TaskGraph assignment.
    pub fn apply_task_capability_scope(
        &mut self,
        task: &crate::task_context::ResolvedTaskContext,
        task_has_no_write_scope: bool,
    ) {
        let Some(capabilities) = task.required_capabilities.as_deref() else {
            self.read_only = true;
            self.can_run_command = false;
            self.visible_tools
                .retain(|tool| matches!(tool.as_str(), "read_file" | "list_dir" | "search_files"));
            if self.visible_tools.is_empty() {
                self.visible_tools.push("__owo_no_task_tool__".to_string());
            }
            return;
        };
        let required = capabilities
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        self.verification_timeout_ms = task
            .verification
            .as_ref()
            .and_then(|plan| plan.get("requirements"))
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|requirement| {
                requirement
                    .get("validator_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("workspace-command-success-v1")
            })
            .filter_map(|requirement| {
                requirement
                    .pointer("/resources/timeout_ms")
                    .and_then(serde_json::Value::as_u64)
            })
            .min();
        let can_write_files = !task_has_no_write_scope
            && (required.contains("write_file") || required.contains("apply_patch"));
        self.can_run_command =
            self.can_run_command && can_write_files && required.contains("run_command");
        self.visible_tools.retain(|tool| {
            required.contains(tool.as_str()) && (tool != "run_command" || self.can_run_command)
        });
        if !can_write_files {
            self.read_only = true;
            self.visible_tools
                .retain(|tool| !matches!(tool.as_str(), "write_file" | "apply_patch"));
        }
        if self.visible_tools.is_empty() {
            self.visible_tools.push("__owo_no_task_tool__".to_string());
        }
    }

    /// Resolve a task-local write allowlist, intersecting it with the role/workspace scope.
    /// None means the task supplied no additional path constraint; Some([]) denies all writes.
    pub fn resolve_task_write_allowlist(
        task: &crate::task_context::ResolvedTaskContext,
        workspace_root: &Path,
        role_allowed: &[PathBuf],
    ) -> Result<Option<Vec<PathBuf>>, String> {
        let Some(paths) = task.write_paths.as_ref() else {
            return Ok(None);
        };
        let workspace_root = canonicalize_scope_path(workspace_root)
            .map_err(|error| format!("工作区写范围根目录无法解析：{error}"))?;
        let role_allowed = role_allowed
            .iter()
            .map(|base| canonicalize_scope_path(base))
            .collect::<Result<Vec<_>, _>>()?;
        if role_allowed
            .iter()
            .any(|base| !base.starts_with(&workspace_root))
        {
            return Err("角色写范围包含工作区之外的路径".to_string());
        }
        let mut resolved = Vec::with_capacity(paths.len());
        for raw in paths {
            let relative = Path::new(raw);
            if raw.trim().is_empty()
                || relative.is_absolute()
                || !relative
                    .components()
                    .any(|part| matches!(part, std::path::Component::Normal(_)))
                || relative.components().any(|part| {
                    !matches!(
                        part,
                        std::path::Component::Normal(_) | std::path::Component::CurDir
                    )
                })
            {
                return Err(format!("任务写范围路径无效：{raw}"));
            }
            let candidate = canonicalize_scope_path(&workspace_root.join(relative))?;
            if !candidate.starts_with(&workspace_root) {
                return Err(format!("任务写范围越出工作区：{raw}"));
            }
            if role_allowed.is_empty() {
                resolved.push(candidate);
                continue;
            }
            let before = resolved.len();
            for base in &role_allowed {
                if candidate.starts_with(base) {
                    resolved.push(candidate.clone());
                } else if base.starts_with(&candidate) {
                    resolved.push(base.clone());
                }
            }
            if resolved.len() == before {
                return Err(format!("任务写范围超出角色/团队范围：{raw}"));
            }
        }
        resolved.sort();
        resolved.dedup();
        Ok(Some(resolved))
    }

    /// Render authorized paths relative to the workspace without exposing outside paths.
    pub fn workspace_relative_write_paths(workspace_root: &Path, paths: &[PathBuf]) -> Vec<String> {
        let Ok(root) = canonicalize_scope_path(workspace_root) else {
            return vec!["<outside-workspace-denied>".to_string(); paths.len()];
        };
        paths
            .iter()
            .map(|path| {
                let Ok(normalized) = canonicalize_scope_path(path) else {
                    return "<outside-workspace-denied>".to_string();
                };
                if normalized == root {
                    return ".".to_string();
                }
                normalized
                    .strip_prefix(&root)
                    .ok()
                    .map(|relative| {
                        relative
                            .components()
                            .filter_map(|component| match component {
                                std::path::Component::Normal(part) => Some(part.to_string_lossy()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .filter(|relative| !relative.is_empty())
                    .unwrap_or_else(|| "<outside-workspace-denied>".to_string())
            })
            .collect()
    }

    /// 应用模板声明的角色工具面（用户自定义工作流）——**只收窄，不放宽**。
    ///
    /// - `declared` 为空：不声明，保持画像缺省工具面（返回值空）；
    /// - 否则实际可见面 = `declared ∩ 画像本可暴露的面`：只读评审角色即使声明
    ///   `run_command` 也拿不到；不在能力面内的声明被剔除并**返回**（调用方/测试可断言，
    ///   运行期记入 [`Self::declared_tools_dropped`]）；
    /// - 全部声明都不在能力面内时，可见面置为"无任务工具"哨兵：宁可什么都不给，
    ///   也不退回"不过滤"（那会凭空放大权限）。
    pub fn apply_declared_tools(&mut self, declared: &[String]) -> Vec<String> {
        if declared.is_empty() {
            return Vec::new();
        }
        let surface = self.base_registry_names();
        let mut kept: Vec<String> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();
        for tool in declared {
            let tool = tool.trim();
            if tool.is_empty() {
                continue;
            }
            if surface.iter().any(|name| name == tool) {
                if !kept.iter().any(|name| name == tool) {
                    kept.push(tool.to_string());
                }
            } else if !dropped.iter().any(|name| name == tool) {
                dropped.push(tool.to_string());
            }
        }
        self.visible_tools = if kept.is_empty() {
            vec!["__owo_no_task_tool__".to_string()]
        } else {
            kept
        };
        dropped
    }

    /// 画像在**未按 `visible_tools` 裁剪**时能暴露的工具名（能力面上界）。
    fn base_registry_names(&self) -> Vec<String> {
        let mut registry = ToolRegistry::empty();
        registry.register_file_read_tools();
        if !self.read_only {
            registry.register_whitelist_write_file(Vec::new());
            registry.register_whitelist_apply_patch(Vec::new());
        }
        if self.can_run_command {
            registry.register_run_command();
        }
        if self.can_use_browser {
            registry.register_browser_tools();
        }
        registry.specs().into_iter().map(|spec| spec.name).collect()
    }

    /// 按画像装配工具注册表：注册表面即权限边界。
    ///
    /// - `write_allowed`：最终写白名单（角色 ∩ 团队绑定，见 [`intersect_paths`]；
    ///   空 = 工作区内可写）。只读角色忽略该参数（不注册任何写工具）。
    /// - `visible_tools` 非空时按名单裁剪，保证「实际可见工具 == 模板/画像声明」
    ///   （浏览器组里的写工作区变体也会被裁掉）。
    pub fn build_registry(&self, write_allowed: Vec<PathBuf>) -> ToolRegistry {
        let mut registry = ToolRegistry::empty();
        registry.register_file_read_tools();
        if !self.read_only {
            registry.register_whitelist_write_file(write_allowed.clone());
            registry.register_whitelist_apply_patch(write_allowed);
        }
        if self.can_run_command {
            registry.register_run_command();
        }
        if self.can_use_browser {
            registry.register_browser_tools();
        }
        if !self.visible_tools.is_empty() {
            registry.retain_names(&self.visible_tools);
        }
        registry
    }

    /// 角色 Prompt 的「禁止执行/边界」行（八期 · 一路）：按族与工具面派生，
    /// 供 [`crate::team_prompt::compile_prompt_with_profile`] 使用——Prompt 声称的能力边界与
    /// `build_registry` 装配的真实工具面一致（注册表面即权限边界的 Prompt 侧投影）。
    pub fn prompt_guard_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.visible_tools.is_empty() {
            lines
                .push("工具面未声明：仅可使用缺省只读文件面，禁止任何写入/命令/联网。".to_string());
        } else {
            lines.push(format!(
                "可见工具仅限：{}（工具面之外没有其他执行手段）。",
                self.visible_tools.join(" / ")
            ));
        }
        if self.read_only {
            lines.push(
                "禁止写入工作区文件（你是只读角色）：交付与结论一律通过产物正文/评审声明完成。"
                    .to_string(),
            );
        } else {
            lines.push(
                "只允许在允许写路径内用 write_file 或 apply_patch 落盘最终变更；精确补丁应基于 read_file 返回的 sha256 传 expected_hashes；禁止改写白名单外文件，\
                 禁止把变更只留在说明里而不落盘。"
                    .to_string(),
            );
            if !self.write_allowed_paths.is_empty() {
                lines.push(format!(
                    "本次最终写入路径白名单（相对工作区根）：{}。",
                    self.write_allowed_paths.join(" / ")
                ));
            }
        }
        if !self.can_run_command {
            lines.push("禁止执行命令（run_command 不在你的工具面）。".to_string());
        }
        if !self.can_use_browser {
            lines.push("禁止联网浏览（浏览器工具不在你的工具面）。".to_string());
        }
        lines
    }
}

fn canonicalize_scope_path(path: &Path) -> Result<PathBuf, String> {
    let mut current = path.to_path_buf();
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(_) => {
                let canonical = current
                    .canonicalize()
                    .map_err(|error| format!("{}：{error}", current.display()))?;
                let mut canonical = normalize_canonical_path(canonical);
                for part in suffix.iter().rev() {
                    canonical.push(part);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = current.file_name().map(std::ffi::OsString::from) else {
                    return Err(format!("{}：没有可解析的父路径", path.display()));
                };
                suffix.push(name);
                if !current.pop() {
                    return Err(format!("{}：没有可解析的父路径", path.display()));
                }
            }
            Err(error) => return Err(format!("{}：{error}", current.display())),
        }
    }
}

#[cfg(windows)]
fn normalize_canonical_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    let Some(without_prefix) = text.strip_prefix(r"\\?\") else {
        return path;
    };
    if let Some(unc) = without_prefix.strip_prefix("UNC\\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else {
        PathBuf::from(without_prefix)
    }
}

#[cfg(not(windows))]
fn normalize_canonical_path(path: PathBuf) -> PathBuf {
    path
}

/// 路径白名单交集（七期 · 二路）：角色写白名单 ∩ 团队绑定写白名单。
///
/// - 任一侧为空 = 取非空一侧（空表示「未额外约束」）；
/// - 两侧都空 = 空（工作区内可写，仍受审批约束）；
/// - 否则逐条收窄：a 的条目落在 b 某前缀内 → 保留该条目（更窄者）；b 某前缀落在
///   a 条目内 → 保留该 b 前缀（窄者胜，交集语义）。
pub fn intersect_paths(a: &[PathBuf], b: &[PathBuf]) -> Vec<PathBuf> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    let mut intersection = Vec::new();
    for path in a {
        if b.iter().any(|base| path.starts_with(base)) {
            intersection.push(path.clone());
        } else if let Some(narrower) = b.iter().find(|base| base.starts_with(path)) {
            intersection.push(narrower.clone());
        }
    }
    intersection
}

/// Compile the shared Worker system prompt used by production and product evaluation.
pub fn compile_worker_system_prompt(
    profile: &WorkerProfile,
    is_review_role: bool,
    budget_note: &str,
    extra_system_prompt: Option<&str>,
) -> String {
    let base_prompt = if is_review_role {
        "你是只读评审子代理：critic 不得提交 artifact；只能读取/搜索工作区文件，禁止写入或执行命令；独立检查交付并简洁汇报发现。\n"
    } else if profile.is_writer() {
        "你是写角色子代理：凡涉及代码/文件变更，必须在允许路径内真实落盘：小范围修改优先用 apply_patch，并传入 read_file 返回的 sha256 作为 expected_hashes；整文件生成或确需重写时使用 write_file（工具面之外没有其他写入手段）；artifact.content 只写变更说明、影响面与验证方式，不要把完整变更只放在 artifact 里而不落盘。回合预算有限：先做必要读取并完成写入；仅当任务验收需要且权限允许时，运行范围明确的定向检查，避免重复读取和全仓构建。最后一个回合只输出契约 JSON，不再调用工具。工具调用仍需审批；无法验证时如实说明。\n"
    } else {
        "你是通用子代理：独立完成委派任务，工具调用仍需审批，完成后汇报结果。\n"
    };
    format!(
        "{}{base_prompt}{budget_note}{}",
        extra_system_prompt.unwrap_or_default(),
        crate::workswarm_output::contract_system_prompt(is_review_role)
    )
}

#[path = "worker_profile/runner.rs"]
mod runner;
pub use runner::{ProfileSubagentRunError, ProfileSubagentRunReport, ProfileSubagentRunner};

#[cfg(test)]
mod tests {
    #[test]
    fn declared_tools_narrow_the_face_but_never_widen_it() {
        use crate::workswarm::RoleSpec;

        // 写角色：声明 read_file + write_file + run_command 之外的未知工具 → 只保留能力面内的。
        let mut writer = WorkerProfile::for_team_role("impl", &[], 4, true, false);
        let dropped = writer.apply_declared_tools(&[
            "read_file".to_string(),
            "write_file".to_string(),
            "web_search".to_string(),
        ]);
        assert!(writer.visible_tools.iter().any(|tool| tool == "read_file"));
        assert!(writer.visible_tools.iter().any(|tool| tool == "write_file"));
        assert_eq!(
            dropped,
            vec!["web_search".to_string()],
            "能力面外声明应被剔除并报出"
        );

        // 只读评审角色：声明 run_command/write_file 也拿不到（只收窄不放宽）。
        let mut reviewer =
            WorkerProfile::for_team_role("reviewer", &["review".to_string()], 4, false, false);
        let dropped = reviewer.apply_declared_tools(&[
            "read_file".to_string(),
            "run_command".to_string(),
            "write_file".to_string(),
        ]);
        assert_eq!(reviewer.visible_tools, vec!["read_file".to_string()]);
        assert!(dropped.contains(&"run_command".to_string()), "{dropped:?}");
        assert!(dropped.contains(&"write_file".to_string()), "{dropped:?}");

        // 声明全部不在能力面内 → 无任务工具哨兵（不退回"不过滤"）。
        let mut hopeless =
            WorkerProfile::for_team_role("reviewer", &["review".to_string()], 4, false, false);
        let dropped = hopeless.apply_declared_tools(&["run_command".to_string()]);
        assert_eq!(
            hopeless.visible_tools,
            vec!["__owo_no_task_tool__".to_string()]
        );
        assert_eq!(dropped, vec!["run_command".to_string()]);

        // 空声明 = 不声明，画像不变。
        let mut untouched = WorkerProfile::for_team_role("impl", &[], 4, true, false);
        let before = untouched.visible_tools.clone();
        assert!(untouched.apply_declared_tools(&[]).is_empty());
        assert_eq!(untouched.visible_tools, before);

        // RoleSpec 路径：声明通过 for_team_role_spec 落到画像，并记录剔除项。
        let mut spec = RoleSpec::agent("impl");
        spec.tools = vec!["read_file".to_string(), "run_command".to_string()];
        let profile = WorkerProfile::for_team_role_spec(&spec, 4, false, None);
        assert!(profile.visible_tools.iter().any(|tool| tool == "read_file"));
        assert!(
            !profile
                .visible_tools
                .iter()
                .any(|tool| tool == "run_command"),
            "普通写角色默认无命令能力，声明 run_command 不得凭空放权：{:?}",
            profile.visible_tools
        );
        assert_eq!(
            profile.declared_tools_dropped,
            vec!["run_command".to_string()]
        );
    }

    use super::*;

    fn tool_names(registry: &ToolRegistry) -> Vec<String> {
        registry
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect()
    }

    #[test]
    fn implementer_gets_write_and_command_surface() {
        let profile = WorkerProfile::for_role("implementer", 5);
        assert_eq!(profile.family(), RoleFamily::Implementer);
        assert!(profile.is_writer());
        assert!(profile.can_run_command);
        assert_eq!(profile.max_turns, 5);
        let names = tool_names(&profile.build_registry(Vec::new()));
        for expected in [
            "read_file",
            "write_file",
            "apply_patch",
            "list_dir",
            "search_files",
            "run_command",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "implementer 缺工具 {expected}：{names:?}"
            );
        }
        assert_eq!(names.len(), 6, "implementer 不应有多余工具：{names:?}");
    }

    #[test]
    fn team_role_spec_builder_is_shared_for_writer_review_and_template_rules() {
        let ordinary_slot = crate::workswarm::RoleSpec::agent("w1");
        assert!(WorkerProfile::for_team_role_spec(&ordinary_slot, 4, false, None).read_only);
        assert!(!WorkerProfile::for_team_role_spec(&ordinary_slot, 4, true, None).read_only);
        let readonly_role = crate::workswarm::RoleSpec::agent("content_reviewer");
        assert!(WorkerProfile::team_role_write_allowed_paths(
            &readonly_role,
            Path::new("nonexistent-eval-workspace"),
            false
        )
        .unwrap()
        .is_empty());

        let mut scoped = crate::workswarm::RoleSpec::agent("frontend_engineer");
        scoped.write_paths = vec!["apps/web".to_string()];
        let profile = WorkerProfile::for_team_role_spec(&scoped, 6, false, None);
        assert!(profile.is_writer());
        assert_eq!(profile.write_allowed_paths, vec!["apps/web"]);
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("apps/web")).unwrap();
        let expected = super::canonicalize_scope_path(&workspace.path().join("apps/web")).unwrap();
        assert_eq!(
            WorkerProfile::team_role_write_allowed_paths(&scoped, workspace.path(), true).unwrap(),
            vec![expected]
        );

        let fullstack_worker = WorkerProfile::for_team_role_spec(
            &crate::workswarm::RoleSpec {
                role: "w1".to_string(),
                write_paths: vec!["apps/web".to_string()],
                ..Default::default()
            },
            4,
            false,
            Some(crate::builtin_team_templates::FULLSTACK_WEB_V1),
        );
        assert!(!fullstack_worker.can_run_command);

        let mut reviewer = crate::workswarm::RoleSpec::agent("reviewer");
        reviewer.write_paths = vec!["src".to_string()];
        assert!(WorkerProfile::team_role_write_allowed_paths(
            &reviewer,
            Path::new("workspace"),
            false
        )
        .is_err());
    }

    #[test]
    fn team_parallel_writer_slots_start_writable_but_review_roles_stay_read_only() {
        assert!(is_parallel_writer_name("w1"));
        assert!(is_parallel_writer_name("w12"));
        assert!(!is_parallel_writer_name("writer1"));
        assert!(!is_parallel_writer_name("w1x"));

        let writer = WorkerProfile::for_team_role("w1", &[], 4, false, true);
        assert!(!writer.read_only);
        assert!(writer.visible_tools.iter().any(|tool| tool == "write_file"));

        let ordinary = WorkerProfile::for_team_role("w1", &[], 4, false, false);
        assert!(ordinary.read_only);
        assert!(!ordinary
            .visible_tools
            .iter()
            .any(|tool| tool == "write_file"));

        let scoped_custom = WorkerProfile::for_team_role("frontend_engineer", &[], 4, true, false);
        assert!(!scoped_custom.read_only);
        assert!(scoped_custom
            .visible_tools
            .iter()
            .any(|tool| tool == "write_file"));

        let reviewer = WorkerProfile::for_team_role("w1", &["review".to_string()], 4, false, true);
        assert!(reviewer.read_only);
        assert!(!reviewer
            .visible_tools
            .iter()
            .any(|tool| tool == "write_file"));
    }

    #[test]
    fn generic_producer_writer_leader_roles_are_writers() {
        // 回归：`producer_role_name` 对未知分类返回 "producer"，forced team 用
        // "leader"，文档族用 "writer"——这些都必须能落盘，否则团队"成功"却没产出。
        for role in ["producer", "writer", "leader", "builder", "implementer"] {
            let profile = WorkerProfile::for_role(role, 6);
            assert!(profile.is_writer(), "{role} 应为写角色");
            assert!(profile.can_run_command, "{role} 应可执行命令");
            let names = tool_names(&profile.build_registry(Vec::new()));
            assert!(
                names.iter().any(|name| name == "write_file"),
                "{role} 注册表缺 write_file：{names:?}"
            );
        }
        // brief_writer 仍走研究族（只读），不被 "writer" 关键词误判为写角色。
        let brief = WorkerProfile::for_role("brief_writer", 6);
        assert!(brief.read_only, "brief_writer 必须保持只读");
        assert!(!brief.is_writer(), "brief_writer 不应是写角色");
    }

    #[test]
    fn source_writer_profile_removes_command_tool_but_keeps_scoped_file_writes() {
        let profile = WorkerProfile::explicit_writer(8).without_commands();
        assert!(!profile.can_run_command);
        assert!(!profile
            .visible_tools
            .iter()
            .any(|tool| tool == "run_command"));
        assert!(profile
            .visible_tools
            .iter()
            .any(|tool| tool == "write_file"));
        assert!(profile
            .visible_tools
            .iter()
            .any(|tool| tool == "apply_patch"));
        assert!(profile.is_writer());
    }

    #[test]
    fn prompt_guard_shows_the_final_workspace_write_allowlist() {
        let mut profile = WorkerProfile::explicit_writer(4);
        profile.write_allowed_paths = vec!["apps/api".to_string(), "src/lib.rs".to_string()];
        let prompt = profile.prompt_guard_lines().join("\n");
        assert!(prompt.contains("本次最终写入路径白名单（相对工作区根）：apps/api / src/lib.rs"));
    }

    #[test]
    fn workspace_read_only_ceiling_removes_write_and_command_tools() {
        let mut profile = WorkerProfile::explicit_writer(5);
        profile.visible_tools.extend([
            "browser_search".to_string(),
            "browser_navigate".to_string(),
            "browser_snapshot".to_string(),
        ]);
        profile.write_allowed_paths = vec!["src".to_string()];
        let profile = profile.restricted_to_read_only();
        assert!(profile.read_only);
        assert!(!profile.can_run_command);
        assert!(profile.write_allowed_paths.is_empty());
        assert!(profile.visible_tools.iter().any(|tool| tool == "read_file"));
        assert!(profile
            .visible_tools
            .iter()
            .any(|tool| tool == "browser_search"));
        assert!(!profile
            .visible_tools
            .iter()
            .any(|tool| tool == "write_file"));
        assert!(!profile
            .visible_tools
            .iter()
            .any(|tool| tool == "apply_patch"));
        assert!(!profile
            .visible_tools
            .iter()
            .any(|tool| tool == "run_command"));
        let mut empty = WorkerProfile::explicit_writer(1);
        empty.visible_tools.clear();
        let empty = empty.restricted_to_read_only();
        assert_eq!(empty.visible_tools, vec!["__owo_no_task_tool__"]);
    }

    #[test]
    fn task_graph_capability_resolution_is_fail_closed_and_runtime_ready() {
        let mut writer = WorkerProfile::explicit_writer(6);
        let read_only_task = crate::task_context::ResolvedTaskContext {
            origin: crate::task_context::TaskContextOrigin::TaskGraph,
            required_capabilities: Some(vec![
                "read_file".into(),
                "search_files".into(),
                "run_command".into(),
            ]),
            ..Default::default()
        };
        writer.apply_task_capability_scope(&read_only_task, true);
        assert!(writer.read_only);
        assert!(!writer.can_run_command);
        assert!(writer.visible_tools.iter().any(|tool| tool == "read_file"));
        assert!(!writer
            .visible_tools
            .iter()
            .any(|tool| tool == "run_command"));
        assert!(!writer.visible_tools.iter().any(|tool| tool == "write_file"));

        let mut empty = WorkerProfile::explicit_writer(3);
        let empty_task = crate::task_context::ResolvedTaskContext {
            origin: crate::task_context::TaskContextOrigin::TaskGraph,
            required_capabilities: Some(Vec::new()),
            ..Default::default()
        };
        empty.apply_task_capability_scope(&empty_task, true);
        assert_eq!(empty.visible_tools, vec!["__owo_no_task_tool__"]);
        assert!(!empty.can_run_command);
    }

    #[test]
    fn task_write_scope_intersects_parent_and_child_role_paths() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src/api")).unwrap();
        std::fs::write(workspace.path().join("src/api/main.rs"), "fn main() {} ").unwrap();
        let task = crate::task_context::ResolvedTaskContext {
            origin: crate::task_context::TaskContextOrigin::TaskGraph,
            write_paths: Some(vec!["src".into()]),
            ..Default::default()
        };
        let role_scope = vec![workspace.path().join("src/api/main.rs")];
        let resolved =
            WorkerProfile::resolve_task_write_allowlist(&task, workspace.path(), &role_scope)
                .unwrap()
                .unwrap();
        assert_eq!(resolved, role_scope);
        assert_eq!(
            WorkerProfile::workspace_relative_write_paths(workspace.path(), &resolved),
            vec!["src/api/main.rs"]
        );

        let escaped = crate::task_context::ResolvedTaskContext {
            write_paths: Some(vec!["../outside".into()]),
            ..task.clone()
        };
        assert!(
            WorkerProfile::resolve_task_write_allowlist(&escaped, workspace.path(), &[]).is_err()
        );
        let disjoint = vec![workspace.path().join("docs")];
        assert!(
            WorkerProfile::resolve_task_write_allowlist(&task, workspace.path(), &disjoint)
                .is_err()
        );
    }

    #[test]
    fn explicit_writer_profile_overrides_read_only_role_names() {
        // 十一期：声明了写范围的自定义角色（w1/w2…）不能落进只读分支。
        for role in ["w1", "module-b", "some_future_role"] {
            assert!(
                !WorkerProfile::for_role(role, 0).is_writer(),
                "{role} 无名命中 → 缺省只读（权限默认 deny）"
            );
            let profile = WorkerProfile::explicit_writer(4);
            assert!(profile.is_writer(), "{role} 显式写角色应为写面");
            assert_eq!(profile.max_turns, 4, "预算应透传");
            assert!(profile.can_run_command, "写角色应可执行命令");
            let names = tool_names(&profile.build_registry(Vec::new()));
            assert!(
                names.iter().any(|name| name == "write_file"),
                "{role} 显式写角色注册表缺 write_file：{names:?}"
            );
        }
    }

    #[test]
    fn explicit_review_capability_keeps_custom_role_read_only() {
        let profile =
            WorkerProfile::for_role_with_capabilities("quality_gate", &["review".to_string()], 4);
        assert!(profile.read_only);
        assert!(!profile.is_writer());
        assert_eq!(profile.max_turns, 4);
        let names = tool_names(&profile.build_registry(Vec::new()));
        assert!(!names.iter().any(|name| name == "write_file"));
        assert!(!names.iter().any(|name| name == "run_command"));
    }

    #[test]
    fn writer_prompt_recommends_hash_guarded_patch_for_small_changes() {
        let profile = WorkerProfile::for_role("implementer", 5);
        let lines = profile.prompt_guard_lines().join("\n");
        assert!(lines.contains("apply_patch"));
        assert!(lines.contains("expected_hashes"));
        assert!(lines.contains("read_file 返回的 sha256"));

        let read_only = WorkerProfile::for_role("reviewer", 5);
        let read_only_lines = read_only.prompt_guard_lines().join("\n");
        assert!(!read_only_lines.contains("apply_patch"));
        assert!(read_only_lines.contains("禁止写入工作区文件"));
    }

    #[test]
    fn implementer_write_tool_is_whitelist_wrapped_but_names_unchanged() {
        // 白名单包装不改变工具名（模型可见面不变），仅在执行时做前缀校验。
        let profile = WorkerProfile::for_role("implementer", 5);
        let allowed = vec![PathBuf::from("T:/ws/src")];
        let names = tool_names(&profile.build_registry(allowed));
        assert!(names.iter().any(|name| name == "write_file"));
        assert!(names.iter().any(|name| name == "apply_patch"));
    }

    #[test]
    fn read_roles_cannot_write_or_run_commands() {
        for role in [
            "code_analyzer",
            "reviewer",
            "evidence_verifier",
            "schema_validator",
            "extractor",
            "artifact_formatter",
            "content_reviewer",
            // 未知角色默认只读（权限默认 deny）。
            "some_future_role",
        ] {
            let profile = WorkerProfile::for_role(role, 3);
            assert_eq!(profile.family(), RoleFamily::Read, "{role} 应为只读读面");
            assert!(!profile.is_writer(), "{role} 不应是写角色");
            assert!(!profile.can_run_command, "{role} 不应允许 run_command");
            assert!(!profile.can_use_browser, "{role} 不应允许浏览器");
            let names = tool_names(&profile.build_registry(Vec::new()));
            assert!(
                !names.iter().any(|name| name == "write_file"),
                "{role} 注册表含 write_file：{names:?}"
            );
            assert!(
                !names.iter().any(|name| name == "run_command"),
                "{role} 注册表含 run_command：{names:?}"
            );
            assert!(
                !names.iter().any(|name| name.starts_with("browser_")),
                "{role} 注册表含浏览器工具：{names:?}"
            );
            assert_eq!(names.len(), 3, "{role} 工具面应恰为读三件套：{names:?}");
        }
    }

    #[test]
    fn researcher_family_gets_browser_without_write() {
        for role in ["researcher_a", "researcher_b", "brief_writer"] {
            let profile = WorkerProfile::for_role(role, 4);
            assert_eq!(profile.family(), RoleFamily::Researcher, "{role}");
            assert!(profile.read_only);
            assert!(profile.can_use_browser);
            let names = tool_names(&profile.build_registry(Vec::new()));
            assert!(
                !names.iter().any(|name| name == "write_file"),
                "{role} 注册表含 write_file：{names:?}"
            );
            assert!(
                !names.iter().any(|name| name == "run_command"),
                "{role} 注册表含 run_command：{names:?}"
            );
            for expected in ["browser_search", "browser_navigate", "browser_snapshot"] {
                assert!(
                    names.iter().any(|name| name == expected),
                    "{role} 缺 {expected}：{names:?}"
                );
            }
            // 浏览器组里的写工作区变体必须在可见面裁剪后消失。
            assert!(
                !names.iter().any(|name| name == "browser_screenshot"),
                "{role} 不应含写工作区变体 browser_screenshot：{names:?}"
            );
            assert!(
                !names.iter().any(|name| name == "browser_download_image"),
                "{role} 不应含写工作区变体 browser_download_image：{names:?}"
            );
        }
    }

    #[test]
    fn compiled_worker_system_prompt_uses_shared_role_contract() {
        let writer = WorkerProfile::explicit_writer(4);
        let writer_prompt = compile_worker_system_prompt(&writer, false, "budget", Some("host"));
        assert!(writer_prompt.starts_with("host"));
        assert!(writer_prompt.contains("apply_patch"));
        assert!(writer_prompt.contains("write_file"));
        assert!(writer_prompt.contains("无法验证时如实说明"));

        let reviewer = WorkerProfile::for_role("reviewer", 4);
        let review_prompt = compile_worker_system_prompt(&reviewer, true, "budget", None);
        assert!(review_prompt.contains("只读评审子代理"));
        assert!(review_prompt.contains("critic 不得提交 artifact"));
        assert!(!review_prompt.contains("必须在允许路径内真实落盘"));
    }

    #[test]
    fn task_total_call_budget_reserves_one_output_repair_request() {
        let profile = WorkerProfile::for_role("implementer", 12)
            .with_task_model_call_budget(5)
            .unwrap();
        assert_eq!(profile.max_turns, 4);
        assert_eq!(WorkerProfile::task_agent_turn_cap(5).unwrap(), 4);
        assert!(WorkerProfile::task_agent_turn_cap(3).is_err());
        assert!(WorkerProfile::for_role("implementer", 12)
            .with_task_model_call_budget(2)
            .is_err());
    }

    #[test]
    fn budget_maps_to_turn_cap() {
        assert_eq!(
            WorkerProfile::for_role("implementer", 0).max_turns,
            DEFAULT_PROFILE_MAX_TURNS,
            "未声明预算 → 缺省"
        );
        assert_eq!(WorkerProfile::for_role("implementer", 5).max_turns, 5);
        assert_eq!(
            WorkerProfile::for_role("implementer", 99).max_turns,
            PROFILE_MAX_TURNS_CAP,
            "预算超硬上限 → 截到 16"
        );
    }

    #[test]
    fn prompt_guard_lines_match_tool_surface() {
        // 写角色：写面 + 命令提示，无只读禁令。
        let writer = WorkerProfile::for_role("implementer", 5);
        let writer_lines = writer.prompt_guard_lines().join("\n");
        assert!(writer_lines.contains("write_file"));
        assert!(writer_lines.contains("apply_patch"));
        assert!(writer_lines.contains("sha256"));
        assert!(writer_lines.contains("允许写路径"));
        assert!(!writer_lines.contains("禁止写入工作区文件"));
        assert!(writer_lines.contains("禁止联网浏览"));
        // 读角色：只读禁令 + 无命令/浏览器。
        let reader = WorkerProfile::for_role("reviewer", 3);
        let reader_lines = reader.prompt_guard_lines().join("\n");
        assert!(reader_lines.contains("禁止写入工作区文件"));
        assert!(reader_lines.contains("禁止执行命令"));
        assert!(reader_lines.contains("禁止联网浏览"));
        assert!(reader_lines.contains("read_file"));
        // 研究族：浏览器放开，仍只读。
        let researcher = WorkerProfile::for_role("researcher_a", 4);
        let researcher_lines = researcher.prompt_guard_lines().join("\n");
        assert!(researcher_lines.contains("browser_search"));
        assert!(!researcher_lines.contains("禁止联网浏览"));
        assert!(researcher_lines.contains("禁止写入工作区文件"));
        // 未声明工具面（缺省画像不会出现，防御性分支仍如实告知）。
        let empty = WorkerProfile {
            visible_tools: Vec::new(),
            read_only: true,
            write_allowed_paths: Vec::new(),
            max_turns: 3,
            can_use_browser: false,
            can_run_command: false,
            verification_timeout_ms: None,
            declared_tools_dropped: Vec::new(),
        };
        assert!(empty.prompt_guard_lines()[0].contains("未声明"));
    }

    #[test]
    fn intersect_paths_semantics() {
        let src = PathBuf::from("T:/ws/src");
        let lib = PathBuf::from("T:/ws/src/lib");
        let docs = PathBuf::from("T:/ws/docs");
        // 两侧都空 = 空（工作区内可写，仍受审批约束）。
        assert!(intersect_paths(&[], &[]).is_empty());
        // 任一侧空 = 取非空一侧。（四路集成微修：clippy 冗余 clone → std::slice::from_ref 借用）
        assert_eq!(
            intersect_paths(std::slice::from_ref(&src), &[]),
            vec![src.clone()]
        );
        assert_eq!(
            intersect_paths(&[], std::slice::from_ref(&docs)),
            vec![docs.clone()]
        );
        // 窄者胜：profile(src) ∩ scope(src/lib) = src/lib。
        assert_eq!(
            intersect_paths(std::slice::from_ref(&src), std::slice::from_ref(&lib)),
            vec![lib.clone()]
        );
        // 不相交 = 空。
        assert!(
            intersect_paths(std::slice::from_ref(&docs), std::slice::from_ref(&src)).is_empty()
        );
        // 多条目：相交的留下，不相交的丢弃。
        let got = intersect_paths(&[src.clone(), docs.clone()], &[lib.clone(), docs.clone()]);
        assert_eq!(got, vec![lib.clone(), docs.clone()]);
    }
}
