//! Context assembly with bounded, verified CAS reads; authorized paging lives in artifact_read.
use super::artifact_read::{read_context_pages, ArtifactPreviewCache};
use super::coord_artifacts::{
    context_fact_matches_step, truncate_utf8_to_bytes, MAX_SHARED_FACT_CONTEXT_BYTES,
    MAX_SHARED_FACT_INLINE_BYTES,
};
use super::*;

/// Reviewers see the deliverable they are judging plus every transitive upstream
/// artifact, not only the last handoff report. The reviewer still gets normal
/// role permissions; this only widens the read-only context slice.
pub(super) fn collect_review_dependency_ids(
    step_id: &str,
    direct: &[String],
    steps: &std::collections::HashMap<String, crate::plan::StepSpec>,
) -> Vec<String> {
    let mut seen = HashSet::from([step_id.to_string()]);
    let mut pending = direct.to_vec();
    let mut expanded = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(dep_step) = steps.get(&id) {
            expanded.push(id.clone());
            pending.extend(dep_step.depends_on.iter().cloned());
        }
    }
    expanded
}

/// Host-recorded command receipts for one upstream step. Reviewers are read-only
/// and cannot execute `npm test` themselves; exposing only the receipt summary
/// (exit code / validator / duration, never argv or file hashes) lets them tell
/// whether the host observed a successful command without widening permissions.
pub(super) fn command_evidence_for_step(
    events: &[String],
    step_id: &str,
    attempt_id: Option<&str>,
) -> Vec<Value> {
    let mut evidence = events
        .iter()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(|event| event.get("step_id").and_then(Value::as_str) == Some(step_id))
        .filter(|event| {
            attempt_id.is_none_or(|expected| {
                event.get("attempt_id").and_then(Value::as_str) == Some(expected)
            })
        })
        .filter_map(|event| {
            let receipt = event.get("receipt")?;
            Some(json!({
                "command_sha256": receipt.get("command_sha256"),
                "command_preview": event.get("command_preview"),
                "exit_code": receipt.get("exit_code"),
                "duration_ms": receipt.get("duration_ms"),
                "workspace_hashes_complete": receipt.get("workspace_hashes_complete"),
                "validator_id": receipt.get("validator_id"),
            }))
        })
        .collect::<Vec<_>>();
    if evidence.len() > 6 {
        evidence.drain(0..evidence.len() - 6);
    }
    evidence
}

impl TeamCoordinator {
    // -- 上下文切片（handoff 的运行时视图；A3 结构化 context slice） --

    /// 为 (member, step) 组装结构化上下文切片：
    /// `{ team_id, objective, role, handoff_contract, upstream: [{role, artifact_id, version, content, review_state}] }`。
    pub async fn assemble_context_slice(
        &self,
        team_id: &str,
        member_id: &str,
        step_id: &str,
    ) -> WorkSwarmResult<Value> {
        self.assemble_context_slice_for_attempt(team_id, member_id, step_id, None)
            .await
            .map(|(context, _)| context)
    }

