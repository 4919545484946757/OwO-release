//! 用户自定义工作流的校验与规范化（一份实现，UI/CLI/AI 生成共用）。
//!
//! 目标：让"任意特定功能的 Agent 工作流"能被**结构化配置**表达，同时在落盘/执行前
//! 把不安全或跑不起来的配置挡下来。校验是纯函数、错误一次性全量返回（便于 UI 逐条提示）。
//!
//! 边界（不做的事）：不做语义好坏判断（那属于执行反馈/AFlow 式搜索），也不替用户
//! 决定角色拆分；只保证：可解析、可调度（DAG 无环）、权限面不越界、工具名认得。

use owo_agent_protocol::{TeamMode, TeamTemplate};

/// 单个模板的角色数上限（超过基本无法审阅，也会把交接面撑爆）。
pub const MAX_ROLES: usize = 12;
/// 单个角色的工具白名单上限。
pub const MAX_ROLE_TOOLS: usize = 32;
/// 名称/适用条件长度上限。
pub const MAX_NAME_CHARS: usize = 64;
/// 描述长度上限。
pub const MAX_DESCRIPTION_CHARS: usize = 400;

/// 由名字生成安全的模板 id：小写 ASCII 字母数字，非字母数字折叠为 `-`。
///
/// 中文等非 ASCII 字符无法直接进 id（会进文件名与路由），返回 `None`，调用方应让
/// 用户显式提供 id（UI 侧可提示）。
pub fn slugify_template_id(name: &str) -> Option<String> {
    let mut slug = String::new();
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if matches!(character, '-' | '_' | ' ' | '.') && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() || slug.chars().count() > MAX_NAME_CHARS {
        return None;
    }
    if slug.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        // 纯数字/数字开头不利于可读性，也容易与旧数据混淆。
        return None;
    }
    Some(slug)
}

/// 校验模板 id 是否安全（用户显式提供时）。
pub fn validate_template_id(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("template_id 不能为空".to_string());
    }
    if trimmed.chars().count() > MAX_NAME_CHARS {
        return Err(format!("template_id 过长（>{MAX_NAME_CHARS}）"));
    }
    if trimmed.contains("..") || trimmed.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(format!("template_id 含路径/保留字符：{trimmed}"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "template_id 只允许 ASCII 字母数字与 -_.：{trimmed}"
        ));
    }
    if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Err(format!("template_id 不得以数字开头：{trimmed}"));
    }
    Ok(())
}

