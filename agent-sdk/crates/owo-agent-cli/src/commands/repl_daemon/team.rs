//! `/team` 命令的解析、跟踪与状态渲染（自 `repl_daemon.rs` 机械提取）。
//!
//! 仅承载团队相关纯逻辑与终端渲染；调用方通过 `use team::*;` 保持原名称。
use colored::Colorize;

pub(crate) fn team_status(detail: &serde_json::Value) -> String {
    detail
        .get("team")
        .and_then(|team| team.get("status"))
        .and_then(|value| value.as_str())
        .unwrap_or("running")
        .to_string()
}

pub(crate) fn team_status_is_terminal(status: &str) -> bool {
    ["succeeded", "failed", "cancelled", "canceled"]
        .iter()
        .any(|terminal| status.eq_ignore_ascii_case(terminal))
}

pub(super) fn seed_team_step_states(
    detail: &serde_json::Value,
    last: &mut std::collections::HashMap<String, String>,
) {
    if let Some(tasks) = detail.get("tasks").and_then(|value| value.as_array()) {
        for task in tasks {
            if let (Some(id), Some(status)) = (
                task.get("task_id").and_then(|value| value.as_str()),
                task.get("status").and_then(|value| value.as_str()),
            ) {
                last.insert(id.to_string(), status.to_string());
            }
        }
    }
}

pub(super) fn print_team_step_updates(
    steps: &[serde_json::Value],
    last: &mut std::collections::HashMap<String, String>,
) {
    for step in steps {
        let Some(id) = step
            .get("step_id")
            .or_else(|| step.get("task_id"))
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        let role = step
            .get("worker")
            .and_then(|value| value.as_str())
            .unwrap_or("?");
        let state = step
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("Pending");
        if last.get(id).is_some_and(|previous| previous == state) {
            continue;
        }
        let line = match state.to_ascii_lowercase().as_str() {
            "claimed" => format!("  {} {role} 已领取，等待执行", "◷".yellow()),
            "running" => format!("  {} {role} 开始执行", "▶".blue()),
            "succeeded" => format!("  {} {role} 完成", "✔".green()),
            "failed" | "aborted" => format!(
                "  {} {role} 失败：{}",
                "✘".red(),
                step.get("error")
                    .and_then(|value| value.as_str())
                    .unwrap_or("查看 /team status")
            ),
            "skipped" => format!("  {} {role} 跳过", "○".dimmed()),
            _ => format!("  {} {role} {state}", "○".dimmed()),
        };
        println!("{line}");
        last.insert(id.to_string(), state.to_string());
    }
}

pub(super) fn team_audit_signature(event: &serde_json::Value) -> Option<u64> {
    use std::hash::{Hash, Hasher};

    let event_name = event.get("event")?.as_str()?;
    let detail = event
        .get("detail")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let timestamp = event
        .get("ts")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    timestamp.hash(&mut hasher);
    event_name.hash(&mut hasher);
    detail.hash(&mut hasher);
    Some(hasher.finish())
}

fn team_audit_field(detail: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    detail
        .split_whitespace()
        .find_map(|field| field.strip_prefix(&prefix))
        .map(|value| {
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(80)
                .collect::<String>()
        })
        .filter(|value| !value.is_empty())
}

pub(crate) fn team_audit_summary(event: &serde_json::Value) -> Option<String> {
    let name = event.get("event")?.as_str()?;
    let detail = event
        .get("detail")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let field = |key| team_audit_field(detail, key).unwrap_or_else(|| "?".to_string());
    match name {
        "team.worker.started" => Some(format!(
            "{} 开始执行（{}）",
            field("role"),
            field("step_id")
        )),
        "team.tool.started" => Some(format!(
            "{} 正在使用 {} · {}",
            field("role"),
            field("tool"),
            field("step_id")
        )),
        "team.tool.finished" => Some(format!(
            "{} 使用 {}：{} · {} ms · {}",
            field("role"),
            field("tool"),
            field("outcome"),
            field("duration_ms"),
            field("step_id")
        )),
        "team.model.started" => Some(format!(
            "{} 发起模型请求 · {}",
            field("role"),
            field("step_id")
        )),
        "team.model.request_completed" => Some(format!(
            "模型请求：{} · {} · {} ms · {} tokens",
            field("role"),
            field("model"),
            field("latency_ms"),
            field("usage_tokens")
        )),
        "team.lease.wait_completed" => Some(format!(
            "{} 等待写入租约 {} ms",
            field("role"),
            field("wait_ms")
        )),
        "team.worker.finished" => Some(format!(
            "{} 执行结束：{} · {} ms",
            field("role"),
            field("outcome"),
            field("wall_ms")
        )),
        "team.context.fact_published" => Some(format!(
            "共享事实更新：{} · revision {}",
            field("key"),
            field("revision")
        )),
        "team.artifact.validation_started" => Some(format!(
            "产物校验开始（{}，{}）",
            field("member"),
            field("format")
        )),
        "team.artifact.validation_passed" => Some(format!(
            "产物校验通过（{}，{} · {} ms）",
            field("member"),
            field("format"),
            field("duration_ms")
        )),
        "team.artifact.validation_rejected" => Some(format!(
            "产物校验未通过（{} · {} ms）",
            team_audit_field(detail, "step").unwrap_or_else(|| "查看 /team status".to_string()),
            field("duration_ms")
        )),
        _ => None,
    }
}