    pub(super) async fn assemble_context_slice_for_attempt(
        &self,
        team_id: &str,
        member_id: &str,
        step_id: &str,
        phase_epoch: Option<u64>,
    ) -> WorkSwarmResult<(Value, bool)> {
        let phase_snapshot = phase_epoch
            .filter(|epoch| self.current_execution_epoch(team_id) == *epoch)
            .and_then(|epoch| self.phase_context_snapshot(team_id, epoch));
        let snapshot_cache_hit = phase_snapshot.is_some();
        let (space, goal_objective, meta, steps_by_id, attempts, parent_context_refs) =
            match phase_snapshot.as_ref() {
                Some(snapshot) => {
                    let space = self
                        .store
                        .get_project_space(&snapshot.project_id)
                        .await
                        .map_err(WorkSwarmError::Store)?;
                    (
                        space,
                        snapshot.goal_objective.clone(),
                        snapshot.meta.clone(),
                        Arc::clone(&snapshot.steps_by_id),
                        self.load_context_attempts(team_id)?,
                        None,
                    )
                }
                None => {
                    let (team, space, state) = self.load_bundle(team_id).await?;
                    let meta = RunMeta::load(&self.run_dir, team_id)?;
                    let attempts = state
                        .records
                        .iter()
                        .map(|(step_id, record)| (step_id.clone(), record.attempt_id.clone()))
                        .collect();
                    let steps_by_id = Arc::new(
                        state
                            .plan
                            .steps
                            .into_iter()
                            .map(|step| (step.id.clone(), step))
                            .collect(),
                    );
                    (
                        space,
                        state.goal.objective,
                        meta,
                        steps_by_id,
                        attempts,
                        Some(team.shared_context_refs.clone()),
                    )
                }
            };
        let spec = Self::role_spec_of_member(&meta, member_id)?;
        let step = steps_by_id
            .get(step_id)
            .ok_or_else(|| WorkSwarmError::NotFound(format!("步骤 {step_id} 不存在")))?;
        if step.worker != member_id {
            return Err(WorkSwarmError::Validation(
                "请求成员不是当前步骤的承担者".into(),
            ));
        }
        // These reads are independent after the current run bundle is loaded. Keep
        // them in one bounded join so each Worker does not pay three serial storage
        // waits before it can assemble its step-specific Artifact view.
        let core_specs_snapshot = phase_snapshot.clone();
        let parent_context_refs = parent_context_refs.unwrap_or_default();
        let (catalog, shared_context, core_specs) = tokio::try_join!(
            self.artifact_catalog(team_id, &space),
            async {
                self.store
                    .get_team_context(team_id)
                    .await
                    .map_err(|error| WorkSwarmError::Run(error.to_string()))
            },
            async {
                match core_specs_snapshot {
                    Some(snapshot) => snapshot.load_core_specs(&self.cas).await,
                    None => load_parent_core_specs(self.cas.clone(), parent_context_refs)
                        .await
                        .map(Arc::new),
                }
            },
        )?;
        let mut previews =
            ArtifactPreviewCache::with_shared(&self.cas, Arc::clone(&self.artifact_preview_cache));
        let mut fact_candidates = Vec::new();
        let mut latest_fact_keys = HashSet::new();
        for fact in shared_context.facts.iter().rev().take(128) {
            if !latest_fact_keys.insert(fact.key.as_str()) {
                continue;
            }
            // A file-hash-bound fact must be checked against the bound workspace by
            // the server before it can enter a model prompt. Workers can request it
            // through team_context_read, which performs that freshness check.
            if fact.file_hash.is_some() {
                continue;
            }
            if !context_fact_matches_step(fact, step) {
                continue;
            }
            if fact.status != "candidate" && fact.status != "confirmed" {
                continue;
            }
            fact_candidates.push(fact);
        }
        const SHARED_FACT_READ_CONCURRENCY: usize = 4;
        let mut fact_budget = MAX_SHARED_FACT_CONTEXT_BYTES;
        let mut fact_candidates = fact_candidates.into_iter();
        let mut shared_facts = Vec::new();
        'facts: while fact_budget > 0 {
            // Each batch reserves at most the remaining aggregate budget. A short
            // value leaves budget for the next batch, preserving newest-first fill
            // semantics while parallelizing independent CAS pages within a batch.
            let batch_capacity = if fact_budget < MAX_SHARED_FACT_INLINE_BYTES {
                1
            } else {
                (fact_budget / MAX_SHARED_FACT_INLINE_BYTES).min(SHARED_FACT_READ_CONCURRENCY)
            };
            let read_limit = fact_budget.min(MAX_SHARED_FACT_INLINE_BYTES);
            let mut batch = Vec::with_capacity(batch_capacity);
            let mut page_requests = Vec::with_capacity(batch_capacity);
            for _ in 0..batch_capacity {
                let Some(fact) = fact_candidates.next() else {
                    break;
                };
                let hash = context_cas_hash(&fact.value_ref)?.to_string();
                batch.push((fact, read_limit));
                page_requests.push((hash, read_limit));
            }
            if batch.is_empty() {
                break;
            }
            let pages =
                read_context_pages(&self.cas, page_requests, SHARED_FACT_READ_CONCURRENCY).await?;
            for ((fact, inline_budget), page) in batch.into_iter().zip(pages) {
                if page.total_bytes == 0 {
                    continue;
                }
                let (value, budget_truncated) =
                    truncate_utf8_to_bytes(&page.content, inline_budget);
                let value = value.to_string();
                if value.is_empty() {
                    break 'facts;
                }
                let truncated = budget_truncated || !page.eof;
                fact_budget = fact_budget.saturating_sub(value.len());
                shared_facts.push(json!({
                    "key": fact.key, "value": value, "value_ref": fact.value_ref,
                    "truncated": truncated,
                    "revision": fact.revision, "producer": fact.producer,
                    "task_id": fact.task_id, "source_refs": fact.source_refs,
                    "file_hash": fact.file_hash, "confidence": fact.confidence,
                    "status": fact.status
                }));
            }
        }
        let review_change_sets = crate::change_set_store::ChangeSetStore::new(&self.run_dir)
            .list_for_team(team_id)
            .map_err(|error| WorkSwarmError::Run(format!("Review ChangeSet 读取失败：{error}")))?;
        let review_workspace = self.verification_workspace(team_id);
        let mut review_snapshot =
            crate::workspace_snapshot::WorkspaceSnapshotBatch::new(review_workspace.as_deref());
        const UPSTREAM_READ_CONCURRENCY: usize = 4;
        let mut upstream_sources = Vec::new();
        let mut upstream_body_budget = crate::team_prompt::DEFAULT_TOTAL_UPSTREAM_BYTES;
        // A reviewer must see the deliverables it is reviewing, not only the
        // immediately preceding report. Otherwise a critic can identify a defect
        // in an earlier implementation task but the host cannot bind the finding
        // to a reviewed artifact, causing the whole run to fail at rework planning.
        let dependency_ids = if super::util::is_review_role(&spec.role, &spec.capabilities) {
            collect_review_dependency_ids(&step.id, &step.depends_on, &steps_by_id)
        } else {
            step.depends_on.clone()
        };
        for dep in &dependency_ids {
            let Some(dep_step) = steps_by_id.get(dep) else {
                continue;
            };
            let Some(dep_role) = worker_role(&dep_step.worker) else {
                continue;
            };
            let attempt = attempts.get(&dep_step.id).and_then(Option::as_deref);
            let Some(artifact) = catalog.current_for_attempt(dep_step, attempt)? else {
                continue;
            };
            if upstream_body_budget == 0 {
                break;
            }
            let declared_bytes = usize::try_from(artifact.size_bytes).unwrap_or(usize::MAX);
            // Review JSON carries reviewer_id and reviewed-artifact bindings at
            // the end; use a larger cap so identity is not cut off by the
            // ordinary artifact preview limit.
            let per_artifact_limit = if artifact.kind == "review" {
                crate::team_prompt::DEFAULT_REVIEW_BODY_BYTES
            } else {
                crate::team_prompt::DEFAULT_PER_ARTIFACT_BYTES
            };
            let read_limit = upstream_body_budget
                .min(per_artifact_limit)
                .min(declared_bytes);
            // Reserve each artifact's full size when small and the preview limit
            // when large before fan-out. Summed request limits therefore stay
            // within the existing prompt body budget, even for empty artifacts.
            upstream_body_budget = upstream_body_budget.saturating_sub(read_limit);
            upstream_sources.push((dep_step.clone(), dep_role, artifact.clone(), read_limit));
        }
        let preview_requests = upstream_sources
            .iter()
            .map(|(_, _, artifact, max_bytes)| (artifact, *max_bytes))
            .collect();
        let preview_pages = previews
            .read_many(preview_requests, UPSTREAM_READ_CONCURRENCY)
            .await?;
        let mut upstream = Vec::with_capacity(upstream_sources.len());
        let command_events = self.runtime_event_details(team_id, "team.command.executed");
        let mut upstream_body_budget = crate::team_prompt::DEFAULT_TOTAL_UPSTREAM_BYTES;
        for ((dep_step, dep_role, artifact, _), page) in
            upstream_sources.into_iter().zip(preview_pages)
        {
            let content = page.content.as_str();
            let truncated = !page.eof;
            upstream_body_budget = upstream_body_budget.saturating_sub(content.len());
            upstream.push(json!({
                "role": dep_role,
                "step_id": dep_step.id,
                "worker": dep_step.worker,
                "task_id": artifact.task_id,
                "attempt_id": artifact.attempt_id,
                "artifact_id": artifact.artifact_id,
                "kind": artifact.kind,
                "reviewed_source": super::delivery_gate_evidence::review_source_snapshot_with_batch(
                    team_id,
                    &dep_step.id,
                    artifact.attempt_id.as_deref().unwrap_or_default(),
                    &review_change_sets,
                    &mut review_snapshot,
                ),
                "review_requirements": super::delivery_gate_evidence::review_requirements_for_step(
                    &dep_step,
                    &goal_objective,
                ),
                "host_command_evidence": command_evidence_for_step(
                    &command_events,
                    &dep_step.id,
                    artifact.attempt_id.as_deref(),
                ),
                "version": artifact.version,
                "content": content,
                "content_bytes": page.total_bytes,
                "truncated": truncated,
                // Hash covers verified full CAS bytes, never just the inline preview.
                "sha256": artifact.sha256,
                "producer": artifact.producer,
                // 八期一路：CAS ref 随切片透出（大 Artifact 摘要块需带哈希与 ref）。
                "cas_ref": artifact.content_ref,
                "review_state": format!("{:?}", artifact.review_state),
            }));
        }
        let mut review_handoff_contract = spec.handoff_contract.clone();
        if super::util::is_review_role(&spec.role, &spec.capabilities) {
            let source_manifest = upstream
                .iter()
                .map(|artifact| {
                    let source = artifact.get("reviewed_source");
                    json!({
                        "artifact_id": artifact.get("artifact_id"),
                        "task_id": artifact.get("task_id"),
                        "attempt_id": artifact.get("attempt_id"),
                        "kind": artifact.get("kind"),
                        "change_set_ids": source.and_then(|value| value.get("change_set_ids")),
                        "change_set_sha256": source.and_then(|value| value.get("change_set_sha256")),
                        "source_hashes": source.and_then(|value| value.get("source_hashes")),
                        "review_requirements": artifact.get("review_requirements").cloned().unwrap_or_else(|| json!([])),
                    })
                })
                .collect::<Vec<_>>();
            let manifest = serde_json::to_string(&source_manifest)
                .map_err(|error| WorkSwarmError::Serialization(error.to_string()))?;
            if manifest.len() > 64 * 1024 {
                return Err(WorkSwarmError::Validation(
                    "Reviewer 源码快照超过 64 KiB 上限，拒绝以不完整清单继续评审".to_string(),
                ));
            }
            if !source_manifest.is_empty() {
                let expected_ids = source_manifest
                    .iter()
                    .flat_map(|artifact| {
                        artifact
                            .get("review_requirements")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                    })
                    .filter_map(|requirement| {
                        requirement.get("requirement_id").and_then(Value::as_str)
                    })
                    .collect::<Vec<_>>();
                let base = review_handoff_contract
                    .as_deref()
                    .unwrap_or_default()
                    .trim();
                review_handoff_contract = Some(format!(
                    "只允许逐字复制的 requirement_id：{expected_ids:?}。不得自造 REQ-001 等编号；approved 必须完整覆盖，changes_requested 只需报告已发现的问题。
{base}

宿主绑定的评审清单（只读）：{manifest}。逐项审查每个上游任务的 review_requirements，最终在 review_result.reviewed_requirement_ids 中原样列出全部 requirement_id，且不得重复、遗漏或增加；对 code/source artifact 逐个读取清单中的工作区文件并按源码证据提交 findings。host_command_evidence 是宿主记录的命令回执摘要（含命令预览、exit_code 和工作区哈希）；若其中存在 exit_code=0 且命令就是目标测试的回执，视为宿主已完成行为验证，**不得**以“我无法运行测试”“需用户运行”或“无法获取退出码”为由 changes_requested；交付门会独立校验该回执是否覆盖最终源码哈希。只有源码本身有缺陷或确实缺少任何宿主命令回执时，才要求 producer 返修。交付门会校验这些哈希及要求在评审期间和交付时未变化。"
                ));
            }
        }
        Ok((
            json!({
                "team_id": team_id,
                "objective_text": goal_objective,
                "role": spec.role,
                "node_id": spec.node_id,
                "capabilities": spec.capabilities,
                "write_paths": spec.write_paths,
                "member_id": member_id,
                "handoff_contract": review_handoff_contract,
                "core_spec": core_specs.as_ref().clone(),
                "shared_context_revision": shared_context.revision,
                "shared_facts": shared_facts,
                // 八期一路：模板 id + 角色调用预算（角色专属 Prompt 编译输入）。
                "template_id": meta.template_id,
                "budget_calls": meta.budgets.get(&spec.role).copied().unwrap_or(0),
                "retry_note": step.input.get("_workswarm").and_then(|meta| meta.get("retry_note")),
                "upstream": upstream,
            }),
            snapshot_cache_hit,
        ))
    }
}