/// 全量校验（错误一次性返回；空 Vec 表示通过）。
pub fn validate_workflow_template(template: &TeamTemplate) -> Vec<String> {
    let mut errors = Vec::new();

    if let Err(error) = validate_template_id(&template.template_id) {
        errors.push(error);
    }
    if template.name.trim().is_empty() {
        errors.push("name 不能为空".to_string());
    } else if template.name.chars().count() > MAX_NAME_CHARS {
        errors.push(format!("name 过长（>{MAX_NAME_CHARS}）"));
    }
    if template.applicability.trim().is_empty() {
        errors.push("applicability 不能为空（决定自动匹配与人工选择）".to_string());
    }
    if let Some(description) = template.description.as_deref() {
        if description.chars().count() > MAX_DESCRIPTION_CHARS {
            errors.push(format!("description 过长（>{MAX_DESCRIPTION_CHARS}）"));
        }
    }
    if template.mode == TeamMode::Single && template.roles.len() > 1 {
        errors.push("mode=single 只能有一个角色（多角色请用 team/swarmflow）".to_string());
    }
    if template.roles.is_empty() {
        errors.push("roles 不能为空".to_string());
    }
    if template.roles.len() > MAX_ROLES {
        errors.push(format!("roles 过多（>{}）", MAX_ROLES));
    }

    let mut seen_roles: Vec<&str> = Vec::new();
    for role in &template.roles {
        let name = role.role.trim();
        if name.is_empty() {
            errors.push("角色名不能为空".to_string());
            continue;
        }
        if seen_roles.contains(&name) {
            errors.push(format!("角色名重复：{name}"));
        }
        seen_roles.push(name);
        if !matches!(role.assignee.as_str(), "agent" | "human" | "worker") {
            errors.push(format!(
                "角色 {name} 的 assignee 非法：{}（只能是 agent/human/worker）",
                role.assignee
            ));
        }
        if role.assignee == "worker" && role.worker.as_deref().unwrap_or("").trim().is_empty() {
            errors.push(format!("角色 {name} 是 worker，必须给 worker 名"));
        }
        if let Some(verify) = role.verify.as_deref() {
            if let Err(error) = validate_verify_assertion(verify) {
                errors.push(format!("角色 {name} 的 verify 非法：{error}"));
            }
        }
        if let Some(contract) = role.handoff_contract.as_deref() {
            if contract.trim().is_empty() {
                errors.push(format!(
                    "角色 {name} 的 handoff_contract 不能是空串（要么删掉）"
                ));
            }
        }
        if role.tools.len() > MAX_ROLE_TOOLS {
            errors.push(format!("角色 {name} 的 tools 过多（>{}）", MAX_ROLE_TOOLS));
        }
        let mut seen_tools: Vec<&str> = Vec::new();
        for tool in &role.tools {
            let tool = tool.trim();
            if tool.is_empty() {
                errors.push(format!("角色 {name} 的 tools 含空项"));
                continue;
            }
            if seen_tools.contains(&tool) {
                errors.push(format!("角色 {name} 的 tools 重复：{tool}"));
            }
            seen_tools.push(tool);
            if let Err(error) = validate_tool_name(tool) {
                errors.push(format!("角色 {name} 的 tools 非法：{error}"));
            }
        }
        // 评审角色不得声明写/执行类工具：运行期会按能力面剔除，与其静默剔除不如组队期报错。
        if crate::workswarm::is_review_role(&role.role, &role.capabilities) {
            for tool in &role.tools {
                if is_write_or_exec_tool(tool.trim()) {
                    errors.push(format!(
                        "评审角色 {} 不得声明写/执行工具：{tool}",
                        role.role
                    ));
                }
            }
        }
        // 写白名单：必须是工作区内相对路径（与运行期范围租约同一口径）。
        let mut seen_paths: Vec<&str> = Vec::new();
        for path in &role.write_paths {
            let path = path.trim();
            if path.is_empty() {
                errors.push(format!("角色 {name} 的 write_paths 含空项"));
                continue;
            }
            if seen_paths.contains(&path) {
                errors.push(format!("角色 {name} 的 write_paths 重复：{path}"));
            }
            seen_paths.push(path);
            if let Err(error) = validate_write_path(path) {
                errors.push(format!("角色 {name} 的 write_paths 非法：{error}"));
            }
        }
    }

    // 依赖必须存在且无环（可调度性；拓扑排序失败即存在环）。
    let role_names: Vec<String> = template
        .roles
        .iter()
        .map(|role| role.role.trim().to_string())
        .collect();
    for role in &template.roles {
        for dependency in &role.depends_on {
            if !role_names.iter().any(|name| name == dependency) {
                errors.push(format!("角色 {} 依赖不存在的角色：{dependency}", role.role));
            }
            if dependency == &role.role {
                errors.push(format!("角色 {} 不能依赖自己", role.role));
            }
        }
    }
    if let Some(cycle) = detect_cycle(template) {
        errors.push(format!("角色依赖存在环：{}", cycle.join(" → ")));
    }

    errors
}

/// 校验 `verify` 断言语法（与运行期解析口径一致）。
pub fn validate_verify_assertion(assertion: &str) -> Result<(), String> {
    let trimmed = assertion.trim();
    if trimmed.is_empty() {
        return Err("不能为空".to_string());
    }
    if trimmed == "non_empty" {
        return Ok(());
    }
    if let Some(rest) = trimmed.strip_prefix("contains:") {
        if rest.trim().is_empty() {
            return Err("contains: 后需要文本".to_string());
        }
        return Ok(());
    }
    if let Some(rest) = trimmed.strip_prefix("equals:") {
        if rest.is_empty() {
            return Err("equals: 后需要文本".to_string());
        }
        return Ok(());
    }
    Err(format!(
        "未知断言：{trimmed}（可用 non_empty / contains:<文本> / equals:<文本>）"
    ))
}

/// 角色是否具备写入/执行能力（决定"未声明写范围"是否算工作区任意写）。
fn has_write_capability(tools: &[String]) -> bool {
    tools.iter().any(|tool| is_write_or_exec_tool(tool.trim()))
}

/// 写/执行类工具（评审角色声明它们属于配置矛盾）。
fn is_write_or_exec_tool(tool: &str) -> bool {
    matches!(
        tool,
        "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "run_command" | "kill_shell"
    )
}

/// 工具名：内置工具（策略矩阵认得）或 `mcp__{server}__{tool}` 命名空间。
pub fn validate_tool_name(tool: &str) -> Result<(), String> {
    let trimmed = tool.trim();
    if let Some(rest) = trimmed.strip_prefix("mcp__") {
        // 与 N5 命名空间解析同一形状：server 段必须安全、tool 段非空且不含 `__`。
        let Some((server, remote)) = rest.split_once("__") else {
            return Err(format!(
                "MCP 工具名必须形如 mcp__{{server}}__{{tool}}：{trimmed}"
            ));
        };
        if remote.trim().is_empty() {
            return Err(format!("MCP 工具名缺少 tool 段：{trimmed}"));
        }
        if remote.contains("__") {
            // `mcp__{server}__{tool}` 的反解必须唯一：tool 段再出现 `__` 会让归属歧义
            //（与 N5 命名空间规则同一口径）。
            return Err(format!("MCP 工具名的 tool 段不得包含 `__`：{trimmed}"));
        }
        crate::validate_mcp_server_name(server)
            .map_err(|reason| format!("{reason}（来自 {trimmed}）"))?;
        return Ok(());
    }
    if owo_agent_policy::tool_effects::effect_for(trimmed).is_some() {
        return Ok(());
    }
    Err(format!(
        "未知工具：{trimmed}（必须是内置工具名或 mcp__{{server}}__{{tool}}）"
    ))
}

