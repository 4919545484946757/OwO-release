//! Pure planning and durable identity rules for review evidence and owner repairs.
use super::*;

use super::coord_rework::ReworkRequest;

const MAX_REVIEW_FINDINGS: usize = 128;

pub(super) struct ReviewChanges {
    pub(super) issues: Vec<crate::goal::DeliveryIssueV1>,
    pub(super) repairs: Vec<ReworkRequest>,
}

pub(super) fn plan_review_changes(
    team_id: &str,
    state: &GoalRunState,
    artifact: &Artifact,
    document: &Value,
) -> Result<ReviewChanges, String> {
    let findings = document
        .pointer("/result/findings")
        .and_then(Value::as_array)
        .filter(|findings| !findings.is_empty() && findings.len() <= MAX_REVIEW_FINDINGS)
        .ok_or_else(|| "请求修改必须有 1..=128 条结构化 finding，超额不能截断".to_string())?;
    let reviewed = document
        .get("reviewed_artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| "缺少宿主被审快照，不能派发返修".to_string())?;
    let mut issues = Vec::new();
    let mut owners: BTreeMap<String, ReworkRequest> = BTreeMap::new();
    let mut seen = HashSet::new();
    for finding in findings {
        let detail = finding
            .get("detail")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|detail| !detail.is_empty())
            .ok_or_else(|| "finding 缺少可派发的 detail".to_string())?;
        let binding = select_reviewed_owner_binding(reviewed, finding).ok_or_else(|| {
            "finding 未唯一关联被审任务，需要 target_task_id 或 target_artifact_id".to_string()
        })?;
        let producer = binding
            .get("producer")
            .and_then(Value::as_str)
            .filter(|owner| !owner.trim().is_empty() && *owner != artifact.producer)
            .ok_or_else(|| "finding owner 必须是独立生产者".to_string())?;
        let task_id = binding
            .get("task_id")
            .and_then(Value::as_str)
            .filter(|task_id| !task_id.is_empty())
            .ok_or_else(|| "被审任务缺少 task_id".to_string())?;
        let attempt_id = binding
            .get("attempt_id")
            .and_then(Value::as_str)
            .filter(|attempt_id| !attempt_id.is_empty())
            .ok_or_else(|| "被审任务缺少 attempt_id".to_string())?;
        let matching = state
            .plan
            .steps
            .iter()
            .filter(|step| {
                step.worker == producer
                    && (step.id == task_id
                        || step.input.get("assigned_task_id").and_then(Value::as_str)
                            == Some(task_id))
                    && state.records.get(&step.id).is_some_and(|record| {
                        record.status == StepStatus::Succeeded
                            && record.skip_reason.is_none()
                            && record.attempt_id.as_deref() == Some(attempt_id)
                    })
            })
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(format!(
                "被审 owner {producer} 未唯一对应当前已成功任务/attempt"
            ));
        }
        let step = matching[0];
        let attempt = step
            .input
            .get("rework")
            .and_then(|value| value.get("attempt"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if review_rework_budget_exhausted(attempt, state.goal.budget.max_retries_per_step) {
            return Err(format!("任务 {} 已耗尽评审返修预算", step.id));
        }
        let evidence = finding
            .get("evidence_refs")
            .and_then(Value::as_array)
            .map(|refs| refs.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        let severity = finding
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or("major")
            .to_string();
        let requirement_id = finding
            .get("requirement_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let finding_sha256 = crate::CasStore::hash_of(
            &serde_json::to_vec(finding).map_err(|error| format!("finding 序列化失败：{error}"))?,
        );
        let issue_id = delivery_issue_id(
            team_id,
            &artifact.artifact_id,
            &artifact.sha256,
            task_id,
            attempt_id,
            requirement_id.as_deref(),
            &severity,
            &finding_sha256,
        )
        .map_err(|error| format!("Issue 身份序列化失败：{error}"))?;
        if !seen.insert(issue_id.clone()) {
            continue;
        }
        let now = now_ts();
        issues.push(crate::goal::DeliveryIssueV1 {
            issue_id: issue_id.clone(),
            source_review_artifact_id: artifact.artifact_id.clone(),
            source_review_sha256: artifact.sha256.clone(),
            finding_sha256,
            severity,
            detail: detail.to_string(),
            requirement_id,
            target_task_id: task_id.to_string(),
            target_attempt_id: attempt_id.to_string(),
            target_artifact_id: binding
                .get("artifact_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            owner_step_id: step.id.clone(),
            status: crate::goal::DeliveryIssueStatusV1::Open,
            repair_attempt: attempt.saturating_add(1).min(u64::from(u32::MAX)) as u32,
            resolution_review_artifact_id: None,
            resolution_review_sha256: None,
            resolution_attempt_id: None,
            opened_at: now.clone(),
            updated_at: now,
        });
        let request = owners
            .entry(step.id.clone())
            .or_insert_with(|| ReworkRequest {
                step_id: step.id.clone(),
                instruction: "保持原任务、验收条件和写范围；一次修复下列全部问题，并重新验证。\n"
                    .into(),
                note: format!("评审返修 {}", artifact.artifact_id),
                actor: "reviewer".into(),
                source_id: Some(artifact.artifact_id.clone()),
                expected_attempt_id: Some(attempt_id.to_string()),
                issue_ids: Vec::new(),
            });
        request.instruction.push_str(&format!(
            "Issue {issue_id}：{detail}。依据：{}。\n",
            if evidence.is_empty() {
                "未提供可定位证据".into()
            } else {
                evidence.join(", ")
            }
        ));
        request.issue_ids.push(issue_id);
        if request.instruction.len() > 32 * 1024 {
            return Err(format!(
                "任务 {} 的完整修复指令超过 32 KiB，不能静默遗漏 finding",
                step.id
            ));
        }
    }
    Ok(ReviewChanges {
        issues,
        repairs: owners.into_values().collect(),
    })
}

pub(super) fn combine_review_requests(
    requests: Vec<ReworkRequest>,
) -> Result<Vec<ReworkRequest>, String> {
    let mut owners: BTreeMap<String, (ReworkRequest, std::collections::BTreeSet<String>)> =
        BTreeMap::new();
    for request in requests {
        let source = request
            .source_id
            .clone()
            .ok_or_else(|| "评审返修缺少来源".to_string())?;
        match owners.entry(request.step_id.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((request, [source].into_iter().collect()));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let (existing, sources) = entry.get_mut();
                if existing.expected_attempt_id != request.expected_attempt_id {
                    return Err("多个评审引用了同一 owner 的不同 attempt".into());
                }
                existing.instruction.push_str(&request.instruction);
                existing.issue_ids.extend(request.issue_ids);
                sources.insert(source);
                if existing.instruction.len() > 32 * 1024 {
                    return Err("合并评审指令超过 32 KiB，不能截断问题".into());
                }
            }
        }
    }
    owners
        .into_values()
        .map(|(mut request, sources)| {
            request.issue_ids.sort();
            request.issue_ids.dedup();
            if sources.len() > 1 {
                let identity =
                    serde_json::to_vec(&json!({"owner": request.step_id, "reviews": sources}))
                        .map_err(|error| error.to_string())?;
                request.source_id = Some(format!(
                    "review-batch-{}",
                    crate::CasStore::hash_of(&identity)
                ));
                request.note = "多个独立评审的合并返修".into();
            }
            Ok(request)
        })
        .collect()
}

fn review_rework_budget_exhausted(attempt: u64, max_retries_per_step: u32) -> bool {
    attempt >= u64::from(max_retries_per_step)
}

/// Stable for replay of one review result, distinct when the source review or
/// target attempt changes so a previously resolved issue cannot suppress recurrence.
#[allow(clippy::too_many_arguments)] // 身份由全部来源字段共同构成，缺一不可
fn delivery_issue_id(
    team_id: &str,
    source_review_artifact_id: &str,
    source_review_sha256: &str,
    task_id: &str,
    attempt_id: &str,
    requirement_id: Option<&str>,
    severity: &str,
    finding_sha256: &str,
) -> Result<String, serde_json::Error> {
    let identity = serde_json::json!({
        "team_id": team_id,
        "source_review_artifact_id": source_review_artifact_id,
        "source_review_sha256": source_review_sha256,
        "task_id": task_id,
        "attempt_id": attempt_id,
        "requirement_id": requirement_id,
        "severity": severity,
        "finding_sha256": finding_sha256,
    });
    let identity_bytes = serde_json::to_vec(&identity)?;
    Ok(format!(
        "issue-{}",
        crate::CasStore::hash_of(&identity_bytes)
    ))
}

/// Resolve a finding only to a reviewed host-bound Artifact. Old findings remain
/// compatible when the suggested producer owns exactly one reviewed task.
pub(super) fn resolve_review_issues(
    state: &mut GoalRunState,
    review_artifact_id: &str,
    review_sha256: &str,
    reviewed_artifacts: &[Value],
) -> usize {
    let now = now_ts();
    let mut resolved = 0usize;
    for binding in reviewed_artifacts {
        let Some(task_id) = binding.get("task_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(attempt_id) = binding.get("attempt_id").and_then(Value::as_str) else {
            continue;
        };
        let matching_steps = state
            .plan
            .steps
            .iter()
            .filter(|step| {
                step.id == task_id
                    || step.input.get("assigned_task_id").and_then(Value::as_str) == Some(task_id)
            })
            .collect::<Vec<_>>();
        let owner_step_id = match matching_steps.as_slice() {
            [step] => step.id.as_str(),
            [] if state.records.contains_key(task_id) => task_id,
            _ => continue,
        };
        let current_attempt_matches = state.records.get(owner_step_id).is_some_and(|record| {
            record.status == StepStatus::Succeeded
                && record.skip_reason.is_none()
                && record.attempt_id.as_deref() == Some(attempt_id)
        });
        if !current_attempt_matches {
            continue;
        }
        for issue in &mut state.delivery_issues {
            if issue.status == crate::goal::DeliveryIssueStatusV1::RepairDispatched
                && issue.owner_step_id == owner_step_id
                && issue.target_attempt_id != attempt_id
            {
                issue.status = crate::goal::DeliveryIssueStatusV1::Resolved;
                issue.resolution_review_artifact_id = Some(review_artifact_id.to_string());
                issue.resolution_review_sha256 = Some(review_sha256.to_string());
                issue.resolution_attempt_id = Some(attempt_id.to_string());
                issue.updated_at = now.clone();
                resolved += 1;
            }
        }
    }
    resolved
}

fn select_reviewed_owner_binding<'a>(reviewed: &'a [Value], finding: &Value) -> Option<&'a Value> {
    let owner = finding
        .get("suggested_owner")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let task_id = finding
        .get("target_task_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let artifact_id = finding
        .get("target_artifact_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let matches = reviewed
        .iter()
        .filter(|artifact| {
            owner.is_none_or(|owner| {
                artifact.get("producer").and_then(Value::as_str) == Some(owner)
                    || artifact.get("task_id").and_then(Value::as_str) == Some(owner)
                    || artifact.get("role").and_then(Value::as_str) == Some(owner)
            }) && task_id.is_none_or(|task_id| {
                artifact.get("task_id").and_then(Value::as_str) == Some(task_id)
            }) && artifact_id.is_none_or(|artifact_id| {
                artifact.get("artifact_id").and_then(Value::as_str) == Some(artifact_id)
            })
        })
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0])
}
#[cfg(test)]
mod review_owner_binding_tests {
    use super::select_reviewed_owner_binding;
    use serde_json::{json, Value};