pub(super) fn print_team_watch_terminal(status: &str, detail: &serde_json::Value) {
    if let Some(tasks) = detail.get("tasks").and_then(|value| value.as_array()) {
        for task in tasks.iter().filter(|task| {
            task.get("status")
                .and_then(|value| value.as_str())
                .is_some_and(|state| {
                    matches!(state.to_ascii_lowercase().as_str(), "failed" | "aborted")
                })
        }) {
            let role = task
                .get("role")
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            let error = task
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("未知");
            println!("  {} {role} 失败：{error}", "✘".red());
        }
    }
    let mark = if status.eq_ignore_ascii_case("succeeded") {
        "✓".green().to_string()
    } else {
        "✘".red().to_string()
    };
    println!("{mark} 团队终态：{status}（/team diff 看真实变更集，/team status 看详情）");
}

/// 团队详情渲染（状态 / 成员 / 任务图 / 审计尾迹）——`/team status` 与跟踪共用。
pub(super) fn print_team_detail(detail: &serde_json::Value) {
    let team = detail.get("team").cloned().unwrap_or_default();
    let id = team.get("team_id").and_then(|v| v.as_str()).unwrap_or("?");
    let status = team.get("status").and_then(|v| v.as_str()).unwrap_or("?");
    let mode = team.get("mode").and_then(|v| v.as_str()).unwrap_or("?");
    let template = team
        .get("template_id")
        .and_then(|v| v.as_str())
        .unwrap_or("（动态组队）");
    let interrupted = detail
        .get("interrupted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    println!(
        "{} {id}  {status}  mode={mode}  模板={template}{}",
        "团队：".bold(),
        if interrupted { "  · 已中断" } else { "" }
    );
    if let Some(members) = team.get("members").and_then(|v| v.as_array()) {
        let names = members
            .iter()
            .filter_map(|member| member.get("user_id").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        if !names.is_empty() {
            println!("  成员：{names}");
        }
    }
    if let Some(tasks) = detail.get("tasks").and_then(|v| v.as_array()) {
        println!("  任务图：");
        for task in tasks {
            let task_id = task.get("task_id").and_then(|v| v.as_str()).unwrap_or("?");
            let role = task.get("role").and_then(|v| v.as_str()).unwrap_or("?");
            let state = task
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Pending");
            let mark = match state {
                "Succeeded" => "✔".green().to_string(),
                "Running" => "▶".blue().to_string(),
                "Failed" => "✘".red().to_string(),
                _ => "○".dimmed().to_string(),
            };
            let error = task
                .get("error")
                .and_then(|v| v.as_str())
                .map(|error| format!("：{error}"))
                .unwrap_or_default();
            println!("    {mark} {task_id}  {role}  {state}{error}");
        }
    }
    if let Some(tail) = detail.get("audit_tail").and_then(|v| v.as_array()) {
        if !tail.is_empty() {
            println!("  审计尾迹（最近 {} 条）：", tail.len());
            for entry in tail.iter().take(5) {
                let event = entry.get("event").and_then(|v| v.as_str()).unwrap_or("?");
                let text = entry.get("detail").and_then(|v| v.as_str()).unwrap_or("");
                println!("    {event}  {text}");
            }
        }
    }
}

/// CLI 决定 ChangeSet 的参数；stable idempotency identity 使网络重试可安全重放。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TeamChangeSetDecision {
    pub(super) action: String,
    pub(super) change_set_id: String,
    pub(super) note: Option<String>,
}

pub(super) fn team_change_set_idempotency_key(
    team_id: &str,
    decision: &TeamChangeSetDecision,
) -> String {
    format!(
        "cli-team:{team_id}:{}:{}",
        decision.change_set_id, decision.action
    )
}

