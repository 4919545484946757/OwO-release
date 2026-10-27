//! Phase claim, lock-free execution and epoch-checked result merge.
use super::coord_strategy::is_auto_reviewer_role;
use super::task_graph::{
    host_manifest_can_replace_integration, parallel_tasks_require_integration,
    PARALLEL_LEADER_HOST_MANIFEST_SKIP_REASON,
};
use super::*;

use super::coord_admission::reject_attempt_admissions;

impl TeamCoordinator {
    // -- 运行阶段（server 运行循环驱动） --

    /// 执行一个阶段（当前就绪的 agent 批次）。人节点不进入 runner（由运行任务开门闩等待）。
    ///
    /// 每阶段结束后把子状态合并回完整状态并落盘（steer/replace/human 的改盘变更在
    /// 下一阶段重新加载时生效）。
    pub async fn run_phase(
        &self,
        team_id: &str,
        registry: &WorkerRegistry,
    ) -> WorkSwarmResult<PhaseOutcome> {
        // ---- 阶段 A（短锁）：领取 ready 步骤、标记 Running、持久化 ----
        // 锁只覆盖领取与落盘，Worker/模型执行的整段时间**不持锁**，
        // GET 详情 / 任务图 / 产物 / SSE 等读路径不再被长 Worker 阻塞。
        struct PhaseClaimPlan {
            epoch: u64,
            sub_state: GoalRunState,
            claimed: Vec<ProgressStep>,
            space: ProjectSpace,
            goal_objective: String,
            context_steps: Vec<StepSpec>,
            shared_context_refs: Vec<String>,
            meta: RunMeta,
            /// 八期一路：本阶段运行期跳过的角色（role, reason, saved_calls）。
            runtime_skips: Vec<(String, String, usize)>,
            review_issues_pending: bool,
            integration_required: bool,
            base_steps_taken: u32,
            base_total_retries: u32,
        }
        let claim: PhaseClaimPlan = {
            let lock = self.team_lock(team_id);
            let _guard = lock.lock().await;

            let (mut team, space, mut state) = self.load_bundle(team_id).await?;
            if team.status.is_terminal() {
                return Ok(PhaseOutcome::Finished);
            }
            let mut meta = RunMeta::load(&self.run_dir, team_id)?;
            let parallel_assignment_missing = meta.parallel
                && !state
                    .plan
                    .steps
                    .iter()
                    .any(|step| step.input.get("assigned_task_id").is_some());
            let lead_already_succeeded = state.plan.steps.iter().any(|step| {
                worker_role(&step.worker).is_some_and(|worker_role| {
                    meta.roles
                        .iter()
                        .find(|role| role.role == worker_role)
                        .is_some_and(RoleSpec::is_orchestrator)
                }) && state
                    .records
                    .get(&step.id)
                    .is_some_and(|record| record.status == StepStatus::Succeeded)
            });
            if parallel_assignment_missing && lead_already_succeeded {
                if let Err(error) = self.apply_parallel_assignment(team_id, &mut state, &mut meta) {
                    // A template orchestrator may return a prose design brief
                    // instead of a TaskGraph. Keep the validated fixed-role
                    // pipeline as a safe fallback instead of killing the run.
                    tracing::warn!(
                        team_id,
                        %error,
                        "dynamic TaskGraph invalid; continuing with fixed template roles"
                    );
                    meta.parallel = false;
                    meta.save(&self.run_dir)?;
                }
                self.persist_state(&state)?;
            }
            let integration_required = parallel_tasks_require_integration(&state.plan.steps);

            let ready = Self::ready_steps(&state);
            let ready = super::phase_subplan::filter_pre_fanout_ready_steps(&meta, &state, &ready);
            let is_human_step = |s: &StepSpec, meta: &RunMeta| -> bool {
                Self::role_spec_of_member(meta, &s.worker)
                    .map(|r| r.assignee == "human")
                    .unwrap_or(false)
            };
            let mut agent_steps: Vec<StepSpec> = ready
                .iter()
                .filter(|s| !is_human_step(s, &meta))
                .cloned()
                .collect();
            let human_steps: Vec<StepSpec> = ready
                .iter()
                .filter(|s| is_human_step(s, &meta))
                .cloned()
                .collect();

            // 八期一路：运行期可选角色跳过——code-change 模板的 reviewer 仅在不存在
            // 未解决评审 Issue 且宿主变更跟踪明确报告无改动时才跳过。跟踪未知时保留评审。
            let mut runtime_skips: Vec<(String, String, usize)> = Vec::new();
            let review_issues_pending = state
                .delivery_issues
                .iter()
                .any(|issue| issue.status != crate::goal::DeliveryIssueStatusV1::Resolved);
            {
                let parallel_tasks_require_integration =
                    parallel_tasks_require_integration(&state.plan.steps);
                let mut remaining: Vec<StepSpec> = Vec::with_capacity(agent_steps.len());
                for step in agent_steps {
                    let role = worker_role(&step.worker).unwrap_or_default();
                    let leader_can_use_host_manifest = host_manifest_can_replace_integration(
                        meta.parallel,
                        &role,
                        parallel_tasks_require_integration,
                    );
                    let skip_reason = if leader_can_use_host_manifest {
                        Some(PARALLEL_LEADER_HOST_MANIFEST_SKIP_REASON.to_string())
                    } else if meta
                        .template_id
                        .as_deref()
                        .is_some_and(crate::builtin_team_templates::is_code_change_template)
                        || meta
                            .roles
                            .iter()
                            .any(|spec| spec.role == role && is_auto_reviewer_role(spec))
                    {
                        let is_reviewer = meta
                            .roles
                            .iter()
                            .find(|spec| spec.role == role)
                            .is_some_and(RoleSpec::is_reviewer);
                        crate::team_strategy::review_runtime_skip_for_workspace_status(
                            is_reviewer,
                            review_issues_pending,
                            self.workspace_change_status(team_id),
                        )
                    } else {
                        None
                    };
                    if let Some(reason) = skip_reason {
                        let skippable = state
                            .records
                            .get(&step.id)
                            .is_some_and(|r| r.status.can_resume());
                        if skippable {
                            if let Some(record) = state.records.get_mut(&step.id) {
                                record.status = StepStatus::Succeeded;
                                record.skip_reason = Some(reason.clone());
                            }
                            let saved = meta.budgets.get(&role).copied().unwrap_or(0);
                            runtime_skips.push((role, reason, saved));
                            continue;
                        }
                    }
                    remaining.push(step);
                }
                agent_steps = remaining;
            }

            // 无就绪：全部完成 → Done；否则死锁（上游失败等）→ Failed。
            if agent_steps.is_empty() && human_steps.is_empty() {
                if Self::all_succeeded(&state) {
                    if !runtime_skips.is_empty() {
                        // 跳过标记必须先落盘（否则磁盘 reviewer 停留在 Pending 而团队已终态）。
                        self.persist_state(&state)?;
                        drop(_guard);
                        // 锁外记录自适应指标与审计（note_adaptive_event 自行持锁）。
                        for (role, reason, saved) in &runtime_skips {
                            self.audit(
                                team_id,
                                "team.role_skipped",
                                format!("运行期跳过角色 {role}：{reason}"),
                            );
                            self.note_adaptive_event(
                                team_id,
                                json!({
                                    "kind": "role_skipped",
                                    "role": role,
                                    "reason": reason,
                                    "saved_budget_calls": saved,
                                    "role_skipped": {"role": role, "reason": reason},
                                }),
                            )
                            .await;
                        }
                        self.audit(
                            team_id,
                            "team.early_exit",
                            "运行期跳过使全部完成条件满足，DAG 提前结束".to_string(),
                        );
                        self.note_adaptive_event(
                            team_id,
                            json!({
                                "kind": "early_exit",
                                "early_exit": {
                                    "reason": "运行期跳过使全部完成条件满足，DAG 提前结束",
                                    "skipped_roles": runtime_skips
                                        .iter()
                                        .map(|(r, _, _)| r.clone())
                                        .collect::<Vec<_>>(),
                                },
                            }),
                        )
                        .await;
                    }
                    return Ok(PhaseOutcome::Done);
                }
                self.fail_run_internal(
                    team_id,
                    &mut team,
                    &mut state,
                    "死锁：存在未完成步骤但无就绪步骤（检查上游失败依赖）",
                )
                .await?;
                return Ok(PhaseOutcome::Failed);
            }
            if agent_steps.is_empty() {
                // 仅人节点就绪：进入门闩（无 Worker 执行，无长锁窗口）。
                let waits = self.build_human_waits(team_id, &team, &state, &meta, &human_steps);
                self.mark_awaiting_human(team_id, &mut team, &waits).await?;
                self.persist_state(&state)?;
                return Ok(PhaseOutcome::AwaitingHuman { waits });
            }

            let epoch = self.claim_execution_epoch(team_id, &state)?;
            self.set_run_active(team_id, true);
            // 磁盘状态 → Running（R2 恢复底座）：进程若在本阶段内崩溃，
            // 磁盘留下 Running 且无活动循环 → 重启后被识别为 interrupted。
            self.mark_team_running(&mut team).await?;
            let claimed_at = now_ts();
            let mut claimed: Vec<ProgressStep> = Vec::with_capacity(agent_steps.len());
            for step in &agent_steps {
                if let Some(record) = state.records.get_mut(&step.id) {
                    // Disk remains Running for crash recovery; progress says Claimed until Worker::run starts.
                    record.status = StepStatus::Running;
                    record.phase_epoch = Some(epoch);
                    record.attempt_id = Some(uuid::Uuid::new_v4().to_string());
                    for receipt in &mut record.validation_receipts {
                        receipt.verdict = crate::plan::ValidationVerdictV1::Stale;
                        receipt.detail =
                            Some("new attempt claimed; prior receipt invalidated".into());
                    }
                    let role = worker_role(&step.worker).unwrap_or_else(|| step.worker.clone());
                    claimed.push(ProgressStep {
                        step_id: step.id.clone(),
                        worker: role,
                        status: "Claimed".to_string(),
                        attempts: record.attempts.saturating_add(1),
                        claimed_at: claimed_at.clone(),
                        started_at: String::new(),
                    });
                }
            }
            self.persist_state(&state)?;
            // 子计划覆盖当前已就绪任务及其所有无需等待 Human 的 agent 后继。
            // GoalRunner 收到每个任务结果后重算 ready 队列，使 A2 不必等同批慢任务 B。
            // 子图仅带本阶段可达节点与必要的成功依赖锚点；Human 分支留待门闩。
            let batch_ids: std::collections::HashSet<&str> =
                agent_steps.iter().map(|step| step.id.as_str()).collect();
            let defer_parallel_fanout = meta.parallel
                && !state
                    .plan
                    .steps
                    .iter()
                    .any(|step| step.input.get("assigned_task_id").is_some())
                && agent_steps.iter().any(|step| {
                    worker_role(&step.worker).is_some_and(|worker_role| {
                        meta.roles
                            .iter()
                            .find(|role| role.role == worker_role)
                            .is_some_and(RoleSpec::is_orchestrator)
                    })
                });
            let sub_ids = super::phase_subplan::phase_subplan_step_ids(
                &state,
                &meta,
                &agent_steps,
                defer_parallel_fanout,
            );
            for step in &state.plan.steps {
                if !sub_ids.contains(&step.id)
                    || batch_ids.contains(step.id.as_str())
                    || state.records[&step.id].status == StepStatus::Succeeded
                {
                    continue;
                }
                let record = state.records.get_mut(&step.id).expect("step record exists");
                record.phase_epoch = Some(epoch);
                record.attempt_id = Some(uuid::Uuid::new_v4().to_string());
                for receipt in &mut record.validation_receipts {
                    receipt.verdict = crate::plan::ValidationVerdictV1::Stale;
                    receipt.detail =
                        Some("queued in a new phase; prior receipt invalidated".into());
                }
                let role = worker_role(&step.worker).unwrap_or_else(|| step.worker.clone());
                claimed.push(ProgressStep {
                    step_id: step.id.clone(),
                    worker: role,
                    status: "Queued".to_string(),
                    attempts: record.attempts.saturating_add(1),
                    claimed_at: claimed_at.clone(),
                    started_at: String::new(),
                });
            }
            self.persist_state(&state)?;
            self.note_phase_claim(
                team_id,
                PhaseClaim {
                    epoch,
                    steps: claimed.clone(),
                },
            );
            self.advance_progress(team_id);

            let sub_steps = super::phase_subplan::phase_subplan_steps(&state, &sub_ids, epoch);
            let sub_records: BTreeMap<String, _> = state
                .records
                .iter()
                .filter(|(k, _)| sub_ids.contains(*k))
                .map(|(k, v)| {
                    let mut record = v.clone();
                    if batch_ids.contains(k.as_str()) && record.status == StepStatus::Running {
                        record.status = StepStatus::Pending;
                    }
                    (k.clone(), record)
                })
                .collect();
            let mut sub_state = GoalRunState {
                run_id: state.run_id.clone(),
                execution_epoch: epoch,
                team_task_events: None,
                goal: state.goal.clone(),
                plan: Plan {
                    id: state.plan.id.clone(),
                    goal_id: state.plan.goal_id.clone(),
                    description: format!("{}（阶段子计划）", state.plan.description),
                    steps: sub_steps,
                    created_at: now_ts(),
                },
                records: sub_records,
                validation_receipts: Vec::new(),
                completion_record: None,
                delivery_issues: Vec::new(),
                steps_taken: 0,
                total_retries: 0,
                replan_count: 0,
                started_at: now_ts(),
                events: Vec::new(),
                aborted: false,
            };
            // Phase runners validate individual tasks; root Goal acceptance belongs to
            // the final Team DeliveryGate and must not be applied to an incomplete phase.
            sub_state.goal.acceptance.clear();
            sub_state.goal.verification_plan = None;
            if let Err(e) = sub_state.plan.validate() {
                self.set_run_active(team_id, false);
                self.clear_phase_claim(team_id, epoch);
                return Err(WorkSwarmError::Run(format!("阶段子计划非法：{e}")));
            }
            // 子目标状态：强制可运行（整体 goal 已终态时上面会 Finished；这里处理 Failed 后 continue 的场景）。
            if sub_state.goal.status.is_terminal() {
                sub_state.goal.transition(GoalStatus::Running);
                sub_state.goal.error = None;
            }
            let goal_objective = state.goal.objective.clone();
            let context_steps = std::mem::take(&mut state.plan.steps);
            PhaseClaimPlan {
                epoch,
                sub_state,
                claimed,
                space,
                goal_objective,
                context_steps,
                shared_context_refs: team.shared_context_refs.clone(),
                meta,
                runtime_skips,
                review_issues_pending: state
                    .delivery_issues
                    .iter()
                    .any(|issue| issue.status != crate::goal::DeliveryIssueStatusV1::Resolved),
                integration_required,
                base_steps_taken: state.steps_taken,
                base_total_retries: state.total_retries,
            }
        }; // —— 阶段 A 结束：锁已释放 ——

        // Stable team inputs are prepared once from the same phase claim. Step state,
        // shared facts and artifacts remain live reads in each Worker context assembly.
        self.install_phase_context_snapshot(
            team_id,
            claim.epoch,
            claim.space.clone(),
            claim.meta.clone(),
            claim.goal_objective.clone(),
            claim.context_steps,
            claim.shared_context_refs.clone(),
        );
        // ---- 阶段 A'（锁外）：运行期跳过 → 自适应指标 + 审计（Done 路径已在锁内处理）。
        for (role, reason, saved) in &claim.runtime_skips {
            self.audit(
                team_id,
                "team.role_skipped",
                format!("运行期跳过角色 {role}：{reason}"),
            );
            self.note_adaptive_event(
                team_id,
                json!({
                    "kind": "role_skipped",
                    "role": role,
                    "reason": reason,
                    "saved_budget_calls": saved,
                    "role_skipped": {"role": role, "reason": reason},
                }),
            )
            .await;
        }

        // ---- 阶段 B（无锁）：Worker/模型执行 ----
        // 十一期：并行度来自团队预算 `max_parallel`（缺省 4；1..=8 收敛）。
        let config = RunnerConfig {
            max_parallel: (claim.sub_state.goal.budget.max_parallel as usize).clamp(1, 8),
            persist_dir: None,   // 阶段结束由协调器合并完整状态后统一落盘
            allow_replan: false, // 团队运行失败 = 显式失败（由 continue 决定重试）
            ..Default::default()
        };
        let code_change_template = claim
            .meta
            .template_id
            .as_deref()
            .is_some_and(crate::builtin_team_templates::is_code_change_template);
        let integration_required = claim.integration_required;
        let parallel_enabled = claim.meta.parallel;
        let parallel_host_manifest_enabled = parallel_enabled && !integration_required;
        let cancel = self.cancel_token(team_id);
        let mut runner = GoalRunner::from_state(claim.sub_state, config);
        runner.attach_abort_signal(cancel.abort_signal());
        runner.seed_execution_budget_usage(claim.base_steps_taken, claim.base_total_retries);
        runner.defer_goal_acceptance_to_delivery_gate();
        if let Some(root) = self.verification_workspace(team_id) {
            runner.attach_workspace_verification_root(root);
        }
        let verifier_coordinator = self.clone();
        let verifier_team_id = team_id.to_string();
        let verifier_run_dir = self.run_dir.clone();
        runner.attach_workspace_command_verifier(move |step_id, attempt_id, requirement| {
            let command_events = verifier_coordinator
                .runtime_event_details(&verifier_team_id, "team.command.executed");
            let change_sets = match crate::change_set_store::ChangeSetStore::new(&verifier_run_dir)
                .list_for_team(&verifier_team_id)
            {
                Ok(change_sets) => change_sets,
                Err(error) => {
                    return crate::goal::HostCommandValidationV1 {
                        verdict: crate::plan::ValidationVerdictV1::Unverified,
                        detail: Some(format!("读取宿主 ChangeSet 失败：{error}")),
                        subject_sha256: std::collections::BTreeMap::new(),
                        evidence_ref: None,
                    };
                }
            };
            let (verdict, detail, subject_sha256, evidence_ref) =
                super::delivery_gate_evidence::evaluate_workspace_command_receipt(
                    &verifier_team_id,
                    requirement,
                    &command_events,
                    step_id,
                    attempt_id,
                    &change_sets,
                );
            crate::goal::HostCommandValidationV1 {
                verdict,
                detail,
                subject_sha256,
                evidence_ref,
            }
        });
        let (progress_tx, mut progress_rx) = crate::goal::step_progress_channel();
        runner.attach_step_progress(progress_tx);
        let (admission_tx, mut admission_rx) = crate::goal::attempt_admission_channel();
        runner.attach_attempt_admission(admission_tx);
        let auto_reviewer_roles = claim
            .meta
            .roles
            .iter()
            .filter(|spec| is_auto_reviewer_role(spec))
            .map(|spec| spec.role.clone())
            .collect::<std::collections::HashSet<_>>();
        if code_change_template || parallel_host_manifest_enabled || !auto_reviewer_roles.is_empty()
        {
            let changes_path = self
                .run_dir
                .join(format!("{team_id}-workspace-changes.json"));
            let reviewer_roles = claim
                .meta
                .roles
                .iter()
                .filter(|spec| spec.is_reviewer())
                .map(|spec| spec.role.clone())
                .collect::<std::collections::HashSet<_>>();
            let review_issues_pending = claim.review_issues_pending;
            runner.attach_step_skipper(move |step| {
                let role = worker_role(&step.worker).unwrap_or_default();
                if host_manifest_can_replace_integration(
                    parallel_enabled,
                    &role,
                    integration_required,
                ) {
                    return Some(PARALLEL_LEADER_HOST_MANIFEST_SKIP_REASON.to_string());
                }
                if code_change_template || auto_reviewer_roles.contains(&role) {
                    let is_reviewer = reviewer_roles.contains(&role);
                    return crate::team_strategy::review_runtime_skip_for_workspace_status(
                        is_reviewer,
                        review_issues_pending,
                        super::coord_artifacts::workspace_change_status(&changes_path),
                    );
                }
                None
            });
        }
        let mut dynamic_skips: Vec<(String, String)> = Vec::new();
        if let Some(audit) = &self.audit {
            runner.attach_audit(Arc::clone(audit));
        }
        // 十期·四路 R2：取消链「先通知停止、再有界清理」。不直接丢弃 run Future——
        // 丢弃会跳过在飞 worker 的变更收尾（TrackedRoleWorker 的后快照/变更登记
        // 在其 Future 内）。取消时置位 abort 标志（协作式 worker 在回合边界快速
        // 返回并完成收尾），再等 run 以 [`GoalRunner::run`] 的协作退出路径自然收束。
        // 内层作用域：run_fut 借用 runner 到 select 结束即释放，便于阶段 C 读取状态。
        let result = {
            let abort_signal = runner.abort_signal();
            let run_fut = runner.run(registry);
            tokio::pin!(run_fut);
            let cancel_fut = wait_cancel(&cancel);
            tokio::pin!(cancel_fut);
            let mut cancel_watch_done = false;
            let result = 'run: loop {
                tokio::select! {
                    r = &mut run_fut => break 'run r,
                    Some(request) = admission_rx.recv() => {
                        let mut requests = vec![request];
                        while let Ok(request) = admission_rx.try_recv() {
                            requests.push(request);
                        }
                        if cancel.is_cancelled()
                            || abort_signal.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            reject_attempt_admissions(requests, "团队已取消，拒绝新的 Worker 调用");
                        } else {
                            self.persist_attempt_admissions(team_id, claim.epoch, &cancel, requests).await?;
                        }
                    }
                    Some(updates) = progress_rx.recv() => {
                        self.persist_step_progress_batch(
                            team_id,
                            claim.epoch,
                            updates,
                            &mut dynamic_skips,
                        ).await?;
                    }
                    cancelled = &mut cancel_fut, if !cancel_watch_done => {
                        if !cancelled {
                            cancel_watch_done = true;
                            continue;
                        }
                        abort_signal.store(true, std::sync::atomic::Ordering::SeqCst);
                        let deadline = tokio::time::Instant::now()
                            + crate::goal::PHASE_CANCELLATION_CLEANUP_GRACE;
                        loop {
                            tokio::select! {
                                r = &mut run_fut => break 'run r,
                                Some(request) = admission_rx.recv() => {
                                    let mut requests = vec![request];
                                    while let Ok(request) = admission_rx.try_recv() {
                                        requests.push(request);
                                    }
                                    if cancel.is_cancelled()
                                        || abort_signal.load(std::sync::atomic::Ordering::SeqCst)
                                    {
                                        reject_attempt_admissions(
                                            requests,
                                            "团队已取消，拒绝新的 Worker 调用",
                                        );
                                    } else {
                                        self.persist_attempt_admissions(
                                            team_id,
                                            claim.epoch,
                                            &cancel,
                                            requests,
                                        )
                                        .await?;
                                    }
                                }
                                Some(updates) = progress_rx.recv() => {
                                    self.persist_step_progress_batch(
                                        team_id,
                                        claim.epoch,
                                        updates,
                                        &mut dynamic_skips,
                                    ).await?;
                                }
                                _ = tokio::time::sleep_until(deadline) => {
                                    break 'run Ok(GoalStatus::Aborted);
                                }
                            }
                        }
                    }
                }
            };
            let mut pending_admissions = Vec::new();
            while let Ok(request) = admission_rx.try_recv() {
                pending_admissions.push(request);
            }
            if !pending_admissions.is_empty() {
                if cancel.is_cancelled()
                    || abort_signal.load(std::sync::atomic::Ordering::SeqCst)
                    || matches!(&result, Ok(GoalStatus::Aborted))
                {
                    reject_attempt_admissions(
                        pending_admissions,
                        "阶段已取消，拒绝未确认的 Worker 调用",
                    );
                } else {
                    self.persist_attempt_admissions(
                        team_id,
                        claim.epoch,
                        &cancel,
                        pending_admissions,
                    )
                    .await?;
                }
            }
            while let Ok(updates) = progress_rx.try_recv() {
                self.persist_step_progress_batch(team_id, claim.epoch, updates, &mut dynamic_skips)
                    .await?;
            }
            result
        };
        if matches!(&result, Ok(GoalStatus::Succeeded)) && !dynamic_skips.is_empty() {
            self.note_adaptive_event(
                team_id,
                json!({
                    "kind": "early_exit",
                    "early_exit": {
                        "reason": "运行期跳过使全部完成条件满足，DAG 提前结束",
                        "skipped_roles": dynamic_skips.iter().map(|(role, _)| role).collect::<Vec<_>>(),
                    },
                }),
            )
            .await;
        }

        self.finalize_phase(
            team_id,
            claim.epoch,
            claim.claimed,
            runner.state,
            claim.meta,
            result,
            &cancel,
        )
        .await
    }
}