    fn reviewed() -> Vec<Value> {
        vec![
            json!({"producer":"m-w1", "task_id":"step-a", "artifact_id":"artifact-a", "role":"builder"}),
            json!({"producer":"m-w1", "task_id":"step-b", "artifact_id":"artifact-b", "role":"builder"}),
        ]
    }

    #[test]
    fn legacy_owner_binding_remains_compatible_when_unique() {
        let finding = json!({"suggested_owner":"m-w1"});
        let only_artifact = vec![reviewed()[0].clone()];
        assert_eq!(
            select_reviewed_owner_binding(&only_artifact, &finding)
                .and_then(|artifact| artifact.get("task_id"))
                .and_then(Value::as_str),
            Some("step-a")
        );
    }

    #[test]
    fn ambiguous_legacy_owner_does_not_pick_the_first_task() {
        let finding = json!({"suggested_owner":"m-w1"});
        assert!(select_reviewed_owner_binding(&reviewed(), &finding).is_none());
    }

    #[test]
    fn suggested_owner_may_be_a_task_id_or_role_name() {
        let by_task = json!({"suggested_owner":"step-b"});
        assert_eq!(
            select_reviewed_owner_binding(&reviewed(), &by_task)
                .and_then(|artifact| artifact.get("artifact_id"))
                .and_then(Value::as_str),
            Some("artifact-b")
        );
        let by_role = json!({"suggested_owner":"builder", "target_task_id":"step-a"});
        assert_eq!(
            select_reviewed_owner_binding(&reviewed(), &by_role)
                .and_then(|artifact| artifact.get("artifact_id"))
                .and_then(Value::as_str),
            Some("artifact-a")
        );
    }