pub(super) fn parse_team_change_set_decision(
    action: &str,
    args: &[String],
) -> Result<TeamChangeSetDecision, String> {
    if !matches!(action, "accept" | "reject" | "revert") {
        return Err(format!("未知 ChangeSet 操作：{action}"));
    }
    let change_set_id = args
        .first()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("用法：/team {action} <ChangeSet ID> [说明]"))?;
    if matches!(change_set_id, "." | "..")
        || !change_set_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
    {
        return Err("ChangeSet ID 只能包含 ASCII 字母、数字、连字符、下划线、冒号或点".to_string());
    }
    let note = args
        .iter()
        .skip(1)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    Ok(TeamChangeSetDecision {
        action: action.to_string(),
        change_set_id: change_set_id.to_string(),
        note: (!note.is_empty()).then_some(note),
    })
}

/// `/team` 前置参数解析结果（纯逻辑，便于单测）。
#[derive(Debug, Default, PartialEq)]
pub(super) struct TeamArgs {
    /// 显式策略（`--single|--team|--auto`；None = 按是否声明角色/并行取缺省）。
    pub(super) strategy: Option<String>,
    /// 自定义角色（RoleSpec JSON：role/assignee/depends_on[/model/write_paths]）。
    pub(super) roles: Vec<serde_json::Value>,
    /// 非前置参数（子命令或目标片段）；空 = 仅打印用法。
    pub(super) rest: Vec<String>,
    /// 团队统一模型（`--model <模型>`，全队 agent 共用；None = 用默认常量）。
    pub(super) model: Option<String>,
    /// 并行路数（`--parallel N`：lead 拆解 → w1..wN 并行 → leader 汇总）。
    pub(super) parallel: Option<usize>,
}

/// 解析 `/team` 前置参数：
/// - `--single|--team|--auto`：策略开关；
/// - `--parallel <N>`（2..=8）：**并行开发**——生成 `lead`（只读拆解）→ `w1..wN`
///   （依赖 lead，彼此无依赖 = 同 wave 真并行）→ `leader`（汇总）；运行期 lead
///   产出的 `subtasks`（子任务 + 写范围）动态应用到对应 writer；
/// - `--role <名[:依赖1|依赖2]>`（可重复）：自定义角色；与 `--parallel` 互斥；
/// - `--model <模型>`：团队统一模型（全队共用）；`--model <角色>=<模型>[,…]`：
///   角色级覆盖（高级用法）；不传则读 `<workspace>/settings.json` 的 `team.model`；
/// - `--write <角色>=<路径>[;<路径>]`：角色写范围（互不重叠 → 并发落盘）。
///
/// 错误：缺参、角色重复/为空、`--parallel` 与 `--role` 同用、并行路数越界、
/// `--model`/`--write` 指向未声明角色、映射格式非法。
pub(super) fn apply_team_execution_intent(
    body: &mut serde_json::Value,
    parallel: Option<usize>,
    automatic_parallel: bool,
) {
    if let Some(writers) = parallel {
        body["parallel"] = serde_json::json!(true);
        body["max_agent_members"] = serde_json::json!(writers + 2);
        body["budget"] = serde_json::json!({ "max_parallel": writers });
    } else if automatic_parallel {
        // Capacity remains unset so the server can honor workspace settings.
        body["parallel"] = serde_json::json!(true);
    }
}

pub(super) fn resolve_team_create_intent(
    strategy: Option<String>,
    roles: &[serde_json::Value],
    parallel: Option<usize>,
) -> (String, bool) {
    let automatic_parallel = strategy.is_none() && roles.is_empty() && parallel.is_none();
    let strategy = strategy.unwrap_or_else(|| {
        if automatic_parallel || !roles.is_empty() || parallel.is_some() {
            "team".to_string()
        } else {
            "auto".to_string()
        }
    });
    (strategy, automatic_parallel)
}

