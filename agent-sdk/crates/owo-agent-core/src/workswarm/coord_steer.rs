use super::*;
impl TeamCoordinator {
    /// 应用 steer 指令。
    ///
    /// 并发语义（阶段调度锁与并行节点）：
    /// - `Cancel`：**立即**置位取消令牌（无锁；进行中的阶段 select 立即感知并自行收尾），
    ///   随后取锁收尾（若阶段已先行收尾则幂等跳过）——因此 cancel 永远可达成，
    ///   且不会与运行中阶段竞争写状态。
    /// - `Continue / Steer / Replace / Retry`：前置无锁检查运行标志，运行中 → 立即 Conflict
    ///   （409 语义：待阶段结束后重试，或先 cancel）；随后取锁复检（阶段在检查后启动的窗口也被覆盖）。
    ///   人节点等待窗口（暂停）允许改盘，下一阶段读取最新状态。
    /// - R2 中断恢复：`Continue / Retry` 进入前会先就地识别「磁盘 Running 但无活动运行」的
    ///   中断残留（幂等标记 + Running 步骤转 Aborted 可恢复），随后按显式指令恢复；
    ///   绝不静默重放任何步骤执行。
    pub async fn apply_steer(&self, team_id: &str, cmd: &SteerCommand) -> WorkSwarmResult<TeamRun> {
        match cmd {
            SteerCommand::Cancel => {
                let cancel = self.cancel_token(team_id);
                cancel.cancel(); // 立即（无锁）：进行中的阶段 select 立即感知
                                 // 代次立即失效：旧阶段的合并与产物回传即刻被拒（阶段 C / 回传校验）。
                self.bump_phase_epoch(team_id)?;
                self.advance_progress(team_id);
                let lock = self.team_lock(team_id);
                let _guard = lock.lock().await; // 运行中阶段会先完成取消收尾（同一把锁）
                let (mut team, mut state) = {
                    let (t, _s, st) = self.load_bundle(team_id).await?;
                    (t, st)
                };
                if !team.status.is_terminal() {
                    self.cancel_run_internal(
                        team_id,
                        &mut team,
                        &mut state,
                        "steer cancel（运行取消）",
                    )
                    .await?;
                }
                Ok(team)
            }
            _ => {
                if self.is_run_active(team_id) {
                    return Err(WorkSwarmError::Conflict(
                        "运行正在执行中，该 steer 不可用（待阶段结束后重试，或先 cancel）"
                            .to_string(),
                    ));
                }
                let lock = self.team_lock(team_id);
                let _guard = lock.lock().await;
                if self.is_run_active(team_id) {
                    return Err(WorkSwarmError::Conflict(
                        "运行正在执行中，该 steer 不可用（待阶段结束后重试，或先 cancel）"
                            .to_string(),
                    ));
                }
                // 转向接管现场：阶段代次 +1（此刻无在飞阶段，防御性使旧回传即刻失效）。
                self.bump_phase_epoch(team_id)?;
                self.advance_progress(team_id);
                // R2：continue/retry 前先就地识别「磁盘 Running 但无活动运行」的中断残留
                // （幂等；不满足条件时是空操作）。恢复仍必须显式发起——这里只是把
                // 中断遗留的 Running 步骤转成可恢复状态并落识别标记。
                if matches!(cmd, SteerCommand::Continue | SteerCommand::Retry { .. }) {
                    // R2/R3：显式恢复先重新武装取消令牌——历史取消不得粘滞到下一轮
                    // （否则 continue 重启循环的瞬间又被旧取消打断，取消链不可收敛；
                    // 此时已确认无活动运行，在飞旧阶段不存在，重置安全且原子）。
                    self.reset_cancel_token(team_id);
                    self.mark_interrupted_if_applicable(team_id).await?;
                    let (mut team, mut state) = {
                        let (t, _s, st) = self.load_bundle(team_id).await?;
                        (t, st)
                    };
                    return match cmd {
                        SteerCommand::Continue => {
                            self.steer_continue(team_id, &mut team, &mut state).await
                        }
                        SteerCommand::Retry { step_id, note } => {
                            self.steer_retry(team_id, &mut team, &mut state, step_id, note)
                                .await
                        }
                        _ => unreachable!("matches! 已过滤"),
                    };
                }
                let (mut team, mut state) = {
                    let (t, _s, st) = self.load_bundle(team_id).await?;
                    (t, st)
                };
                match cmd {
                    SteerCommand::Cancel => {
                        // 防御：正常走上面的分支（cancel 不取前置检查）。
                        let (mut t2, mut st2) = {
                            let (t, _s, st) = self.load_bundle(team_id).await?;
                            (t, st)
                        };
                        self.cancel_run_internal(team_id, &mut t2, &mut st2, "steer cancel")
                            .await?;
                        Ok(t2)
                    }
                    SteerCommand::Continue => {
                        self.steer_continue(team_id, &mut team, &mut state).await
                    }
                    SteerCommand::Steer {
                        step_id,
                        new_input,
                        note,
                    } => {
                        self.steer_nodes(team_id, &mut team, &mut state, step_id, new_input, note)
                            .await
                    }
                    SteerCommand::Replace {
                        role,
                        new_worker,
                        new_user_id,
                        note,
                    } => {
                        self.replace_member(
                            team_id,
                            &mut team,
                            role,
                            new_worker.as_deref(),
                            new_user_id.as_deref(),
                            note,
                        )
                        .await
                    }
                    SteerCommand::Retry { step_id, note } => {
                        self.steer_retry(team_id, &mut team, &mut state, step_id, note)
                            .await
                    }
                }
            }
        }
    }