    #[test]
    fn task_or_artifact_identity_resolves_the_exact_reviewed_task() {
        let finding = json!({"suggested_owner":"m-w1", "target_task_id":"step-b"});
        assert_eq!(
            select_reviewed_owner_binding(&reviewed(), &finding)
                .and_then(|artifact| artifact.get("artifact_id"))
                .and_then(Value::as_str),
            Some("artifact-b")
        );
        let finding = json!({"target_artifact_id":"artifact-a"});
        assert_eq!(
            select_reviewed_owner_binding(&reviewed(), &finding)
                .and_then(|artifact| artifact.get("task_id"))
                .and_then(Value::as_str),
            Some("step-a")
        );
        let inconsistent = json!({
            "target_task_id":"step-b",
            "target_artifact_id":"artifact-a"
        });
        assert!(select_reviewed_owner_binding(&reviewed(), &inconsistent).is_none());
    }
}

#[cfg(test)]
mod delivery_issue_resolution_tests {
    use super::{delivery_issue_id, resolve_review_issues};
    use crate::goal::{DeliveryIssueStatusV1, DeliveryIssueV1, Goal, GoalRunState, StepRecord};
    use crate::plan::{Plan, StepStatus};
    use serde_json::json;

    #[test]
    fn issue_identity_is_idempotent_per_review_but_changes_with_attempt_or_review() {
        let issue_id = delivery_issue_id(
            "team-1",
            "review-1",
            "review-sha-1",
            "task-1",
            "attempt-1",
            Some("requirement-1"),
            "major",
            "finding-sha-1",
        )
        .unwrap();
        assert_eq!(
            issue_id,
            delivery_issue_id(
                "team-1",
                "review-1",
                "review-sha-1",
                "task-1",
                "attempt-1",
                Some("requirement-1"),
                "major",
                "finding-sha-1",
            )
            .unwrap()
        );
        assert_ne!(
            issue_id,
            delivery_issue_id(
                "team-1",
                "review-1",
                "review-sha-1",
                "task-1",
                "attempt-2",
                Some("requirement-1"),
                "major",
                "finding-sha-1",
            )
            .unwrap()
        );
        assert_ne!(
            issue_id,
            delivery_issue_id(
                "team-1",
                "review-2",
                "review-sha-2",
                "task-1",
                "attempt-1",
                Some("requirement-1"),
                "major",
                "finding-sha-1",
            )
            .unwrap()
        );
    }

