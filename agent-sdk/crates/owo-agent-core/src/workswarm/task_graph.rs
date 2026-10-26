//! Host validation and worker placement for untrusted TaskGraph inputs.
use super::RoleSpec;
use serde_json::Value;

/// 显式声明 task_worker 能力的 Agent 节点；旧 wN 槽位仍兼容。
#[derive(Debug)]
pub(super) struct TaskGraphV1 {
    pub(super) version: u32,
    pub(super) tasks: Vec<ParallelTaskAssignment>,
}

// TaskGraph executor membership comes from configured tools and scope, not a
// fixed role-name allowlist. Legacy wN slots remain readable.
pub(super) fn is_parallel_task_worker(role: &RoleSpec) -> bool {
    matches!(role.assignee.as_str(), "agent" | "worker")
        && !role.is_reviewer()
        && !role.is_orchestrator()
        && (is_parallel_writer_name(&role.role) || role.has_capability("task_worker"))
}

pub(super) const TASK_GRAPH_ORCHESTRATOR_CONTRACT: &str = r#"你是本工作流的编排 Agent。先理解用户目标，再把确实可以并行的独立工作拆给已配置的 Agent 节点；节点按其职责、工具和模型选择，角色名由用户自定义；任务执行节点标记 task_worker 能力。请输出 WorkerOutputV1，artifact.content 为 TaskGraphV1 JSON：{"version":1,"tasks":[{"task_id":"...","worker":"已配置的 task_worker 节点名","task":"...","acceptance":"...","depends_on":[],"requirement_quotes":["用户原文中的对应要求"],"read_refs":[],"write_paths":[],"contract_refs":[],"required_capabilities":[],"estimated_effort":1,"verification":"non_empty","risk":"normal","priority":50}]}。任务数为 1 到 128；worker 可省略以便宿主在 task_worker 节点间均衡分配；只为真实依赖设置 depends_on；同一写路径的任务必须建立依赖顺序。required_capabilities 必须使用对应 task_worker 节点 tools 中声明的精确工具名（内置文件/命令能力沿用现有工具面）；隐式分配只会选择声明了所需工具的节点。写入范围必须同时落在节点声明范围内；非工作区工具调用仍须遵守节点工具声明、宿主策略和审批。代码修改任务必须请求 run_command 并声明宿主已登记的行为验证计划；媒体生成等外部服务任务可用 non_empty 验证产物，不要伪造服务调用。显式用户验收项须逐字引用并分配给对应任务；高风险任务需要独立只读评审节点；需要收尾汇总时将该节点标记 integrator。没有必要时避免拆分，所有任务都必须可追踪到交付物与验收证据。用户只给简短想法时，先在 artifact.content 顶层的 notes 写一份可执行的 Design Brief（产品目标、核心循环、模块/数据流、视觉或文风、验收与非目标），并把风格和验收落实到每个任务；不要用固定模板猜玩法。"#;

const MIN_TASK_MODEL_CALLS_PER_ATTEMPT: usize = 8;
const TASK_GRAPH_SCOPED_CAPABILITIES: &[&str] = &[
    "read_file",
    "list_dir",
    "search_files",
    "write_file",
    "apply_patch",
    "run_command",
];

fn role_supports_task_capabilities(role: &RoleSpec, capabilities: &[String]) -> bool {
    capabilities.iter().all(|capability| {
        role.tools.iter().any(|tool| tool == capability)
            || (role.tools.is_empty()
                && TASK_GRAPH_SCOPED_CAPABILITIES.contains(&capability.as_str()))
    })
}

