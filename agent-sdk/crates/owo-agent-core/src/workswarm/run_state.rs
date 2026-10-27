//! Pure reducer for durable Team phase-execution state transitions.
//!
//! Attempt admission, progress/phase projection, retry/continue resets, cancellation,
//! interruption recovery, and prepared rework resets pass through this module.
//! Coordinators retain control-plane validation, plan edits, decision records, locking,
//! persistence, member health, and external side effects.
//! Only `AttemptAdmitted` may advance durable run-wide budget counters; projected
//! runner snapshots can update records but never own those counters.
use crate::goal::{GoalRunState, StepProgressUpdate};
use crate::plan::StepStatus;
use std::collections::HashSet;

pub(super) struct MergedStepProgress {
    pub(super) state_changed: bool,
    pub(super) skip_events: Vec<(String, String, String)>,
    pub(super) dynamic_skips: Vec<(String, String)>,
}

pub(super) enum RunExecutionEvent<'a> {
    AttemptAdmitted {
        phase_epoch: u64,
        step_id: &'a str,
        attempt_id: &'a str,
        is_retry: bool,
    },
    ProgressBatch(Vec<StepProgressUpdate>),
    PhaseProjection(&'a GoalRunState),
    ReworkBatch {
        items: Vec<ReworkStepMutation>,
        affected_step_ids: Vec<String>,
        requested_at: String,
    },
    RetryReset {
        target_step_id: String,
        affected_step_ids: Vec<String>,
        note: String,
    },
    ContinueReset,
    CancelRun,
    MarkInterrupted {
        error: String,
    },
}

pub(super) enum RunExecutionEffect {
    AttemptAdmitted,
    ProgressMerged(MergedStepProgress),
    PhaseProjected,
    ReworkApplied(Vec<String>),
    RetryApplied(Vec<String>),
    ContinueApplied(Vec<String>),
    RunCancelled,
    RunInterrupted(Vec<String>),
}

pub(super) struct ReworkStepMutation {
    pub(super) step_id: String,
    pub(super) instruction: String,
    pub(super) note: String,
    pub(super) actor: String,
    pub(super) source_id: Option<String>,
    pub(super) issue_ids: Vec<String>,
    pub(super) attempt: u64,
}