    fn state_with_issue() -> GoalRunState {
        let mut state = GoalRunState::new(
            Goal::new("issue-goal", "review issue closure"),
            Plan::new("issue-plan", "issue-goal"),
        );
        state.records.insert(
            "step-a".to_string(),
            StepRecord {
                step_id: "step-a".to_string(),
                status: StepStatus::Succeeded,
                attempts: 2,
                attempt_id: Some("attempt-new".to_string()),
                output: Some("fixed".to_string()),
                error: None,
                skip_reason: None,
                phase_epoch: Some(2),
                validation_receipts: Vec::new(),
            },
        );
        state.delivery_issues.push(DeliveryIssueV1 {
            issue_id: "issue-1".to_string(),
            source_review_artifact_id: "review-old".to_string(),
            source_review_sha256: "review-hash".to_string(),
            finding_sha256: "finding-hash".to_string(),
            severity: "major".to_string(),
            detail: "fix boundary".to_string(),
            requirement_id: Some("req-1".to_string()),
            target_task_id: "step-a".to_string(),
            target_attempt_id: "attempt-old".to_string(),
            target_artifact_id: Some("artifact-old".to_string()),
            owner_step_id: "step-a".to_string(),
            status: DeliveryIssueStatusV1::RepairDispatched,
            repair_attempt: 1,
            resolution_review_artifact_id: None,
            resolution_review_sha256: None,
            resolution_attempt_id: None,
            opened_at: "t1".to_string(),
            updated_at: "t1".to_string(),
        });
        state
    }

