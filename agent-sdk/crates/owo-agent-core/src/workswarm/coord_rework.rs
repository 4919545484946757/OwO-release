//! Shared rework state transition for user, validator and grouped review repairs.
//! Validate the whole batch before resetting any owner or downstream record.
use super::*;

#[derive(Debug, Clone)]
pub(super) struct ReworkRequest {
    pub(super) step_id: String,
    pub(super) instruction: String,
    pub(super) note: String,
    pub(super) actor: String,
    pub(super) source_id: Option<String>,
    pub(super) expected_attempt_id: Option<String>,
    pub(super) issue_ids: Vec<String>,
}

struct PreparedRework {
    request: ReworkRequest,
    step: StepSpec,
    attempt: u64,
}

enum ReworkPreparation {
    AlreadyApplied,
    Ready(Vec<PreparedRework>),
}

fn prepare_reworks(
    state: &GoalRunState,
    requests: &[ReworkRequest],
) -> WorkSwarmResult<ReworkPreparation> {
    if requests.is_empty() {
        return Err(WorkSwarmError::Validation("返修批次不能为空".into()));
    }
    let mut owners = HashSet::new();
    let mut prepared = Vec::new();
    let mut applied = 0usize;
    for request in requests {
        if request.instruction.trim().is_empty() || request.instruction.len() > 32 * 1024 {
            return Err(WorkSwarmError::Validation(
                "返工指令必须为 1..=32768 字节，不能截断问题".into(),
            ));
        }
        if !owners.insert(&request.step_id) {
            return Err(WorkSwarmError::Validation(
                "同一返修批次必须先按 owner 合并".into(),
            ));
        }
        let step = state
            .plan
            .steps
            .iter()
            .find(|step| step.id == request.step_id)
            .ok_or_else(|| WorkSwarmError::NotFound(format!("任务 {} 不存在", request.step_id)))?;
        let record = state
            .records
            .get(&step.id)
            .ok_or_else(|| WorkSwarmError::Run(format!("任务 {} 缺少执行记录", step.id)))?;
        let previous = step.input.get("rework");
        if let Some(source) = request.source_id.as_deref() {
            if previous
                .and_then(|value| value.get("source_id"))
                .and_then(Value::as_str)
                == Some(source)
            {
                if previous
                    .and_then(|value| value.get("instruction"))
                    .and_then(Value::as_str)
                    != Some(request.instruction.trim())
                {
                    return Err(WorkSwarmError::Conflict(
                        "相同评审 ID 不能更换返工指令".into(),
                    ));
                }
                applied += 1;
                continue;
            }
        }
        if record.status != StepStatus::Succeeded || record.skip_reason.is_some() {
            return Err(WorkSwarmError::Conflict(format!(
                "返工目标必须已执行成功（任务 {} 当前 {:?}）",
                step.id, record.status
            )));
        }
        if request
            .expected_attempt_id
            .as_deref()
            .is_some_and(|expected| record.attempt_id.as_deref() != Some(expected))
        {
            return Err(WorkSwarmError::Conflict(format!(
                "任务 {} 的被审 attempt 已变化",
                step.id
            )));
        }
        if !step.input.is_object() {
            return Err(WorkSwarmError::Validation(format!(
                "任务 {} 输入无法承载返修指令",
                step.id
            )));
        }
        let attempt = previous
            .and_then(|value| value.get("attempt"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if matches!(request.actor.as_str(), "reviewer" | "host_validator")
            && attempt >= u64::from(state.goal.budget.max_retries_per_step)
        {
            return Err(WorkSwarmError::Validation(format!(
                "任务 {} 已耗尽 {} 次评审返修预算",
                step.id, state.goal.budget.max_retries_per_step
            )));
        }
        for id in &request.issue_ids {
            let issue = state
                .delivery_issues
                .iter()
                .find(|issue| issue.issue_id == *id)
                .ok_or_else(|| WorkSwarmError::Conflict(format!("返修 Issue 不存在：{id}")))?;
            if request.actor != "reviewer"
                || issue.owner_step_id != step.id
                || issue.status != crate::goal::DeliveryIssueStatusV1::Open
                || Some(issue.target_attempt_id.as_str()) != record.attempt_id.as_deref()
            {
                return Err(WorkSwarmError::Conflict(format!(
                    "返修 Issue 身份或状态已变化：{id}"
                )));
            }
        }
        prepared.push(PreparedRework {
            request: request.clone(),
            step: step.clone(),
            attempt: attempt.saturating_add(1),
        });
    }
    if applied == requests.len() {
        return Ok(ReworkPreparation::AlreadyApplied);
    }
    if applied != 0 {
        return Err(WorkSwarmError::Conflict(
            "返修批次只有部分已应用，拒绝重置未核对的状态".into(),
        ));
    }
    let automatic = prepared
        .iter()
        .filter(|item| matches!(item.request.actor.as_str(), "reviewer" | "host_validator"))
        .count() as u64;
    if automatic > 0 {
        let prior: u64 = state
            .plan
            .steps
            .iter()
            .map(|step| {
                step.input
                    .get("rework")
                    .and_then(|v| v.get("attempt"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            })
            .fold(0u64, u64::saturating_add);
        if u64::from(state.total_retries)
            .saturating_add(prior)
            .saturating_add(automatic)
            > u64::from(state.goal.budget.max_total_retries)
        {
            return Err(WorkSwarmError::Validation("全队自动返工预算已耗尽".into()));
        }
    }
    Ok(ReworkPreparation::Ready(prepared))
}

/// One union closure ensures dependent owners are checked before any is reset.
/// Untouched siblings retain their successful records and validation receipts.
fn apply_prepared_reworks(state: &mut GoalRunState, prepared: &[PreparedRework]) -> Vec<String> {
    let mut affected = std::collections::BTreeSet::new();
    for item in prepared {
        affected.insert(item.step.id.clone());
        affected.extend(TeamCoordinator::downstream_recheck_closure(
            state,
            &item.step.id,
        ));
    }
    let items = prepared
        .iter()
        .map(|item| super::run_state::ReworkStepMutation {
            step_id: item.step.id.clone(),
            instruction: item.request.instruction.clone(),
            note: item.request.note.clone(),
            actor: item.request.actor.clone(),
            source_id: item.request.source_id.clone(),
            issue_ids: item.request.issue_ids.clone(),
            attempt: item.attempt,
        })
        .collect();
    match super::run_state::apply_run_execution_event(
        state,
        super::run_state::RunExecutionEvent::ReworkBatch {
            items,
            affected_step_ids: affected.into_iter().collect(),
            requested_at: now_ts(),
        },
    ) {
        Ok(super::run_state::RunExecutionEffect::ReworkApplied(affected)) => affected,
        _ => unreachable!("rework batch must produce the matching effect"),
    }
}

impl TeamCoordinator {
    pub(super) async fn rework_step_with_actor(
        &self,
        team_id: &str,
        step_id: &str,
        instruction: &str,
        note: &str,
        actor: &str,
        source_id: Option<&str>,
    ) -> WorkSwarmResult<TeamRun> {
        let request = ReworkRequest {
            step_id: step_id.into(),
            instruction: instruction.trim().into(),
            note: if note.trim().is_empty() {
                "rework".into()
            } else {
                note.trim().into()
            },
            actor: actor.into(),
            source_id: source_id.map(str::to_string),
            expected_attempt_id: None,
            issue_ids: Vec::new(),
        };
        self.rework_batch(team_id, &[request], None).await
    }

    pub(super) async fn rework_batch(
        &self,
        team_id: &str,
        requests: &[ReworkRequest],
        expected_epoch: Option<u64>,
    ) -> WorkSwarmResult<TeamRun> {
        let lock = self.team_lock(team_id);
        let _guard = lock.lock().await;
        if expected_epoch.is_some_and(|epoch| self.phase_epoch(team_id) != epoch) {
            return Err(WorkSwarmError::Conflict("返修领取的阶段代次已变化".into()));
        }
        let (mut team, mut space, mut state) = self.load_bundle(team_id).await?;
        let automatic = requests
            .iter()
            .any(|request| matches!(request.actor.as_str(), "reviewer" | "host_validator"));
        if automatic
            && (self.cancel_token(team_id).is_cancelled()
                || state.aborted
                || team.status == TeamRunStatus::Cancelled
                || expected_epoch.is_some_and(|epoch| self.phase_epoch(team_id) != epoch))
        {
            return Err(WorkSwarmError::Conflict(
                "自动返工已取消或代次变化，不能重开任务".into(),
            ));
        }

        let prepared = match prepare_reworks(&state, requests)? {
            ReworkPreparation::AlreadyApplied => {
                if team.status == TeamRunStatus::Succeeded
                    && requests.iter().any(|request| {
                        state
                            .records
                            .get(&request.step_id)
                            .is_some_and(|record| record.status != StepStatus::Succeeded)
                    })
                {
                    team.status = TeamRunStatus::Created;
                    team.updated_at = now_ts();
                    self.store.save_team_run(&team).await?;
                }
                return Ok(team);
            }
            ReworkPreparation::Ready(items) => items,
        };
        if self.is_run_active(team_id) {
            return Err(WorkSwarmError::Conflict(
                "运行正在执行中，不能发起返工".into(),
            ));
        }
        self.bump_phase_epoch(team_id)?;
        let affected = apply_prepared_reworks(&mut state, &prepared);
        let affected_members: HashSet<&str> = state
            .plan
            .steps
            .iter()
            .filter(|step| affected.contains(&step.id))
            .map(|step| step.worker.as_str())
            .collect();
        for member in &mut team.members {
            if member.health == MemberHealth::Degraded
                && affected_members.contains(member.member_id.as_str())
            {
                member.health = MemberHealth::Active;
            }
        }
        team.status = TeamRunStatus::Created;
        team.updated_at = now_ts();
        for item in &prepared {
            let decision_id = match item.request.source_id.as_deref() {
                Some(source) if prepared.len() == 1 => format!("{team_id}:rework:{source}"),
                Some(source) => format!("{team_id}:rework:{source}:{}", item.step.id),
                None => format!("{team_id}:rework:{}:{}", now_ms(), item.step.id),
            };
            let decision = DecisionRecord {
                decision_id: decision_id.clone(),
                proposer: item.request.actor.clone(),
                choice: format!(
                    "rework：{}（目标 {}；批次共 {} 个 owner）",
                    item.request.note,
                    item.step.id,
                    prepared.len()
                ),
                affected_refs: affected.clone(),
                rationale: "校验所有 owner 后统一重置目标及下游，保留任务、权限、历史产物与评审"
                    .into(),
                created_at: now_ts(),
            };
            self.store
                .save_decision(&decision, &space.project_id)
                .await?;
            if !space.decisions.contains(&decision_id) {
                space.decisions.push(decision_id);
            }
        }
        // One durable state publication per batch, not one reset per finding/owner.
        self.persist_state(&state)?;
        space.version += 1;
        self.store.save_project_space(&space).await?;
        self.store.save_team_run(&team).await?;
        self.clear_interrupted_marker(team_id);
        self.advance_progress(team_id);
        self.space_activity(
            team_id,
            &format!(
                "team.rework：{} 个 owner，影响 {} 个节点（批量指令已注入）",
                prepared.len(),
                affected.len()
            ),
        )
        .await?;
        self.audit(
            team_id,
            "team.rework",
            format!(
                "owner_count={} issue_count={} affected={}",
                prepared.len(),
                requests
                    .iter()
                    .map(|request| request.issue_ids.len())
                    .sum::<usize>(),
                affected.join(", ")
            ),
        );
        Ok(team)
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::goal::StepRecord;

    fn state() -> GoalRunState {
        let mut plan = Plan::new("plan", "goal");
        for (id, deps) in [
            ("a", vec![]),
            ("b", vec!["a"]),
            ("review", vec!["b"]),
            ("sibling", vec![]),
        ] {
            let mut step = StepSpec::new(id, format!("m-{id}"));
            step.input = json!({"objective":id,"write_paths":[format!("{id}.rs")]});
            step.depends_on = deps.into_iter().map(str::to_string).collect();
            plan.add_step(step);
        }
        let mut state = GoalRunState::new(Goal::new("goal", "repair"), plan);
        state.goal.budget.max_retries_per_step = 2;
        for step in &state.plan.steps {
            state.records.insert(
                step.id.clone(),
                StepRecord {
                    step_id: step.id.clone(),
                    status: StepStatus::Succeeded,
                    attempts: 1,
                    attempt_id: Some(format!("attempt-{}", step.id)),
                    output: Some("done".into()),
                    error: None,
                    skip_reason: None,
                    phase_epoch: Some(1),
                    validation_receipts: Vec::new(),
                },
            );
        }
        state
    }

    fn request(id: &str) -> ReworkRequest {
        ReworkRequest {
            step_id: id.into(),
            instruction: format!("fix {id}"),
            note: "review".into(),
            actor: "reviewer".into(),
            source_id: Some("review-1".into()),
            expected_attempt_id: Some(format!("attempt-{id}")),
            issue_ids: Vec::new(),
        }
    }

    #[test]
    fn dependent_owners_are_validated_together_and_reset_once() {
        let mut state = state();
        let ReworkPreparation::Ready(prepared) =
            prepare_reworks(&state, &[request("a"), request("b")]).unwrap()
        else {
            panic!("new batch must be ready")
        };
        let affected = apply_prepared_reworks(&mut state, &prepared);
        assert_eq!(affected, vec!["a", "b", "review"]);
        assert_eq!(state.records["sibling"].status, StepStatus::Succeeded);
        assert_eq!(
            state.records["sibling"].attempt_id.as_deref(),
            Some("attempt-sibling")
        );
        for id in ["a", "b"] {
            assert_eq!(state.records[id].status, StepStatus::Pending);
            assert!(
                state.records[id].attempt_id.is_none() && state.records[id].phase_epoch.is_none()
            );
            let step = state.plan.steps.iter().find(|step| step.id == id).unwrap();
            assert_eq!(step.input["rework"]["attempt"], 1);
            assert_eq!(step.input["objective"], id);
            assert_eq!(step.input["write_paths"], json!([format!("{id}.rs")]));
        }
    }

    #[test]
    fn stale_second_owner_rejects_without_mutation() {
        let state = state();
        let before = serde_json::to_value(&state).unwrap();
        let mut stale = request("b");
        stale.expected_attempt_id = Some("old".into());
        assert!(prepare_reworks(&state, &[request("a"), stale]).is_err());
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }

    #[test]
    fn replay_does_not_consume_another_attempt_and_changed_instruction_conflicts() {
        let mut state = state();
        let requests = vec![request("a"), request("b")];
        let ReworkPreparation::Ready(prepared) = prepare_reworks(&state, &requests).unwrap() else {
            panic!()
        };
        apply_prepared_reworks(&mut state, &prepared);
        assert!(matches!(
            prepare_reworks(&state, &requests).unwrap(),
            ReworkPreparation::AlreadyApplied
        ));
        let mut changed = requests.clone();
        changed[0].instruction = "different".into();
        assert!(prepare_reworks(&state, &changed).is_err());
    }

    #[test]
    fn missing_input_or_exhausted_budget_prevents_silent_rework() {
        let mut state = state();
        state.plan.steps[0].input = Value::Null;
        assert!(prepare_reworks(&state, &[request("a")]).is_err());
        state.plan.steps[0].input = json!({"rework":{"attempt":2}});
        assert!(prepare_reworks(&state, &[request("a")]).is_err());
    }

    #[test]
    fn partial_replay_or_duplicate_owner_never_applies_a_partial_batch() {
        let mut state = state();
        state.plan.steps[0].input["rework"] = json!({"source_id":"review-1","instruction":"fix a"});
        assert!(prepare_reworks(&state, &[request("a"), request("b")]).is_err());
        assert!(prepare_reworks(&state, &[request("b"), request("b")]).is_err());
    }
    #[test]
    fn host_validator_uses_the_same_per_owner_budget() {
        let mut state = state();
        state.plan.steps[0].input = json!({"rework":{"attempt":2}});
        let mut req = request("a");
        req.actor = "host_validator".into();
        req.issue_ids.clear();
        assert!(prepare_reworks(&state, &[req]).is_err());
    }
    #[test]
    fn automatic_batch_cannot_exceed_shared_retry_budget() {
        let mut state = state();
        state.goal.budget.max_total_retries = 1;
        let before = serde_json::to_value(&state).unwrap();
        assert!(prepare_reworks(&state, &[request("a"), request("b")]).is_err());
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        state.goal.budget.max_total_retries = 3;
        state.total_retries = 2;
        state.plan.steps[1].input["rework"] = json!({"attempt":1});
        assert!(prepare_reworks(&state, &[request("a")]).is_err());
    }
}