    /// continue 内部逻辑（调用方已完成运行中检查与加锁）。
    pub(crate) async fn steer_continue(
        &self,
        team_id: &str,
        team: &mut TeamRun,
        state: &mut GoalRunState,
    ) -> WorkSwarmResult<TeamRun> {
        {
            match team.status {
                TeamRunStatus::Failed | TeamRunStatus::Cancelled => {}
                TeamRunStatus::Created => {}
                // R2：磁盘 Running 但无活动阶段 = 中断遗留或阶段间暂停窗口，
                // 允许显式 continue 恢复（前置检查已确认非运行中）。
                TeamRunStatus::Running if !self.is_run_active(team_id) => {}
                _ => {
                    return Err(WorkSwarmError::Conflict(format!(
                        "当前状态 {:?} 不可 continue（仅 Failed/Cancelled/Created/中断遗留）",
                        team.status
                    )));
                }
            }
            // Durable attempt reset and receipt invalidation are one reducer transition.
            let reset_steps = match super::run_state::apply_run_execution_event(
                state,
                super::run_state::RunExecutionEvent::ContinueReset,
            ) {
                Ok(super::run_state::RunExecutionEffect::ContinueApplied(steps)) => steps,
                _ => unreachable!("continue must produce the matching run-state effect"),
            };
            team.status = TeamRunStatus::Created;
            team.updated_at = now_ts();
            for m in &mut team.members {
                if m.health == MemberHealth::Degraded {
                    m.health = MemberHealth::Active;
                }
            }
            self.store.save_team_run(&*team).await?;
            self.persist_state(&*state)?;
            // 恢复成功：清除中断识别标记。
            self.clear_interrupted_marker(team_id);
            self.space_activity(
                team_id,
                &format!(
                    "team.steer.continue：重置 {} 个未完成步骤（已完成产物保留）",
                    reset_steps.len()
                ),
            )
            .await?;
            self.audit(
                team_id,
                "team.steer.continue",
                "continue：重置未完成步骤并重跑".to_string(),
            );
            Ok(team.clone())
        }
    }