    #[test]
    fn approved_review_closes_only_repaired_current_task_attempts() {
        let mut state = state_with_issue();
        let reviewed = vec![json!({"task_id":"step-a","attempt_id":"attempt-new"})];

        assert_eq!(
            resolve_review_issues(&mut state, "review-new", "review-sha-new", &reviewed),
            1
        );
        let issue = &state.delivery_issues[0];
        assert_eq!(issue.status, DeliveryIssueStatusV1::Resolved);
        assert_eq!(
            issue.resolution_review_artifact_id.as_deref(),
            Some("review-new")
        );
        assert_eq!(
            issue.resolution_review_sha256.as_deref(),
            Some("review-sha-new")
        );
        assert_eq!(issue.resolution_attempt_id.as_deref(), Some("attempt-new"));
    }

    #[test]
    fn approved_review_cannot_close_an_issue_with_a_stale_attempt() {
        let mut state = state_with_issue();
        let stale_review = vec![json!({"task_id":"step-a","attempt_id":"attempt-old"})];

        assert_eq!(
            resolve_review_issues(
                &mut state,
                "review-stale",
                "review-sha-stale",
                &stale_review
            ),
            0
        );
        assert_eq!(
            state.delivery_issues[0].status,
            DeliveryIssueStatusV1::RepairDispatched
        );
    }
}

#[cfg(test)]
mod review_rework_budget_tests {
    use super::review_rework_budget_exhausted;

    #[test]
    fn review_rework_uses_the_configured_per_step_retry_budget() {
        assert!(!review_rework_budget_exhausted(0, 2));
        assert!(!review_rework_budget_exhausted(1, 2));
        assert!(review_rework_budget_exhausted(2, 2));
        assert!(!review_rework_budget_exhausted(2, 4));
        assert!(review_rework_budget_exhausted(0, 0));
        assert!(review_rework_budget_exhausted(u64::MAX, u32::MAX));
    }
}

#[cfg(test)]
mod repair_planning_tests {
    use super::*;
    use crate::goal::{DeliveryIssueStatusV1, StepRecord};

