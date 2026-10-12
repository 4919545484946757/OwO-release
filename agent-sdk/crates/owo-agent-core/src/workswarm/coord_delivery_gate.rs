use super::coord_lifecycle::{validate_independent_review_coverage, DeliveryGateAcceptance};
use super::delivery_gate_evidence::{
    collect_attempt_changeset_evidence, make_validation_receipt, store_validation_receipt,
    ValidationReceiptInput,
};
use super::*;

impl TeamCoordinator {
    /// 宿主交付门：校验步骤绑定的交接、CAS 内容哈希、产物格式、任务断言和未解决问题，
    /// 并为通过验收的实际产物版本生成可追溯收据。
    pub(super) async fn validate_delivery_gate(
        &self,
        team_id: &str,
        space: &ProjectSpace,
        state: &mut GoalRunState,
    ) -> WorkSwarmResult<DeliveryGateAcceptance> {
        let mut pending_delivery_repair: Option<WorkSwarmError> = None;
        if state.aborted || !Self::all_succeeded(state) {
            return Err(WorkSwarmError::Conflict(
                "存在未完成或已取消的步骤，不能收尾".to_string(),
            ));
        }
        if let Some(issue) = state
            .delivery_issues
            .iter()
            .find(|issue| issue.status != crate::goal::DeliveryIssueStatusV1::Resolved)
        {
            return Err(WorkSwarmError::Conflict(format!(
                "存在未关闭的评审 Issue {}（task={} attempt={}），不能收尾",
                issue.issue_id, issue.target_task_id, issue.target_attempt_id
            )));
        }

        let handoffs = self
            .store
            .list_handoffs_by_project(&space.project_id)
            .await?;
        let run_meta = RunMeta::load(&self.run_dir, team_id)?;
        let catalog = self.artifact_catalog(team_id, space).await?;
        let mut text_cache = super::artifact_catalog::ArtifactTextCache::new(&self.cas);
        let verification_workspace = self.verification_workspace(team_id);
        let mut workspace_validation =
            crate::verification::WorkspaceValidationBatch::new(verification_workspace.as_deref());
        let mut review_snapshot = crate::workspace_snapshot::WorkspaceSnapshotBatch::new(
            verification_workspace.as_deref(),
        );
        let change_sets = crate::change_set_store::ChangeSetStore::new(&self.run_dir)
            .list_for_team(team_id)
            .map_err(|error| WorkSwarmError::Run(format!("ChangeSet 读取失败：{error}")))?;
        let runtime_command_receipts = self.runtime_event_details(team_id, "team.command.executed");
        let reviewer_step_count = state
            .plan
            .steps
            .iter()
            .filter(|step| {
                run_meta.roles.iter().any(|role| {
                    step.worker == format!("m-{}", role.role)
                        && super::task_graph::is_independent_reviewer_role(role)
                })
            })
            .count();
        let mut has_code_changes = false;
        let mut validated_review_count = 0usize;
        let mut validated_review_closures = HashSet::new();
        let mut acceptance_receipts = Vec::new();
        let mut final_workspace_receipts = Vec::new();
        let mut final_review_sources = Vec::new();
        let steps_by_id = state
            .plan
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| (step.id.clone(), index))
            .collect::<std::collections::HashMap<_, _>>();
        for step in state.plan.steps.clone() {
            let record = state.records.get(&step.id).cloned().ok_or_else(|| {
                WorkSwarmError::Conflict(format!("任务 {} 缺少执行记录", step.id))
            })?;
            if let Some(skip_reason) = record.skip_reason.as_deref() {
                let role = worker_role(&step.worker).unwrap_or_default();
                let reviewer = run_meta
                    .roles
                    .iter()
                    .find(|role| step.worker == format!("m-{}", role.role))
                    .is_some_and(|role| role.is_reviewer());
                let allowed_review_skip = reviewer
                    && run_meta
                        .template_id
                        .as_deref()
                        .is_some_and(crate::builtin_team_templates::is_code_change_template)
                    && self.workspace_change_status(team_id) == Some(false);
                let allowed_host_manifest = super::task_graph::host_manifest_can_replace_integration(
                    run_meta.parallel,
                    &role,
                    super::task_graph::parallel_tasks_require_integration(&state.plan.steps),
                )
                    && skip_reason == super::task_graph::PARALLEL_LEADER_HOST_MANIFEST_SKIP_REASON;
                if !allowed_review_skip && !allowed_host_manifest {
                    return Err(WorkSwarmError::Conflict(format!(
                        "任务 {} 的运行期跳过没有得到允许或有效的宿主交付证据，不能作为已验收交付",
                        step.id
                    )));
                }
                continue;
            }
            if record
                .output
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            {
                return Err(WorkSwarmError::Conflict(format!(
                    "必需任务 {} 已报告成功，但没有提交候选输出",
                    step.id
                )));
            }
            let handoff = catalog
                .current_handoff(state, &step, &handoffs)?
                .ok_or_else(|| {
                    WorkSwarmError::Conflict(format!(
                        "任务 {} 没有与当前产物/attempt 绑定的交付记录",
                        step.id
                    ))
                })?;
            if !handoff.open_issues.is_empty() {
                return Err(WorkSwarmError::Conflict(format!(
                    "任务 {} 的交付记录仍有未解决问题：{}",
                    step.id,
                    handoff.open_issues.join("；")
                )));
            }
            if handoff.output_artifact_refs.is_empty() {
                return Err(WorkSwarmError::Conflict(format!(
                    "任务 {} 的交付记录没有产物引用",
                    step.id
                )));
            }
            let verification_plan = step.verification_plan.as_ref().ok_or_else(|| {
                WorkSwarmError::Conflict(format!(
                    "任务 {} 没有宿主解析的 VerificationPlan，结果保持未验证",
                    step.id
                ))
            })?;
            verification_plan.validate().map_err(|reason| {
                WorkSwarmError::Conflict(format!(
                    "任务 {} 的 VerificationPlan 非法：{reason}",
                    step.id
                ))
            })?;
            let epoch = record.phase_epoch.ok_or_else(|| {
                WorkSwarmError::Conflict(format!(
                    "任务 {} 缺少宿主 attempt epoch，验证回执不能绑定当前执行",
                    step.id
                ))
            })?;
            let attempt_id = record.attempt_id.as_deref().ok_or_else(|| {
                WorkSwarmError::Conflict(format!(
                    "任务 {} 缺少宿主 attempt_id，不能绑定验收结果",
                    step.id
                ))
            })?;
            let changeset_contains_code =
                super::delivery_gate_evidence::attempt_changeset_contains_code(
                    &change_sets,
                    team_id,
                    &step.id,
                    attempt_id,
                );
            has_code_changes |= changeset_contains_code;
            let has_behavior_command = verification_plan.requirements.iter().any(|requirement| {
                requirement.required && requirement.validator_id == "workspace-command-success-v1"
            });
            // 模板角色的 verify 只声明 non_empty（模板不写死仓库命令），但源码变更
            // 仍需真实行为证据：接受宿主记录的成功行为命令回执作为兜底（回执必须
            // 覆盖当前 ChangeSet 的全部最终源码哈希，模型自述不算证据）。
            let host_behavior_evidence = if changeset_contains_code && !has_behavior_command {
                super::delivery_gate_evidence::attempt_behavior_receipt_evidence(
                    team_id,
                    &runtime_command_receipts,
                    &step.id,
                    attempt_id,
                    &change_sets,
                )
            } else {
                None
            };
            if has_behavior_command {
                // 计划声明了行为命令：逐一校验其 WorkspacePaths 覆盖全部已改源码。
                let uncovered_source_paths = super::delivery_gate_evidence::uncovered_source_paths(
                    &change_sets,
                    team_id,
                    &step.id,
                    attempt_id,
                    verification_plan,
                );
                if !uncovered_source_paths.is_empty() {
                    return Err(WorkSwarmError::Conflict(format!(
                        "任务 {} 的行为验证范围未覆盖所有已修改源码：{}",
                        step.id,
                        uncovered_source_paths.join(", ")
                    )));
                }
            }
            let input_snapshot = json!({
                "task_input": &step.input,
                "attempt_id": attempt_id,
                "epoch": epoch,
            });
            let input_bytes = serde_json::to_vec(&input_snapshot)?;
            let input_sha256 = CasStore::hash_of(&input_bytes);
            let mut repair_required = false;
            for artifact_id in &handoff.output_artifact_refs {
                let artifact = catalog.get(artifact_id)?.clone();
                if artifact.team_id != team_id || artifact.producer != step.worker {
                    return Err(WorkSwarmError::Conflict(format!(
                        "任务 {} 的产物 {} 与当前团队/成员不匹配",
                        step.id, artifact_id
                    )));
                }
                if artifact.task_id.as_deref() != Some(step.id.as_str())
                    || artifact.attempt_id.as_deref() != Some(attempt_id)
                {
                    return Err(WorkSwarmError::Conflict(format!(
                        "任务 {} 的产物 {} 缺少匹配当前 task/attempt 的宿主身份",
                        step.id, artifact_id
                    )));
                }
                let verified_content = text_cache.read(&artifact).await?;
                let content = verified_content.as_ref();
                let code_artifact = super::delivery_gate_evidence::requires_source_delivery(
                    &artifact.kind,
                    &artifact.format,
                    &artifact.file_name,
                    content,
                );
                if code_artifact && !changeset_contains_code {
                    repair_required = true;
                    pending_delivery_repair.get_or_insert(WorkSwarmError::DeliveryRepair {
                        step_id: step.id.clone(), attempt_id: attempt_id.to_string(),
                        validator_id: "workspace-command-success-v1".into(),
                        reason: "可运行产物必须在已有授权写范围内落盘，形成当前 attempt 的源码 ChangeSet，再运行登记的真实行为测试；不得增加权限或只交静态 HTML".into(),
                    });
                }
                if (changeset_contains_code || code_artifact)
                    && !has_behavior_command
                    && host_behavior_evidence.is_none()
                {
                    repair_required = true;
                    pending_delivery_repair.get_or_insert(WorkSwarmError::DeliveryRepair {
                        step_id: step.id.clone(), attempt_id: attempt_id.to_string(),
                        validator_id: "workspace-command-success-v1".into(),
                        reason: "当前源码缺少匹配最终文件哈希的宿主行为验证证据；请在修改后运行获准测试（例如 node --test outputs/page.test.cjs），检查后不要修改源码，重新交付并独立评审".into(),
                    });
                }
                let actual_hash = artifact.sha256.clone();
                let reviewer = run_meta
                    .roles
                    .iter()
                    .find(|role| step.worker == format!("m-{}", role.role))
                    .is_some_and(|role| role.is_reviewer());
                super::delivery_gate_evidence::validate_review_artifact_kind(
                    reviewer,
                    &artifact.kind,
                )
                .map_err(|reason| {
                    WorkSwarmError::Conflict(format!(
                        "任务 {} 的产物类型不符合评审职责（kind={}）：{reason}",
                        step.id, artifact.kind
                    ))
                })?;
                if reviewer {
                    let review: Value = serde_json::from_str(content).map_err(|_| {
                        WorkSwarmError::Conflict(format!(
                            "review Artifact {} 缺少有效的结构化 ReviewResult",
                            artifact.artifact_id
                        ))
                    })?;
                    let review_schema = review.get("schema").and_then(Value::as_str);
                    let reviewer_id = review.get("reviewer_id").and_then(Value::as_str);
                    if review_schema != Some("team-review-result-v1") {
                        return Err(WorkSwarmError::Conflict(format!(
                            "review Artifact {} 的 schema 无效: {:?}",
                            artifact.artifact_id, review_schema
                        )));
                    }
                    if reviewer_id != Some(artifact.producer.as_str()) {
                        return Err(WorkSwarmError::Conflict(format!(
                            "review Artifact {} 的 reviewer 身份无效: reviewer_id={:?}, producer={:?}",
                            artifact.artifact_id, reviewer_id, artifact.producer
                        )));
                    }
                    let review_result = review.get("result").ok_or_else(|| {
                        WorkSwarmError::Conflict(format!(
                            "review Artifact {} 缺少结构化 ReviewResult",
                            artifact.artifact_id
                        ))
                    })?;
                    super::delivery_gate_evidence::validate_review_approval(review_result)
                        .map_err(|reason| {
                            WorkSwarmError::Conflict(format!(
                                "review Artifact {} 未通过宿主裁决：{reason}",
                                artifact.artifact_id
                            ))
                        })?;
                    let review_is_approved =
                        review_result.get("verdict").and_then(Value::as_str) == Some("approved");
                    let bound = review
                        .get("reviewed_artifacts")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            WorkSwarmError::Conflict(
                                "ReviewResult 缺少宿主绑定的 reviewed_artifacts".to_string(),
                            )
                        })?;
                    let mut expected = Vec::new();
                    for dependency in &step.depends_on {
                        let Some(dep_index) = steps_by_id.get(dependency).copied() else {
                            continue;
                        };
                        let Some(dep_step) = state.plan.steps.get(dep_index) else {
                            continue;
                        };
                        let Some(_dep_role) = worker_role(&dep_step.worker) else {
                            continue;
                        };
                        if let Some(current) = catalog.current_for_step(state, dep_step)?.cloned() {
                            let current_attempt = state
                                .records
                                .get(&dep_step.id)
                                .and_then(|record| record.attempt_id.as_deref())
                                .unwrap_or_default();
                            let reviewed_source =
                                super::delivery_gate_evidence::review_source_snapshot_with_batch(
                                    team_id,
                                    &dep_step.id,
                                    current_attempt,
                                    &change_sets,
                                    &mut review_snapshot,
                                );
                            expected.push((
                                current.artifact_id,
                                current.sha256,
                                current.producer,
                                current.kind,
                                reviewed_source,
                                dep_step.id.clone(),
                                current_attempt.to_string(),
                                super::delivery_gate_evidence::review_requirements_for_step(
                                    dep_step,
                                    &state.goal.objective,
                                ),
                            ));
                        }
                    }
                    if expected.is_empty() || bound.len() != expected.len() {
                        return Err(WorkSwarmError::Conflict(format!(
                            "ReviewResult {} 的被审查范围与当前任务上游不一致",
                            artifact.artifact_id
                        )));
                    }
                    let mut expected_requirement_ids = std::collections::BTreeSet::new();
                    for (
                        artifact_id,
                        hash,
                        producer,
                        kind,
                        reviewed_source,
                        reviewed_task_id,
                        reviewed_attempt_id,
                        review_requirements,
                    ) in expected
                    {
                        let Some(binding) = bound.iter().find(|item| {
                            item.get("artifact_id").and_then(Value::as_str)
                                == Some(artifact_id.as_str())
                        }) else {
                            let bound_ids = bound
                                .iter()
                                .filter_map(|item| item.get("artifact_id").and_then(Value::as_str))
                                .collect::<Vec<_>>();
                            return Err(WorkSwarmError::Conflict(format!(
                                "ReviewResult {} 未覆盖当前上游产物 {}；收据快照包含 {:?}",
                                artifact.artifact_id, artifact_id, bound_ids
                            )));
                        };
                        if binding.get("sha256").and_then(Value::as_str) != Some(hash.as_str())
                            || binding.get("producer").and_then(Value::as_str)
                                != Some(producer.as_str())
                            || binding.get("task_id").and_then(Value::as_str)
                                != Some(reviewed_task_id.as_str())
                            || binding.get("attempt_id").and_then(Value::as_str)
                                != Some(reviewed_attempt_id.as_str())
                            || binding.get("reviewed_source") != Some(&reviewed_source)
                            || binding.get("review_requirements").and_then(Value::as_array)
                                != Some(&review_requirements)
                            || producer == artifact.producer
                        {
                            return Err(WorkSwarmError::Conflict(format!(
                                "ReviewResult {} 的 Artifact、源码快照或独立评审身份已过期",
                                artifact.artifact_id
                            )));
                        }
                        for requirement in &review_requirements {
                            if let Some(id) =
                                requirement.get("requirement_id").and_then(Value::as_str)
                            {
                                expected_requirement_ids.insert(id.to_string());
                            }
                        }
                        if review_is_approved {
                            validated_review_closures.insert((
                                artifact.artifact_id.clone(),
                                actual_hash.clone(),
                                reviewed_task_id,
                                reviewed_attempt_id,
                            ));
                        }
                        if super::delivery_gate_evidence::is_code_artifact_kind(&kind)
                            || reviewed_source
                                .get("contains_source_code")
                                .and_then(Value::as_bool)
                                == Some(true)
                            || reviewed_source
                                .get("source_hashes")
                                .and_then(Value::as_object)
                                .is_some_and(|hashes| !hashes.is_empty())
                        {
                            let source_hashes = reviewed_source
                                .get("source_hashes")
                                .and_then(Value::as_object);
                            if reviewed_source
                                .get("workspace_observed")
                                .and_then(Value::as_bool)
                                != Some(true)
                                || reviewed_source
                                    .get("changeset_source_consistent")
                                    .and_then(Value::as_bool)
                                    != Some(true)
                                || source_hashes.is_none_or(|hashes| {
                                    hashes.is_empty()
                                        || hashes.values().any(|entry| {
                                            entry.get("observed").and_then(Value::as_bool)
                                                != Some(true)
                                        })
                                })
                            {
                                return Err(WorkSwarmError::Conflict(format!(
                                    "ReviewResult {} 没有完整的最终源码读取证据",
                                    artifact.artifact_id
                                )));
                            }
                            let subjects = source_hashes
                                .unwrap()
                                .iter()
                                .map(|(path, entry)| {
                                    let hash = entry
                                        .get("sha256")
                                        .and_then(Value::as_str)
                                        .map(str::to_string)
                                        .unwrap_or_else(
                                            crate::verification::workspace_path_absence_sha256,
                                        );
                                    (format!("workspace-path:{path}"), hash)
                                })
                                .collect::<std::collections::HashMap<_, _>>();
                            final_review_sources.push(subjects);
                        }
                    }
                    let result = review
                        .get("result")
                        .and_then(|value| {
                            serde_json::from_value::<owo_agent_workswarm::WorkerReviewResultV1>(
                                value.clone(),
                            )
                            .ok()
                        })
                        .ok_or_else(|| {
                            WorkSwarmError::Conflict(
                                "ReviewResult 的结构化 requirement 覆盖声明无效".to_string(),
                            )
                        })?;
                    super::delivery_gate_evidence::validate_review_requirement_coverage(
                        &result,
                        &expected_requirement_ids,
                    )
                    .map_err(WorkSwarmError::Conflict)?;
                }
                if !artifact.open_issues.is_empty() {
                    return Err(WorkSwarmError::Conflict(format!(
                        "任务 {} 的产物仍有未解决问题：{}",
                        step.id,
                        artifact.open_issues.join("；")
                    )));
                }
                let evidence = artifact
                    .evidence_refs
                    .iter()
                    .map(|source| crate::workswarm_output::WorkerEvidenceV1 {
                        source: source.clone(),
                        note: None,
                    })
                    .collect::<Vec<_>>();
                let mut validation_receipts = Vec::new();
                let (changeset_sha256, changeset_refs) = collect_attempt_changeset_evidence(
                    team_id,
                    &step.id,
                    attempt_id,
                    &change_sets,
                )?;
                let format_started_at = now_ts();
                let format_arguments_sha256 = CasStore::hash_of(
                    serde_json::json!({"format": &artifact.format})
                        .to_string()
                        .as_bytes(),
                );
                let format_validation = crate::artifact_pipeline::validate_artifact_content(
                    &artifact.format,
                    content,
                    &evidence,
                );
                let format_detail = format_validation.reason.clone();
                let format_verdict = if format_validation.valid {
                    crate::plan::ValidationVerdictV1::Passed
                } else {
                    crate::plan::ValidationVerdictV1::Failed
                };
                let format_receipt = make_validation_receipt(ValidationReceiptInput {
                    team_id,
                    step_id: &step.id,
                    attempts: record.attempts,
                    attempt_id,
                    epoch,
                    requirement_id: &format!("{}:artifact-format", step.id),
                    scope: &crate::plan::VerificationScopeV1::ArtifactRefs {
                        artifact_ids: vec![artifact.artifact_id.clone()],
                    },
                    validator_id: "artifact-format-v1",
                    validator_version: "1",
                    arguments_sha256: &format_arguments_sha256,
                    input_sha256: &input_sha256,
                    artifact_id: &artifact.artifact_id,
                    artifact_sha256: &actual_hash,
                    changeset_sha256: changeset_sha256.clone(),
                    changeset_refs: changeset_refs.clone(),
                    evidence_ref: &artifact.content_ref,
                    started_at: &format_started_at,
                    verdict: format_verdict,
                    detail: format_detail.clone(),
                    subject_hashes: std::collections::BTreeMap::new(),
                    additional_evidence_refs: Vec::new(),
                });
                store_validation_receipt(state, &step.id, &format_receipt);
                validation_receipts.push(format_receipt);
                if !format_validation.valid {
                    return Err(WorkSwarmError::Conflict(format!(
                        "任务 {} 的产物 {} 格式验收失败：{}",
                        step.id,
                        artifact_id,
                        format_detail.as_deref().unwrap_or("无原因")
                    )));
                }

