use super::*;
// ---------------------------------------------------------------------------
// 角色 worker（worker 注册表层：内层 worker + 完成即登记产物/交接）
// ---------------------------------------------------------------------------

/// 角色 worker：步骤完成后自动登记版本化 Artifact + HandoffRecord（产物经 ref 传递）。
pub struct RoleWorker {
    coordinator: Arc<TeamCoordinator>,
    team_id: String,
    member_id: String,
    role: String,
    capabilities: Vec<String>,
    inner: Arc<dyn Worker>,
}

fn apply_retry_context(ctx: &mut Value, input: &Value) {
    let Some(note) = input
        .get("_workswarm")
        .and_then(|meta| meta.get("retry_note"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|note| !note.is_empty())
    else {
        return;
    };
    let Some(context) = ctx.as_object_mut() else {
        return;
    };
    let objective = context
        .get("objective_text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let note = note.chars().take(1600).collect::<String>();
    context.insert(
        "objective_text".into(),
        json!(format!(
            "{objective}\n\n## 上一尝试诊断（仅作上下文，不得放宽验收或扩大权限）\n{note}"
        )),
    );
}

fn apply_assigned_task_context(
    ctx: &mut Value,
    task: &crate::task_context::ResolvedTaskContext,
) -> Result<(), String> {
    let Some(task_text) = task.objective.as_deref() else {
        if task.task_id.is_some() {
            return Err("TaskGraph 任务缺少宿主解析的任务目标".to_string());
        }
        return Ok(());
    };
    let Some(context) = ctx.as_object_mut() else {
        return Err("团队上下文不是可修改对象".to_string());
    };
    let paths = json!(task.write_paths.clone().unwrap_or_default());
    context.insert("objective_text".into(), json!(task_text));
    let task_contract = task
        .prompt_contract()
        .ok_or_else(|| "TaskGraph 任务提示契约无法编译".to_string())?;
    let role_contract = context
        .get("handoff_contract")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|contract| !contract.is_empty());
    let handoff_contract = role_contract
        .map(|contract| format!("{contract}\n\n{task_contract}"))
        .unwrap_or(task_contract);
    context.insert("handoff_contract".into(), json!(handoff_contract));
    context.insert("write_paths".into(), paths);
    context.insert("task_scoped".into(), json!(true));
    context.insert("_resolved_task_context".into(), task.to_value()?);
    Ok(())
}

impl RoleWorker {
    pub fn new(
        coordinator: Arc<TeamCoordinator>,
        team_id: String,
        member_id: String,
        role: String,
        inner: Arc<dyn Worker>,
    ) -> Self {
        let capabilities = if is_review_role_name(&role) {
            vec!["review".to_string()]
        } else {
            Vec::new()
        };
        Self::new_with_capabilities(coordinator, team_id, member_id, role, capabilities, inner)
    }

    pub fn new_with_capabilities(
        coordinator: Arc<TeamCoordinator>,
        team_id: String,
        member_id: String,
        role: String,
        capabilities: Vec<String>,
        inner: Arc<dyn Worker>,
    ) -> Self {
        Self {
            coordinator,
            team_id,
            member_id,
            role,
            capabilities,
            inner,
        }
    }
}

#[async_trait]
impl Worker for RoleWorker {
    fn name(&self) -> &str {
        self.member_id.as_str()
    }

    async fn run(&self, input: &Value) -> Result<String, String> {
        let task_context = crate::task_context::ResolvedTaskContext::from_assignment_input(input)?;
        task_context.validate_runtime_binding()?;
        let step_id = task_context.step_id.clone().unwrap_or_default();
        // 领取代次：cancel/retry/replace 接管现场后，旧阶段回传凭此被拒收。
        let phase_epoch = task_context.phase_epoch;
        let attempt_id = task_context.attempt_id.clone();
        // 单独量化上下文读取成本。Worker span 的 wall_ms 覆盖上下文、模型执行
        // 与产物登记；这个无内容审计事件让诊断能把上下文开销从模型等待中区分出来。
        let context_started = std::time::Instant::now();
        let context_result = self
            .coordinator
            .assemble_context_slice_for_attempt(
                &self.team_id,
                &self.member_id,
                &step_id,
                phase_epoch,
            )
            .await;
        let snapshot_cache_hit = context_result
            .as_ref()
            .is_ok_and(|(_, cache_hit)| *cache_hit);
        let context_assembly_ms = context_started
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.coordinator.record_context_assembly_timing(
            &self.team_id,
            &self.role,
            &step_id,
            task_context.task_id.as_deref(),
            attempt_id.as_deref(),
            phase_epoch,
            context_assembly_ms,
            if context_result.is_ok() {
                "succeeded"
            } else {
                "failed"
            },
            snapshot_cache_hit,
        );
        let ctx = match context_result {
            Ok((context, _)) => context,
            Err(error) => return Err(format!("上下文切片组装失败：{error}")),
        };
        let worker_kind = self.inner.name().to_string();
        let mut ctx = ctx;
        apply_assigned_task_context(&mut ctx, &task_context)?;
        apply_retry_context(&mut ctx, input);
        let reviewed_sources = ctx
            .get("upstream")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let effective_capabilities = ctx
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| self.capabilities.clone());
        let is_reviewer = super::util::is_review_role(&self.role, &effective_capabilities);
        let mut enriched = TeamCoordinator::build_enriched_input_owned(ctx, input, &worker_kind);
        // Prompt 上下文指标在 Server 使用最终 WorkerProfile 编译后记录，
        // 避免这里先用粗略角色画像生成一份与实际工具面不一致的 Prompt。
        // Worker 上下文装配完成、即将进入真实执行时推进进度状态。
        // Server 指标包装器同样调用该方法；核心入口的调用保证内嵌/测试宿主行为一致，重复调用幂等。
        self.coordinator
            .mark_phase_step_running(&self.team_id, &step_id);
        let plan_epoch = self.coordinator.current_execution_epoch(&self.team_id);
        let mut out = self.inner.run(&enriched).await?;
        if self.role == "lead" {
            // One semantic correction per worker invocation. Both calls go through
            // the same metered, scoped inner worker; GoalRunner retry budgets remain.
            for correction in 0..=1 {
                let parsed = crate::workswarm_output::parse_worker_output(&out);
                let candidate = match &parsed {
                    crate::workswarm_output::WorkerOutputParse::Parsed(ref contract)
                        if contract.status != crate::workswarm_output::WorkerOutputStatus::Done =>
                    {
                        break
                    }
                    crate::workswarm_output::WorkerOutputParse::Parsed(ref contract) => contract
                        .artifact
                        .as_ref()
                        .map(|a| a.content.as_str())
                        .unwrap_or(""),
                    _ => out.as_str(),
                };
                match self
                    .coordinator
                    .validate_parallel_candidate(&self.team_id, candidate)
                {
                    Ok(()) => break,
                    Err(WorkSwarmError::Validation(reason)) if correction == 0 => {
                        if self.coordinator.current_execution_epoch(&self.team_id) != plan_epoch
                            || self.coordinator.cancel_token(&self.team_id).is_cancelled()
                        {
                            return Err("指挥计划纠正前任务已取消或代次变化".into());
                        }
                        let reason = reason.chars().take(2000).collect::<String>();
                        let note = format!("上次 TaskGraph 被宿主拒绝：{reason}。修正同一用户目标的计划，禁止扩大权限、写范围或改变需求。聊天文本任务 verification 必须为 JSON 字符串 \"non_empty\"，不要包装成文件 VerificationPlan；文件验证必须列出具体文件路径。返回完整 WorkerOutputV1，artifact.content 为修正后的 TaskGraph JSON。");
                        enriched["task_graph_repair"] = json!({"reason": reason});
                        if let Some(context) = enriched
                            .get_mut("team_prompt_context")
                            .and_then(Value::as_object_mut)
                        {
                            let handoff = context
                                .get("handoff_contract")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            context.insert(
                                "handoff_contract".into(),
                                json!(format!("{handoff}\n\n{note}")),
                            );
                        } else if let Some(object) = enriched.as_object_mut() {
                            object.insert("task_graph_repair_note".into(), json!(note));
                        }
                        out = self.inner.run(&enriched).await?;
                    }
                    Err(error) => return Err(format!("task_graph_invalid:{error}")),
                }
            }
        }
        // 输出契约（V1）：结构化 JSON → 登记前防御——producer 取 artifact.content
        //（交付物正文，不再拿整段自由文本/信封当产物），critic 禁止携带 artifact
        //（评审无权覆盖交付物，越权即 scope_violation）。契约失败已在 worker 层
        // 定向修复过一次；此处 Invalid 视为 legacy 纯文本登记（不二次重试）。
        // 七期（第三路）：Parsed 输出走契约登记（格式门控 + 交付元数据 + 证据链
        // 随 Artifact/HandoffRecord 落盘）；legacy 纯文本登记行为不变。
        let out = match crate::workswarm_output::parse_worker_output(&out) {
            crate::workswarm_output::WorkerOutputParse::Parsed(output) => {
                if is_reviewer {
                    if let Err(e) = output.validate_critic() {
                        return Err(format!("scope_violation:{e}"));
                    }
                    if output.status != crate::workswarm_output::WorkerOutputStatus::Done {
                        return Err(format!(
                            "worker_{}:{}",
                            output.status.as_str(),
                            output.summary
                        ));
                    }
                    if let Err(e) = self
                        .coordinator
                        .register_step_output_contract_bound(
                            &self.team_id,
                            &self.member_id,
                            &self.role,
                            &step_id,
                            &output,
                            OutputAttemptBinding {
                                phase_epoch,
                                attempt_id: attempt_id.as_deref(),
                                reviewed_sources: Some(&reviewed_sources),
                            },
                        )
                        .await
                    {
                        return Err(format!("产物登记失败：{e}"));
                    }
                    output.summary
                } else {
                    if let Err(e) = output.validate() {
                        return Err(format!("output_contract_invalid:{e}"));
                    }
                    match output.status {
                        crate::workswarm_output::WorkerOutputStatus::Done => {
                            if let Err(e) = self
                                .coordinator
                                .register_step_output_contract_bound(
                                    &self.team_id,
                                    &self.member_id,
                                    &self.role,
                                    &step_id,
                                    &output,
                                    OutputAttemptBinding {
                                        phase_epoch,
                                        attempt_id: attempt_id.as_deref(),
                                        reviewed_sources: Some(&reviewed_sources),
                                    },
                                )
                                .await
                            {
                                return Err(format!("产物登记失败：{e}"));
                            }
                            output.artifact.map(|a| a.content).unwrap_or_default()
                        }
                        other => {
                            // producer 如实申报 failed/blocked：步骤失败（可局部重试），不登记空产物。
                            return Err(format!("worker_{}:{}", other.as_str(), output.summary));
                        }
                    }
                }
            }
            // 契约解析/校验失败：worker 层已做一次定向修复并失败会直接 Err，
            // 走不到这里；此处保守按 legacy 纯文本登记（服务端旧流程不受影响）。
            _ => {
                if let Err(e) = self
                    .coordinator
                    .register_step_output_checked_bound(
                        &self.team_id,
                        &self.member_id,
                        &self.role,
                        &step_id,
                        &out,
                        OutputAttemptBinding {
                            phase_epoch,
                            attempt_id: attempt_id.as_deref(),
                            reviewed_sources: Some(&reviewed_sources),
                        },
                    )
                    .await
                {
                    return Err(format!("产物登记失败：{e}"));
                }
                out
            }
        };
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_task_replaces_unscoped_prompt_objective() {
        let mut context = json!({
            "objective_text": "完整团队目标：交付整个产品",
            "role": "implementer",
            "handoff_contract": "实现者固定边界：只写 apps/api。",
        });
        let input = json!({
            "assigned_task": "实现 posts API 分页",
            "assigned_acceptance": "覆盖默认值和上限",
            "assigned_verification": "cargo test posts_api",
            "assigned_write_paths": ["apps/api"],
            "assigned_read_refs": ["src/posts.rs"],
            "assigned_contract_refs": ["contract/posts"],
            "required_capabilities": ["read", "write"],
        });

        let task = crate::task_context::ResolvedTaskContext::from_worker_input(&input).unwrap();
        apply_assigned_task_context(&mut context, &task).unwrap();
        let (prompt, _) = TeamCoordinator::compile_role_prompt_with_meta(&context);

        assert!(prompt.contains("实现 posts API 分页"));
        assert!(prompt.contains("覆盖默认值和上限"));
        assert!(prompt.contains("实现者固定边界：只写 apps/api。"));
        assert!(!prompt.contains("完整团队目标：交付整个产品"));
        assert_eq!(context["write_paths"], json!(["apps/api"]));
        assert_eq!(context["task_scoped"], json!(true));

        let retry = json!({
            "_workswarm": { "retry_note": "上次失败：分页边界测试缺失" }
        });
        apply_retry_context(&mut context, &retry);
        let (retry_prompt, _) = TeamCoordinator::compile_role_prompt_with_meta(&context);
        assert!(retry_prompt.contains("分页边界测试缺失"));
        assert!(retry_prompt.contains("不得放宽验收或扩大权限"));
    }
}