    fn state() -> GoalRunState {
        let mut plan = Plan::new("plan", "goal");
        for (id, worker) in [
            ("step-a", "m-a"),
            ("step-b", "m-b"),
            ("step-review", "m-review"),
        ] {
            let mut step = StepSpec::new(id, worker);
            step.input = json!({"assigned_task_id": format!("task-{id}"), "write_paths":[format!("{id}.rs")]});
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

    fn artifact() -> Artifact {
        serde_json::from_value(json!({
            "artifact_id":"review-1", "kind":"review", "version":1, "producer":"m-review",
            "content_ref":"cas://sha256:hash", "created_at":"same-time", "team_id":"team",
            "task_id":"step-review", "attempt_id":"attempt-step-review", "sha256":"hash"
        }))
        .unwrap()
    }

    fn document(findings: Vec<Value>) -> Value {
        json!({"result":{"verdict":"changes_requested", "findings":findings},
        "reviewed_artifacts":[
            {"task_id":"step-a","attempt_id":"attempt-step-a","producer":"m-a","artifact_id":"a"},
            {"task_id":"step-b","attempt_id":"attempt-step-b","producer":"m-b","artifact_id":"b"}
        ]})
    }

    fn finding(task: &str, detail: &str) -> Value {
        json!({"target_task_id":task,"detail":detail,"severity":"major","evidence_refs":["source"]})
    }

    #[test]
    fn all_findings_group_by_owner_without_repeating_repair_rounds() {
        let state = state();
        let changes = plan_review_changes(
            "team",
            &state,
            &artifact(),
            &document(vec![
                finding("step-a", "fix zero limit"),
                finding("step-a", "restore mobile style"),
                finding("step-b", "fix query matching"),
            ]),
        )
        .unwrap();
        assert_eq!(changes.issues.len(), 3);
        assert_eq!(changes.repairs.len(), 2);
        let owner = changes
            .repairs
            .iter()
            .find(|request| request.step_id == "step-a")
            .unwrap();
        assert_eq!(owner.issue_ids.len(), 2);
        assert!(owner.instruction.contains("fix zero limit"));
        assert!(owner.instruction.contains("restore mobile style"));
        assert!(
            state.delivery_issues.is_empty(),
            "planning must not mutate authoritative state"
        );
    }

    #[test]
    fn malformed_second_finding_rejects_entire_plan() {
        assert!(plan_review_changes(
            "team",
            &state(),
            &artifact(),
            &document(vec![
                finding("step-a", "valid"),
                json!({"target_task_id":"step-b"})
            ])
        )
        .is_err());
    }

    #[test]
    fn stale_owner_attempt_and_self_review_cannot_dispatch() {
        let mut stale = document(vec![finding("step-a", "fix")]);
        stale["reviewed_artifacts"][0]["attempt_id"] = json!("old");
        assert!(plan_review_changes("team", &state(), &artifact(), &stale).is_err());
        let mut own = artifact();
        own.producer = "m-a".into();
        assert!(plan_review_changes(
            "team",
            &state(),
            &own,
            &document(vec![finding("step-a", "fix")])
        )
        .is_err());
    }

    #[test]
    fn duplicate_finding_is_one_issue_and_one_instruction() {
        let finding = finding("step-a", "fix");
        let changes = plan_review_changes(
            "team",
            &state(),
            &artifact(),
            &document(vec![finding.clone(), finding]),
        )
        .unwrap();
        assert_eq!(changes.issues.len(), 1);
        assert_eq!(changes.repairs[0].issue_ids.len(), 1);
    }

    #[test]
    fn oversized_review_or_instruction_fails_without_truncation() {
        assert!(plan_review_changes(
            "team",
            &state(),
            &artifact(),
            &document((0..129).map(|_| finding("step-a", "fix")).collect())
        )
        .is_err());
        assert!(plan_review_changes(
            "team",
            &state(),
            &artifact(),
            &document(vec![finding("step-a", &"x".repeat(32 * 1024))])
        )
        .is_err());
    }

    #[test]
    fn task_alias_resolution_closes_only_new_approved_owner_attempt() {
        let mut state = state();
        let changes = plan_review_changes(
            "team",
            &state,
            &artifact(),
            &document(vec![finding("step-a", "fix")]),
        )
        .unwrap();
        state.delivery_issues = changes.issues;
        state.delivery_issues[0].status = DeliveryIssueStatusV1::RepairDispatched;
        state.records.get_mut("step-a").unwrap().attempt_id = Some("repaired".into());
        let bindings = vec![json!({"task_id":"task-step-a","attempt_id":"repaired"})];
        assert_eq!(
            resolve_review_issues(&mut state, "review-new", "new", &bindings),
            1
        );
        assert_eq!(
            state.delivery_issues[0].status,
            DeliveryIssueStatusV1::Resolved
        );
    }

    #[test]
    fn multiple_reviewers_share_one_owner_repair_with_distinct_issue_ids() {
        let state = state();
        let first = plan_review_changes(
            "team",
            &state,
            &artifact(),
            &document(vec![finding("step-a", "first")]),
        )
        .unwrap();
        let mut second_artifact = artifact();
        second_artifact.artifact_id = "review-2".into();
        let second = plan_review_changes(
            "team",
            &state,
            &second_artifact,
            &document(vec![finding("step-a", "second")]),
        )
        .unwrap();
        let mut requests = first.repairs;
        requests.extend(second.repairs);
        let merged = combine_review_requests(requests).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].issue_ids.len(), 2);
        assert!(merged[0]
            .source_id
            .as_deref()
            .unwrap()
            .starts_with("review-batch-"));
        assert!(
            merged[0].instruction.contains("first") && merged[0].instruction.contains("second")
        );
    }
}
