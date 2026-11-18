//! `.owflow` 定义校验、条件表达式求值与 ProgramNode 编译（从 workflow.rs 拆出）。

use super::*;
use crate::action_program::ProgramNode;
use std::collections::{BTreeMap, HashMap};

/// 校验 .owflow 定义；`known_flows` 为可引用的子流程 id 集合。
/// 返回全部错误（非法定义明确报错，不含模糊失败）。
pub fn validate_definition(
    flow: &WorkflowDefinition,
    known_flows: &[String],
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if flow.id.trim().is_empty() {
        errors.push("flow.id 不能为空".to_string());
    }
    if flow.name.trim().is_empty() {
        errors.push("flow.name 不能为空".to_string());
    }
    if flow.version == 0 {
        errors.push("flow.version 必须 >= 1".to_string());
    }
    if flow.triggers.is_empty() {
        errors.push("flow.triggers 至少需要一个触发器（如 manual）".to_string());
    }
    let trigger_ids: Vec<&str> = flow.triggers.iter().map(|t| t.id.as_str()).collect();
    if trigger_ids.len() != flow.triggers.len() {
        errors.push("flow.triggers.id 重复".to_string());
    }
    if flow.steps.is_empty() {
        errors.push("flow.steps 不能为空".to_string());
    }
    let mut ids = Vec::new();
    collect_step_ids(&flow.steps, &mut ids);
    let mut seen = HashMap::new();
    for id in &ids {
        if seen.insert(id.clone(), ()).is_some() {
            errors.push(format!("步骤 id 重复：{id}"));
        }
    }
    for point in &flow.rollback_points {
        if !seen.contains_key(point) {
            errors.push(format!("rollback_points 引用不存在的步骤：{point}"));
        }
    }
    if flow.max_steps == 0 {
        errors.push("flow.max_steps 必须 >= 1".to_string());
    }
    if flow.subflow_depth_limit == 0 {
        errors.push("flow.subflow_depth_limit 必须 >= 1".to_string());
    }
    // 子流程引用存在性
    for step in &flow.steps {
        collect_subflow_refs(step, &mut errors, known_flows);
    }
    // 权限 scope 非空
    for claim in &flow.permissions {
        if claim.scope.trim().is_empty() {
            errors.push("permissions.scope 不能为空".to_string());
        }
    }
    // 前置条件表达式合法性
    let empty_ctx = BTreeMap::new();
    for expr in &flow.preconditions {
        if eval_expr(expr, &empty_ctx).is_err() {
            errors.push(format!("前置条件表达式非法：{expr}"));
        }
    }
    // 断言/条件表达式合法性
    for id in &ids {
        if let Some(expr) = find_expr(flow, id) {
            if eval_expr(&expr, &empty_ctx).is_err() {
                errors.push(format!("表达式非法（步骤 {id}）：{expr}"));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn step_id(step: &WorkflowStep) -> Option<&str> {
    match step {
        WorkflowStep::Sense { id, .. }
        | WorkflowStep::Locate { id, .. }
        | WorkflowStep::Act { id, .. }
        | WorkflowStep::Assert { id, .. }
        | WorkflowStep::InvokeSkill { id, .. }
        | WorkflowStep::InvokeMcp { id, .. }
        | WorkflowStep::HumanApprove { id, .. }
        | WorkflowStep::Notify { id, .. }
        | WorkflowStep::Subflow { id, .. }
        | WorkflowStep::Loop { id, .. }
        | WorkflowStep::Cond { id, .. }
        | WorkflowStep::RollbackPoint { id, .. } => Some(id),
    }
}

fn collect_subflow_refs(step: &WorkflowStep, errors: &mut Vec<String>, known_flows: &[String]) {
    match step {
        WorkflowStep::Subflow { flow, .. } => {
            if !known_flows.iter().any(|f| f == flow) {
                errors.push(format!("子流程引用不存在：{flow}"));
            }
        }
        WorkflowStep::Loop { body, .. } => {
            for sub in body {
                collect_subflow_refs(sub, errors, known_flows);
            }
        }
        WorkflowStep::Cond {
            then, otherwise, ..
        } => {
            for sub in then.iter().chain(otherwise.iter()) {
                collect_subflow_refs(sub, errors, known_flows);
            }
        }
        _ => {}
    }
}

fn find_expr(flow: &WorkflowDefinition, id: &str) -> Option<String> {
    fn walk(steps: &[WorkflowStep], id: &str) -> Option<String> {
        for step in steps {
            match step {
                WorkflowStep::Assert { expr, .. } if step_id(step) == Some(id) => {
                    return Some(expr.clone())
                }
                WorkflowStep::Cond {
                    expr,
                    then,
                    otherwise,
                    ..
                } => {
                    if step_id(step) == Some(id) {
                        return Some(expr.clone());
                    }
                    if let Some(found) = walk(then, id).or_else(|| walk(otherwise, id)) {
                        return Some(found);
                    }
                }
                WorkflowStep::Loop { body, .. } => {
                    if let Some(found) = walk(body, id) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }
    walk(&flow.steps, id)
}

// ---------------------------------------------------------------------------
// 条件表达式求值（v1 子集）
// ---------------------------------------------------------------------------

/// Increment whenever evaluator semantics change: cached recovery decisions
/// must not silently survive a different interpretation of the same program.
pub const EXPRESSION_SEMANTICS_VERSION: u32 = 2;

/// 表达式求值：`exists(k)`、`k == v`、`k != v`、`k > n`、`k >= n`、`k < n`、`k <= n`、`true`/`false`。
/// ctx 为引擎运行上下文（key → 值）。
pub fn eval_expr(expr: &str, ctx: &BTreeMap<String, serde_json::Value>) -> Result<bool, String> {
    let trimmed = expr.trim();
    if trimmed == "true" {
        return Ok(true);
    }
    if trimmed == "false" {
        return Ok(false);
    }
    if let Some(inner) = trimmed.strip_prefix("exists(") {
        let key = inner
            .strip_suffix(')')
            .ok_or_else(|| format!("表达式括号不匹配：{expr}"))?
            .trim();
        if key.is_empty() {
            return Err(format!("exists() 参数为空：{expr}"));
        }
        return Ok(ctx.contains_key(key));
    }
    for (op, check) in [
        (
            "==",
            cmp_eq as fn(&serde_json::Value, &serde_json::Value) -> bool,
        ),
        ("!=", cmp_ne),
        (">=", cmp_ge),
        ("<=", cmp_le),
        (">", cmp_gt),
        ("<", cmp_lt),
    ] {
        if let Some((left, right)) = split_once_op(trimmed, op) {
            let left = left.trim();
            let right = right.trim();
            if !ctx.contains_key(left) {
                // 未知变量：视为不成立（前置条件/断言语义），而非表达式错误。
                return Ok(false);
            }
            let value = parse_literal(right)
                .ok_or_else(|| format!("表达式右侧无法解析：{right}（{expr}）"))?;
            return Ok(check(&ctx[left], &value));
        }
    }
    Err(format!("无法解析的表达式：{expr}"))
}

fn split_once_op<'a>(s: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    let index = s.find(op)?;
    // 防误匹配（如 `>=` 内部包含 `>`）：按长度降序匹配已在调用方保证。
    Some((&s[..index], &s[index + op.len()..]))
}

fn parse_literal(s: &str) -> Option<serde_json::Value> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return Some(serde_json::json!(n));
    }
    if let Ok(f) = s.parse::<f64>() {
        return Some(serde_json::json!(f));
    }
    if s == "true" {
        return Some(serde_json::json!(true));
    }
    if s == "false" {
        return Some(serde_json::json!(false));
    }
    let unquoted = s.trim_matches('"').trim_matches('\'');
    Some(serde_json::json!(unquoted))
}

fn as_num(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| {
        value
            .as_str()
            .and_then(|s| s.trim_matches('"').parse::<f64>().ok())
    })
}

fn cmp_eq(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    if let (Some(na), Some(nb)) = (as_num(a), as_num(b)) {
        return (na - nb).abs() < 1e-9;
    }
    // Numeric/string coercion above is intentional. Other JSON values need
    // real equality; None == None is not evidence that two values match.
    a == b
}

fn cmp_ne(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    !cmp_eq(a, b)
}

fn cmp_ge(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (as_num(a), as_num(b)) {
        (Some(na), Some(nb)) => na >= nb,
        _ => false,
    }
}

fn cmp_le(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (as_num(a), as_num(b)) {
        (Some(na), Some(nb)) => na <= nb,
        _ => false,
    }
}

fn cmp_gt(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (as_num(a), as_num(b)) {
        (Some(na), Some(nb)) => na > nb,
        _ => false,
    }
}

fn cmp_lt(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (as_num(a), as_num(b)) {
        (Some(na), Some(nb)) => na < nb,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// 编译到 action_program（结构映射，供复用/预览）
// ---------------------------------------------------------------------------

/// 把 .owflow 编译/翻译为 `ActionProgram`（Step/Assert/Branch/Loop/Sub 结构映射；
/// 语义执行由 `WorkflowEngine` 保证一致）。`known_flows` 为可引用的子流程 id 集合。
pub fn compile_to_program(
    flow: &WorkflowDefinition,
    known_flows: &[String],
) -> Result<ActionProgram, String> {
    if let Err(errors) = validate_definition(flow, known_flows) {
        return Err(format!("工作流定义非法：{}", errors.join("；")));
    }
    let mut program = ActionProgram::new(&flow.name);
    let nodes = compile_steps(&flow.steps)?;
    program.nodes = nodes;
    Ok(program)
}

fn compile_steps(steps: &[WorkflowStep]) -> Result<Vec<ProgramNode>, String> {
    let mut nodes = Vec::new();
    for step in steps {
        match step {
            WorkflowStep::Act { id, spec, .. } => {
                let action = match spec.action.as_str() {
                    "click" => ActionType::Click,
                    "type" => ActionType::Type,
                    "launch" => ActionType::Launch,
                    "scroll" => ActionType::Scroll,
                    "wait" => ActionType::Wait,
                    _ => ActionType::Inject,
                };
                nodes.push(ProgramNode::Step {
                    id: id.clone(),
                    action,
                    anchor: SemanticAnchor {
                        app_id: Some(spec.target.clone()),
                        name: spec.target.clone(),
                        role: None,
                        element_id: None,
                        parent: None,
                    },
                    value_template: spec.value.clone(),
                    verify: None,
                });
            }
            WorkflowStep::Assert { id, .. } => {
                nodes.push(ProgramNode::Assert {
                    id: id.clone(),
                    assertion: crate::assert::Assertion::ClipboardChanged { expected: None },
                });
            }
            WorkflowStep::Cond {
                id,
                then,
                otherwise,
                ..
            } => {
                nodes.push(ProgramNode::Branch {
                    id: id.clone(),
                    cond: crate::assert::Assertion::StateDiff {
                        entity: "_workflow".to_string(),
                        from: None,
                        to: None,
                    },
                    then: compile_steps(then)?,
                    otherwise: compile_steps(otherwise)?,
                });
            }
            WorkflowStep::Loop {
                id, body, max_iter, ..
            } => {
                nodes.push(ProgramNode::Loop {
                    id: id.clone(),
                    cond: None,
                    body: compile_steps(body)?,
                    max_iter: *max_iter,
                });
            }
            WorkflowStep::Subflow { id, flow, .. } => {
                nodes.push(ProgramNode::Sub {
                    id: id.clone(),
                    program: flow.clone(),
                });
            }
            WorkflowStep::RollbackPoint { id } => {
                nodes.push(ProgramNode::Assert {
                    id: id.clone(),
                    assertion: crate::assert::Assertion::StateDiff {
                        entity: "_rollback".to_string(),
                        from: None,
                        to: None,
                    },
                });
            }
            // Sense/Locate/InvokeSkill/InvokeMcp/HumanApprove/Notify 不直接映射，
            // 由引擎语义执行（编译产物保留为注释级占位 Assert）。
            other => {
                nodes.push(ProgramNode::Assert {
                    id: step_id(other)
                        .ok_or_else(|| "步骤缺 id".to_string())?
                        .to_string(),
                    assertion: crate::assert::Assertion::StateDiff {
                        entity: "_placeholder".to_string(),
                        from: None,
                        to: None,
                    },
                });
            }
        }
    }
    Ok(nodes)
}