    pub(crate) async fn steer_nodes(
        &self,
        team_id: &str,
        team: &mut TeamRun,
        state: &mut GoalRunState,
        step_id: &Option<String>,
        new_input: &Option<Value>,
        note: &str,
    ) -> WorkSwarmResult<TeamRun> {
        // 运行中检查由 apply_steer 前置统一处理（无锁快速路径 + 锁内复检）。
        // R2：磁盘 Running 但无活动阶段（阶段间暂停窗口 / 中断遗留）同样放行——
        // 与旧行为一致：人节点等待/批次间隙允许修改未完成节点。
        match team.status {
            TeamRunStatus::Created
            | TeamRunStatus::Failed
            | TeamRunStatus::Cancelled
            | TeamRunStatus::AwaitingHuman => {}
            TeamRunStatus::Running if !self.is_run_active(team_id) => {}
            _ => {
                return Err(WorkSwarmError::Conflict(format!(
                    "当前状态 {:?} 不可 steer",
                    team.status
                )));
            }
        }
        let note = if note.trim().is_empty() {
            "steer".to_string()
        } else {
            note.trim().to_string()
        };
        let targets: Vec<StepSpec> = match step_id {
            Some(id) => {
                let step = state
                    .plan
                    .steps
                    .iter()
                    .find(|s| &s.id == id)
                    .ok_or_else(|| WorkSwarmError::NotFound(format!("任务 {id} 不存在")))?;
                let record = &state.records[&step.id];
                if record.status.is_terminal() && record.status != StepStatus::Aborted {
                    return Err(WorkSwarmError::Conflict(format!(
                        "任务 {} 已完成（{:?}），不能 steer——已完成成果不受新指令影响",
                        step.id, record.status
                    )));
                }
                vec![step.clone()]
            }
            None => state
                .plan
                .steps
                .iter()
                .filter(|s| {
                    let r = &state.records[&s.id];
                    !r.status.is_terminal() || r.status == StepStatus::Aborted
                })
                .cloned()
                .collect(),
        };
        if targets.is_empty() {
            return Err(WorkSwarmError::Conflict(
                "没有可 steer 的未完成节点（全部已完成）".to_string(),
            ));
        }
        let affected: Vec<String> = targets.iter().map(|s| s.id.clone()).collect();
        for step in &mut state.plan.steps {
            if !targets.iter().any(|t| t.id == step.id) {
                continue;
            }
            if let Some(new_input) = new_input {
                if new_input.is_object() {
                    if let (Some(a), Some(b)) = (step.input.as_object_mut(), new_input.as_object())
                    {
                        for (k, v) in b {
                            a.insert(k.clone(), v.clone());
                        }
                    } else {
                        step.input = new_input.clone();
                    }
                } else {
                    step.input = new_input.clone();
                }
            }
            // 重新注入 _workswarm 标记（防被 new_input 覆盖）。
            if let Some(obj) = step.input.as_object_mut() {
                obj.insert(
                    "_workswarm".to_string(),
                    json!({
                        "team_id": team_id,
                        "member_id": step.worker,
                        "step_id": step.id,
                    }),
                );
            }
        }
        // 变更必须留 DecisionRecord（结论不留在聊天里）。
        let decision = DecisionRecord {
            decision_id: format!("{team_id}:steer:{}", now_ms()),
            proposer: "user".to_string(),
            choice: format!("steer：{note}（备选：维持原计划继续）"),
            affected_refs: affected.clone(),
            rationale: "steer：只修改未完成节点，已完成产物保留".to_string(),
            created_at: now_ts(),
        };
        let pid = team
            .project_space_id
            .clone()
            .ok_or_else(|| WorkSwarmError::Run("缺少 project_space_id".to_string()))?;
        self.store.save_decision(&decision, &pid).await?;
        team.updated_at = now_ts();
        if let Some(space_id) = &team.project_space_id {
            if let Ok(mut space) = self.store.get_project_space(space_id).await {
                space.decisions.push(decision.decision_id.clone());
                space.version += 1;
                let _ = self.store.save_project_space(&space).await;
            }
        }
        self.persist_state(state)?;
        self.store.save_team_run(team).await?;
        self.space_activity(
            team_id,
            &format!("team.steer：{} 影响 {} 个未完成节点", note, affected.len()),
        )
        .await?;
        self.audit(
            team_id,
            "team.steer",
            format!("steer：{note}；影响节点：{}", affected.join(", ")),
        );
        Ok(team.clone())
    }