/// 写白名单路径：工作区内相对路径，不得含 `..`/绝对路径/盘符。
pub fn validate_write_path(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("路径为空".to_string());
    }
    if trimmed.starts_with(['/', '\\']) || trimmed.contains(':') {
        return Err(format!("只接受工作区内相对路径：{trimmed}"));
    }
    for component in std::path::Path::new(trimmed).components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            other => {
                return Err(format!("路径不得包含 {other:?}：{trimmed}"));
            }
        }
    }
    Ok(())
}

/// 一条工作流的运行结果摘要（统计接口给的口径）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowRunSummary {
    pub runs: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: usize,
    /// 最近一次运行是否为失败（按时间取最新一条）。
    #[serde(default)]
    pub last_run_failed: bool,
}

/// 运行期"需要修复"的判定（空 = 正常）。
///
/// 规则简单可解释，只提示不阻断：
/// - 失败过但**从未成功** → 配置/环境很可能坏了；
/// - 终态 ≥ 2 且失败率 ≥ 50% → 稳定性不足；
/// - 运行 ≥ 5 且成功率 < 50% → 长期低成功率；
/// - 最近一次运行失败 → 现在就需要看。
pub fn workflow_attention(summary: &WorkflowRunSummary) -> Vec<String> {
    let mut reasons = Vec::new();
    if summary.failed > 0 && summary.succeeded == 0 {
        reasons.push(format!("从未成功（失败 {} 次）", summary.failed));
    }
    let terminal = summary.succeeded + summary.failed + summary.cancelled;
    if terminal >= 2 && summary.failed * 2 >= terminal {
        let rate = summary.failed as f64 / terminal as f64 * 100.0;
        reasons.push(format!("失败率 {rate:.0}%（终态 {terminal} 次）"));
    }
    if summary.runs >= 5 && summary.succeeded * 2 < summary.runs {
        reasons.push(format!(
            "长期低成功率（{} 次运行仅成功 {} 次）",
            summary.runs, summary.succeeded
        ));
    }
    if summary.last_run_failed {
        reasons.push("最近一次运行失败".to_string());
    }
    reasons
}

/// 工作流健康摘要（卡片徽标/列表用）：不编造分数，只给可复核的计数与风险等级。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkflowHealth {
    pub roles: usize,
    pub tools: usize,
    /// 执行/联网类工具（去重）。
    pub elevated_tools: Vec<String>,
    /// MCP 依赖（去重）。
    pub mcp_tools: Vec<String>,
    /// 未声明写范围、却带工具的角色（= 工作区内任意写）。
    pub whole_workspace_writers: Vec<String>,
    pub human_roles: usize,
    /// 依赖链最大深度（无环时的最长路径；有环按 0 计）。
    pub max_depth: usize,
    /// `low` | `medium` | `high`（规则见下，便于在 UI 上统一口径）。
    pub risk_level: String,
    /// 逐角色健康行（UI/CLI 直接渲染；风险标记见 [`RoleHealth::risks`]）。
    pub role_rows: Vec<RoleHealth>,
}

/// 单个角色的健康行（谁、能干什么、有什么风险标记）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RoleHealth {
    pub role: String,
    pub assignee: String,
    pub tools: Vec<String>,
    pub write_paths: Vec<String>,
    /// 该角色的风险标记（人话短语，直接可展示）。
    pub risks: Vec<String>,
}

/// 计算单个角色的健康行（diff/健康摘要共用同一判定）。
pub fn role_health(role: &owo_agent_protocol::TeamTemplateRole) -> RoleHealth {
    let tools: Vec<String> = role
        .tools
        .iter()
        .map(|tool| tool.trim().to_string())
        .collect();
    let mut risks = Vec::new();
    for tool in &tools {
        if ELEVATED_TOOLS.contains(&tool.as_str()) {
            risks.push(format!("执行/联网工具:{tool}"));
        } else if tool.starts_with("mcp__") {
            risks.push(format!("MCP 依赖:{tool}"));
        }
    }
    if role.assignee == "agent" && role.write_paths.is_empty() && has_write_capability(&tools) {
        risks.push("未声明写范围（工作区任意写）".to_string());
    }
    if role.assignee == "human" {
        risks.push("需要人工确认".to_string());
    }
    if role.assignee == "agent" && tools.is_empty() {
        risks.push("无工具（只产出文本）".to_string());
    }
    RoleHealth {
        role: role.role.clone(),
        assignee: role.assignee.clone(),
        tools,
        write_paths: role.write_paths.clone(),
        risks,
    }
}