pub(super) fn parse_team_args(args: &[String]) -> Result<TeamArgs, String> {
    let mut parsed = TeamArgs::default();
    let mut role_specs: Vec<String> = Vec::new();
    let mut model_specs: Vec<String> = Vec::new();
    let mut write_specs: Vec<String> = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--single" => {
                parsed.strategy = Some("single".to_string());
                index += 1;
            }
            "--team" => {
                parsed.strategy = Some("team".to_string());
                index += 1;
            }
            "--auto" => {
                parsed.strategy = Some("auto".to_string());
                index += 1;
            }
            "--parallel" => {
                let value = args
                    .get(index + 1)
                    .map(String::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| "--parallel 需要参数（2..=8）".to_string())?;
                let writers: usize = value
                    .parse()
                    .map_err(|_| format!("--parallel 需要 2..=8 的数字：{value}"))?;
                if !(2..=8).contains(&writers) {
                    return Err(format!("--parallel 需要在 2..=8 之间：{writers}"));
                }
                parsed.parallel = Some(writers);
                index += 2;
            }
            flag @ ("--role" | "--model" | "--write") => {
                let value = args
                    .get(index + 1)
                    .map(String::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| format!("{flag} 需要参数"))?
                    .to_string();
                index += 2;
                match flag {
                    "--role" => role_specs.push(value),
                    "--model" => model_specs.push(value),
                    _ => write_specs.push(value),
                }
            }
            _ => break,
        }
    }
    parsed.rest = args[index..].to_vec();

    if parsed.parallel.is_some() && !role_specs.is_empty() {
        return Err(
            "--parallel 与 --role 互斥（并行模式已内置 lead/w1..wN/leader 角色）".to_string(),
        );
    }

    if let Some(writers) = parsed.parallel {
        // 十一期：并行开发角色组由核心提供（契约与运行期分配口径同源）。
        parsed.roles = owo_agent_core::workswarm::parallel_roles(writers)
            .into_iter()
            .map(|role| serde_json::to_value(role).unwrap_or_default())
            .collect();
    } else {
        // `名[:依赖1|依赖2]` → RoleSpec JSON（assignee=agent，缺省模型/写范围由服务端解析）。
        for spec in &role_specs {
            let (name, deps) = match spec.split_once(':') {
                Some((name, deps)) => (
                    name.trim(),
                    deps.split(['|', ','])
                        .map(str::trim)
                        .filter(|d| !d.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                ),
                None => (spec.trim(), Vec::new()),
            };
            if name.is_empty() {
                return Err("--role 角色名不能为空".to_string());
            }
            if parsed
                .roles
                .iter()
                .any(|r| r.get("role").and_then(|v| v.as_str()) == Some(name))
            {
                return Err(format!("--role 角色重复：{name}"));
            }
            parsed.roles.push(serde_json::json!({
                "role": name,
                "assignee": "agent",
                "depends_on": deps,
            }));
        }
    }
    let role_names: Vec<String> = parsed
        .roles
        .iter()
        .filter_map(|r| r.get("role").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    let find_role = |roles: &mut [serde_json::Value], role: &str| -> Result<(), String> {
        roles
            .iter_mut()
            .find(|r| r.get("role").and_then(|v| v.as_str()) == Some(role))
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "角色 {role} 不在 --role 列表（现有：{}）",
                    role_names.join(", ")
                )
            })
    };
    for spec in &model_specs {
        let spec = spec.trim();
        // 无 `=`：团队统一模型（`--model glm-5.3-flashx` 全队共用，最后一条生效）。
        if !spec.contains('=') {
            if spec.contains(',') {
                return Err(format!("--model 统一模型不能含逗号：{spec}"));
            }
            parsed.model = Some(spec.to_string());
            continue;
        }
        for pair in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (role, model) = pair
                .split_once('=')
                .ok_or_else(|| format!("--model 需要 <角色>=<模型>：{pair}"))?;
            let (role, model) = (role.trim(), model.trim());
            if role.is_empty() || model.is_empty() {
                return Err(format!("--model 需要 <角色>=<模型>：{pair}"));
            }
            find_role(&mut parsed.roles, role)?;
            let target = parsed
                .roles
                .iter_mut()
                .find(|r| r.get("role").and_then(|v| v.as_str()) == Some(role))
                .expect("find_role 已校验存在");
            target["model"] = serde_json::json!(model);
        }
    }
    for spec in &write_specs {
        let (role, paths) = spec
            .split_once('=')
            .ok_or_else(|| format!("--write 需要 <角色>=<路径[;路径]>：{spec}"))?;
        let role = role.trim();
        let paths: Vec<String> = paths
            .split(';')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        if role.is_empty() || paths.is_empty() {
            return Err(format!("--write 需要 <角色>=<路径[;路径]>：{spec}"));
        }
        find_role(&mut parsed.roles, role)?;
        let target = parsed
            .roles
            .iter_mut()
            .find(|r| r.get("role").and_then(|v| v.as_str()) == Some(role))
            .expect("find_role 已校验存在");
        target["write_paths"] = serde_json::json!(paths);
    }
    Ok(parsed)
}