pub(super) async fn load_parent_core_specs(
    cas: CasStore,
    references: Vec<String>,
) -> WorkSwarmResult<Vec<Value>> {
    use futures::stream::{self, StreamExt, TryStreamExt};

    const PARENT_CONTEXT_READ_CONCURRENCY: usize = 4;
    let hashes = references
        .iter()
        .map(|reference| context_cas_hash(reference).map(str::to_string))
        .collect::<WorkSwarmResult<Vec<_>>>()?;
    stream::iter(hashes.into_iter().map(|hash| {
        let cas = cas.clone();
        async move {
            let raw = super::artifact_read::read_context_cas_text(
                &cas,
                &hash,
                super::coord_artifacts::MAX_PARENT_CONTEXT_SNAPSHOT_BYTES,
            )
            .await?;
            let snapshot: Value = serde_json::from_str(&raw).map_err(|error| {
                WorkSwarmError::Conflict(format!("父会话上下文快照损坏：{error}"))
            })?;
            if snapshot.get("kind").and_then(Value::as_str) != Some("source_session_context_v1") {
                return Err(WorkSwarmError::Conflict(
                    "父会话上下文快照类型不受支持".to_string(),
                ));
            }
            let spec = snapshot
                .get("core_spec")
                .filter(|value| value.is_object())
                .ok_or_else(|| {
                    WorkSwarmError::Conflict("父会话上下文快照缺少 core_spec".to_string())
                })?;
            Ok(spec.clone())
        }
    }))
    .buffered(PARENT_CONTEXT_READ_CONCURRENCY)
    .try_collect()
    .await
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // 测试模块历史位置靠前；移动会打乱同文件阅读顺序
mod tests {
    use super::load_parent_core_specs;
    use crate::cas_store::CasStore;

    #[tokio::test]
    async fn parallel_parent_snapshot_reads_preserve_reference_order() {
        let dir = tempfile::tempdir().unwrap();
        let cas = CasStore::new(dir.path().to_path_buf()).unwrap();
        let expected = vec![
            serde_json::json!({"constraint": "first source"}),
            serde_json::json!({"constraint": "second source"}),
            serde_json::json!({"constraint": "third source"}),
            serde_json::json!({"constraint": "fourth source"}),
            serde_json::json!({"constraint": "fifth source"}),
        ];
        let references = expected
            .iter()
            .map(|core_spec| {
                let snapshot = serde_json::json!({
                    "kind": "source_session_context_v1",
                    "core_spec": core_spec,
                });
                let hash = cas.put(snapshot.to_string().as_bytes()).unwrap();
                format!("cas://sha256:{hash}")
            })
            .collect::<Vec<_>>();

        let actual = load_parent_core_specs(cas, references).await.unwrap();

        assert_eq!(actual, expected);
    }
}

fn context_cas_hash(content_ref: &str) -> WorkSwarmResult<&str> {
    content_ref
        .strip_prefix("cas://sha256:")
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| WorkSwarmError::Validation("上下文缺少合法 CAS 哈希引用".into()))
}