/// 计算健康摘要。
///
/// 风险规则（简单、可解释，不做"安全评分"）：
/// - `high`：存在执行/联网类工具，或存在"工作区任意写"的角色；
/// - `medium`：依赖 MCP、含人工节点、或角色数 > 6；
/// - `low`：其余。
pub fn workflow_health(template: &TeamTemplate) -> WorkflowHealth {
    let mut tools = 0usize;
    let mut elevated: Vec<String> = Vec::new();
    let mut mcp: Vec<String> = Vec::new();
    let mut whole_workspace_writers: Vec<String> = Vec::new();
    let mut human_roles = 0usize;
    for role in &template.roles {
        tools += role.tools.len();
        for tool in &role.tools {
            let tool = tool.trim();
            if ELEVATED_TOOLS.contains(&tool) && !elevated.iter().any(|item| item == tool) {
                elevated.push(tool.to_string());
            }
            if tool.starts_with("mcp__") && !mcp.iter().any(|item| item == tool) {
                mcp.push(tool.to_string());
            }
        }
        if role.assignee == "human" {
            human_roles += 1;
        }
        if role.assignee == "agent"
            && role.write_paths.is_empty()
            && has_write_capability(&role.tools)
        {
            whole_workspace_writers.push(role.role.clone());
        }
    }
    let max_depth = if detect_cycle(template).is_some() {
        0
    } else {
        template
            .roles
            .iter()
            .map(|role| depth_of(template, &role.role, 0))
            .max()
            .unwrap_or(0)
    };
    let risk_level = if !elevated.is_empty() || !whole_workspace_writers.is_empty() {
        "high"
    } else if !mcp.is_empty() || human_roles > 0 || template.roles.len() > 6 {
        "medium"
    } else {
        "low"
    };
    WorkflowHealth {
        roles: template.roles.len(),
        role_rows: template.roles.iter().map(role_health).collect(),
        tools,
        elevated_tools: elevated,
        mcp_tools: mcp,
        whole_workspace_writers,
        human_roles,
        max_depth,
        risk_level: risk_level.to_string(),
    }
}

/// 依赖链深度（自底向上；`guard` 防御异常数据造成的无限递归）。
fn depth_of(template: &TeamTemplate, role: &str, guard: usize) -> usize {
    if guard > template.roles.len() + 1 {
        return 0;
    }
    let Some(current) = template.roles.iter().find(|item| item.role == role) else {
        return 0;
    };
    let mut depth = 1;
    for dependency in &current.depends_on {
        depth = depth.max(1 + depth_of(template, dependency, guard + 1));
    }
    depth
}

/// 逐条评审发现（社区导入/社区下载时的安全门禁输入）。
///
/// `level`：`block`（阻止导入）/ `warn`（可导入但需用户知情）/ `info`（信息）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkflowFinding {
    pub level: String,
    pub category: String,
    pub detail: String,
}

impl WorkflowFinding {
    fn new(level: &str, category: &str, detail: impl Into<String>) -> Self {
        Self {
            level: level.to_string(),
            category: category.to_string(),
            detail: detail.into(),
        }
    }
}

/// 需要"执行/联网/MCP"类能力时的高风险提示（不阻止，但必须让用户看见）。
const ELEVATED_TOOLS: &[&str] = &[
    "run_command",
    "web_fetch",
    "web_search",
    "browser_click",
    "browser_type",
    "computer_use",
];