/// Assign a bounded per-attempt request budget from host role limits and task effort.
///
/// Template role budgets are **suggestions**, not security limits: a TaskGraph coding
/// task needs at least eight requests (write test, write implementation, run the
/// command, one fix, final answer, plus one reserved output repair). A lower template
/// suggestion is therefore raised to that minimum; the team-wide `max_model_calls`
/// remains the hard ceiling. Effort only raises the cap.
pub(super) fn assign_task_model_call_budgets(
    tasks: &mut [ParallelTaskAssignment],
    role_budgets: &std::collections::BTreeMap<String, usize>,
) -> Result<(), String> {
    for task in tasks {
        let declared_budget = role_budgets.get(&task.worker).copied().unwrap_or(0);
        let role_budget = if declared_budget == 0 {
            crate::worker_profile::DEFAULT_PROFILE_MAX_TURNS
        } else {
            declared_budget
        }
        .max(MIN_TASK_MODEL_CALLS_PER_ATTEMPT)
        .clamp(1, crate::worker_profile::PROFILE_MAX_TURNS_CAP);
        let effort = task.estimated_effort.max(1);
        let effort_turns = u64::BITS.saturating_sub((effort - 1).leading_zeros()) as usize;
        task.model_calls_per_attempt = MIN_TASK_MODEL_CALLS_PER_ATTEMPT
            .saturating_add(effort_turns)
            .min(role_budget);
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct ParallelTaskAssignment {
    pub(super) task_id: String,
    pub(super) worker: String,
    pub(super) worker_explicit: bool,
    pub(super) task: String,
    pub(super) acceptance: String,
    pub(super) user_requirement_quotes: Vec<String>,
    pub(super) depends_on: Vec<String>,
    pub(super) effective_paths: Vec<String>,
    pub(super) read_refs: Vec<String>,
    pub(super) contract_refs: Vec<String>,
    pub(super) required_capabilities: Vec<String>,
    pub(super) estimated_effort: u64,
    /// Host-computed total model-request ceiling for one task attempt, including one output repair.
    pub(super) model_calls_per_attempt: usize,
    pub(super) verification: Option<String>,
    pub(super) verification_plan: Option<crate::plan::VerificationPlanV1>,
    pub(super) risk: String,
    pub(super) priority: u8,
}

pub(super) use super::task_graph_policy::{
    bind_dynamic_follow_up_dependencies, host_manifest_can_replace_integration,
    is_independent_reviewer_role, parallel_tasks_require_integration,
    should_enable_parallel_assignment, PARALLEL_LEADER_HOST_MANIFEST_SKIP_REASON,
};

fn validate_task_quote_scope(
    quotes: &[String],
    task: &str,
    acceptance: &str,
) -> Result<(), String> {
    let normalize = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    let task_contract = normalize(&format!("{task}\n{acceptance}"));
    for quote in quotes {
        let normalized_quote = normalize(quote);
        if !task_contract.contains(&normalized_quote) {
            return Err(format!(
                "用户原文要求未逐字体现在该任务目标或验收中：{quote}"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_task_requirement_coverage(
    task_quote_sets: &[Vec<String>],
    user_request: &str,
) -> Result<(), String> {
    let normalize = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    let explicit_items = crate::request_requirements::explicit_acceptance_items(user_request);
    let mut covered = std::collections::BTreeSet::new();
    for (index, quotes) in task_quote_sets.iter().enumerate() {
        if quotes.is_empty() {
            // 没有显式验收清单时，模型可以省略引用；有清单时仍必须逐项引用，
            // 否则空引用会静默绕过验收覆盖检查。
            if explicit_items.is_empty() {
                continue;
            }
            return Err(format!(
                "task at index {index}: 任务没有引用任何用户原文要求"
            ));
        }
        crate::request_requirements::validate_exact_user_request_quotes(quotes, user_request)
            .map_err(|error| format!("task at index {index}: {error}"))?;
        covered.extend(quotes.iter().map(|quote| normalize(quote)));
    }
    for item in explicit_items {
        let normalized = normalize(&item);
        if !covered.contains(&normalized) {
            return Err(format!(
                "显式用户验收项没有精确分配给任何 TaskGraph 任务：{item}"
            ));
        }
    }
    Ok(())
}

/// 验收文案既可能是字符串，也可能是字符串数组或 `{id,criterion,...}` 数组。
/// 统一拼成审核用文本；纯空数组/空对象仍返回 None，交给调用方报空验收。
fn acceptance_text(value: &Value) -> Option<String> {
    fn item_text(item: &Value) -> Option<String> {
        match item {
            Value::String(text) => {
                let text = text.trim();
                (!text.is_empty()).then(|| text.to_string())
            }
            Value::Object(object) => ["criterion", "text", "description"]
                .iter()
                .find_map(|key| object.get(*key).and_then(item_text)),
            _ => None,
        }
    }
    match value {
        Value::String(_) | Value::Object(_) => item_text(value),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(item_text).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        _ => None,
    }
}

pub(super) fn validate_parallel_subtasks(
    roles: &[RoleSpec],
    subtasks: &[Value],
    versioned: bool,
) -> Result<TaskGraphV1, String> {
    let writers: Vec<&RoleSpec> = roles
        .iter()
        .filter(|role| is_parallel_task_worker(role))
        .collect();
    if writers.is_empty() {
        return Err("RunMeta contains no eligible task_worker roles".into());
    }
    if subtasks.is_empty() || subtasks.len() > 128 {
        return Err(format!(
            "task count must be 1..=128, got {}",
            subtasks.len()
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut tasks = Vec::with_capacity(subtasks.len());
    for (index, item) in subtasks.iter().enumerate() {
        let task_id = item
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| (!versioned).then(|| format!("task-{}", index + 1)))
            .ok_or_else(|| format!("task at index {index} requires task_id"))?;
        if task_id.len() > 120
            || !task_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("invalid task_id {task_id}"));
        }
        if !ids.insert(task_id.clone()) {
            return Err(format!("duplicate task_id {task_id}"));
        }
        let estimated_effort = item
            .get("estimated_effort")
            .and_then(Value::as_u64)
            .or_else(|| (!versioned).then_some(1))
            .filter(|effort| (1..=1_000_000).contains(effort))
            .ok_or_else(|| format!("task {task_id} requires estimated_effort in 1..=1000000"))?;
        let (worker, worker_explicit) = match item.get("worker") {
            Some(Value::String(worker)) => {
                if !writers
                    .iter()
                    .any(|role| role.role.as_str() == worker.as_str())
                {
                    return Err(format!("task {task_id} refers to unknown worker {worker}"));
                }
                (worker.clone(), true)
            }
            None => (String::new(), false),
            Some(_) => return Err(format!("task {task_id} worker must be a string")),
        };
        // 模型常把任务描述写成 description/deliverable；三者语义相同，宿主统一取
        // 第一个非空字段，避免因字段别名让一份可执行图被判空目标。
        let task = ["task", "description", "deliverable"]
            .iter()
            .find_map(|key| {
                item.get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            })
            .ok_or_else(|| format!("task {task_id} has empty goal"))?;
        if task.chars().count() > 4_000 {
            return Err(format!("task {task_id} goal exceeds 4000 characters"));
        }
        let acceptance = ["acceptance", "acceptance_criteria"]
            .iter()
            .find_map(|key| item.get(*key).and_then(acceptance_text))
            .ok_or_else(|| format!("task {task_id} has empty acceptance"))?;
        if acceptance.chars().count() > 2_000 {
            return Err(format!("task {task_id} acceptance exceeds 2000 characters"));
        }
        let user_requirement_quotes = match item.get("requirement_quotes") {
            Some(Value::Array(values)) if values.len() <= 32 => values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|quote| !quote.is_empty() && quote.len() <= 2_048)
                        .map(str::to_string)
                        .ok_or_else(|| format!("task {task_id} has invalid requirement quote"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
            _ => {
                return Err(format!(
                    "task {task_id} requirement_quotes must be an array of at most 32 strings"
                ))
            }
        };
        // A model may paraphrase a user clause while the task/acceptance still
        // capture the behavior. Keep the quote as traceability context, but do
        // not reject an otherwise executable task solely on wording. Explicit
        // user acceptance items are still checked against the full request below.
        let _ = validate_task_quote_scope(&user_requirement_quotes, &task, &acceptance);
        let raw = item
            .get("write_paths")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("task {task_id} has no write_paths array"))?;
        if raw.len() > 64 {
            return Err(format!("task {task_id} has more than 64 write paths"));
        }
        let paths: Vec<String> = raw
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && s.len() <= 512)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        format!("task {task_id} has invalid write path (empty or over 512 bytes)")
                    })
            })
            .collect::<Result<_, _>>()?;
        if paths
            .iter()
            .any(|path| normalized_write_path(path).is_empty())
        {
            return Err(format!(
                "task {task_id} write_paths cannot target the workspace root"
            ));
        }
        let string_refs = |key: &str, required: bool| -> Result<Vec<String>, String> {
            match item.get(key) {
                Some(Value::Array(values)) if values.len() <= 64 => values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::trim)
                            .filter(|value| !value.is_empty() && value.len() <= 512)
                            .map(str::to_owned)
                            .ok_or_else(|| format!("task {task_id} has invalid {key} entry"))
                    })
                    .collect(),
                None if !required => Ok(Vec::new()),
                _ => Err(format!("task {task_id} requires {key} string array")),
            }
        };
        // read_refs/contract_refs 是可选引用列表，缺省为空；required_capabilities
        // 在版本化图中仍是必需字段，避免漏声明工具后越权或无法执行。
        let read_refs = string_refs("read_refs", false)?;
        let contract_refs = string_refs("contract_refs", false)?;
        let mut required_capabilities = string_refs("required_capabilities", false)?;
        if required_capabilities.len() > 8 {
            return Err(format!(
                "task {task_id} has more than 8 required capabilities"
            ));
        }
        // Capability names are planning hints; the role tool surface still
        // enforces execution. Normalize common aliases and drop unrecognized
        // hints rather than failing an otherwise valid plan.
        for capability in &mut required_capabilities {
            let normalized = match capability.as_str() {
                "read_files" | "read" => "read_file",
                "write_files" | "write" => "write_file",
                "list_files" | "list_dirs" => "list_dir",
                "run_commands" | "shell" | "node" => "run_command",
                "apply_patches" | "patch" => "apply_patch",
                "browser" | "web" | "browser_automation" => "browser_navigate",
                "image" | "image_generation" => "generate_image",
                "tts" | "speech" => "text_to_speech",
                other => other,
            };
            if normalized != capability {
                *capability = normalized.to_string();
            }
        }
        required_capabilities
            .retain(|capability| crate::user_workflow::validate_tool_name(capability).is_ok());
        let uses_builtin_media = required_capabilities
            .iter()
            .any(|capability| matches!(capability.as_str(), "generate_image" | "text_to_speech"));
        if !paths.is_empty()
            && !uses_builtin_media
            && !required_capabilities
                .iter()
                .any(|capability| capability == "write_file")
        {
            // A task that declares files must expose the write tool, otherwise
            // the worker profile becomes read-only and cannot execute its plan.
            // Media tools write through their own publisher, not write_file.
            required_capabilities.push("write_file".to_string());
        }
        if !writers
            .iter()
            .any(|role| role_supports_task_capabilities(role, &required_capabilities))
        {
            return Err(format!(
                "task {task_id} requires tools not declared by any task_worker node: {}",
                required_capabilities.join(", ")
            ));
        }
        if versioned
            && paths.is_empty()
            && required_capabilities
                .iter()
                .any(|cap| matches!(cap.as_str(), "write_file" | "apply_patch"))
        {
            return Err(format!(
                "task {task_id} requires a write capability but declares no write_paths"
            ));
        }
        if required_capabilities
            .iter()
            .any(|cap| matches!(cap.as_str(), "generate_image" | "text_to_speech"))
            && !paths
                .iter()
                .any(|path| write_path_is_within("outputs", path))
        {
            return Err(format!(
                "task {task_id} uses a built-in media tool that writes workspace/outputs; write_paths must include outputs"
            ));
        }
        let (verification, verification_plan) = match item.get("verification") {
            Some(Value::String(value)) => {
                let value = value.trim();
                let valid = value.len() <= 2_048
                    && (value == "non_empty"
                        || value
                            .strip_prefix("contains:")
                            .is_some_and(|expected| !expected.is_empty())
                        || value
                            .strip_prefix("equals:")
                            .is_some_and(|expected| !expected.is_empty()));
                if !valid {
                    return Err(format!("task {task_id} has invalid legacy verification"));
                }
                (Some(value.to_string()), None)
            }
            Some(value @ Value::Object(_)) => {
                let mut plan: crate::plan::VerificationPlanV1 =
                    serde_json::from_value(value.clone()).map_err(|error| {
                        format!("task {task_id} has invalid VerificationPlan: {error}")
                    })?;
                plan.plan_id = format!("verify-{task_id}");
                if plan.requirements.len() > 16 {
                    return Err(format!("task {task_id} has more than 16 verification requirements"));
                }
                plan.validate()
                    .map_err(|error| format!("task {task_id} has invalid VerificationPlan: {error}"))?;
                for (index, requirement) in plan.requirements.iter_mut().enumerate() {
                    let crate::plan::VerificationScopeV1::WorkspacePaths { relative_paths } =
                        &mut requirement.scope
                    else {
                        return Err(format!(
                            "task {task_id} dynamic VerificationPlan only permits WorkspacePaths"
                        ));
                    };
                    if !requirement.required
                        || requirement.resources.cpu_slots != 1
                        || !(8..=128).contains(&requirement.resources.memory_mb)
                        || requirement.resources.exclusive_workspace
                        || requirement.resources.timeout_ms == 0
                        || requirement.resources.timeout_ms > 30_000
                    {
                        return Err(format!(
                            "task {task_id} verification requirements must be required and stay within the host resource budget"
                        ));
                    }
                    if !crate::verification::is_registered_workspace_validator(
                        &requirement.validator_id,
                    ) || !crate::verification::workspace_validator_arguments_supported(
                        &requirement.validator_id,
                        &requirement.arguments,
                    ) {
                        return Err(format!(
                            "task {task_id} uses unregistered workspace validator {}",
                            requirement.validator_id
                        ));
                    }
                    if relative_paths.iter().any(|path| path.len() > 512) {
                        return Err(format!(
                            "task {task_id} VerificationPlan path exceeds 512 bytes"
                        ));
                    }
                    super::roles::validate_role_write_paths(
                        &format!("task-{task_id}-validator"),
                        relative_paths,
                    )?;
                    // Verification commonly scopes a directory (tests/systems)
                    // while the task declares exact files. Either direction is
                    // safe and still bounded by the task write lease.
                    if relative_paths.iter().any(|path| {
                        !paths.iter().any(|write_scope| {
                            write_path_is_within(path, write_scope)
                                || write_path_is_within(write_scope, path)
                        })
                    }) {
                        return Err(format!(
                            "task {task_id} VerificationPlan scope exceeds its assigned write_paths"
                        ));
                    }
                    requirement.requirement_id =
                        format!("{task_id}:requirement:{index}");
                    requirement.covers_requirement_ids.clear();
                }
                plan.validate()
                    .map_err(|error| format!("task {task_id} has invalid VerificationPlan: {error}"))?;
                (None, Some(plan))
            }
            None if !versioned => (Some("non_empty".to_string()), None),
            _ => {
                return Err(format!(
                    "task {task_id} requires verification: a registered WorkspacePaths VerificationPlan or legacy non_empty|contains:<text>|equals:<text>"
                ))
            }
        };
        let has_command_validator = verification_plan.as_ref().is_some_and(|plan| {
            plan.requirements
                .iter()
                .any(|requirement| requirement.validator_id == "workspace-command-success-v1")
        });
        if has_command_validator
            && !required_capabilities
                .iter()
                .any(|capability| capability == "run_command")
        {
            // A behavior command cannot run without this capability. Add the
            // hint when the plan proves it is needed; the role scope check below
            // still rejects workers that do not expose run_command.
            required_capabilities.push("run_command".to_string());
        }
        let requires_command_capability = required_capabilities
            .iter()
            .any(|capability| capability == "run_command");
        let declared_source_code = paths
            .iter()
            .any(|path| super::delivery_gate_evidence::is_source_code_path(path));
        if declared_source_code && !has_command_validator {
            return Err(format!(
                "task {task_id} declares a source-code write scope but no registered behavior command"
            ));
        }
        // required_capabilities is an optional execution hint. When omitted,
        // the role's own tool scope still controls whether run_command exists;
        // only reject an explicit contradictory declaration.
        if !required_capabilities.is_empty() && has_command_validator != requires_command_capability
        {
            return Err(format!(
                "task {task_id} must request run_command exactly when its VerificationPlan includes workspace-command-success-v1"
            ));
        }
        let risk = item
            .get("risk")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| matches!(*value, "low" | "normal" | "high" | "critical"))
            .map(str::to_owned)
            .unwrap_or_else(|| "normal".to_string());
        if matches!(risk.as_str(), "high" | "critical")
            && !roles.iter().any(is_independent_reviewer_role)
        {
            return Err(format!(
                "high-risk task {task_id} requires an independent reviewer role",
            ));
        }
        let priority = item
            .get("priority")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .min(100) as u8;
        let depends_on = match item.get("depends_on") {
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|dependency| !dependency.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| format!("task {task_id} has invalid dependency"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            None | Some(Value::Null) if versioned => {
                return Err(format!(
                    "task {task_id} requires depends_on array in versioned graph"
                ))
            }
            None | Some(Value::Null) => Vec::new(),
            // A versioned TaskGraph is a host contract: a malformed dependency
            // field must not silently become "no dependency" and reorder writes.
            _ if versioned => return Err(format!("task {task_id} has invalid depends_on array")),
            // Some models emit a scalar or an object for an independent task.
            // Treat it as no dependency rather than losing the whole plan.
            _ => Vec::new(),
        };
        if depends_on.len() > 128 {
            return Err(format!("task {task_id} has more than 128 dependencies"));
        }
        let unique_dependencies: std::collections::BTreeSet<_> = depends_on.iter().collect();
        if unique_dependencies.len() != depends_on.len() {
            return Err(format!("task {task_id} has duplicate dependencies"));
        }
        tasks.push(ParallelTaskAssignment {
            task_id,
            worker,
            worker_explicit,
            task,
            acceptance,
            user_requirement_quotes,
            depends_on,
            effective_paths: paths,
            read_refs,
            contract_refs,
            required_capabilities,
            estimated_effort,
            model_calls_per_attempt: 0,
            verification,
            verification_plan,
            risk,
            priority,
        });
    }
    for task in &tasks {
        for dep in &task.depends_on {
            if dep == &task.task_id || !ids.contains(dep) {
                return Err(format!(
                    "task {} has invalid dependency {dep}",
                    task.task_id
                ));
            }
        }
    }
    fn visit(
        id: &str,
        tasks: &[ParallelTaskAssignment],
        path: &mut std::collections::BTreeSet<String>,
        done: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        if done.contains(id) {
            return Ok(());
        }
        if !path.insert(id.to_owned()) {
            return Err(format!("task graph cycle at {id}"));
        }
        if let Some(task) = tasks.iter().find(|task| task.task_id == id) {
            for dep in &task.depends_on {
                visit(dep, tasks, path, done)?;
            }
        }
        path.remove(id);
        done.insert(id.to_owned());
        Ok(())
    }
    let mut path = std::collections::BTreeSet::new();
    let mut done = std::collections::BTreeSet::new();
    for id in &ids {
        visit(id, &tasks, &mut path, &mut done)?;
    }
    let mut critical_paths = std::collections::BTreeMap::new();
    for id in &ids {
        let effort = task_critical_path_effort(id, &tasks, &mut critical_paths);
        critical_paths.insert(id.clone(), effort);
    }

    // Respect explicit worker commitments first, then place implicit tasks largest/longest
    // first onto the currently lightest worker. This avoids JSON input order bias and keeps
    // declared work in the load estimate before assigning the remaining queue.
    let mut assigned_effort = std::collections::BTreeMap::<String, u64>::new();
    for task in tasks.iter().filter(|task| task.worker_explicit) {
        let load = assigned_effort.entry(task.worker.clone()).or_default();
        *load = load.saturating_add(task.estimated_effort);
    }
    let mut implicit_indices: Vec<usize> = tasks
        .iter()
        .enumerate()
        .filter_map(|(index, task)| (!task.worker_explicit).then_some(index))
        .collect();
    implicit_indices.sort_by(|left, right| {
        let left_task = &tasks[*left];
        let right_task = &tasks[*right];
        let left_path = critical_paths
            .get(&left_task.task_id)
            .copied()
            .unwrap_or(left_task.estimated_effort);
        let right_path = critical_paths
            .get(&right_task.task_id)
            .copied()
            .unwrap_or(right_task.estimated_effort);
        (
            right_path,
            right_task.priority,
            right_task.estimated_effort,
            &right_task.task_id,
        )
            .cmp(&(
                left_path,
                left_task.priority,
                left_task.estimated_effort,
                &left_task.task_id,
            ))
    });
    for index in implicit_indices {
        let task = &tasks[index];
        let worker = writers
            .iter()
            .filter(|role| {
                role_supports_task_capabilities(role, &task.required_capabilities)
                    && (role.write_paths.is_empty()
                        || task.effective_paths.iter().all(|path| {
                            role.write_paths
                                .iter()
                                .any(|base| write_path_is_within(path, base))
                        }))
            })
            .min_by_key(|role| {
                (
                    assigned_effort.get(&role.role).copied().unwrap_or(0),
                    role.role.as_str(),
                )
            })
            .ok_or_else(|| {
                format!(
                    "task {} has no task_worker node whose declared tools and write scope cover: tools=[{}], paths=[{}]",
                    task.task_id,
                    task.required_capabilities.join(", "),
                    task.effective_paths.join(", ")
                )
            })?
            .role
            .clone();
        let task = &mut tasks[index];
        task.worker = worker.clone();
        let load = assigned_effort.entry(worker).or_default();
        *load = load.saturating_add(task.estimated_effort);
    }
    for task in &tasks {
        let role = writers
            .iter()
            .find(|role| role.role == task.worker)
            .expect("all tasks are assigned to a writer");
        if !role_supports_task_capabilities(role, &task.required_capabilities) {
            return Err(format!(
                "task {} assigned to {} requires tools outside that node's declared tool surface: {}",
                task.task_id,
                task.worker,
                task.required_capabilities.join(", ")
            ));
        }
        super::roles::validate_role_write_paths(&task.worker, &task.effective_paths)?;
        if !role.write_paths.is_empty() {
            for path in &task.effective_paths {
                let inside = role
                    .write_paths
                    .iter()
                    .any(|base| write_path_is_within(path, base));
                if !inside {
                    return Err(format!(
                        "task {} exceeds pre-authorized writer scope: {path}",
                        task.task_id
                    ));
                }
            }
        }
    }

    let mut ordered = Vec::with_capacity(tasks.len());
    let mut remaining = tasks;
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                task.depends_on.iter().all(|dependency| {
                    ordered
                        .iter()
                        .any(|done: &ParallelTaskAssignment| done.task_id == *dependency)
                })
            })
            .max_by_key(|(_, task)| {
                (
                    critical_paths
                        .get(&task.task_id)
                        .copied()
                        .unwrap_or(task.estimated_effort),
                    task.priority,
                    task.estimated_effort,
                )
            })
            .map(|(index, _)| index);
        let Some(index) = ready else {
            return Err("task graph has no topological order".into());
        };
        ordered.push(remaining.remove(index));
    }
    let tasks = ordered;
    for left in 0..tasks.len() {
        for right in left + 1..tasks.len() {
            if tasks[left].effective_paths.iter().any(|a| {
                tasks[right]
                    .effective_paths
                    .iter()
                    .any(|b| write_paths_overlap(a, b))
            }) {
                let ordered = depends_on(&tasks[left].task_id, &tasks[right].task_id, &tasks)
                    || depends_on(&tasks[right].task_id, &tasks[left].task_id, &tasks);
                if !ordered {
                    return Err(format!(
                        "overlapping task write scopes need a dependency: {} / {}",
                        tasks[left].task_id, tasks[right].task_id
                    ));
                }
            }
        }
    }
    Ok(TaskGraphV1 { version: 1, tasks })
}

