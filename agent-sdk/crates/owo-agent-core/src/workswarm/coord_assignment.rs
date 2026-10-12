//! Apply a validated task graph to persistent worker slots. No model or tool execution.
use super::task_graph::{
    assign_task_model_call_budgets, bind_dynamic_follow_up_dependencies,
    is_independent_reviewer_role, is_parallel_writer_name, parse_parallel_subtasks,
    validate_parallel_subtasks, validate_task_requirement_coverage,
};
use super::*;

impl TeamCoordinator {
    /// Validate the lead candidate before publishing it. Uses exactly the same
    /// host schema, coverage, scopes and budget placement as final assignment.
    pub(super) fn validate_parallel_candidate(
        &self,
        team_id: &str,
        output: &str,
    ) -> WorkSwarmResult<()> {
        let meta = RunMeta::load(&self.run_dir, team_id)?;
        if !meta.parallel {
            return Ok(());
        }
        let (items, versioned) = parse_parallel_subtasks(output).ok_or_else(|| {
            WorkSwarmError::Validation("指挥计划须为可解析的 TaskGraph JSON".into())
        })?;
        let mut graph = validate_parallel_subtasks(&meta.roles, &items, versioned)
            .map_err(WorkSwarmError::Validation)?;
        if versioned {
            let state = self.load_run_state(team_id)?;
            let quotes = graph
                .tasks
                .iter()
                .map(|task| task.user_requirement_quotes.clone())
                .collect::<Vec<_>>();
            validate_task_requirement_coverage(&quotes, &state.goal.objective)
                .map_err(WorkSwarmError::Validation)?;
        }
        assign_task_model_call_budgets(&mut graph.tasks, &meta.budgets)
            .map_err(WorkSwarmError::Validation)?;
        Ok(())
    }
    /// 十一期：并行开发的任务主动分配（幂等）。
    ///
    /// 运行条件：`RunMeta.parallel` 为真，且 **lead 步骤已成功**且产物可解析为
    /// TaskGraphV1（兼容旧 subtasks 数组、围栏与前后缀文本）。先校验任务字段、依赖
    /// DAG、任务数量和写范围，再绑定到有限 worker 槽位；同一槽位内的任务顺序串行。
    /// 动态步骤保留任务 ID、验收、验证、引用和能力需求，写路径权限不超出已有授权。
    ///
    /// 无变化不落盘、不重复审计；lead 产物不可解析时保持 writer 原有权限（缺省
    /// 只读），并记 `team.parallel_assign_skip` 审计——绝不因调度信息缺失而放宽权限。
    pub(super) fn apply_parallel_assignment(
        &self,
        team_id: &str,
        state: &mut GoalRunState,
        meta: &mut RunMeta,
    ) -> WorkSwarmResult<()> {
        if !meta.parallel {
            return Ok(());
        }
        // Lead output is revisited after every phase; do not append the same DAG twice.
        if state
            .plan
            .steps
            .iter()
            .any(|step| step.input.get("assigned_task_id").is_some())
        {
            return Ok(());
        }
        let Some(lead_step_id) = state
            .plan
            .steps
            .iter()
            .find(|s| worker_role(&s.worker).as_deref() == Some("lead"))
            .map(|step| step.id.clone())
        else {
            return Ok(());
        };
        if state.records.get(&lead_step_id).map(|record| record.status)
            != Some(StepStatus::Succeeded)
        {
            return Ok(());
        }
        let output = state
            .records
            .get(&lead_step_id)
            .and_then(|record| record.output.clone())
            .ok_or_else(|| WorkSwarmError::Validation("lead 成功步骤缺少子任务产物".to_string()))?;
        let (subtasks, versioned) = parse_parallel_subtasks(&output).ok_or_else(|| {
            WorkSwarmError::Validation("lead 产物不是可解析的 TaskGraph JSON".to_string())
        })?;
        let task_graph = validate_parallel_subtasks(&meta.roles, &subtasks, versioned)
            .map_err(WorkSwarmError::Validation)?;
        if versioned {
            let task_quote_sets = task_graph
                .tasks
                .iter()
                .map(|task| task.user_requirement_quotes.clone())
                .collect::<Vec<_>>();
            validate_task_requirement_coverage(&task_quote_sets, &state.goal.objective)
                .map_err(WorkSwarmError::Validation)?;
        }
        if task_graph.version != 1 {
            return Err(WorkSwarmError::Validation(
                "unsupported TaskGraph version".to_string(),
            ));
        }
        let mut assignments = task_graph.tasks;
        assign_task_model_call_budgets(&mut assignments, &meta.budgets)
            .map_err(WorkSwarmError::Validation)?;
        let writers: Vec<String> = meta
            .roles
            .iter()
            .filter(|role| is_parallel_writer_name(&role.role))
            .map(|role| role.role.clone())
            .collect();
        let original_inputs: std::collections::BTreeMap<String, Value> = writers
            .iter()
            .filter_map(|worker| {
                state
                    .plan
                    .steps
                    .iter()
                    .find(|step| step.id == format!("s-{worker}"))
                    .map(|step| (worker.clone(), step.input.clone()))
            })
            .collect();
        let original_ids: std::collections::BTreeSet<String> =
            writers.iter().map(|worker| format!("s-{worker}")).collect();
        state
            .plan
            .steps
            .retain(|step| !original_ids.contains(&step.id));
        for worker in &writers {
            state.records.remove(&format!("s-{worker}"));
        }

        // Bind a DAG of any size to the finite set of worker slots. A slot is a
        // capacity limit, not a serial dependency: independent tasks may overlap and
        // task-scoped sessions, permissions, and write leases preserve isolation.
        let mut step_ids = std::collections::BTreeMap::new();
        let mut first_for_worker = std::collections::BTreeSet::new();
        for task in &assignments {
            let id = if first_for_worker.insert(task.worker.clone()) {
                format!("s-{}", task.worker)
            } else {
                format!("s-task-{}", task.task_id)
            };
            step_ids.insert(task.task_id.clone(), id);
        }
        // Bind stable task-to-step identities using source order; sort only the runnable
        // plan entries so priority hints never rename or reassign an existing task slot.
        let mut scheduled_indices: Vec<usize> = (0..assignments.len()).collect();
        scheduled_indices.sort_by(|left, right| {
            let left = &assignments[*left];
            let right = &assignments[*right];
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| right.estimated_effort.cmp(&left.estimated_effort))
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        let mut scope_union = std::collections::BTreeMap::<String, Vec<String>>::new();
        for task_index in scheduled_indices {
            let task = &assignments[task_index];
            let step_id = step_ids[&task.task_id].clone();
            let mut dependencies: Vec<String> = task
                .depends_on
                .iter()
                .map(|dep| step_ids[dep].clone())
                .collect();
            if dependencies.is_empty() {
                dependencies.push(lead_step_id.clone());
            }
            let mut input = original_inputs
                .get(&task.worker)
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !input.is_object() {
                input = json!({});
            }
            if let Some(obj) = input.as_object_mut() {
                obj.insert("assigned_task_id".into(), json!(task.task_id));
                obj.insert(
                    "assigned_model_calls_per_attempt".into(),
                    json!(task.model_calls_per_attempt),
                );
                obj.insert("assigned_task".into(), json!(task.task));
                obj.insert("assigned_acceptance".into(), json!(task.acceptance));
                obj.insert(
                    "assigned_user_requirement_quotes".into(),
                    json!(task.user_requirement_quotes),
                );
                let assigned_verification = task
                    .verification_plan
                    .as_ref()
                    .map(|plan| json!(plan))
                    .or_else(|| task.verification.as_ref().map(|value| json!(value)))
                    .unwrap_or(Value::Null);
                obj.insert("assigned_verification".into(), assigned_verification);
                obj.insert("assigned_write_paths".into(), json!(task.effective_paths));
                obj.insert("assigned_read_refs".into(), json!(task.read_refs));
                obj.insert("assigned_contract_refs".into(), json!(task.contract_refs));
                obj.insert(
                    "required_capabilities".into(),
                    json!(task.required_capabilities),
                );
                obj.insert("estimated_effort".into(), json!(task.estimated_effort));
                obj.insert("risk".into(), json!(task.risk));
                obj.insert("priority".into(), json!(task.priority));
                let ws = obj.entry("_workswarm").or_insert_with(|| json!({}));
                if let Some(ws) = ws.as_object_mut() {
                    ws.insert("step_id".into(), json!(step_id));
                    ws.insert("member_id".into(), json!(format!("m-{}", task.worker)));
                }
            }
            scope_union
                .entry(task.worker.clone())
                .or_default()
                .extend(task.effective_paths.iter().cloned());
            let (verification, verification_plan) = if let Some(plan) = &task.verification_plan {
                (None, plan.clone())
            } else {
                let verification = parse_verify(
                    task.verification
                        .as_deref()
                        .expect("TaskGraph parser requires legacy verification or a plan"),
                );
                let plan = verification_plan_for_step(&step_id, &verification);
                (Some(verification), plan)
            };
            state.plan.steps.push(StepSpec {
                id: step_id.clone(),
                depends_on: dependencies,
                parallel: true,
                worker: format!("m-{}", task.worker),
                input,
                verify: verification,
                verification_plan: Some(verification_plan),
                retries: 0,
            });
            state.records.insert(
                step_id.clone(),
                crate::goal::StepRecord {
                    step_id,
                    status: StepStatus::Pending,
                    attempts: 0,
                    output: None,
                    error: None,
                    skip_reason: None,
                    phase_epoch: None,
                    attempt_id: None,
                    validation_receipts: Vec::new(),
                },
            );
        }
        let high_risk_task_step_ids: Vec<String> = assignments
            .iter()
            .filter(|task| matches!(task.risk.as_str(), "high" | "critical"))
            .map(|task| step_ids[&task.task_id].clone())
            .collect();
        let assignment_step_ids = assignments
            .iter()
            .map(|task| step_ids[&task.task_id].clone())
            .collect::<Vec<_>>();
        let scheduled_reviewer_step_ids = state
            .plan
            .steps
            .iter()
            .filter(|step| {
                meta.roles.iter().any(|role| {
                    step.worker == format!("m-{}", role.role) && is_independent_reviewer_role(role)
                })
            })
            .map(|step| step.id.clone())
            .collect::<Vec<_>>();
        if !high_risk_task_step_ids.is_empty() && scheduled_reviewer_step_ids.is_empty() {
            return Err(WorkSwarmError::Validation(
                "high-risk TaskGraph has no scheduled independent reviewer step".to_string(),
            ));
        }
        bind_dynamic_follow_up_dependencies(
            &mut state.plan.steps,
            &assignment_step_ids,
            &scheduled_reviewer_step_ids,
        );
        for role in meta
            .roles
            .iter_mut()
            .filter(|role| is_parallel_writer_name(&role.role))
        {
            let mut paths = scope_union.remove(&role.role).unwrap_or_default();
            paths.sort();
            paths.dedup();
            role.write_paths = paths;
            let role_contract = role
                .handoff_contract
                .as_deref()
                .filter(|contract| !contract.trim().is_empty())
                .unwrap_or("遵守模板为该 Worker 槽位声明的任务边界。");
            role.handoff_contract = Some(format!(
                "{role_contract}\n\n你是并行执行者 {}。只执行当前输入 assigned_task，按 assigned_acceptance 验收；\
                 写操作只允许命中 assigned_write_paths。逐任务提交结果和证据。",
                role.role
            ));
        }
        state.plan.validate().map_err(WorkSwarmError::Validation)?;
        meta.save(&self.run_dir)?;
        self.audit(
            team_id,
            "team.parallel_assigned",
            format!(
                "并行任务图已应用：{} 个任务复用 {} 个 Worker 槽位",
                assignments.len(),
                writers.len()
            ),
        );

        Ok(())
    }
}