/// 评审用户/社区工作流：校验错误 → `block`；能力面与人工节点 → `warn`/`info`。
///
/// 与 [`validate_workflow_template`] 的关系：校验器保证"可解析、可调度、名字认得"，
/// 本函数回答另一个问题——"把它跑起来会碰到什么权限面/人工环节"，供导入前知情确认。
pub fn review_workflow_template(template: &TeamTemplate) -> Vec<WorkflowFinding> {
    let mut findings = Vec::new();
    for error in validate_workflow_template(template) {
        findings.push(WorkflowFinding::new("block", "schema", error));
    }

    let mut elevated: Vec<String> = Vec::new();
    let mut mcp_tools: Vec<String> = Vec::new();
    let mut whole_workspace_writers: Vec<String> = Vec::new();
    let mut human_roles: Vec<String> = Vec::new();
    let mut worker_roles: Vec<String> = Vec::new();
    let mut total_tools = 0usize;

    for role in &template.roles {
        total_tools += role.tools.len();
        for tool in &role.tools {
            let tool = tool.trim();
            if tool.starts_with("mcp__") {
                mcp_tools.push(tool.to_string());
            }
            if ELEVATED_TOOLS.contains(&tool) && !elevated.iter().any(|item| item == tool) {
                elevated.push(tool.to_string());
            }
        }
        // 空 write_paths + 具备写/执行能力 = 工作区内任意写：社区包里必须让用户看见。
        if role.write_paths.is_empty()
            && role.assignee == "agent"
            && has_write_capability(&role.tools)
        {
            whole_workspace_writers.push(role.role.clone());
        }
        if role.assignee == "human" {
            human_roles.push(role.role.clone());
        }
        if role.assignee == "worker" {
            worker_roles.push(role.role.clone());
        }
    }

    if !elevated.is_empty() {
        findings.push(WorkflowFinding::new(
            "warn",
            "capability",
            format!(
                "包含执行/联网类工具：{}（运行时会按权限档位逐次审批）",
                elevated.join(", ")
            ),
        ));
    }
    if !mcp_tools.is_empty() {
        findings.push(WorkflowFinding::new(
            "warn",
            "mcp",
            format!(
                "依赖 MCP 工具：{}（需先在本机配置对应 MCP server，否则角色会拿不到工具）",
                mcp_tools.join(", ")
            ),
        ));
    }
    if !whole_workspace_writers.is_empty() {
        findings.push(WorkflowFinding::new(
            "warn",
            "write_scope",
            format!(
                "以下角色未声明写范围（= 工作区内任意写）：{}",
                whole_workspace_writers.join(", ")
            ),
        ));
    }
    if !human_roles.is_empty() {
        findings.push(WorkflowFinding::new(
            "info",
            "human_in_loop",
            format!(
                "包含人工节点：{}（推进时会等待确认）",
                human_roles.join(", ")
            ),
        ));
    }
    if !worker_roles.is_empty() {
        findings.push(WorkflowFinding::new(
            "info",
            "worker",
            format!(
                "使用内置 worker：{}（不需要模型凭据）",
                worker_roles.join(", ")
            ),
        ));
    }
    findings.push(WorkflowFinding::new(
        "info",
        "summary",
        format!(
            "角色 {} 个 / 工具引用 {} 个 / 模板 {}@{}",
            template.roles.len(),
            total_tools,
            template.template_id,
            template.version.as_deref().unwrap_or("1.0.0")
        ),
    ));
    findings
}

/// 评审是否阻止导入（存在任一 `block`）。
pub fn workflow_review_blocked(findings: &[WorkflowFinding]) -> bool {
    findings.iter().any(|finding| finding.level == "block")
}