#[cfg(test)]
mod review_scope_tests {
    use super::{collect_review_dependency_ids, command_evidence_for_step};
    use crate::plan::StepSpec;
    use serde_json::json;
    use std::collections::HashMap;

    fn step(id: &str, deps: &[&str]) -> StepSpec {
        let mut step = StepSpec::new(id, "worker");
        step.depends_on = deps.iter().map(|dep| dep.to_string()).collect();
        step
    }

    #[test]
    fn review_command_evidence_is_bound_to_step_and_attempt_without_leaking_hashes() {
        let events = vec![
            json!({
                "step_id": "s-build",
                "attempt_id": "a1",
                "command_preview": "npm test",
                "receipt": {
                    "command_sha256": "c1",
                    "exit_code": 0,
                    "duration_ms": 12,
                    "workspace_hashes_complete": true,
                    "validator_id": "workspace-command-success-v1",
                    "workspace_hashes": {"src/lib.rs": "secret-hash"}
                }
            })
            .to_string(),
            json!({
                "step_id": "s-build",
                "attempt_id": "a2",
                "receipt": {"command_sha256": "c2", "exit_code": 1}
            })
            .to_string(),
            json!({
                "step_id": "s-other",
                "attempt_id": "a1",
                "receipt": {"command_sha256": "c3", "exit_code": 0}
            })
            .to_string(),
            "not json".to_string(),
        ];
        let evidence = command_evidence_for_step(&events, "s-build", Some("a1"));
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0]["command_sha256"], "c1");
        assert_eq!(evidence[0]["command_preview"], "npm test");
        assert_eq!(evidence[0]["exit_code"], 0);
        assert_eq!(evidence[0]["validator_id"], "workspace-command-success-v1");
        assert!(evidence[0].get("workspace_hashes").is_none());
    }

    #[test]
    fn reviewer_scope_walks_transitive_ancestors_without_cycles() {
        let steps = HashMap::from([
            ("research".to_string(), step("research", &[])),
            ("design".to_string(), step("design", &["research"])),
            ("test".to_string(), step("test", &["design"])),
            ("review".to_string(), step("review", &["test"])),
        ]);
        let mut ids = collect_review_dependency_ids("review", &steps["review"].depends_on, &steps);
        ids.sort();
        assert_eq!(ids, vec!["design", "research", "test"]);
    }
}
