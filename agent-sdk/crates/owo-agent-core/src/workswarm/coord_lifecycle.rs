use super::*;

fn failed_run_completion_status(
    previous: Option<owo_agent_protocol::CompletionStatusV1>,
) -> owo_agent_protocol::CompletionStatusV1 {
    previous
        .filter(|status| {
            matches!(
                status,
                owo_agent_protocol::CompletionStatusV1::Candidate
                    | owo_agent_protocol::CompletionStatusV1::Unverified
            )
        })
        .unwrap_or(owo_agent_protocol::CompletionStatusV1::Blocked)
}

fn failed_run_candidate_version_sha256(
    previous: Option<&owo_agent_protocol::TaskCompletionRecordV1>,
) -> Option<String> {
    previous.and_then(|record| record.candidate_version_sha256.clone())
}

/// Evidence produced by the gate for this exact accepted candidate. Keep the
/// workspace snapshot typed and transfer it together with the artifact receipts;
/// callers must not reconstruct it from mutable state after validation.
pub(super) struct DeliveryGateAcceptance {
    pub(super) receipts: Vec<Value>,
    pub(super) workspace_receipts: Vec<(String, std::collections::HashMap<String, String>)>,
}

fn team_candidate_version_sha256(
    accepted_artifacts: &[(String, String, String)],
    final_workspace_receipts: &[(String, std::collections::HashMap<String, String>)],
) -> Result<String, String> {
    let mut artifacts = accepted_artifacts.to_vec();
    artifacts.sort();

    let mut workspace_paths = std::collections::BTreeMap::new();
    for (_, subjects) in final_workspace_receipts {
        for (subject, hash) in subjects {
            let Some(path) = subject.strip_prefix("workspace-path:") else {
                continue;
            };
            let path = path.replace('\\', "/");
            if path.is_empty() {
                return Err("候选版本收据包含空工作区路径".to_string());
            }
            if let Some(previous) = workspace_paths.get(&path) {
                if previous != hash {
                    return Err(format!("候选版本的工作区路径摘要冲突：{path}"));
                }
            } else {
                workspace_paths.insert(path, hash.clone());
            }
        }
    }

    let snapshot = serde_json::json!({
        "schema": "team-candidate-version-v2",
        "accepted_artifacts": artifacts,
        "workspace_paths": workspace_paths,
    });
    crate::completion::hash_candidate_version(&snapshot).map_err(|error| error.to_string())
}

pub(super) fn validate_independent_review_coverage(
    has_code_changes: bool,
    reviewer_step_count: usize,
) -> Result<bool, String> {
    if has_code_changes && reviewer_step_count == 0 {
        return Err(
            "源码候选缺少独立 Reviewer，不能通过交付门；请重新运行包含只读 Reviewer 的任务图"
                .to_string(),
        );
    }
    Ok(has_code_changes)
}

impl TeamCoordinator {
    /// 失败收尾（team → Failed；产物保留；成员 Degraded）。
    pub(crate) async fn fail_run_internal(
        &self,
        team_id: &str,
        team: &mut TeamRun,
        state: &mut GoalRunState,
        reason: &str,
    ) -> WorkSwarmResult<()> {
        if state.goal.status == GoalStatus::Succeeded {
            // A host validator may reject the candidate after GoalRunner completes but
            // before DeliveryGate commits the TeamRun; this is a valid Succeeded → Failed
            // delivery transition because no user-facing delivery has been committed.
            state.goal.status = GoalStatus::Failed;
        } else if !state.goal.status.is_terminal() || state.goal.status == GoalStatus::Aborted {
            state.goal.transition(GoalStatus::Failed);
        }
        state.goal.error = Some(reason.to_string());
        let evidence_receipt_ids = state
            .validation_receipts
            .iter()
            .chain(
                state
                    .records
                    .values()
                    .flat_map(|record| record.validation_receipts.iter()),
            )
            .map(|receipt| receipt.receipt_id.clone())
            .collect::<Vec<_>>();
        let previous_completion_record = state.completion_record.as_ref();
        let completion_status =
            failed_run_completion_status(previous_completion_record.map(|record| record.status));
        let candidate_version_sha256 =
            failed_run_candidate_version_sha256(previous_completion_record);
        state.completion_record = Some(crate::completion::build_completion_record(
            team_id,
            &state.run_id,
            completion_status,
            evidence_receipt_ids,
            candidate_version_sha256,
        ));
        self.persist_state(state)?;
        let failed_members: Vec<String> = state
            .records
            .values()
            .filter(|r| matches!(r.status, StepStatus::Failed | StepStatus::Aborted))
            .filter_map(|r| {
                state
                    .plan
                    .steps
                    .iter()
                    .find(|s| s.id == r.step_id)
                    .map(|s| s.worker.clone())
            })
            .collect();
        team.status = TeamRunStatus::Failed;
        team.updated_at = now_ts();
        for m in &mut team.members {
            if failed_members.contains(&m.member_id) {
                m.health = MemberHealth::Degraded;
            }
        }
        self.store.save_team_run(team).await?;
        self.space_activity(team_id, &format!("team.failed：{reason}（已完成产物保留）"))
            .await?;
        self.audit(team_id, "team.failed", format!("失败：{reason}"));
        // 终态落定：过期中断标记不再有意义。
        self.clear_interrupted_marker(team_id);
        Ok(())
    }