/// 依赖图找环（DFS 三色标记）；返回环上的角色序列（有环时）。
fn detect_cycle(template: &TeamTemplate) -> Option<Vec<String>> {
    let names: Vec<String> = template
        .roles
        .iter()
        .map(|role| role.role.trim().to_string())
        .collect();
    let index_of = |name: &str| names.iter().position(|candidate| candidate == name);
    // state: 0 未访问 / 1 栈中 / 2 已完成
    let mut state = vec![0u8; names.len()];
    let mut stack: Vec<usize> = Vec::new();

    fn visit(
        node: usize,
        template: &TeamTemplate,
        names: &[String],
        index_of: &dyn Fn(&str) -> Option<usize>,
        state: &mut Vec<u8>,
        stack: &mut Vec<usize>,
    ) -> Option<Vec<String>> {
        state[node] = 1;
        stack.push(node);
        for dependency in &template.roles[node].depends_on {
            let Some(next) = index_of(dependency.trim()) else {
                continue;
            };
            match state[next] {
                0 => {
                    if let Some(cycle) = visit(next, template, names, index_of, state, stack) {
                        return Some(cycle);
                    }
                }
                1 => {
                    // 回边：从栈中该节点到栈顶构成环。
                    let start = stack.iter().position(|item| *item == next).unwrap_or(0);
                    let mut cycle: Vec<String> = stack[start..]
                        .iter()
                        .map(|item| names[*item].clone())
                        .collect();
                    cycle.push(names[next].clone());
                    return Some(cycle);
                }
                _ => {}
            }
        }
        stack.pop();
        state[node] = 2;
        None
    }

    for node in 0..names.len() {
        if state[node] == 0 {
            if let Some(cycle) = visit(node, template, &names, &index_of, &mut state, &mut stack) {
                return Some(cycle);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use owo_agent_protocol::TeamTemplateRole;

    fn role(name: &str, depends_on: &[&str]) -> TeamTemplateRole {
        TeamTemplateRole {
            role: name.to_string(),
            assignee: "agent".to_string(),
            worker: None,
            depends_on: depends_on.iter().map(|item| item.to_string()).collect(),
            handoff_contract: None,
            verify: None,
            model: None,
            write_paths: Vec::new(),
            capabilities: Vec::new(),
            tools: Vec::new(),
        }
    }

    fn template(roles: Vec<TeamTemplateRole>) -> TeamTemplate {
        TeamTemplate {
            template_id: "video-recreate".to_string(),
            name: "视频复刻流水线".to_string(),
            mode: TeamMode::Team,
            roles,
            applicability: "视频复刻 分镜 配音".to_string(),
            source_team_id: None,
            created_at: "2026-10-19T00:00:00Z".to_string(),
            origin: Some("user".to_string()),
            description: Some("从素材到成片的多角色流水线".to_string()),
            version: Some("1.0.0".to_string()),
            tags: vec!["video".to_string()],
        }
    }

    #[test]
    fn slugify_produces_safe_ids_and_rejects_unicode_only_names() {
        assert_eq!(
            slugify_template_id("Video Recreation").as_deref(),
            Some("video-recreation")
        );
        assert_eq!(
            slugify_template_id("game__agent").as_deref(),
            Some("game-agent")
        );
        assert_eq!(slugify_template_id("  Rig 3 ").as_deref(), Some("rig-3"));
        // 纯中文没有 ASCII 字母数字 → 需要用户显式给 id。
        assert_eq!(slugify_template_id("视频复刻"), None);
        assert_eq!(slugify_template_id("123"), None);
        assert_eq!(slugify_template_id(""), None);
    }

    #[test]
    fn template_id_validation_rejects_paths_and_reserved_chars() {
        assert!(validate_template_id("video-recreate").is_ok());
        assert!(validate_template_id("video.recreate_2").is_ok());
        for bad in [
            "",
            "../escape",
            "a/b",
            "a\\b",
            "C:evil",
            "a*b",
            "1abc",
            "视频",
        ] {
            assert!(validate_template_id(bad).is_err(), "{bad} 应被拒绝");
        }
    }

    #[test]
    fn valid_workflow_passes_and_dag_is_checked() {
        let valid = template(vec![
            role("script", &[]),
            role("storyboard", &["script"]),
            role("render", &["storyboard"]),
        ]);
        assert!(validate_workflow_template(&valid).is_empty());

        let cyclic = template(vec![role("a", &["b"]), role("b", &["a"])]);
        let errors = validate_workflow_template(&cyclic);
        assert!(
            errors.iter().any(|error| error.contains("环")),
            "{errors:?}"
        );

        let missing = template(vec![role("a", &["ghost"])]);
        let errors = validate_workflow_template(&missing);
        assert!(
            errors.iter().any(|error| error.contains("不存在的角色")),
            "{errors:?}"
        );

        let self_cycle = template(vec![role("a", &["a"])]);
        let errors = validate_workflow_template(&self_cycle);
        assert!(
            errors.iter().any(|error| error.contains("不能依赖自己")),
            "{errors:?}"
        );
    }

    #[test]
    fn roles_assignee_tools_and_write_paths_are_validated() {
        let mut workflow = template(vec![role("a", &[])]);
        workflow.roles[0].assignee = "robot".into();
        workflow.roles[0].tools = vec![
            "read_file".into(),
            "totally_unknown_tool".into(),
            "mcp__bad server__tool".into(),
        ];
        workflow.roles[0].write_paths = vec!["../outside".into(), "ok/dir".into()];
        workflow.roles[0].verify = Some("looks_good".into());
        let errors = validate_workflow_template(&workflow);
        let joined = errors.join("\n");
        assert!(joined.contains("assignee 非法"), "{joined}");
        assert!(
            joined.contains("未知工具：totally_unknown_tool"),
            "{joined}"
        );
        assert!(
            joined.contains("mcp__bad server__tool"),
            "非法 MCP 服务名必须被拒：{joined}"
        );
        assert!(joined.contains("../outside"), "{joined}");
        assert!(joined.contains("verify 非法"), "{joined}");
        assert!(
            errors.iter().all(|error| !error.contains("ok/dir")),
            "合法路径不应报错：{joined}"
        );
    }

    #[test]
    fn mcp_namespaced_tools_are_accepted_with_valid_server_names() {
        assert!(validate_tool_name("mcp__owo_plugin_translate__translate").is_ok());
        assert!(validate_tool_name("comfyui__run").is_err());
        assert!(
            validate_tool_name("mcp__a__b__c").is_err(),
            "server 段不得含 __"
        );
        assert!(validate_tool_name("mcp__srv__").is_err(), "tool 段必填");
        assert!(validate_tool_name("read_file").is_ok());
        assert!(validate_tool_name("search_files").is_ok());
    }

    #[test]
    fn single_mode_with_multiple_roles_is_rejected() {
        let mut workflow = template(vec![role("a", &[]), role("b", &["a"])]);
        workflow.mode = TeamMode::Single;
        let errors = validate_workflow_template(&workflow);
        assert!(
            errors.iter().any(|error| error.contains("mode=single")),
            "{errors:?}"
        );
    }

    #[test]
    fn review_blocks_schema_errors_and_reports_capability_surface() {
        // 非法配置 → block。
        let broken = template(vec![role("a", &["ghost"])]);
        let findings = review_workflow_template(&broken);
        assert!(workflow_review_blocked(&findings));
        assert!(findings.iter().any(|item| item.level == "block"));

        // 合法但"危险面"要 warn：执行/联网工具、未声明写范围、MCP 依赖、人工节点。
        let mut risky = template(vec![role("a", &[]), role("b", &["a"])]);
        risky.roles[0].tools = vec!["read_file".into(), "run_command".into()];
        risky.roles[1].tools = vec![
            "web_fetch".into(),
            "mcp__owo_plugin_translate__translate".into(),
        ];
        risky.roles[1].write_paths = Vec::new();
        risky.roles.push(TeamTemplateRole {
            role: "human-review".into(),
            assignee: "human".into(),
            ..Default::default()
        });
        let findings = review_workflow_template(&risky);
        assert!(!workflow_review_blocked(&findings), "{findings:?}");
        let categories: Vec<&str> = findings.iter().map(|item| item.category.as_str()).collect();
        assert!(categories.contains(&"capability"), "{findings:?}");
        assert!(categories.contains(&"mcp"), "{findings:?}");
        assert!(categories.contains(&"write_scope"), "{findings:?}");
        assert!(categories.contains(&"human_in_loop"), "{findings:?}");
        assert!(categories.contains(&"summary"), "{findings:?}");
        let summary = findings
            .iter()
            .find(|item| item.category == "summary")
            .unwrap();
        assert!(summary.detail.contains("角色 3 个"), "{}", summary.detail);
    }

    #[test]
    fn reviewer_roles_may_not_declare_write_or_exec_tools() {
        let mut workflow = template(vec![
            role("writer", &[]),
            TeamTemplateRole {
                role: "reviewer".to_string(),
                assignee: "agent".to_string(),
                capabilities: vec!["review".to_string()],
                tools: vec![
                    "read_file".to_string(),
                    "run_command".to_string(),
                    "write_file".to_string(),
                ],
                ..Default::default()
            },
        ]);
        workflow.roles[1].depends_on = vec!["writer".to_string()];
        let errors = validate_workflow_template(&workflow);
        let joined = errors.join("\n");
        assert!(
            joined.contains("评审角色 reviewer 不得声明写/执行工具"),
            "{joined}"
        );
        assert!(joined.contains("run_command"), "{joined}");
        assert!(joined.contains("write_file"), "{joined}");

        // 只读工具声明不报错。
        workflow.roles[1].tools = vec!["read_file".to_string(), "search_files".to_string()];
        let errors = validate_workflow_template(&workflow);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn health_reports_counts_depth_and_risk_levels() {
        // low：纯只读、无人工、无 MCP、有写范围的单角色。
        let safe = template(vec![role("reader", &[])]);
        let health = workflow_health(&safe);
        assert_eq!(health.roles, 1);
        assert_eq!(health.risk_level, "low");
        assert_eq!(health.max_depth, 1);

        // medium：含人工节点。
        let mut with_human = template(vec![role("a", &[])]);
        with_human.roles.push(TeamTemplateRole {
            role: "accept".to_string(),
            assignee: "human".to_string(),
            depends_on: vec!["a".to_string()],
            ..Default::default()
        });
        let health = workflow_health(&with_human);
        assert_eq!(health.human_roles, 1);
        assert_eq!(health.risk_level, "medium");
        assert_eq!(health.max_depth, 2);

        // high：执行类工具。
        let mut exec = template(vec![role("a", &[])]);
        exec.roles[0].tools = vec!["read_file".into(), "run_command".into()];
        let health = workflow_health(&exec);
        assert_eq!(health.risk_level, "high");
        assert_eq!(health.elevated_tools, vec!["run_command".to_string()]);

        // high：未声明写范围却有工具（工作区任意写）。
        let mut wide = template(vec![role("a", &[])]);
        wide.roles[0].tools = vec!["write_file".into()];
        let health = workflow_health(&wide);
        assert_eq!(health.risk_level, "high");
        assert_eq!(health.whole_workspace_writers, vec!["a".to_string()]);

        // medium：MCP 依赖。
        let mut mcp = template(vec![role("a", &[])]);
        mcp.roles[0].tools = vec!["mcp__owo_plugin_translate__translate".into()];
        let health = workflow_health(&mcp);
        assert_eq!(health.risk_level, "medium");
        assert_eq!(health.mcp_tools.len(), 1);

        // 工具总数与去重。
        let mut mixed = template(vec![role("a", &[]), role("b", &["a"])]);
        mixed.roles[0].tools = vec!["read_file".into(), "run_command".into()];
        mixed.roles[1].tools = vec!["read_file".into(), "run_command".into()];
        let health = workflow_health(&mixed);
        assert_eq!(health.tools, 4);
        assert_eq!(health.elevated_tools, vec!["run_command".to_string()]);
        assert_eq!(health.max_depth, 2);
    }

    #[test]
    fn role_health_flags_exec_mcp_write_scope_and_human() {
        let mut workflow = template(vec![
            role("impl", &[]),
            role("fetch", &["impl"]),
            TeamTemplateRole {
                role: "accept".to_string(),
                assignee: "human".to_string(),
                depends_on: vec!["fetch".to_string()],
                ..Default::default()
            },
            TeamTemplateRole {
                role: "talker".to_string(),
                assignee: "agent".to_string(),
                depends_on: vec!["accept".to_string()],
                ..Default::default()
            },
        ]);
        // impl：有写能力但未声明写范围 → 工作区任意写。
        workflow.roles[0].tools = vec!["write_file".into()];
        // fetch：执行/联网 + MCP 依赖。
        workflow.roles[1].tools = vec![
            "run_command".into(),
            "web_fetch".into(),
            "mcp__owo_plugin_translate__translate".into(),
        ];
        let health = workflow_health(&workflow);
        assert_eq!(health.role_rows.len(), 4);

        let impl_row = health
            .role_rows
            .iter()
            .find(|row| row.role == "impl")
            .unwrap();
        assert_eq!(
            impl_row.risks,
            vec!["未声明写范围（工作区任意写）".to_string()]
        );

        let fetch_row = health
            .role_rows
            .iter()
            .find(|row| row.role == "fetch")
            .unwrap();
        assert!(
            fetch_row
                .risks
                .iter()
                .any(|risk| risk == "执行/联网工具:run_command"),
            "{:?}",
            fetch_row.risks
        );
        assert!(
            fetch_row
                .risks
                .iter()
                .any(|risk| risk.starts_with("MCP 依赖:")),
            "{:?}",
            fetch_row.risks
        );

        let accept_row = health
            .role_rows
            .iter()
            .find(|row| row.role == "accept")
            .unwrap();
        assert_eq!(accept_row.assignee, "human");
        assert!(accept_row.risks.iter().any(|risk| risk == "需要人工确认"));

        let talker_row = health
            .role_rows
            .iter()
            .find(|row| row.role == "talker")
            .unwrap();
        assert!(talker_row
            .risks
            .iter()
            .any(|risk| risk.starts_with("无工具")));
    }

    #[test]
    fn attention_flags_never_succeeded_and_high_failure_rates() {
        // 从未成功。
        let broken = WorkflowRunSummary {
            runs: 2,
            succeeded: 0,
            failed: 2,
            cancelled: 0,
            last_run_failed: true,
        };
        let reasons = workflow_attention(&broken);
        assert!(
            reasons.iter().any(|reason| reason.contains("从未成功")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|reason| reason.contains("失败率 100%")),
            "{reasons:?}"
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("最近一次运行失败")),
            "{reasons:?}"
        );

        // 健康：全部成功。
        let healthy = WorkflowRunSummary {
            runs: 3,
            succeeded: 3,
            failed: 0,
            cancelled: 0,
            last_run_failed: false,
        };
        assert!(workflow_attention(&healthy).is_empty());

        // 只有取消（还没真正跑出结论）不提示。
        let cancelled_only = WorkflowRunSummary {
            runs: 4,
            succeeded: 0,
            failed: 0,
            cancelled: 4,
            last_run_failed: false,
        };
        assert!(workflow_attention(&cancelled_only).is_empty());

        // 长期低成功率：运行 6 次成功 2 次。
        let low = WorkflowRunSummary {
            runs: 6,
            succeeded: 2,
            failed: 4,
            cancelled: 0,
            last_run_failed: false,
        };
        let reasons = workflow_attention(&low);
        assert!(
            reasons.iter().any(|reason| reason.contains("长期低成功率")),
            "{reasons:?}"
        );

        // 偶发失败（成功 3 失败 1）不提示。
        let occasional = WorkflowRunSummary {
            runs: 4,
            succeeded: 3,
            failed: 1,
            cancelled: 0,
            last_run_failed: false,
        };
        assert!(workflow_attention(&occasional).is_empty());
    }

    #[test]
    fn limits_are_enforced() {
        let mut workflow = template(vec![role("a", &[])]);
        workflow.roles[0].tools = (0..(MAX_ROLE_TOOLS + 1))
            .map(|_| "read_file".to_string())
            .collect();
        workflow.description = Some("字".repeat(MAX_DESCRIPTION_CHARS + 1));
        let errors = validate_workflow_template(&workflow);
        let joined = errors.join("\n");
        assert!(
            joined.contains("tools 过多") || joined.contains("tools 重复"),
            "{joined}"
        );
        assert!(joined.contains("description 过长"), "{joined}");
    }
}