                for requirement in &verification_plan.requirements {
                    let started_at = now_ts();
                    let (verdict, detail, subject_hashes, command_evidence_ref) =
                        if requirement.validator_id == "workspace-command-success-v1" {
                            super::delivery_gate_evidence::evaluate_workspace_command_receipt(
                                team_id,
                                requirement,
                                &runtime_command_receipts,
                                &step.id,
                                attempt_id,
                                &change_sets,
                            )
                        } else {
                            let (verdict, detail, subject_hashes) =
                                workspace_validation.execute_registered(requirement, content);
                            (verdict, detail, subject_hashes, None)
                        };
                    let repairable_evidence = matches!(
                        verdict,
                        crate::plan::ValidationVerdictV1::Failed
                            | crate::plan::ValidationVerdictV1::Unverified
                    );
                    if verdict != crate::plan::ValidationVerdictV1::Unsupported
                        && (verdict != crate::plan::ValidationVerdictV1::Unverified
                            || !subject_hashes.is_empty())
                    {
                        if let crate::plan::VerificationScopeV1::WorkspacePaths { relative_paths } =
                            &requirement.scope
                        {
                            super::delivery_gate_evidence::validate_workspace_receipt_snapshot(
                                team_id,
                                &step.id,
                                attempt_id,
                                relative_paths,
                                &subject_hashes,
                                &change_sets,
                            )
                            .map_err(|reason| {
                                WorkSwarmError::Conflict(format!(
                                    "验收证据与源码快照不一致，不能自动改写：{reason}"
                                ))
                            })?;
                        }
                    }
                    let arguments_sha256 =
                        CasStore::hash_of(requirement.arguments.to_string().as_bytes());
                    let mut additional_evidence_refs = subject_hashes
                        .iter()
                        .map(|(subject, hash)| format!("{subject}@sha256:{hash}"))
                        .collect::<Vec<_>>();
                    if let Some(evidence_ref) = command_evidence_ref {
                        additional_evidence_refs.push(evidence_ref);
                    }
                    let receipt = make_validation_receipt(ValidationReceiptInput {
                        team_id,
                        step_id: &step.id,
                        attempts: record.attempts,
                        attempt_id,
                        epoch,
                        requirement_id: &requirement.requirement_id,
                        scope: &requirement.scope,
                        validator_id: &requirement.validator_id,
                        validator_version: requirement.validator_version.as_deref().unwrap_or(""),
                        arguments_sha256: &arguments_sha256,
                        input_sha256: &input_sha256,
                        artifact_id: &artifact.artifact_id,
                        artifact_sha256: &actual_hash,
                        changeset_sha256: changeset_sha256.clone(),
                        changeset_refs: changeset_refs.clone(),
                        evidence_ref: &artifact.content_ref,
                        started_at: &started_at,
                        verdict,
                        detail: detail.clone(),
                        additional_evidence_refs,
                        subject_hashes,
                    });
                    if verdict == crate::plan::ValidationVerdictV1::Passed
                        && receipt
                            .subject_sha256
                            .keys()
                            .any(|subject| subject.starts_with("workspace-path:"))
                    {
                        final_workspace_receipts
                            .push((receipt.receipt_id.clone(), receipt.subject_sha256.clone()));
                    }
                    store_validation_receipt(state, &step.id, &receipt);
                    validation_receipts.push(receipt);
                    if requirement.required
                        && repairable_evidence
                        && requirement.validator_id == "workspace-command-success-v1"
                        && matches!(
                            verdict,
                            crate::plan::ValidationVerdictV1::Failed
                                | crate::plan::ValidationVerdictV1::Unverified
                        )
                    {
                        repair_required = true;
                        pending_delivery_repair.get_or_insert(WorkSwarmError::DeliveryRepair {
                            step_id: step.id.clone(),
                            attempt_id: attempt_id.to_string(),
                            validator_id: requirement.validator_id.clone(),
                            reason: detail.clone().unwrap_or_else(|| {
                                "必须重新执行登记的行为测试并交付有效证据".into()
                            }),
                        });
                        continue;
                    }
                    if requirement.required && verdict != crate::plan::ValidationVerdictV1::Passed {
                        return Err(WorkSwarmError::Conflict(format!(
                            "任务 {} 的验收要求 {} 未通过或未验证：{}",
                            step.id,
                            requirement.requirement_id,
                            detail.as_deref().unwrap_or("验证器未返回通过")
                        )));
                    }
                }
                if changeset_contains_code && !has_behavior_command {
                    if let Some(evidence_ref) = &host_behavior_evidence {
                        // 兜底路径也要留下可追溯的宿主验收收据（审计面可见）。
                        let started_at = now_ts();
                        let receipt = make_validation_receipt(ValidationReceiptInput {
                            team_id,
                            step_id: &step.id,
                            attempts: record.attempts,
                            attempt_id,
                            epoch,
                            requirement_id: &format!("{}:host-behavior-receipt", step.id),
                            scope: &crate::plan::VerificationScopeV1::ArtifactRefs {
                                artifact_ids: vec![artifact.artifact_id.clone()],
                            },
                            validator_id: "workspace-command-success-v1",
                            validator_version: "1",
                            arguments_sha256: "",
                            input_sha256: &input_sha256,
                            artifact_id: &artifact.artifact_id,
                            artifact_sha256: &actual_hash,
                            changeset_sha256: changeset_sha256.clone(),
                            changeset_refs: changeset_refs.clone(),
                            evidence_ref: &artifact.content_ref,
                            started_at: &started_at,
                            verdict: crate::plan::ValidationVerdictV1::Passed,
                            detail: Some(
                                "模板未声明行为命令；宿主按真实成功命令回执接受该源码变更"
                                    .to_string(),
                            ),
                            subject_hashes: std::collections::BTreeMap::new(),
                            additional_evidence_refs: vec![evidence_ref.clone()],
                        });
                        store_validation_receipt(state, &step.id, &receipt);
                        validation_receipts.push(receipt);
                    }
                }
                let required_validation_count = 1 + verification_plan
                    .requirements
                    .iter()
                    .filter(|requirement| requirement.required)
                    .count();
                let passed_required_validation_count = 1 + verification_plan
                    .requirements
                    .iter()
                    .filter(|requirement| requirement.required)
                    .filter(|requirement| {
                        validation_receipts.iter().any(|receipt| {
                            receipt.requirement_id == requirement.requirement_id
                                && receipt.verdict == crate::plan::ValidationVerdictV1::Passed
                        })
                    })
                    .count();
                let failed_required_validation_count = verification_plan
                    .requirements
                    .iter()
                    .filter(|requirement| requirement.required)
                    .filter(|requirement| {
                        validation_receipts.iter().any(|receipt| {
                            receipt.requirement_id == requirement.requirement_id
                                && receipt.verdict == crate::plan::ValidationVerdictV1::Failed
                        })
                    })
                    .count();
                let completion_status =
                    crate::completion::decide_completion(crate::completion::CompletionEvidence {
                        response_finished: true,
                        has_candidate_changes: true,
                        required_validation_count,
                        passed_required_validation_count,
                        failed_required_validation_count,
                        ..crate::completion::CompletionEvidence::default()
                    });
                if completion_status != owo_agent_protocol::CompletionStatusV1::Accepted {
                    if repair_required {
                        continue;
                    }
                    return Err(WorkSwarmError::Conflict(format!(
                        "任务 {} 的共享完成裁决没有接受该候选结果：{completion_status:?}",
                        step.id
                    )));
                }
                if reviewer {
                    validated_review_count += 1;
                }
                acceptance_receipts.push(json!({
                    "completion_status": completion_status,
                    "step_id": &step.id,
                    "attempt_id": attempt_id,
                    "artifact_id": &artifact.artifact_id,
                    "artifact_kind": &artifact.kind,
                    "content_sha256": &actual_hash,
                    "format_validator": {
                        "id": "artifact-format-v1",
                        "format": &format_validation.format,
                        "verdict": "passed",
                    },
                    "validation_receipts": validation_receipts,
                }));
            }
        }

        let final_workspace_root = self.verification_workspace(team_id);
        let mut final_snapshot =
            crate::workspace_snapshot::WorkspaceSnapshotBatch::new(final_workspace_root.as_deref());
        for (receipt_id, subjects) in &final_workspace_receipts {
            if !final_snapshot.subjects_match(subjects) {
                return Err(WorkSwarmError::Conflict(format!(
                    "最终交付时工作区已偏离已通过的验证收据 {receipt_id}"
                )));
            }
        }

        for subjects in &final_review_sources {
            if !final_snapshot.subjects_match(subjects) {
                return Err(WorkSwarmError::Conflict(
                    "最终交付时工作区已偏离独立评审绑定的文件快照".into(),
                ));
            }
        }

        for issue in &state.delivery_issues {
            let Some(review_id) = issue.resolution_review_artifact_id.as_deref() else {
                return Err(WorkSwarmError::Conflict(format!(
                    "已关闭评审 Issue {} 缺少关闭评审身份",
                    issue.issue_id
                )));
            };
            let Some(review_sha256) = issue.resolution_review_sha256.as_deref() else {
                return Err(WorkSwarmError::Conflict(format!(
                    "已关闭评审 Issue {} 缺少关闭评审版本哈希",
                    issue.issue_id
                )));
            };
            let Some(attempt_id) = issue.resolution_attempt_id.as_deref() else {
                return Err(WorkSwarmError::Conflict(format!(
                    "已关闭评审 Issue {} 缺少关闭 attempt 身份",
                    issue.issue_id
                )));
            };
            if issue.target_attempt_id == attempt_id
                || !validated_review_closures.contains(&(
                    review_id.to_string(),
                    review_sha256.to_string(),
                    issue.target_task_id.clone(),
                    attempt_id.to_string(),
                ))
            {
                return Err(WorkSwarmError::Conflict(format!(
                    "评审 Issue {} 的关闭证据没有绑定到修复后的最终 task/attempt 版本",
                    issue.issue_id
                )));
            }
        }
        let required_validation_count = acceptance_receipts.len();
        let independent_review_required =
            validate_independent_review_coverage(has_code_changes, reviewer_step_count)
                .map_err(WorkSwarmError::Conflict)?;
        let completion_status =
            crate::completion::decide_completion(crate::completion::CompletionEvidence {
                response_finished: true,
                has_candidate_changes: !acceptance_receipts.is_empty(),
                required_validation_count,
                passed_required_validation_count: required_validation_count,
                independent_review_required,
                independent_review_passed: validated_review_count >= reviewer_step_count,
                ..crate::completion::CompletionEvidence::default()
            });
        if completion_status != owo_agent_protocol::CompletionStatusV1::Accepted {
            return Err(WorkSwarmError::Conflict(format!(
                "Team DeliveryGate 的共享完成裁决未接受最终候选版本：{completion_status:?}"
            )));
        }

        // 仅检查本次 ProjectSpace 产物，确保传入空间仍属于本次运行。
        if space.project_id.is_empty() {
            return Err(WorkSwarmError::Conflict(
                "交付空间缺少 project_id，不能确认产物归属".to_string(),
            ));
        }
        if let Some(repair) = pending_delivery_repair {
            return Err(repair);
        }
        Ok(DeliveryGateAcceptance {
            receipts: acceptance_receipts,
            workspace_receipts: final_workspace_receipts,
        })
    }
}