fn task_critical_path_effort(
    task_id: &str,
    tasks: &[ParallelTaskAssignment],
    memo: &mut std::collections::BTreeMap<String, u64>,
) -> u64 {
    if let Some(effort) = memo.get(task_id) {
        return *effort;
    }
    let Some(task) = tasks.iter().find(|task| task.task_id == task_id) else {
        return 0;
    };
    let longest_child = tasks
        .iter()
        .filter(|candidate| {
            candidate
                .depends_on
                .iter()
                .any(|dependency| dependency == task_id)
        })
        .map(|child| task_critical_path_effort(&child.task_id, tasks, memo))
        .max()
        .unwrap_or(0);
    let effort = task.estimated_effort.saturating_add(longest_child);
    memo.insert(task_id.to_string(), effort);
    effort
}

fn depends_on(before: &str, after: &str, tasks: &[ParallelTaskAssignment]) -> bool {
    let mut pending = vec![after.to_owned()];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if id == before {
            return true;
        }
        if seen.insert(id.clone()) {
            if let Some(task) = tasks.iter().find(|task| task.task_id == id) {
                pending.extend(task.depends_on.iter().cloned());
            }
        }
    }
    false
}

fn normalized_write_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join("/")
        .to_lowercase()
}

fn write_path_is_within(path: &str, base: &str) -> bool {
    let path = normalized_write_path(path);
    let base = normalized_write_path(base);
    base.is_empty() || path == base || path.starts_with(&format!("{base}/"))
}

