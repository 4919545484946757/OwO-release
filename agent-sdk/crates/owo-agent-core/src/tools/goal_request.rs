//! `goal_request`：模型把"长任务"登记为会话目标（G4 的宿主交接切片）。
//!
//! 为什么不是"模型直接起目标"：目标循环是宿主的后台运行体，会占用会话/回合锁；
//! 在回合内直接启动会与当前回合死锁。所以模型只能**登记**——写会话目标
//! （origin=model）并返回结构化回执，由宿主（CLI `/goal run`、桌面、自动化）在
//! 本回合结束后决定是否接管。这也是"不谎报已启动"的诚实边界。

use super::{Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use serde_json::{json, Value};

/// 目标描述长度上限（会进会话库、上下文与 UI）。
pub(super) const MAX_GOAL_CHARS: usize = 500;

pub(super) struct GoalRequestTool;

#[async_trait]
impl Tool for GoalRequestTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "goal_request".into(),
            description: "把需要长跑（多轮推进）的目标登记到会话，由宿主在本次回合结束后决定是否接管。\
                          只登记、不启动：若会话已有未完成目标则拒绝覆盖。返回 status=recorded|rejected。"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "objective": { "type": "string", "description": "目标描述（一句话，≤500 字符）" },
                    "reason": { "type": "string", "description": "为什么需要长跑（可选，用于给用户解释）" }
                },
                "required": ["objective"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let objective = args
            .get("objective")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if objective.is_empty() {
            return Err("goal_request.objective 不能为空".to_string());
        }
        if objective.chars().count() > MAX_GOAL_CHARS {
            return Err(format!(
                "目标描述过长（{} 字符 > {MAX_GOAL_CHARS}）：一句话说清要达成什么",
                objective.chars().count()
            ));
        }
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        // 已有未完成目标时不覆盖：静默替换会让用户/宿主丢失正在推进的目标。
        if let Some(existing) = ctx.session.goal.as_ref() {
            if !existing.done {
                return Ok(json!({
                    "status": "rejected",
                    "reason": "会话已有未完成目标，未覆盖",
                    "objective": existing.objective,
                    "iterations": existing.iterations,
                    "origin": existing.origin,
                }));
            }
        }

        ctx.session.set_model_goal(objective.clone());
        Ok(json!({
            "status": "recorded",
            "objective": objective,
            "reason": reason,
            "origin": crate::session::GOAL_ORIGIN_MODEL,
            "note": "已登记，未启动：宿主会在本回合结束后决定是否接管（CLI：/goal run）",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Session, GOAL_ORIGIN_MODEL, GOAL_ORIGIN_USER};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn with_context<T>(f: impl FnOnce(&mut ToolContext<'_>) -> T) -> T {
        let workspace = std::env::temp_dir();
        let policy = crate::Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "mock", None);
        let mut context = ToolContext {
            workspace: Path::new(&workspace),
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
        f(&mut context)
    }

    #[test]
    fn records_model_goal_without_starting_it() {
        with_context(|context| {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(GoalRequestTool.run(
                    context,
                    json!({ "objective": "把 README 补全", "reason": "需要多轮验证" }),
                ))
                .unwrap();
            assert_eq!(result["status"], "recorded");
            assert_eq!(result["objective"], "把 README 补全");
            assert_eq!(result["origin"], GOAL_ORIGIN_MODEL);
            assert!(result["note"].as_str().unwrap().contains("未启动"));
            let goal = context.session.goal.as_ref().expect("目标应写入会话");
            assert!(goal.is_model_requested());
            assert_eq!(goal.iterations, 0, "登记阶段不得推进轮次");
            assert!(!goal.done);
        });
    }

    #[test]
    fn refuses_to_overwrite_an_unfinished_goal() {
        with_context(|context| {
            context.session.set_goal("用户正在推进的目标");
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime
                .block_on(GoalRequestTool.run(context, json!({ "objective": "模型想插队的目标" })))
                .unwrap();
            assert_eq!(result["status"], "rejected");
            assert_eq!(result["objective"], "用户正在推进的目标");
            let goal = context.session.goal.as_ref().unwrap();
            assert_eq!(goal.objective, "用户正在推进的目标");
            assert_eq!(goal.origin, GOAL_ORIGIN_USER, "拒绝时不得改写来源");
        });
    }

    #[test]
    fn allows_replacing_a_finished_goal() {
        with_context(|context| {
            context.session.set_goal("上一轮目标");
            context.session.complete_goal();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime
                .block_on(GoalRequestTool.run(context, json!({ "objective": "新的长任务" })))
                .unwrap();
            assert_eq!(result["status"], "recorded");
            assert_eq!(
                context.session.goal.as_ref().unwrap().objective,
                "新的长任务"
            );
        });
    }

    #[test]
    fn goal_request_is_registered_in_production_registry_but_not_read_only() {
        // 生产默认注册表（Agent 实际使用的那份）必须带 goal_request；
        // 只读表（explore/plan）不带——只读会话里不允许登记待执行目标。
        let names = |registry: crate::tools::ToolRegistry| -> Vec<String> {
            registry.specs().into_iter().map(|spec| spec.name).collect()
        };
        let default_names = names(crate::tools::ToolRegistry::new());
        assert!(
            default_names.iter().any(|name| name == "goal_request"),
            "默认注册表缺少 goal_request：{default_names:?}"
        );
        let read_only_names = names(crate::tools::ToolRegistry::read_only());
        assert!(
            !read_only_names.iter().any(|name| name == "goal_request"),
            "只读注册表不得包含 goal_request：{read_only_names:?}"
        );
    }

    #[test]
    fn validates_objective_and_has_no_approval_effect() {
        with_context(|context| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            assert!(runtime
                .block_on(GoalRequestTool.run(context, json!({})))
                .is_err());
            let too_long = "字".repeat(MAX_GOAL_CHARS + 1);
            let error = runtime
                .block_on(GoalRequestTool.run(context, json!({ "objective": too_long })))
                .unwrap_err();
            assert!(error.contains("过长"), "{error}");
            assert!(context.session.goal.is_none(), "校验失败不得写入目标");
            // 会话内状态写入，不触发审批（与 todo 同档）。
            assert!(GoalRequestTool.spec().effect.is_none());
        });
    }
}