/// Apply one fenced event to the in-memory durable-state candidate. Persistence remains
/// the caller's transaction boundary; callers must only publish effects after commit.
pub(super) fn apply_run_execution_event(
    state: &mut GoalRunState,
    event: RunExecutionEvent<'_>,
) -> Result<RunExecutionEffect, String> {
    match event {
        RunExecutionEvent::AttemptAdmitted {
            phase_epoch,
            step_id,
            attempt_id,
            is_retry,
        } => {
            let record = state
                .records
                .get(step_id)
                .ok_or_else(|| format!("attempt admission references unknown step: {step_id}"))?;
            if record.phase_epoch != Some(phase_epoch)
                || record.attempt_id.as_deref() != Some(attempt_id)
            {
                return Err(format!("attempt admission is stale for step {step_id}"));
            }

            state.steps_taken = state.steps_taken.saturating_add(1);
            if is_retry {
                state.total_retries = state.total_retries.saturating_add(1);
            }
            let record = state
                .records
                .get_mut(step_id)
                .expect("record checked above");
            record.attempts = record.attempts.saturating_add(1);
            Ok(RunExecutionEffect::AttemptAdmitted)
        }
        RunExecutionEvent::ProgressBatch(updates) => Ok(RunExecutionEffect::ProgressMerged(
            merge_progress_batch(state, updates),
        )),
        RunExecutionEvent::PhaseProjection(phase) => {
            let phase_epoch = phase.execution_epoch;
            let phase_step_ids: HashSet<&str> = phase
                .plan
                .steps
                .iter()
                .map(|step| step.id.as_str())
                .collect();
            for (step_id, phase_record) in &phase.records {
                if !phase_step_ids.contains(step_id.as_str()) {
                    continue;
                }
                if let Some(durable_record) = state.records.get_mut(step_id) {
                    // A phase snapshot may project only the exact host-claimed task attempt.
                    // Epoch fences the run; attempt_id fences same-epoch retries and takeover.
                    if durable_record.status == StepStatus::Succeeded
                        || durable_record.phase_epoch != Some(phase_epoch)
                        || phase_record.phase_epoch != Some(phase_epoch)
                        || durable_record.attempt_id != phase_record.attempt_id
                    {
                        continue;
                    }
                    let admitted_attempts = durable_record.attempts;
                    *durable_record = phase_record.clone();
                    durable_record.attempts = durable_record.attempts.max(admitted_attempts);
                }
            }
            if phase.goal.error.is_some() {
                state.goal.error = phase.goal.error.clone();
            }
            Ok(RunExecutionEffect::PhaseProjected)
        }
        RunExecutionEvent::ReworkBatch {
            items,
            affected_step_ids,
            requested_at,
        } => {
            let affected = affected_step_ids
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            for item in &items {
                let current_attempt = state
                    .records
                    .get(&item.step_id)
                    .and_then(|record| record.attempt_id.as_deref());
                for issue in &mut state.delivery_issues {
                    let explicitly_bound = item.issue_ids.contains(&issue.issue_id);
                    let legacy_bound = item.issue_ids.is_empty()
                        && current_attempt == Some(issue.target_attempt_id.as_str())
                        && item.source_id.as_deref()
                            == Some(issue.source_review_artifact_id.as_str());
                    if item.actor == "reviewer"
                        && (explicitly_bound || legacy_bound)
                        && issue.owner_step_id == item.step_id
                        && issue.status == crate::goal::DeliveryIssueStatusV1::Open
                    {
                        issue.status = crate::goal::DeliveryIssueStatusV1::RepairDispatched;
                        issue.repair_attempt = item.attempt.min(u64::from(u32::MAX)) as u32;
                        issue.updated_at = requested_at.clone();
                    }
                }
                let step = state
                    .plan
                    .steps
                    .iter_mut()
                    .find(|step| step.id == item.step_id)
                    .expect("prepared owner remains in the plan");
                step.input
                    .as_object_mut()
                    .expect("prepared input is an object")
                    .insert(
                        "rework".into(),
                        serde_json::json!({
                            "instruction": item.instruction.trim(),
                            "note": item.note,
                            "attempt": item.attempt,
                            "requested_at": requested_at,
                            "source_id": item.source_id,
                            "issue_ids": item.issue_ids,
                        }),
                    );
            }
            let affected_lookup = affected.iter().map(String::as_str).collect::<HashSet<_>>();
            for record in state.records.values_mut() {
                if affected_lookup.contains(record.step_id.as_str()) {
                    reset_record_for_rerun(
                        record,
                        "retry/rework invalidated the prior attempt receipt",
                    );
                }
            }
            prepare_goal_for_rerun(state);
            Ok(RunExecutionEffect::ReworkApplied(
                affected.into_iter().collect(),
            ))
        }
        RunExecutionEvent::RetryReset {
            target_step_id,
            affected_step_ids,
            note,
        } => {
            let affected = affected_step_ids.into_iter().collect::<HashSet<_>>();
            for record in state.records.values_mut() {
                if affected.contains(&record.step_id) && record.status != StepStatus::Succeeded {
                    reset_record_for_rerun(
                        record,
                        "retry/rework invalidated the prior attempt receipt",
                    );
                }
            }
            let retried_at = super::util::now_ts();
            for issue in &mut state.delivery_issues {
                if !affected.contains(issue.owner_step_id.as_str())
                    || issue.status != crate::goal::DeliveryIssueStatusV1::Resolved
                {
                    continue;
                }
                issue.status = crate::goal::DeliveryIssueStatusV1::RepairDispatched;
                issue.repair_attempt = issue.repair_attempt.saturating_add(1);
                issue.resolution_review_artifact_id = None;
                issue.resolution_review_sha256 = None;
                issue.resolution_attempt_id = None;
                issue.updated_at = retried_at.clone();
            }
            if let Some(step) = state.plan.step_mut(&target_step_id) {
                if let Some(input) = step.input.as_object_mut() {
                    let metadata = input
                        .entry("_workswarm")
                        .or_insert_with(|| serde_json::json!({}));
                    if let Some(metadata) = metadata.as_object_mut() {
                        metadata.insert(
                            "retry_note".to_string(),
                            serde_json::json!(note.chars().take(1600).collect::<String>()),
                        );
                    }
                }
            }
            prepare_goal_for_rerun(state);
            Ok(RunExecutionEffect::RetryApplied(
                affected.into_iter().collect(),
            ))
        }
        RunExecutionEvent::ContinueReset => {
            let mut affected = Vec::new();
            for record in state.records.values_mut() {
                if !record.status.is_terminal() || record.status == StepStatus::Aborted {
                    reset_record_for_rerun(
                        record,
                        "continue invalidated the prior attempt receipt",
                    );
                    affected.push(record.step_id.clone());
                }
            }
            affected.sort();
            prepare_goal_for_rerun(state);
            Ok(RunExecutionEffect::ContinueApplied(affected))
        }
        RunExecutionEvent::CancelRun => {
            state.aborted = true;
            for record in state.records.values_mut() {
                if !record.status.is_terminal() {
                    record.status = StepStatus::Aborted;
                }
            }
            if !state.goal.status.is_terminal() {
                state.goal.transition(crate::goal::GoalStatus::Aborted);
            }
            Ok(RunExecutionEffect::RunCancelled)
        }
        RunExecutionEvent::MarkInterrupted { error } => {
            let mut interrupted = Vec::new();
            for record in state.records.values_mut() {
                if record.status == StepStatus::Running {
                    record.status = StepStatus::Aborted;
                    record.error = Some(error.clone());
                    interrupted.push(record.step_id.clone());
                }
            }
            interrupted.sort();
            Ok(RunExecutionEffect::RunInterrupted(interrupted))
        }
    }
}