pub(super) fn write_paths_overlap(left: &str, right: &str) -> bool {
    let left = normalized_write_path(left);
    let right = normalized_write_path(right);
    left.is_empty()
        || right.is_empty()
        || left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

pub(super) fn is_parallel_writer_name(role: &str) -> bool {
    crate::worker_profile::is_parallel_writer_name(role)
}

/// 解析 lead 产物的 TaskGraphV1：裸 JSON / 围栏 / 前后缀文本取首个 `{...}`；
/// 兼容旧裸数组与 subtasks 对象。解析失败返回 None，由调用方终止团队，避免虚假成功。
pub(super) fn parse_parallel_subtasks(output: &str) -> Option<(Vec<Value>, bool)> {
    let text = crate::workswarm_output::strip_code_fences(output);
    let parsed: Value = serde_json::from_str(text.trim()).ok().or_else(|| {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        (end > start).then(|| serde_json::from_str(&text[start..=end]).ok())?
    })?;
    match parsed {
        Value::Array(items) => Some((items, false)),
        Value::Object(obj) => {
            let versioned = obj.contains_key("tasks");
            // 真实模型常把版本写成 `"1"`（字符串）。TaskGraphV1 的语义版本仍是 1，
            // 数字/字符串两种写法都应接受；其他版本继续拒绝，避免把未知协议当 V1。
            let mut version = obj
                .get("version")
                .or_else(|| obj.get("graph_version"))
                .and_then(|value| {
                    value.as_u64().or_else(|| {
                        value
                            .as_str()
                            .and_then(|text| text.trim().parse::<u64>().ok())
                    })
                })
                .or_else(|| {
                    obj.get("graph_version")
                        .and_then(Value::as_str)
                        .is_some_and(|marker| marker.trim().eq_ignore_ascii_case("TaskGraphV1"))
                        .then_some(1)
                });
            // 真实模型也常写 `"schema":"TaskGraphV1"` 而不写 version。schema 标记
            // 已明确是 V1 时按 version=1 接受；未知 schema 仍交给 version 校验。
            if version.is_none()
                && obj
                    .get("schema")
                    .and_then(Value::as_str)
                    .is_some_and(|schema| schema.trim().eq_ignore_ascii_case("TaskGraphV1"))
            {
                version = Some(1);
            }
            if versioned && version != Some(1) {
                return None;
            }
            if version.is_some_and(|version| version != 1) {
                return None;
            }
            obj.get("tasks")
                .or_else(|| obj.get("subtasks"))
                .and_then(Value::as_array)
                .cloned()
                .map(|items| (items, versioned))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "task_graph_tests.rs"]
mod tests;