    /// retry 内部逻辑（R2 局部重试；调用方已完成运行中检查、加锁与中断就地识别）。
    ///
    /// 契约：
    /// - 目标步骤必须处于 Failed / Aborted（中断识别会把遗留 Running 转 Aborted）；
    ///   已成功（Succeeded）目标 → Conflict——重复发送同一 retry 不产生额外副作用；
    /// - 只重置目标步骤及其**尚未成功**的下游闭包；其余状态（包括并行分支的失败）
    ///   保持原样，由用户逐个显式处理；
    /// - 已成功步骤执行次数不增加；已有 Artifact 版本/CAS ref、Handoff、DecisionRecord 不删除不重跑；
    /// - 所有校验先于任何持久化发生（全部拒绝路径零写副作用）；
    /// - 变更留下 DecisionRecord（结论不留在聊天里），恢复后重启运行循环（server 侧负责 spawn）。
    pub(crate) async fn steer_retry(
        &self,
        team_id: &str,
        team: &mut TeamRun,
        state: &mut GoalRunState,
        step_id: &str,
        note: &str,
    ) -> WorkSwarmResult<TeamRun> {
        let note = if note.trim().is_empty() {
            "retry".to_string()
        } else {
            note.trim().to_string()
        };
        let step = state
            .plan
            .steps
            .iter()
            .find(|s| s.id == step_id)
            .ok_or_else(|| WorkSwarmError::NotFound(format!("任务 {step_id} 不存在")))?
            .clone();
        let target_status = state
            .records
            .get(&step.id)
            .map(|r| r.status)
            .ok_or_else(|| {
                WorkSwarmError::Run(format!("任务 {} 缺少执行记录（状态不一致）", step.id))
            })?;
        // 目标校验先于团队状态闸门：重复发送同一 retry（目标已成功/已重置）必须
        // 返回明确的目标级冲突，而不是笼统的状态冲突——且全部拒绝路径零写副作用。
        if target_status == StepStatus::Succeeded {
            return Err(WorkSwarmError::Conflict(format!(
                "任务 {} 已成功，不能 retry——重复发送同一 retry 不产生额外副作用",
                step.id
            )));
        }
        if !matches!(target_status, StepStatus::Failed | StepStatus::Aborted) {
            return Err(WorkSwarmError::Conflict(format!(
                "仅 Failed/Aborted/中断中的步骤可重试（任务 {} 当前 {:?}）",
                step.id, target_status
            )));
        }
        // 团队状态闸门：只拦「真正运行中」；Created/Failed/Cancelled/AwaitingHuman、
        // 中断遗留与暂停窗口（Running 且无活动阶段）均放行。
        if team.status == TeamRunStatus::Running && self.is_run_active(team_id) {
            return Err(WorkSwarmError::Conflict(format!(
                "当前状态 {:?} 不可 retry（运行中）",
                team.status
            )));
        }

        // ---- 校验全部通过，开始变更 ----
        let downstream = Self::downstream_reset_closure(state, &step.id);
        let mut affected = vec![step.id.clone()];
        affected.extend(downstream.iter().cloned());
        let expected_affected = affected.iter().cloned().collect::<HashSet<_>>();
        match super::run_state::apply_run_execution_event(
            state,
            super::run_state::RunExecutionEvent::RetryReset {
                target_step_id: step.id.clone(),
                affected_step_ids: affected.clone(),
                note: note.clone(),
            },
        ) {
            Ok(super::run_state::RunExecutionEffect::RetryApplied(applied))
                if applied.iter().cloned().collect::<HashSet<_>>() == expected_affected => {}
            _ => unreachable!("retry reducer must reset exactly the validated closure"),
        }

        // 受影响成员恢复健康（Degraded → Active；只动受影响成员）。
        let affected_members: HashSet<String> = state
            .plan
            .steps
            .iter()
            .filter(|s| expected_affected.contains(&s.id))
            .map(|s| s.worker.clone())
            .collect();
        for m in &mut team.members {
            if m.health == MemberHealth::Degraded && affected_members.contains(&m.member_id) {
                m.health = MemberHealth::Active;
            }
        }
        team.status = TeamRunStatus::Created;
        team.updated_at = now_ts();

        // 变更留痕：DecisionRecord（affected_refs = 目标 + 未完成下游闭包）。
        let decision = DecisionRecord {
            decision_id: format!("{team_id}:retry:{}", now_ms()),
            proposer: "user".to_string(),
            choice: format!(
                "retry：{note}（目标 {} 及未完成下游共 {} 个节点）",
                step.id,
                affected.len()
            ),
            affected_refs: affected.clone(),
            rationale: "retry：仅重置目标步骤及未完成下游；已成功步骤与既有产物/交接/决策不动"
                .to_string(),
            created_at: now_ts(),
        };
        let pid = team
            .project_space_id
            .clone()
            .ok_or_else(|| WorkSwarmError::Run("缺少 project_space_id".to_string()))?;
        self.store.save_decision(&decision, &pid).await?;
        if let Some(space_id) = &team.project_space_id {
            if let Ok(mut space) = self.store.get_project_space(space_id).await {
                space.decisions.push(decision.decision_id.clone());
                space.version += 1;
                let _ = self.store.save_project_space(&space).await;
            }
        }

        self.persist_state(state)?;
        self.store.save_team_run(team).await?;
        // 恢复成功：清除中断识别标记。
        self.clear_interrupted_marker(team_id);
        self.space_activity(
            team_id,
            &format!(
                "team.steer.retry：{} 影响 {} 个节点（其余成功产物保持不变）",
                note,
                affected.len()
            ),
        )
        .await?;
        self.audit(
            team_id,
            "team.steer.retry",
            format!("retry：{note}；影响节点：{}", affected.join(", ")),
        );
        Ok(team.clone())
    }