    /// 取消收尾（team → Cancelled；未完成步骤 Aborted；已完成产物保留）。
    pub(crate) async fn cancel_run_internal(
        &self,
        team_id: &str,
        team: &mut TeamRun,
        state: &mut GoalRunState,
        reason: &str,
    ) -> WorkSwarmResult<()> {
        match super::run_state::apply_run_execution_event(
            state,
            super::run_state::RunExecutionEvent::CancelRun,
        ) {
            Ok(super::run_state::RunExecutionEffect::RunCancelled) => {}
            _ => unreachable!("cancel must produce the matching run-state effect"),
        }
        let evidence_receipt_ids = state
            .validation_receipts
            .iter()
            .chain(
                state
                    .records
                    .values()
                    .flat_map(|record| record.validation_receipts.iter()),
            )
            .map(|receipt| receipt.receipt_id.clone())
            .collect::<Vec<_>>();
        state.completion_record = Some(crate::completion::build_completion_record(
            team_id,
            &state.run_id,
            crate::completion::decide_completion(crate::completion::CompletionEvidence {
                aborted: true,
                ..crate::completion::CompletionEvidence::default()
            }),
            evidence_receipt_ids,
            None,
        ));
        self.persist_state(state)?;
        team.status = TeamRunStatus::Cancelled;
        team.updated_at = now_ts();
        self.store.save_team_run(team).await?;
        self.space_activity(
            team_id,
            &format!("team.cancelled：{reason}（已完成产物保留）"),
        )
        .await?;
        // 十期·四路 R5：取消审计明确写入 Provider 计费限制——客户端取消（置位
        // 取消令牌/abort 标志、断开流、终止子进程）只能停止**我方发起**的后续
        // 请求与执行；对云端 Provider 的**已在途请求**，我方无法证明其对账侧
        // 已停止计费（不同 Provider 的结算粒度/停账语义各异），因此**不得宣称**
        // 「继续计费为 0」。可证明为零的只有由本进程全程掌控计数的离线/脚本化
        // Provider（详见 eval 执行器）。此限制同样适用于 gateway 的流式响应丢弃。
        self.audit(
            team_id,
            "team.cancelled",
            format!(
                "取消：{reason}（完成后快照/变更登记已在协作收尾中完成；\
                 Provider 在途请求计费停止无法由客户端证明，不以「继续计费为 0」宣称）"
            ),
        );
        // 终态落定：过期中断标记不再有意义。
        self.clear_interrupted_marker(team_id);
        Ok(())
    }

    pub(crate) async fn space_activity(&self, team_id: &str, msg: &str) -> WorkSwarmResult<()> {
        let team = self.store.get_team_run(team_id).await?;
        let pid = team
            .project_space_id
            .clone()
            .ok_or_else(|| WorkSwarmError::Run(format!("团队 {team_id} 缺少 project_space_id")))?;
        let mut space = self.store.get_project_space(&pid).await?;
        space.activity_stream.push(msg.to_string());
        if space.activity_stream.len() > 200 {
            let drain = space.activity_stream.len() - 200;
            space.activity_stream.drain(..drain);
        }
        space.version += 1;
        space.updated_at = now_ts();
        self.store.save_project_space(&space).await?;
        Ok(())
    }

    // -- 收尾：交付清单 + 模板提案（§6.7：只提案，不自动启用） --

    /// Persist a host-side validation failure before delivery has been committed.
    ///
    /// This is the terminal counterpart to the success finalizer: it preserves candidate
    /// artifacts while ensuring a stopped coordinator cannot leave the TeamRun Running.
    pub async fn fail_delivery_validation(
        &self,
        team_id: &str,
        reason: &str,
    ) -> WorkSwarmResult<TeamRun> {
        if reason.trim().is_empty() {
            return Err(WorkSwarmError::Validation(
                "宿主验收失败原因不能为空".to_string(),
            ));
        }
        if self.is_run_active(team_id) {
            return Err(WorkSwarmError::Conflict(
                "TeamRun 仍有活动阶段，不能收尾宿主验收失败".to_string(),
            ));
        }
        let lock = self.team_lock(team_id);
        let _guard = lock.lock().await;
        if self.is_run_active(team_id) {
            return Err(WorkSwarmError::Conflict(
                "TeamRun 阶段已启动，不能收尾宿主验收失败".to_string(),
            ));
        }
        let (mut team, mut state) = {
            let (team, _space, state) = self.load_bundle(team_id).await?;
            (team, state)
        };
        if team.status == TeamRunStatus::Succeeded {
            return Err(WorkSwarmError::Conflict(
                "DeliveryGate 已提交成功交付，宿主验收不能事后撤销".to_string(),
            ));
        }
        if team.status.is_terminal() {
            return Ok(team);
        }
        self.fail_run_internal(team_id, &mut team, &mut state, reason)
            .await?;
        Ok(team)
    }