fn reset_record_for_rerun(record: &mut crate::goal::StepRecord, receipt_detail: &str) {
    record.status = StepStatus::Pending;
    record.attempts = 0;
    record.attempt_id = None;
    record.phase_epoch = None;
    record.output = None;
    record.error = None;
    record.skip_reason = None;
    for receipt in &mut record.validation_receipts {
        receipt.verdict = crate::plan::ValidationVerdictV1::Stale;
        receipt.detail = Some(receipt_detail.to_string());
    }
}

fn prepare_goal_for_rerun(state: &mut GoalRunState) {
    state.aborted = false;
    if state.goal.status.is_terminal() {
        state.goal.transition(crate::goal::GoalStatus::Pending);
    }
    state.goal.error = None;
}

fn merge_progress_batch(
    state: &mut GoalRunState,
    updates: Vec<StepProgressUpdate>,
) -> MergedStepProgress {
    let mut record_changed = false;
    let mut skip_events = Vec::new();
    let mut dynamic_skips = Vec::new();

    for update in updates {
        let StepProgressUpdate {
            step_id,
            worker,
            record: mut next,
            skip_reason,
            ..
        } = update;
        let skip_reason = skip_reason.or_else(|| next.skip_reason.clone());
        if skip_reason.is_some() {
            next.skip_reason = skip_reason.clone();
        }

        let Some(current) = state.records.get_mut(&step_id) else {
            continue;
        };
        // Epoch fences the whole phase; attempt identity also fences same-phase rework.
        if current.phase_epoch != next.phase_epoch || current.attempt_id != next.attempt_id {
            continue;
        }
        if let Some(reason) = skip_reason {
            let role = worker.strip_prefix("m-").unwrap_or(&worker).to_string();
            dynamic_skips.push((role.clone(), reason.clone()));
            skip_events.push((step_id.clone(), role, reason));
        }
        // Admission durably accounts attempts before execution. Progress snapshots may lag.
        next.attempts = next.attempts.max(current.attempts);
        let changed = current.status != next.status
            || current.attempts != next.attempts
            || current.output != next.output
            || current.error != next.error
            || current.skip_reason != next.skip_reason
            || current.validation_receipts != next.validation_receipts;
        if changed {
            *current = next;
            record_changed = true;
        }
    }

    MergedStepProgress {
        state_changed: record_changed,
        skip_events,
        dynamic_skips,
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_run_execution_event, RunExecutionEffect, RunExecutionEvent};
    use crate::goal::{Goal, GoalRunState, GoalStatus};
    use crate::plan::{Plan, StepSpec, StepStatus, ValidationReceiptV1, ValidationVerdictV1};
    use std::collections::HashMap;

    fn passed_receipt(step_id: &str, attempt_id: &str) -> ValidationReceiptV1 {
        ValidationReceiptV1 {
            receipt_id: format!("receipt-{step_id}"),
            task_id: step_id.to_string(),
            attempt_id: attempt_id.to_string(),
            epoch: 1,
            requirement_id: "behavior".to_string(),
            validator_id: "workspace-command-success-v1".to_string(),
            validator_version: "1".to_string(),
            arguments_sha256: "args".to_string(),
            input_sha256: "input".to_string(),
            environment_id: "workspace".to_string(),
            changeset_sha256: None,
            detail: None,
            review_result: None,
            subject_sha256: HashMap::new(),
            verdict: ValidationVerdictV1::Passed,
            evidence_refs: Vec::new(),
            started_at: "start".to_string(),
            completed_at: "finish".to_string(),
        }
    }

    #[test]
    fn continue_reset_invalidates_only_resumable_attempts_and_their_receipts() {
        let mut plan = Plan::new("plan", "goal");
        for id in ["done", "pending", "interrupted", "failed"] {
            plan.add_step(StepSpec::new(id, format!("worker-{id}")));
        }
        let mut state = GoalRunState::new(Goal::new("goal", "objective"), plan);
        state.steps_taken = 7;
        state.total_retries = 3;
        state.aborted = true;
        state.goal.transition(GoalStatus::Aborted);
        state.goal.error = Some("interrupted".to_string());

        for id in ["done", "pending", "interrupted", "failed"] {
            let record = state.records.get_mut(id).unwrap();
            record.attempts = 2;
            record.attempt_id = Some(format!("attempt-{id}"));
            record.phase_epoch = Some(9);
            record.output = Some(format!("output-{id}"));
            record.error = Some(format!("error-{id}"));
            record.skip_reason = Some(format!("skip-{id}"));
            record
                .validation_receipts
                .push(passed_receipt(id, &format!("attempt-{id}")));
        }
        state.records.get_mut("done").unwrap().status = StepStatus::Succeeded;
        state.records.get_mut("failed").unwrap().status = StepStatus::Failed;
        state.records.get_mut("interrupted").unwrap().status = StepStatus::Aborted;

        let effect =
            apply_run_execution_event(&mut state, RunExecutionEvent::ContinueReset).unwrap();
        let RunExecutionEffect::ContinueApplied(reset_ids) = effect else {
            panic!("continue reset returned the wrong effect");
        };
        assert_eq!(reset_ids, vec!["interrupted", "pending"]);

        for id in ["pending", "interrupted"] {
            let record = &state.records[id];
            assert_eq!(record.status, StepStatus::Pending);
            assert_eq!(record.attempts, 0);
            assert_eq!(record.attempt_id, None);
            assert_eq!(record.phase_epoch, None);
            assert_eq!(record.output, None);
            assert_eq!(record.error, None);
            assert_eq!(record.skip_reason, None);
            assert_eq!(
                record.validation_receipts[0].verdict,
                ValidationVerdictV1::Stale
            );
        }
        assert_eq!(state.records["done"].status, StepStatus::Succeeded);
        assert_eq!(state.records["done"].attempts, 2);
        assert_eq!(
            state.records["done"].validation_receipts[0].verdict,
            ValidationVerdictV1::Passed
        );
        assert_eq!(state.records["failed"].status, StepStatus::Failed);
        assert_eq!(state.steps_taken, 7);
        assert_eq!(state.total_retries, 3);
        assert!(!state.aborted);
        assert_eq!(state.goal.status, GoalStatus::Pending);
        assert_eq!(state.goal.error, None);
    }

    #[test]
    fn stale_phase_projection_cannot_replace_a_newer_attempt_in_the_same_epoch() {
        let mut plan = Plan::new("plan", "goal");
        plan.add_step(StepSpec::new("step-a", "worker-a"));
        let mut durable = GoalRunState::new(Goal::new("goal", "objective"), plan);
        durable.steps_taken = 3;
        durable.total_retries = 1;
        let current = durable.records.get_mut("step-a").unwrap();
        current.status = StepStatus::Running;
        current.attempts = 3;
        current.attempt_id = Some("attempt-current".to_string());
        current.phase_epoch = Some(7);
        current.output = Some("current output".to_string());

        let mut stale_phase = durable.clone();
        stale_phase.execution_epoch = 7;
        let stale = stale_phase.records.get_mut("step-a").unwrap();
        stale.status = StepStatus::Failed;
        stale.attempts = 99;
        stale.attempt_id = Some("attempt-stale".to_string());
        stale.phase_epoch = Some(7);
        stale.output = Some("stale output".to_string());

        apply_run_execution_event(
            &mut durable,
            RunExecutionEvent::PhaseProjection(&stale_phase),
        )
        .unwrap();

        let current = &durable.records["step-a"];
        assert_eq!(current.status, StepStatus::Running);
        assert_eq!(current.attempts, 3);
        assert_eq!(current.attempt_id.as_deref(), Some("attempt-current"));
        assert_eq!(current.output.as_deref(), Some("current output"));
        assert_eq!(durable.steps_taken, 3);
        assert_eq!(durable.total_retries, 1);
    }

    #[test]
    fn current_phase_projection_merges_attempt_result_and_preserves_admission_high_water() {
        let mut plan = Plan::new("plan", "goal");
        plan.add_step(StepSpec::new("step-a", "worker-a"));
        let mut durable = GoalRunState::new(Goal::new("goal", "objective"), plan);
        durable.steps_taken = 2;
        durable.total_retries = 1;
        let current = durable.records.get_mut("step-a").unwrap();
        current.status = StepStatus::Running;
        current.attempts = 2;
        current.attempt_id = Some("attempt-current".to_string());
        current.phase_epoch = Some(7);

        let mut completed_phase = durable.clone();
        completed_phase.execution_epoch = 7;
        let completed = completed_phase.records.get_mut("step-a").unwrap();
        completed.status = StepStatus::Succeeded;
        completed.attempts = 1;
        completed.attempt_id = Some("attempt-current".to_string());
        completed.phase_epoch = Some(7);
        completed.output = Some("verified output".to_string());

        apply_run_execution_event(
            &mut durable,
            RunExecutionEvent::PhaseProjection(&completed_phase),
        )
        .unwrap();

        let current = &durable.records["step-a"];
        assert_eq!(current.status, StepStatus::Succeeded);
        assert_eq!(current.attempts, 2);
        assert_eq!(current.output.as_deref(), Some("verified output"));
        assert_eq!(durable.steps_taken, 2);
        assert_eq!(durable.total_retries, 1);
    }
}