    /// 评审返工（V1 五期 · 第二路）：重置**已成功**的生产步骤及其未成功下游，
    /// 并把返工指令注入步骤输入（`rework.instruction`），供重跑 Worker 消费。
    ///
    /// 与 [`Self::steer_retry`] 的差异：
    /// - retry 面向 Failed/Aborted（重复请求零副作用）；rework 面向 Succeeded
    ///   （评审要求修改 → 重新执行产生新版本），目标是已成功步骤本身；
    /// - 已成功步骤执行次数清零重跑；已有 Artifact 版本/CAS/交接/评审记录全部保留
    ///   （新版本经版本链取代旧版，由评审闭环收口 approved head）；
    /// - 所有校验先于任何持久化（拒绝路径零写副作用）。
    pub async fn rework_step(
        &self,
        team_id: &str,
        step_id: &str,
        instruction: &str,
        note: &str,
    ) -> WorkSwarmResult<TeamRun> {
        self.rework_step_with_actor(team_id, step_id, instruction, note, "user", None)
            .await
    }

    /// Rework accepted worker output at the request of a deterministic host validator.
    pub async fn rework_step_from_validator(
        &self,
        team_id: &str,
        step_id: &str,
        instruction: &str,
        note: &str,
    ) -> WorkSwarmResult<TeamRun> {
        self.rework_step_with_actor(team_id, step_id, instruction, note, "host_validator", None)
            .await
    }

    /// Idempotent review rework entry point used by durable artifact rework tasks.
    pub async fn rework_step_for_review(
        &self,
        team_id: &str,
        step_id: &str,
        instruction: &str,
        review_id: &str,
    ) -> WorkSwarmResult<TeamRun> {
        if review_id.trim().is_empty() {
            return Err(WorkSwarmError::Validation("review_id 不能为空".to_string()));
        }
        self.rework_step_with_actor(
            team_id,
            step_id,
            instruction,
            &format!("评审返工 {review_id}"),
            "reviewer",
            Some(review_id),
        )
        .await
    }