    /// 成功收尾：交付清单（CAS ref）+ ProjectSpace Completed + 模板提案。
    pub async fn finalize_success(&self, team_id: &str) -> WorkSwarmResult<TeamRun> {
        let lock = self.team_lock(team_id);
        let _guard = lock.lock().await;
        let repair_epoch = self.phase_epoch(team_id);
        let (mut team, mut space, mut state) = self.load_bundle(team_id).await?;
        if self.phase_epoch(team_id) != repair_epoch || self.cancel_token(team_id).is_cancelled() {
            return Err(WorkSwarmError::Conflict(
                "交付检查期间任务已取消或代次变化".into(),
            ));
        }
        let gate_result = self
            .validate_delivery_gate(team_id, &space, &mut state)
            .await;
        if self.phase_epoch(team_id) != repair_epoch || self.cancel_token(team_id).is_cancelled() {
            return Err(WorkSwarmError::Conflict(
                "交付验收期间任务已取消或代次变化".into(),
            ));
        }
        let accepted = match gate_result {
            Ok(receipts) => receipts,
            Err(error @ WorkSwarmError::DeliveryPending(_)) => {
                let reason = format!("delivery_pending:changeset:{error}");
                state.goal.transition(GoalStatus::Verifying);
                state.goal.error = Some(reason);
                self.persist_state(&state)?;
                team.status = TeamRunStatus::AwaitingHuman;
                team.updated_at = now_ts();
                self.store.save_team_run(&team).await?;
                self.audit(
                    team_id,
                    "team.delivery_awaiting_human",
                    format!("交付门等待 ChangeSet 人工处理：{error}"),
                );
                return Err(error);
            }
            Err(error @ WorkSwarmError::DeliveryRepair { .. }) => {
                let (step_id, attempt_id, validator_id, reason) = match &error {
                    WorkSwarmError::DeliveryRepair {
                        step_id,
                        attempt_id,
                        validator_id,
                        reason,
                    } => (
                        step_id.clone(),
                        attempt_id.clone(),
                        validator_id.clone(),
                        reason.clone(),
                    ),
                    _ => unreachable!(),
                };
                self.persist_state(&state)?;
                drop(_guard);
                let request = super::coord_rework::ReworkRequest {
                    step_id: step_id.clone(),
                    instruction: reason.clone(),
                    note: "宿主交付检查返工".into(),
                    actor: "host_validator".into(),
                    source_id: Some(format!("gate:{step_id}:{attempt_id}:{validator_id}")),
                    expected_attempt_id: Some(attempt_id),
                    issue_ids: Vec::new(),
                };
                match self
                    .rework_batch(team_id, &[request], Some(repair_epoch))
                    .await
                {
                    Ok(_) => {
                        self.audit(
                            team_id,
                            "team.delivery_rework_dispatched",
                            format!("任务 {step_id}：{reason}"),
                        );
                        return Err(error);
                    }
                    Err(failure) => {
                        let lock = self.team_lock(team_id);
                        let _guard = lock.lock().await;
                        if self.phase_epoch(team_id) != repair_epoch
                            || self.cancel_token(team_id).is_cancelled()
                        {
                            return Err(failure);
                        }
                        let (mut team, _, mut state) = self.load_bundle(team_id).await?;
                        self.fail_run_internal(
                            team_id,
                            &mut team,
                            &mut state,
                            &format!("交付返工无法继续：{failure}；{reason}"),
                        )
                        .await?;
                        return Err(failure);
                    }
                }
            }
            Err(error) => {
                let reason = format!("交付验收未通过：{error}");
                self.fail_run_internal(team_id, &mut team, &mut state, &reason)
                    .await?;
                return Err(error);
            }
        };
        let DeliveryGateAcceptance {
            receipts: acceptance_receipts,
            workspace_receipts: final_workspace_receipts,
        } = accepted;
        let evidence_receipt_ids = acceptance_receipts
            .iter()
            .filter_map(|item| item.get("validation_receipts").and_then(Value::as_array))
            .flatten()
            .filter_map(|receipt| receipt.get("receipt_id").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut candidate_versions = Vec::<(String, String, String)>::new();
        for item in &acceptance_receipts {
            let artifact_id = item
                .get("artifact_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    WorkSwarmError::Conflict("Accepted 收据缺少 artifact_id".to_string())
                })?;
            let attempt_id = item
                .get("attempt_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    WorkSwarmError::Conflict("Accepted 收据缺少 attempt_id".to_string())
                })?;
            let content_sha256 = item
                .get("content_sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    WorkSwarmError::Conflict("Accepted 收据缺少候选内容 SHA-256".to_string())
                })?;
            candidate_versions.push((
                artifact_id.to_string(),
                attempt_id.to_string(),
                content_sha256.to_string(),
            ));
        }
        let candidate_version_sha256 =
            team_candidate_version_sha256(&candidate_versions, &final_workspace_receipts)
                .map_err(WorkSwarmError::Conflict)?;
        let completion_record = crate::completion::build_completion_record(
            team_id,
            &state.run_id,
            owo_agent_protocol::CompletionStatusV1::Accepted,
            evidence_receipt_ids,
            Some(candidate_version_sha256),
        );
        // ValidationReceipt 与失败/成功的 task state 一起落盘；若后续发布清单失败，
        // 也不能丢失刚刚执行过的宿主验收证据。
        self.persist_state(&state)?;

        // 解析 ProjectSpace 的全部引用以拒绝悬空/跨团队索引；最终清单只发布通过
        // 本次任务验收的产物，返工旧版和未验收的附加产物继续保留在工作空间历史中。
        let accepted_artifact_ids: HashSet<String> = acceptance_receipts
            .iter()
            .filter_map(|receipt| receipt.get("artifact_id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        let delivery_artifact_ids: HashSet<String> = acceptance_receipts
            .iter()
            .filter(|receipt| {
                receipt
                    .get("artifact_kind")
                    .and_then(Value::as_str)
                    .is_some_and(super::delivery_gate_evidence::is_user_delivery_artifact)
            })
            .filter_map(|receipt| receipt.get("artifact_id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        let mut indexed_accepted_artifact_ids = HashSet::new();
        let mut published_artifact_ids = HashSet::new();
        let mut final_artifacts: Vec<Value> = Vec::new();
        for id in &space.artifacts {
            let artifact = self.store.get_artifact(id).await?;
            if artifact.team_id != team_id {
                return Err(WorkSwarmError::Conflict(format!(
                    "交付产物 {} 不属于当前团队 {}",
                    artifact.artifact_id, team_id
                )));
            }
            if accepted_artifact_ids.contains(id) {
                indexed_accepted_artifact_ids.insert(id.clone());
                if delivery_artifact_ids.contains(id) {
                    published_artifact_ids.insert(id.clone());
                    final_artifacts.push(json!({
                        "artifact_id": artifact.artifact_id,
                        "kind": artifact.kind,
                        "version": artifact.version,
                        "content_ref": artifact.content_ref,
                        "producer": artifact.producer,
                    }));
                }
            }
        }
        if indexed_accepted_artifact_ids != accepted_artifact_ids
            || published_artifact_ids != delivery_artifact_ids
        {
            return Err(WorkSwarmError::Conflict(
                "验收收据与 ProjectSpace 产物索引不一致，不能发布交付清单".to_string(),
            ));
        }
        let manifest = json!({
            "team_id": team_id,
            "objective": state.goal.objective,
            "artifacts": final_artifacts,
            "acceptance_receipts": acceptance_receipts,
            "delivery_issues": &state.delivery_issues,
            "completion_record": &completion_record,
            "created_at": now_ts(),
        });
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        let manifest_hash = self
            .cas
            .put(&manifest_bytes)
            .map_err(|e| WorkSwarmError::Run(format!("交付清单 CAS 落盘失败：{e}")))?;

        // Artifact reads above can yield while Cancel invalidates this generation.
        if self.phase_epoch(team_id) != repair_epoch || self.cancel_token(team_id).is_cancelled() {
            return Err(WorkSwarmError::Conflict(
                "交付提交前任务已取消或代次变化".into(),
            ));
        }
        let mut succeeded_state = state.clone();
        succeeded_state.completion_record = Some(completion_record);
        succeeded_state.goal.transition(GoalStatus::Succeeded);
        self.persist_state(&succeeded_state)?;
        team.status = TeamRunStatus::Succeeded;
        team.updated_at = now_ts();

        space.status = ProjectSpaceStatus::Completed;
        space.delivery_manifest_ref = Some(format!("cas://sha256:{manifest_hash}"));
        space.version += 1;
        space.updated_at = now_ts();
        space.activity_stream.push(format!(
            "{} team.succeeded：交付 {} 项产物",
            now_ts(),
            final_artifacts.len()
        ));
        if let Err(error) = self.store.commit_team_delivery(&team, &space).await {
            self.persist_state(&state)?;
            return Err(error.into());
        }
        state = succeeded_state;

        // 模板提案是成功交付后的可选沉淀；不能让提案存储故障把已提交的交付
        // 伪装成 finalize 失败。
        if team.mode != TeamMode::Single {
            let meta = RunMeta::load(&self.run_dir, team_id)?;
            let proposal =
                self.build_template_proposal(team_id, &team, &state, &meta, &final_artifacts);
            match self.templates.save_proposal(&proposal) {
                Ok(()) => {
                    let _ = self
                        .space_activity(
                            team_id,
                            &format!(
                                "template.proposed：{}（只提案，未自动启用；采纳后进入模板注册表）",
                                proposal.proposal_id
                            ),
                        )
                        .await;
                    self.audit(
                        team_id,
                        "team.template_proposed",
                        format!("模板提案 {}（来源运行 {}）", proposal.proposal_id, team_id),
                    );
                }
                Err(error) => self.audit(
                    team_id,
                    "team.template_proposal_failed",
                    format!(
                        "交付已成功；模板提案 {} 落盘失败：{error}",
                        proposal.proposal_id
                    ),
                ),
            }
        }
        self.audit(
            team_id,
            "team.succeeded",
            format!("目标达成：{}", state.goal.objective),
        );
        // 终态落定：过期中断标记不再有意义。
        self.clear_interrupted_marker(team_id);
        Ok(team)
    }

    pub(crate) fn build_template_proposal(
        &self,
        team_id: &str,
        team: &TeamRun,
        state: &GoalRunState,
        meta: &RunMeta,
        final_artifacts: &[Value],
    ) -> TeamTemplateProposal {
        let roles: Vec<TeamTemplateRole> = meta
            .roles
            .iter()
            .map(|r| TeamTemplateRole {
                role: r.role.clone(),
                assignee: r.assignee.clone(),
                worker: r.worker.clone(),
                depends_on: r.depends_on.clone(),
                handoff_contract: r.handoff_contract.clone(),
                verify: r.verify.clone(),
                model: r.model.clone(),
                write_paths: r.write_paths.clone(),
                capabilities: r.capabilities.clone(),
                tools: Vec::new(),
            })
            .collect();
        let template = TeamTemplate {
            template_id: format!("tpl-{team_id}"),
            name: preview(&state.goal.objective, 60),
            mode: team.mode,
            roles,
            applicability: preview(&state.goal.objective, 200),
            source_team_id: Some(team_id.to_string()),
            created_at: now_ts(),
            origin: None,
            description: None,
            version: None,
            tags: Vec::new(),
        };
        TeamTemplateProposal {
            proposal_id: format!("prop-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            template,
            source_team_id: team_id.to_string(),
            evidence: final_artifacts
                .iter()
                .filter_map(|a| {
                    a.get("artifact_id")
                        .and_then(Value::as_str)
                        .map(String::from)
                })
                .collect(),
            status: TeamTemplateProposalStatus::Proposed,
            created_at: now_ts(),
        }
    }

    // -- 产物注册 / 接力（A3：handoff 使用结构化 context slice） --
}

#[cfg(test)]
mod validation_receipt_identity_tests {
    use super::super::delivery_gate_evidence::{
        collect_attempt_changeset_evidence, make_validation_receipt, ValidationReceiptInput,
    };
    use crate::plan::{ValidationVerdictV1, VerificationScopeV1};
    use crate::workswarm::delivery_gate_evidence::{
        attempt_changeset_contains_code, is_source_code_path, uncovered_source_paths,
    };

    const STEP_SCOPE: VerificationScopeV1 = VerificationScopeV1::StepOutput;
    const MANUAL_SCOPE: VerificationScopeV1 = VerificationScopeV1::Manual;

    pub(super) fn changeset(
        id: &str,
        status: owo_agent_protocol::ChangeSetStatus,
        created_at: &str,
    ) -> owo_agent_protocol::ChangeSet {
        owo_agent_protocol::ChangeSet {
            change_set_id: id.to_string(),
            team_id: "team-1".to_string(),
            step_id: "task-1".to_string(),
            attempt_id: Some("attempt-1".to_string()),
            role: "builder".to_string(),
            base_hashes: Vec::new(),
            result_hashes: Vec::new(),
            changed_files: vec![format!("src/{id}.rs")],
            diff_ref: None,
            status,
            created_at: created_at.to_string(),
            decision: (status == owo_agent_protocol::ChangeSetStatus::Accepted).then(|| {
                owo_agent_protocol::ChangeSetDecision {
                    action: "accept".to_string(),
                    idempotency_key: format!("accept-{id}"),
                    decided_at: created_at.to_string(),
                    note: None,
                }
            }),
            conflicts: Vec::new(),
        }
    }

    #[test]
    fn final_delivery_recheck_requires_the_bound_workspace_snapshot() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src")).unwrap();
        let source = workspace.path().join("src/lib.rs");
        std::fs::write(&source, "pub fn ready() {}\n").unwrap();
        let subjects = std::collections::HashMap::from([(
            "workspace-path:src/lib.rs".to_string(),
            crate::CasStore::hash_of(b"pub fn ready() {}\n"),
        )]);
        let matches = |root: Option<&std::path::Path>| {
            crate::workspace_snapshot::WorkspaceSnapshotBatch::new(root).subjects_match(&subjects)
        };
        assert!(matches(Some(workspace.path())));
        assert!(!matches(None));

        std::fs::write(&source, "pub fn changed() {}\n").unwrap();
        assert!(!matches(Some(workspace.path())));
    }

    #[test]
    fn behavior_validation_scope_must_cover_every_changed_source_path() {
        use crate::plan::{
            VerificationRequirementV1, VerificationResourcesV1, VerificationScopeV1,
        };
        use owo_agent_protocol::{ChangeSetFileHash, ChangeSetStatus};

        let mut change_set = changeset("cs-source", ChangeSetStatus::Accepted, "2026-10-04");
        change_set.changed_files = vec!["src/lib.rs".to_string(), "src/api.rs".to_string()];
        change_set.result_hashes = change_set
            .changed_files
            .iter()
            .map(|path| ChangeSetFileHash {
                path: path.clone(),
                sha256: Some("final-hash".to_string()),
                content_available: false,
            })
            .collect();
        let requirement = |paths: Vec<String>| VerificationRequirementV1 {
            requirement_id: "source-behavior".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-command-success-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: VerificationScopeV1::WorkspacePaths {
                relative_paths: paths,
            },
            arguments: serde_json::json!({"command":"cargo test -p owo-agent-core"}),
            required: true,
            resources: VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 8,
                exclusive_workspace: false,
                timeout_ms: 30_000,
            },
        };
        let plan = crate::plan::VerificationPlanV1 {
            plan_id: "source-plan".to_string(),
            requirements: vec![requirement(vec!["src/lib.rs".to_string()])],
        };
        assert_eq!(
            uncovered_source_paths(
                std::slice::from_ref(&change_set),
                "team-1",
                "task-1",
                "attempt-1",
                &plan,
            ),
            vec!["src/api.rs".to_string()]
        );
        let covered = crate::plan::VerificationPlanV1 {
            plan_id: "source-plan".to_string(),
            requirements: vec![requirement(vec![
                "src/lib.rs".to_string(),
                "src/api.rs".to_string(),
            ])],
        };
        assert!(uncovered_source_paths(
            std::slice::from_ref(&change_set),
            "team-1",
            "task-1",
            "attempt-1",
            &covered,
        )
        .is_empty());
    }

    #[test]
    fn host_classifies_source_from_paths_and_exact_attempt_not_artifact_label() {
        assert!(is_source_code_path("src/lib.rs"));
        assert!(is_source_code_path("apps/web/src/App.tsx"));
        assert!(is_source_code_path("package.json"));
        assert!(is_source_code_path("Sources/AppDelegate.m"));
        assert!(is_source_code_path("infra/main.tf"));
        assert!(is_source_code_path("CMakeLists.txt"));
        assert!(is_source_code_path("app/Example.csproj"));
        assert!(!is_source_code_path("docs/design.md"));
        assert!(!is_source_code_path("src"));

        let mut source_change = changeset(
            "cs-source",
            owo_agent_protocol::ChangeSetStatus::Accepted,
            "2026-10-04",
        );
        source_change.changed_files = vec!["src/lib.rs".to_string()];
        assert!(attempt_changeset_contains_code(
            std::slice::from_ref(&source_change),
            "team-1",
            "task-1",
            "attempt-1",
        ));
        assert!(!attempt_changeset_contains_code(
            std::slice::from_ref(&source_change),
            "team-1",
            "task-1",
            "attempt-stale",
        ));
        assert!(!attempt_changeset_contains_code(
            std::slice::from_ref(&source_change),
            "other-team",
            "task-1",
            "attempt-1",
        ));
    }

    fn input() -> ValidationReceiptInput<'static> {
        ValidationReceiptInput {
            team_id: "team-1",
            step_id: "task-1",
            attempts: 1,
            attempt_id: "attempt-1",
            epoch: 2,
            requirement_id: "req-1",
            scope: &STEP_SCOPE,
            validator_id: "artifact-output-non-empty-v1",
            validator_version: "1",
            arguments_sha256: "args",
            input_sha256: "input",
            artifact_id: "artifact-a",
            artifact_sha256: "same-content",
            changeset_sha256: Some("changeset-a".to_string()),
            changeset_refs: vec!["changeset://a".to_string()],
            evidence_ref: "cas://sha256:same-content",
            started_at: "2026-10-03T00:00:00Z",
            verdict: ValidationVerdictV1::Passed,
            detail: None,
            subject_hashes: std::collections::BTreeMap::new(),
            additional_evidence_refs: Vec::new(),
        }
    }

    #[test]
    fn review_artifacts_are_evidence_but_not_user_delivery_items() {
        let is_delivery = super::super::delivery_gate_evidence::is_user_delivery_artifact;
        assert!(is_delivery("code"));
        assert!(is_delivery("markdown"));
        assert!(!is_delivery("review"));
    }

    #[test]
    fn reviewer_cannot_pass_with_a_non_review_artifact() {
        let validate = super::super::delivery_gate_evidence::validate_review_artifact_kind;
        assert!(validate(true, "review").is_ok());
        assert!(validate(false, "code").is_ok());
        assert!(validate(true, "markdown")
            .unwrap_err()
            .contains("必须提交 review"));
        assert!(validate(false, "review")
            .unwrap_err()
            .contains("只有独立 reviewer"));
    }

    #[test]
    fn approved_review_cannot_hide_blockers_or_malformed_findings() {
        let approved = serde_json::json!({"verdict":"approved", "findings":[]});
        assert!(super::super::delivery_gate_evidence::validate_review_approval(&approved).is_ok());

        let hidden_blocker = serde_json::json!({
            "verdict":"approved",
            "findings":[{"severity":"blocker", "detail":"authentication bypass", "evidence_refs":[]}]
        });
        assert!(
            super::super::delivery_gate_evidence::validate_review_approval(&hidden_blocker)
                .unwrap_err()
                .contains("blocker")
        );

        let hidden_major = serde_json::json!({
            "verdict":"approved",
            "findings":[{"severity":"major", "detail":"required behavior is missing", "evidence_refs":[]}]
        });
        assert!(
            super::super::delivery_gate_evidence::validate_review_approval(&hidden_major)
                .unwrap_err()
                .contains("major")
        );

        let malformed = serde_json::json!({
            "verdict":"approved",
            "findings":[{"severity":"urgent", "detail":"unknown severity"}]
        });
        assert!(
            super::super::delivery_gate_evidence::validate_review_approval(&malformed).is_err()
        );

        let changes_requested = serde_json::json!({"verdict":"changes_requested", "findings":[]});
        assert!(
            super::super::delivery_gate_evidence::validate_review_approval(&changes_requested)
                .unwrap_err()
                .contains("未批准")
        );
    }

    #[test]
    fn delivery_requires_accepted_changesets() {
        use super::super::delivery_gate_evidence::changeset_delivery_error;
        use owo_agent_protocol::ChangeSetStatus;

        assert_eq!(changeset_delivery_error(ChangeSetStatus::Accepted), None);
        assert!(changeset_delivery_error(ChangeSetStatus::PendingReview).is_some());
        assert!(changeset_delivery_error(ChangeSetStatus::Conflicted).is_some());
        assert!(changeset_delivery_error(ChangeSetStatus::Rejected).is_some());
        assert!(changeset_delivery_error(ChangeSetStatus::Reverted).is_some());
    }

    #[test]
    fn changeset_evidence_checks_every_record_and_is_order_independent() {
        use owo_agent_protocol::ChangeSetStatus;

        let pending = changeset("cs-old", ChangeSetStatus::PendingReview, "2026-10-02");
        let accepted = changeset("cs-new", ChangeSetStatus::Accepted, "2026-10-03");
        let error = collect_attempt_changeset_evidence(
            "team-1",
            "task-1",
            "attempt-1",
            &[pending.clone(), accepted.clone()],
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("cs-old") && error.to_string().contains("仍待人工接受"),
            "{error}"
        );

        let mut accepted_without_decision = accepted.clone();
        accepted_without_decision.decision = None;
        let error = collect_attempt_changeset_evidence(
            "team-1",
            "task-1",
            "attempt-1",
            &[accepted_without_decision],
        )
        .unwrap_err();
        assert!(error.to_string().contains("缺少宿主接受决定"), "{error}");

        let accepted_old = changeset("cs-old", ChangeSetStatus::Accepted, "2026-10-02");
        let forward = collect_attempt_changeset_evidence(
            "team-1",
            "task-1",
            "attempt-1",
            &[accepted_old.clone(), accepted.clone()],
        )
        .unwrap();
        let reversed = collect_attempt_changeset_evidence(
            "team-1",
            "task-1",
            "attempt-1",
            &[accepted, accepted_old],
        )
        .unwrap();
        assert_eq!(forward, reversed);
        assert_eq!(forward.1, vec!["changeset://cs-new", "changeset://cs-old"]);
    }

    #[test]
    fn workspace_receipts_must_match_the_accepted_changeset_result_hash() {
        use owo_agent_protocol::{ChangeSetFileHash, ChangeSetStatus};

        let mut accepted = changeset("cs-source", ChangeSetStatus::Accepted, "2026-10-03");
        accepted.changed_files = vec!["src\\lib.rs".to_string()];
        accepted.result_hashes = vec![ChangeSetFileHash {
            path: "src/lib.rs".to_string(),
            sha256: Some("final-source-hash".to_string()),
            content_available: false,
        }];
        let scope = vec!["src\\lib.rs".to_string()];
        let evidence = std::collections::BTreeMap::from([(
            "workspace-path:src\\lib.rs".to_string(),
            "final-source-hash".to_string(),
        )]);
        let validate = super::super::delivery_gate_evidence::validate_workspace_receipt_snapshot;
        assert!(validate(
            "team-1",
            "task-1",
            "attempt-1",
            &scope,
            &evidence,
            &[accepted.clone()]
        )
        .is_ok());

        let stale_evidence = std::collections::BTreeMap::from([(
            "workspace-path:src\\lib.rs".to_string(),
            "stale-source-hash".to_string(),
        )]);
        assert!(validate(
            "team-1",
            "task-1",
            "attempt-1",
            &scope,
            &stale_evidence,
            &[accepted.clone()]
        )
        .unwrap_err()
        .contains("快照不一致"));

        assert!(validate(
            "other-team",
            "task-1",
            "attempt-1",
            &scope,
            &evidence,
            &[accepted]
        )
        .is_ok());
    }

    #[test]
    fn review_source_snapshot_binds_the_seen_workspace_and_stable_changeset_bytes() {
        use owo_agent_protocol::{ChangeSetFileHash, ChangeSetStatus};

        let workspace =
            std::env::temp_dir().join(format!("owo-review-snapshot-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(workspace.join("src")).expect("source directory");
        let source_path = workspace.join("src").join("task-1.rs");
        std::fs::write(&source_path, b"final source").expect("initial source");
        let mut accepted = changeset("cs-source", ChangeSetStatus::Accepted, "2026-10-03");
        accepted.changed_files = vec!["src/task-1.rs".to_string()];
        accepted.result_hashes = vec![ChangeSetFileHash {
            path: "src/task-1.rs".to_string(),
            sha256: Some(crate::CasStore::hash_of(b"final source")),
            content_available: false,
        }];

        let mut foreign_team_change = accepted.clone();
        foreign_team_change.team_id = "team-2".to_string();
        foreign_team_change.change_set_id = "cs-foreign-team".to_string();
        foreign_team_change.changed_files = vec!["src/foreign.rs".to_string()];
        let mixed_team_changes = [accepted.clone(), foreign_team_change];
        let accepted_snapshot = super::super::delivery_gate_evidence::review_source_snapshot(
            "team-1",
            "task-1",
            "attempt-1",
            &mixed_team_changes,
            Some(&workspace),
        );
        assert_eq!(
            accepted_snapshot
                .get("change_set_ids")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            accepted_snapshot
                .get("changeset_source_consistent")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            accepted_snapshot
                .get("contains_source_code")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let mut pending = accepted.clone();
        pending.status = ChangeSetStatus::PendingReview;
        pending.decision = None;
        let pending_snapshot = super::super::delivery_gate_evidence::review_source_snapshot(
            "team-1",
            "task-1",
            "attempt-1",
            &[pending],
            Some(&workspace),
        );
        assert_eq!(
            accepted_snapshot.get("change_set_sha256"),
            pending_snapshot.get("change_set_sha256"),
            "accept/review status transitions do not alter the reviewed source identity"
        );

        std::fs::write(&source_path, b"edited after review").expect("changed source");
        let stale_snapshot = super::super::delivery_gate_evidence::review_source_snapshot(
            "team-1",
            "task-1",
            "attempt-1",
            std::slice::from_ref(&accepted),
            Some(&workspace),
        );
        assert_ne!(
            accepted_snapshot.get("source_hashes"),
            stale_snapshot.get("source_hashes")
        );
        assert_eq!(
            stale_snapshot
                .get("changeset_source_consistent")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        std::fs::remove_dir_all(workspace).expect("remove temporary workspace");
    }

    #[test]
    fn behavior_receipts_require_approved_matching_attempt_and_final_source_hashes() {
        use crate::plan::{
            VerificationRequirementV1, VerificationResourcesV1, VerificationScopeV1,
        };
        use owo_agent_protocol::{ChangeSetFileHash, ChangeSetStatus};

        let mut accepted = changeset("cs-source", ChangeSetStatus::Accepted, "2026-10-03");
        accepted.changed_files = vec!["src/lib.rs".to_string()];
        accepted.result_hashes = vec![ChangeSetFileHash {
            path: "src/lib.rs".to_string(),
            sha256: Some("source-final".to_string()),
            content_available: false,
        }];
        let requirement = VerificationRequirementV1 {
            requirement_id: "task-1:behavior".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-command-success-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["src/lib.rs".to_string()],
            },
            arguments: serde_json::json!({"command":"cargo test -p owo-agent-core"}),
            required: true,
            resources: VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 8,
                exclusive_workspace: false,
                timeout_ms: 30_000,
            },
        };
        let command_hash = crate::CasStore::hash_of(b"cargo test -p owo-agent-core");
        let event = serde_json::json!({
            "step_id":"task-1",
            "attempt_id":"attempt-1",
            "receipt": {
                "command_sha256":command_hash,
                "exit_code":0,
                "result_sha256":"command-output-hash",
                "duration_ms":12,
                "workspace_hashes_complete":true,
                "validator_id":"workspace-command-success-v1",
                "validator_version":"1",
                "workspace_hashes_before":{"src/lib.rs":"source-final"},
                "workspace_hashes":{"src/lib.rs":"source-final"}
            }
        })
        .to_string();
        let evaluate = super::super::delivery_gate_evidence::evaluate_workspace_command_receipt;
        let mut foreign_team_change = accepted.clone();
        foreign_team_change.team_id = "team-2".to_string();
        foreign_team_change.change_set_id = "cs-foreign-team".to_string();
        foreign_team_change.result_hashes[0].sha256 = Some("other-team-hash".to_string());
        let mixed_team_changes = [accepted.clone(), foreign_team_change];
        let (verdict, _, subjects, output_ref) = evaluate(
            "team-1",
            &requirement,
            std::slice::from_ref(&event),
            "task-1",
            "attempt-1",
            &mixed_team_changes,
        );
        assert_eq!(verdict, ValidationVerdictV1::Passed);
        assert_eq!(
            subjects.get("workspace-path:src/lib.rs"),
            Some(&"source-final".to_string())
        );
        assert_eq!(
            output_ref.as_deref(),
            Some("command-result:sha256:command-output-hash")
        );

        let bad_validator = event.replace(
            "\"validator_id\":\"workspace-command-success-v1\"",
            "\"validator_id\":\"unregistered-validator\"",
        );
        let (verdict, _, _, _) = evaluate(
            "team-1",
            &requirement,
            &[bad_validator],
            "task-1",
            "attempt-1",
            std::slice::from_ref(&accepted),
        );
        assert_eq!(verdict, ValidationVerdictV1::Unverified);

        let over_budget = event.replace("\"duration_ms\":12", "\"duration_ms\":30001");
        let (verdict, detail, _, _) = evaluate(
            "team-1",
            &requirement,
            &[over_budget],
            "task-1",
            "attempt-1",
            &[accepted.clone()],
        );
        assert_eq!(verdict, ValidationVerdictV1::Failed);
        assert!(detail.unwrap().contains("超过验证计划预算"));

        let (verdict, _, _, _) = evaluate(
            "team-1",
            &requirement,
            std::slice::from_ref(&event),
            "task-1",
            "attempt-old",
            &[accepted.clone()],
        );
        assert_eq!(verdict, ValidationVerdictV1::Unverified);

        let stale = event.replace("source-final", "source-before");
        let (verdict, detail, _, _) = evaluate(
            "team-1",
            &requirement,
            &[stale],
            "task-1",
            "attempt-1",
            &[accepted],
        );
        assert_eq!(verdict, ValidationVerdictV1::Failed);
        assert!(detail.unwrap().contains("快照不一致"));

        let mut deleted = changeset("cs-deleted", ChangeSetStatus::Accepted, "2026-10-04");
        deleted.changed_files = vec!["src/lib.rs".to_string()];
        deleted.result_hashes = vec![ChangeSetFileHash {
            path: "src/lib.rs".to_string(),
            sha256: None,
            content_available: false,
        }];
        let deleted_event = serde_json::json!({
            "step_id":"task-1",
            "attempt_id":"attempt-1",
            "receipt": {
                "command_sha256":command_hash,
                "exit_code":0,
                "result_sha256":"deletion-command-output-hash",
                "duration_ms":12,
                "workspace_hashes_complete":true,
                "validator_id":"workspace-command-success-v1",
                "validator_version":"1",
                "workspace_hashes_before":{"src/lib.rs":null},
                "workspace_hashes":{"src/lib.rs":null}
            }
        })
        .to_string();
        let (verdict, _, subjects, _) = evaluate(
            "team-1",
            &requirement,
            &[deleted_event],
            "task-1",
            "attempt-1",
            &[deleted],
        );
        assert_eq!(verdict, ValidationVerdictV1::Passed);
        assert_eq!(
            subjects.get("workspace-path:src/lib.rs"),
            Some(&crate::verification::workspace_path_absence_sha256())
        );
    }

    #[test]
    fn receipt_identity_is_idempotent_but_separates_subject_and_scope() {
        let base_id = make_validation_receipt(input()).receipt_id;
        assert_eq!(base_id, make_validation_receipt(input()).receipt_id);

        let mut other_artifact = input();
        other_artifact.artifact_id = "artifact-b";
        assert_ne!(base_id, make_validation_receipt(other_artifact).receipt_id);

        let mut other_changeset = input();
        other_changeset.changeset_sha256 = Some("changeset-b".to_string());
        assert_ne!(base_id, make_validation_receipt(other_changeset).receipt_id);

        let mut other_scope = input();
        other_scope.scope = &MANUAL_SCOPE;
        assert_ne!(base_id, make_validation_receipt(other_scope).receipt_id);
    }
}

#[cfg(test)]
mod completion_status_tests {
    use super::validation_receipt_identity_tests::changeset;
    use super::{
        failed_run_candidate_version_sha256, failed_run_completion_status,
        team_candidate_version_sha256, validate_independent_review_coverage,
    };
    use owo_agent_protocol::CompletionStatusV1;

    #[test]
    fn source_changes_require_at_least_one_independent_reviewer() {
        assert_eq!(validate_independent_review_coverage(false, 0), Ok(false));
        assert_eq!(validate_independent_review_coverage(true, 1), Ok(true));
        assert!(validate_independent_review_coverage(true, 0)
            .unwrap_err()
            .contains("独立 Reviewer"));
    }

    #[test]
    fn team_candidate_version_binds_workspace_source_hashes_canonically() {
        let artifact_a = (
            "artifact-a".to_string(),
            "attempt-a".to_string(),
            "artifact-sha-a".to_string(),
        );
        let artifact_b = (
            "artifact-b".to_string(),
            "attempt-b".to_string(),
            "artifact-sha-b".to_string(),
        );
        let artifacts = vec![artifact_a.clone(), artifact_b.clone()];
        let reversed_artifacts = vec![artifact_b, artifact_a];
        let source_a = std::collections::HashMap::from([(
            "workspace-path:src/lib.rs".to_string(),
            "source-sha-a".to_string(),
        )]);
        let source_b = std::collections::HashMap::from([(
            "workspace-path:src/main.rs".to_string(),
            "source-sha-b".to_string(),
        )]);
        let source_b_windows = std::collections::HashMap::from([(
            r"workspace-path:src\main.rs".to_string(),
            "source-sha-b".to_string(),
        )]);
        let first_receipts = vec![
            ("receipt-a".to_string(), source_a.clone()),
            ("receipt-b".to_string(), source_b.clone()),
        ];
        let reordered_receipts = vec![
            ("receipt-b".to_string(), source_b_windows),
            ("receipt-a".to_string(), source_a),
        ];
        let changed_receipts = vec![
            (
                "receipt-a".to_string(),
                std::collections::HashMap::from([(
                    "workspace-path:src/lib.rs".to_string(),
                    "source-sha-changed".to_string(),
                )]),
            ),
            (
                "receipt-b".to_string(),
                std::collections::HashMap::from([(
                    "workspace-path:src/main.rs".to_string(),
                    "source-sha-b".to_string(),
                )]),
            ),
        ];

        let first = team_candidate_version_sha256(&artifacts, &first_receipts).unwrap();
        assert_eq!(
            first,
            team_candidate_version_sha256(&reversed_artifacts, &reordered_receipts).unwrap()
        );
        assert_ne!(
            first,
            team_candidate_version_sha256(&artifacts, &changed_receipts).unwrap()
        );
    }

    #[test]
    fn team_candidate_version_rejects_conflicting_workspace_hashes() {
        let receipts = vec![
            (
                "receipt-1".to_string(),
                std::collections::HashMap::from([(
                    "workspace-path:src/lib.rs".to_string(),
                    "source-sha-a".to_string(),
                )]),
            ),
            (
                "receipt-2".to_string(),
                std::collections::HashMap::from([(
                    "workspace-path:src/lib.rs".to_string(),
                    "source-sha-b".to_string(),
                )]),
            ),
        ];
        assert!(team_candidate_version_sha256(&[], &receipts)
            .unwrap_err()
            .contains("摘要冲突"));
    }

    #[test]
    fn failed_team_run_preserves_candidate_version_identity() {
        let record = owo_agent_protocol::TaskCompletionRecordV1 {
            task_id: "task-1".to_string(),
            attempt_id: "attempt-1".to_string(),
            status: CompletionStatusV1::Candidate,
            evidence_receipt_ids: Vec::new(),
            candidate_version_sha256: Some("candidate-sha256".to_string()),
            decided_at: "now".to_string(),
        };
        assert_eq!(
            failed_run_candidate_version_sha256(Some(&record)).as_deref(),
            Some("candidate-sha256")
        );
        let record_without_candidate = owo_agent_protocol::TaskCompletionRecordV1 {
            task_id: "task-1".to_string(),
            attempt_id: "attempt-1".to_string(),
            status: CompletionStatusV1::Candidate,
            evidence_receipt_ids: Vec::new(),
            candidate_version_sha256: None,
            decided_at: "now".to_string(),
        };
        assert_eq!(
            failed_run_candidate_version_sha256(Some(&record_without_candidate)),
            None
        );
    }

    #[test]
    fn failed_team_run_preserves_candidate_and_unverified_states_only() {
        assert_eq!(
            failed_run_completion_status(Some(CompletionStatusV1::Candidate)),
            CompletionStatusV1::Candidate
        );
        assert_eq!(
            failed_run_completion_status(Some(CompletionStatusV1::Unverified)),
            CompletionStatusV1::Unverified
        );
        assert_eq!(
            failed_run_completion_status(Some(CompletionStatusV1::Accepted)),
            CompletionStatusV1::Blocked
        );
        assert_eq!(
            failed_run_completion_status(Some(CompletionStatusV1::Blocked)),
            CompletionStatusV1::Blocked
        );
        assert_eq!(
            failed_run_completion_status(None),
            CompletionStatusV1::Blocked
        );
    }

    #[test]
    fn review_source_snapshot_refuses_oversize_file_identity_and_refreshes_per_operation() {
        use owo_agent_protocol::{ChangeSetFileHash, ChangeSetStatus};
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("result.md");
        let mut change = changeset("cs-bounded", ChangeSetStatus::Accepted, "now");
        change.changed_files = vec!["result.md".into()];
        change.result_hashes = vec![ChangeSetFileHash {
            path: "result.md".into(),
            sha256: Some(crate::CasStore::hash_of(b"ready")),
            content_available: false,
        }];
        std::fs::write(&source, b"ready").unwrap();
        let first = super::super::delivery_gate_evidence::review_source_snapshot(
            "team-1",
            "task-1",
            "attempt-1",
            &[change.clone()],
            Some(root.path()),
        );
        assert_eq!(first["changeset_source_consistent"], true);
        std::fs::File::create(&source)
            .unwrap()
            .set_len(crate::workspace_snapshot::MAX_FILE_BYTES + 1)
            .unwrap();
        let oversized = super::super::delivery_gate_evidence::review_source_snapshot(
            "team-1",
            "task-1",
            "attempt-1",
            &[change.clone()],
            Some(root.path()),
        );
        assert_eq!(oversized["source_hashes"]["result.md"]["observed"], false);
        assert_eq!(oversized["changeset_source_consistent"], false);
        std::fs::write(&source, b"changed").unwrap();
        let changed = super::super::delivery_gate_evidence::review_source_snapshot(
            "team-1",
            "task-1",
            "attempt-1",
            &[change],
            Some(root.path()),
        );
        assert_eq!(
            changed["source_hashes"]["result.md"]["sha256"],
            crate::CasStore::hash_of(b"changed")
        );
        assert_eq!(changed["changeset_source_consistent"], false);
    }
}