    pub(crate) async fn replace_member(
        &self,
        team_id: &str,
        team: &mut TeamRun,
        role: &str,
        new_worker: Option<&str>,
        new_user_id: Option<&str>,
        note: &str,
    ) -> WorkSwarmResult<TeamRun> {
        // 运行中检查由 apply_steer 前置统一处理（无锁快速路径 + 锁内复检）。
        // R2：Running 且无活动阶段（暂停窗口 / 中断遗留）同样放行（与 steer 一致）。
        match team.status {
            TeamRunStatus::Created
            | TeamRunStatus::Failed
            | TeamRunStatus::Cancelled
            | TeamRunStatus::AwaitingHuman => {}
            TeamRunStatus::Running if !self.is_run_active(team_id) => {}
            _ => {
                return Err(WorkSwarmError::Conflict(format!(
                    "当前状态 {:?} 不可 replace",
                    team.status
                )));
            }
        }
        let member = team
            .members
            .iter_mut()
            .find(|m| m.role == role)
            .ok_or_else(|| WorkSwarmError::NotFound(format!("角色 {role} 的成员不存在")))?;
        let note = if note.trim().is_empty() {
            "replace".to_string()
        } else {
            note.trim().to_string()
        };
        let old_binding = format!("{:?}", member.runtime_binding);
        match (new_worker, new_user_id) {
            (Some(w), Some(u)) if !w.is_empty() && !u.is_empty() => {
                return Err(WorkSwarmError::Validation(
                    "new_worker 与 new_user_id 只能提供一个".to_string(),
                ));
            }
            (Some(w), _) if !w.trim().is_empty() => {
                member.runtime_binding = RuntimeBinding::Agent {
                    agent_id: format!("{team_id}:{}:replaced", member.member_id),
                };
                member.health = MemberHealth::Active;
            }
            (None, Some(u)) if !u.trim().is_empty() => {
                member.runtime_binding = RuntimeBinding::Human {
                    user_id: u.to_string(),
                };
                member.health = MemberHealth::Active;
            }
            _ => {
                return Err(WorkSwarmError::Validation(
                    "replace 需要 new_worker 或 new_user_id 之一".to_string(),
                ));
            }
        }
        // 角色规格 sidecar 同步（运行循环每阶段据此重建 worker 注册表 → 新阶段生效）。
        let mut meta = RunMeta::load(&self.run_dir, team_id)?;
        let spec = meta
            .roles
            .iter_mut()
            .find(|r| r.role == role)
            .ok_or_else(|| {
                WorkSwarmError::Run(format!("角色 {role} 无角色规格（元数据不一致）"))
            })?;
        match (new_worker, new_user_id) {
            (Some(w), _) => {
                spec.assignee = "agent".to_string();
                spec.worker = Some(w.trim().to_string());
            }
            (None, Some(u)) => {
                spec.assignee = "human".to_string();
                spec.worker = Some(u.trim().to_string());
            }
            _ => {}
        }
        meta.save(&self.run_dir)?;
        // 只有未完成步骤换人（已完成产物保持原承担者记录 = 历史事实）。
        let affected: Vec<String> = self
            .load_state(team_id)
            .map(|s| {
                s.plan
                    .steps
                    .iter()
                    .filter(|st| {
                        st.worker == member.member_id && !s.records[&st.id].status.is_terminal()
                    })
                    .map(|st| st.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let decision = DecisionRecord {
            decision_id: format!("{team_id}:replace:{}", now_ms()),
            proposer: "user".to_string(),
            choice: format!("replace {role}：{note}（原绑定 {old_binding}）"),
            affected_refs: affected.clone(),
            rationale: "replace：仅未完成节点换人，已完成产物保留原承担者".to_string(),
            created_at: now_ts(),
        };
        let pid = team
            .project_space_id
            .clone()
            .ok_or_else(|| WorkSwarmError::Run("缺少 project_space_id".to_string()))?;
        self.store.save_decision(&decision, &pid).await?;
        team.updated_at = now_ts();
        self.store.save_team_run(team).await?;
        self.space_activity(
            team_id,
            &format!(
                "team.replace：{} 更换承担者（{} 个未完成节点受影响）",
                role,
                affected.len()
            ),
        )
        .await?;
        self.audit(
            team_id,
            "team.replace",
            format!("{role} 换人：{note}；影响节点：{}", affected.join(", ")),
        );
        Ok(team.clone())
    }
}
