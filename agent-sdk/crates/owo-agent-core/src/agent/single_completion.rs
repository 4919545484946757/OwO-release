//! Host-owned Single completion and validation evidence.
//!
//! Agent::run_turn orchestrates model/tool execution; this module checks the
//! request-bound plan, source snapshot, behavior receipts and stale evidence.
//! Independent review and explicit human acceptance stay in their sibling modules.

use super::TurnEvent;
#[cfg(test)]
use super::{single_manual_acceptance, single_review, CommandExecutionReceipt};
use crate::session::Session;

pub(super) fn single_workspace_path_matches(
    root: &std::path::Path,
    relative: &str,
    expected: &str,
) -> bool {
    let absent_digest = crate::verification::workspace_path_absence_sha256();
    let Ok(canonical_root) = root.canonicalize() else {
        return false;
    };
    match workspace_file_hash(&canonical_root, relative) {
        Some(Some(current)) => expected != absent_digest && current == expected,
        Some(None) => expected == absent_digest,
        None => false,
    }
}

/// Read a workspace path without following it outside the session root.
/// Some(None) is a known-absent file; None means the host could not prove its state.
pub(super) fn workspace_file_hash(
    root: &std::path::Path,
    relative: &str,
) -> Option<Option<String>> {
    crate::command_evidence::read_workspace_path(root, relative)
}

/// Recheck prior Single receipts, consume pending host writes only after a registered
/// behavior command succeeds on their exact final bytes, and keep old evidence stale
/// when workspace files change outside the accepted snapshot.
pub(super) fn assess_single_turn_completion(
    session: &mut Session,
    prompt: &str,
    turn_id: &str,
    events: &[TurnEvent],
    reached_turn_limit: bool,
    final_text: Option<&str>,
) -> owo_agent_protocol::CompletionStatusV1 {
    use crate::plan::ValidationVerdictV1;

    let decide = |response_finished,
                  reached_turn_limit,
                  has_candidate_changes,
                  required_validation_count,
                  passed_required_validation_count,
                  failed_required_validation_count,
                  stale_evidence| {
        crate::completion::decide_completion(crate::completion::CompletionEvidence {
            response_finished,
            reached_turn_limit,
            has_candidate_changes,
            required_validation_count,
            passed_required_validation_count,
            failed_required_validation_count,
            stale_evidence,
            ..crate::completion::CompletionEvidence::default()
        })
    };

    let root = session.workspace.canonicalize().ok();
    if let Some(root) = root.as_deref() {
        let mut prior_snapshot = crate::workspace_snapshot::WorkspaceSnapshotBatch::new(Some(root));
        for receipt in &mut session.validation_receipts {
            if receipt.verdict != ValidationVerdictV1::Passed {
                continue;
            }
            // Artifact/output identities are not workspace paths and must not
            // become stale merely because they use another subject namespace.
            let stale = !prior_snapshot.subjects_match(&receipt.subject_sha256);
            if stale {
                receipt.verdict = ValidationVerdictV1::Stale;
                receipt.detail = Some("Single 工作区源码已变化，原行为验证收据失效".to_string());
                receipt.completed_at = chrono::Utc::now().to_rfc3339();
                let stale_id = receipt.receipt_id.clone();
                for execution in &mut session.execution_receipts {
                    if execution.validation_receipt_id.as_deref() == Some(stale_id.as_str()) {
                        execution.status = "stale".to_string();
                    }
                }
            }
        }
    }

    if reached_turn_limit || final_text.is_none_or(|text| text.trim().is_empty()) {
        return decide(false, reached_turn_limit, true, 0, 0, 0, false);
    }

    let mut pending_hashes = std::collections::BTreeMap::new();
    let mut latest_receipt_by_path = std::collections::BTreeMap::new();
    let mut stale_candidate = false;
    let mut missing_write_hash = false;
    for (index, execution) in session.execution_receipts.iter().enumerate() {
        // Completion status describes this user turn. A stale receipt from older
        // work must not turn an unrelated answer into an unverified result.
        if execution.status == "stale" && execution.turn_id == turn_id {
            stale_candidate = true;
        }
        if execution.status != "executed" {
            continue;
        }
        for relative in &execution.changed_files {
            let normalized = relative.replace('\\', "/");
            let Some((_, expected)) = execution
                .after_hashes
                .iter()
                .find(|(path, _)| path.replace('\\', "/") == normalized)
            else {
                missing_write_hash = true;
                continue;
            };
            pending_hashes.insert(normalized.clone(), expected.clone());
            latest_receipt_by_path.insert(normalized, (index, execution.receipt_id.clone()));
        }
    }
    let input_sha256 = crate::CasStore::hash_of(prompt.as_bytes());
    let plan = session.single_verification_plan.clone().filter(|_| {
        session.single_verification_plan_input_sha256.as_deref() == Some(input_sha256.as_str())
            && session.single_verification_plan_turn_id.as_deref() == Some(turn_id)
    });
    let has_current_turn_candidate = session.execution_receipts.iter().any(|execution| {
        execution.turn_id == turn_id
            && execution.status == "executed"
            && !execution.changed_files.is_empty()
    });
    if !has_current_turn_candidate && plan.is_none() {
        // Unaccepted files from prior turns do not turn ordinary conversation into a
        // code candidate. A user can explicitly start a new verification turn by
        // registering a fresh request-bound plan.
        return decide(true, false, false, 0, 0, 0, stale_candidate);
    }
    if missing_write_hash {
        return decide(true, false, true, 1, 0, 0, true);
    }
    if pending_hashes.is_empty() && plan.is_none() {
        return decide(true, false, false, 0, 0, 0, stale_candidate);
    }
    if root.is_none() {
        let has_candidate = !pending_hashes.is_empty() || plan.is_some();
        return decide(true, false, has_candidate, 1, 0, 0, true);
    }
    let root = root.expect("checked above");
    let Some(plan) = plan else {
        // A generic successful command is not enough to claim that a task's declared
        // requirements were covered. The model must register a host-resolvable plan.
        return decide(true, false, !pending_hashes.is_empty(), 0, 0, 0, false);
    };
    execute_single_verification_plan(
        session,
        &plan,
        &input_sha256,
        turn_id,
        events,
        &pending_hashes,
        &root,
    )
}

pub(super) fn single_verification_plan_matches_turn(
    session: &Session,
    prompt: &str,
    turn_id: &str,
) -> bool {
    let input_sha256 = crate::CasStore::hash_of(prompt.as_bytes());
    session.single_verification_plan.is_some()
        && session.single_verification_plan_input_sha256.as_deref() == Some(input_sha256.as_str())
        && session.single_verification_plan_turn_id.as_deref() == Some(turn_id)
}

pub(super) fn single_validation_retry_feedback(
    session: &Session,
    turn_id: &str,
) -> Option<(String, String)> {
    let plan = session.single_verification_plan.as_ref()?;
    let required_ids = plan
        .requirements
        .iter()
        .filter(|requirement| requirement.required)
        .map(|requirement| requirement.requirement_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut latest = std::collections::BTreeMap::new();
    for receipt in session
        .validation_receipts
        .iter()
        .filter(|receipt| receipt.attempt_id.as_str() == turn_id)
    {
        latest.insert(receipt.requirement_id.as_str(), receipt);
    }
    let failures = latest
        .into_iter()
        .filter(|(requirement_id, receipt)| {
            (required_ids.contains(*requirement_id)
                || receipt.validator_id == "workspace-independent-review-v1"
                || (*requirement_id == "host-change-scope-coverage"
                    && receipt.validator_id == "host-change-scope-coverage-v1"))
                && !matches!(
                    receipt.verdict,
                    crate::plan::ValidationVerdictV1::Passed
                        | crate::plan::ValidationVerdictV1::ManualAccepted
                )
        })
        .map(|(requirement_id, receipt)| {
            serde_json::json!({
                "requirement_id": requirement_id,
                "verdict": format!("{:?}", receipt.verdict),
                "detail": receipt.detail,
                "subject_sha256": receipt.subject_sha256,
                "changeset_sha256": receipt.changeset_sha256,
                "evidence_refs": receipt.evidence_refs,
            })
        })
        .collect::<Vec<_>>();
    if failures.is_empty() {
        return None;
    }
    let fingerprint_failures = failures
        .iter()
        .map(|failure| {
            let mut value = failure.clone();
            if value
                .get("requirement_id")
                .and_then(serde_json::Value::as_str)
                == Some("host-independent-review")
            {
                if let Some(object) = value.as_object_mut() {
                    object.remove("detail");
                    object.remove("evidence_refs");
                }
            }
            value
        })
        .collect::<Vec<_>>();
    let fingerprint = crate::CasStore::hash_of(
        serde_json::to_vec(&fingerprint_failures)
            .unwrap_or_default()
            .as_slice(),
    );
    let details = failures
        .iter()
        .map(|failure| serde_json::to_string(failure).unwrap_or_else(|_| "{}".to_string()))
        .collect::<Vec<_>>()
        .join("\n");
    Some((
        fingerprint,
        format!(
            "宿主已按本回合冻结的 VerificationPlan 检查最终工作区，但必需验收尚未通过。请依据以下结构化结果修复实现；如果需要行为命令，使用计划登记的命令，并在所有写入之后运行。不要替换或降低计划，也不要声称任务已验证通过。\n{details}"
        ),
    ))
}

pub(super) fn single_missing_verification_plan_feedback(
    session: &Session,
    turn_id: &str,
) -> Option<(String, String)> {
    let mut changed = std::collections::BTreeMap::<String, Option<String>>::new();
    for receipt in session
        .execution_receipts
        .iter()
        .filter(|receipt| receipt.turn_id == turn_id && receipt.status == "executed")
    {
        for path in &receipt.changed_files {
            let normalized = path.replace('\\', "/");
            let hash = receipt
                .after_hashes
                .iter()
                .find(|(candidate, _)| candidate.replace('\\', "/") == normalized)
                .map(|(_, hash)| hash.clone())
                .unwrap_or(None);
            changed.insert(normalized, hash);
        }
    }
    if changed.is_empty() {
        return None;
    }

    let fingerprint =
        crate::CasStore::hash_of(serde_json::to_vec(&changed).unwrap_or_default().as_slice());
    let changed_files = changed
        .iter()
        .map(|(path, hash)| {
            format!(
                "- {}  sha256:{}",
                path,
                hash.as_deref().unwrap_or("missing-write-hash")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some((
        fingerprint,
        format!(
            "宿主发现本回合写入了文件，但当前用户请求没有绑定有效的 VerificationPlan，因此这些变更仍是候选结果。请不要结束回合或声称已验证：先调用 verification_plan，为每个相关用户验收点登记当前请求原文中的精确引用、覆盖的变更路径和宿主已登记的检查；源码变更还需要登记并实际运行获准的行为命令。之后根据真实检查结果修复失败并重新验收。不得降低、替换验收要求。\n本回合变更：\n{changed_files}\n若显示 missing-write-hash，宿主无法把该文件绑定到写入后的版本；修复或重写后需取得有效宿主写入收据。"
        ),
    ))
}

pub(super) fn single_path_is_source_code(path: &str) -> bool {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "py"
            | "go"
            | "java"
            | "cs"
            | "cpp"
            | "c"
            | "h"
            | "hpp"
            | "vue"
            | "svelte"
            | "php"
            | "rb"
            | "swift"
            | "kt"
            | "scala"
            | "sql"
    )
}

fn execute_single_verification_plan(
    session: &mut Session,
    plan: &crate::plan::VerificationPlanV1,
    input_sha256: &str,
    turn_id: &str,
    events: &[TurnEvent],
    pending_hashes: &std::collections::BTreeMap<String, Option<String>>,
    root: &std::path::Path,
) -> owo_agent_protocol::CompletionStatusV1 {
    use crate::plan::{ValidationReceiptV1, ValidationVerdictV1, VerificationScopeV1};
    use crate::verification::{workspace_validator_arguments_supported, WorkspaceValidationBatch};

    let base_evidence = |required, passed, failed, stale| {
        crate::completion::decide_completion(crate::completion::CompletionEvidence {
            response_finished: true,
            has_candidate_changes: true,
            required_validation_count: required,
            passed_required_validation_count: passed,
            failed_required_validation_count: failed,
            stale_evidence: stale,
            ..crate::completion::CompletionEvidence::default()
        })
    };
    if crate::tools::validate_single_verification_plan(plan).is_err() {
        return base_evidence(1, 0, 0, false);
    }

    let normalized_paths =
        |requirement: &crate::plan::VerificationRequirementV1| match &requirement.scope {
            VerificationScopeV1::WorkspacePaths { relative_paths } => relative_paths
                .iter()
                .map(|path| path.replace('\\', "/"))
                .collect::<std::collections::BTreeSet<_>>(),
            _ => std::collections::BTreeSet::new(),
        };
    let required_requirements = plan
        .requirements
        .iter()
        .filter(|requirement| requirement.required)
        .collect::<Vec<_>>();
    let mut all_covered_paths = std::collections::BTreeSet::new();
    let mut behavior_covered_paths = std::collections::BTreeSet::new();
    let manual_acceptance_covers_candidate = required_requirements.iter().any(|requirement| {
        requirement.validator_id == crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID
            && matches!(&requirement.scope, VerificationScopeV1::Manual)
    });
    if manual_acceptance_covers_candidate {
        all_covered_paths.extend(pending_hashes.keys().cloned());
        behavior_covered_paths.extend(
            pending_hashes
                .keys()
                .filter(|path| single_path_is_source_code(path))
                .cloned(),
        );
    }
    for requirement in &required_requirements {
        let paths = normalized_paths(requirement);
        all_covered_paths.extend(paths.iter().cloned());
        if requirement.validator_id == "workspace-command-success-v1" {
            behavior_covered_paths.extend(paths);
        }
    }
    let missing_paths = pending_hashes
        .keys()
        .filter(|path| !all_covered_paths.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let missing_behavior_paths = pending_hashes
        .keys()
        .filter(|path| single_path_is_source_code(path) && !behavior_covered_paths.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let coverage_ok = missing_paths.is_empty() && missing_behavior_paths.is_empty();

    let started_at = chrono::Utc::now().to_rfc3339();
    let changeset_bytes = serde_json::to_vec(pending_hashes).unwrap_or_default();
    let changeset_sha256 = crate::CasStore::hash_of(&changeset_bytes);
    let environment_id = crate::CasStore::hash_of(session.workspace.to_string_lossy().as_bytes());
    let validation_epoch = session
        .validation_receipts
        .iter()
        .map(|receipt| receipt.epoch)
        .max()
        .unwrap_or(0)
        + 1;
    let verification_plan_sha256 = serde_json::to_vec(plan)
        .map(|bytes| crate::CasStore::hash_of(&bytes))
        .unwrap_or_default();
    let verification_plan_evidence_ref =
        format!("verification-plan-sha256:{verification_plan_sha256}");
    let mut required_count = required_requirements.len();
    let mut passed_count = 0usize;
    let mut failed_count = 0usize;
    let mut stale_evidence = false;
    let mut first_passed_receipt_id = None;
    let mut plan_receipts = Vec::with_capacity(plan.requirements.len() + 1);
    let mut workspace_validation = WorkspaceValidationBatch::new(Some(root));

    for requirement in &plan.requirements {
        let requirement_started = chrono::Utc::now().to_rfc3339();
        let mut subjects = std::collections::HashMap::new();
        let mut evidence_refs = vec![verification_plan_evidence_ref.clone()];
        let (mut verdict, mut detail) = if requirement.validator_id
            == crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID
            && matches!(&requirement.scope, VerificationScopeV1::Manual)
        {
            for (path, hash) in pending_hashes {
                subjects.insert(
                    format!("workspace-path:{path}"),
                    hash.clone()
                        .unwrap_or_else(crate::verification::workspace_path_absence_sha256),
                );
            }
            (
                ValidationVerdictV1::Unverified,
                Some("等待用户对宿主展示的精确候选快照作出验收".to_string()),
            )
        } else if requirement.validator_id == "workspace-command-success-v1" {
            let expected_sha = requirement
                .arguments
                .get("command")
                .and_then(serde_json::Value::as_str)
                .map(|command| crate::CasStore::hash_of(command.trim().as_bytes()));
            let observation =
                events
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(index, event)| match event {
                        TurnEvent::ToolResult {
                            tool,
                            command_receipt: Some(receipt),
                            ..
                        } if tool == "run_command"
                            && Some(receipt.command_sha256.as_str()) == expected_sha.as_deref() =>
                        {
                            Some((index, receipt))
                        }
                        _ => None,
                    });
            match observation {
                None => (
                    ValidationVerdictV1::Unverified,
                    Some("本回合没有运行计划登记的宿主行为命令".to_string()),
                ),
                Some((event_index, receipt)) => {
                    evidence_refs.push(format!("command-result:sha256:{}", receipt.result_sha256));
                    match crate::command_evidence::validate_command_receipt(requirement, receipt) {
                        Err((verdict, detail)) => (verdict, Some(detail)),
                        Ok(hashes) => {
                            subjects.extend(hashes);
                            let snapshot_matches =
                                normalized_paths(requirement).iter().all(|path| {
                                    let observed = receipt.workspace_hashes.get(path);
                                    observed.is_some()
                                        && pending_hashes
                                            .get(path)
                                            .is_none_or(|expected| Some(expected) == observed)
                                });
                            let later_mutation = events.iter().skip(event_index + 1).any(|event| {
                                matches!(event, TurnEvent::ToolResult { tool, ok: true, .. }
                                    if crate::tool_effects::effect_class_for(tool) != crate::tool_effects::EffectClass::Read)
                            });
                            if snapshot_matches && !later_mutation {
                                (
                                    ValidationVerdictV1::Passed,
                                    Some(
                                        "宿主行为命令成功，执行前后与最终源码快照一致".to_string(),
                                    ),
                                )
                            } else {
                                (
                                    ValidationVerdictV1::Stale,
                                    Some("行为命令证据与最终源码快照不一致".to_string()),
                                )
                            }
                        }
                    }
                }
            }
        } else if workspace_validator_arguments_supported(
            &requirement.validator_id,
            &requirement.arguments,
        ) {
            let (workspace_verdict, workspace_detail, hashes) =
                workspace_validation.execute_workspace(requirement);
            let mut matches_pending = true;
            for (subject, actual_hash) in hashes {
                let relative = subject.strip_prefix("workspace-path:").unwrap_or(&subject);
                let normalized = relative.replace('\\', "/");
                subjects.insert(format!("workspace-path:{normalized}"), actual_hash.clone());
                if pending_hashes
                    .get(&normalized)
                    .is_some_and(|expected| expected.as_deref() != Some(actual_hash.as_str()))
                {
                    matches_pending = false;
                }
            }
            if workspace_verdict == ValidationVerdictV1::Passed && !matches_pending {
                (
                    ValidationVerdictV1::Stale,
                    Some("宿主静态验证路径与本次变更或最终文件哈希不一致".to_string()),
                )
            } else {
                (workspace_verdict, workspace_detail)
            }
        } else {
            (
                ValidationVerdictV1::Unsupported,
                Some("VerificationPlan validator 参数未被宿主注册".to_string()),
            )
        };
        if !coverage_ok && requirement.required && verdict == ValidationVerdictV1::Passed {
            verdict = ValidationVerdictV1::Unverified;
            detail = Some(format!(
                "声明的验收项通过，但计划未覆盖所有候选变更；遗漏路径={}，未行为验证源码路径={}",
                missing_paths.join(","),
                missing_behavior_paths.join(",")
            ));
        }
        let receipt_id = format!("single-validation-{}", uuid::Uuid::new_v4());
        if requirement.required {
            match verdict {
                ValidationVerdictV1::Passed | ValidationVerdictV1::ManualAccepted => {
                    passed_count += 1;
                    first_passed_receipt_id.get_or_insert_with(|| receipt_id.clone());
                }
                ValidationVerdictV1::Failed => failed_count += 1,
                ValidationVerdictV1::Stale => stale_evidence = true,
                _ => {}
            }
        }
        plan_receipts.push(ValidationReceiptV1 {
            receipt_id,
            task_id: session.id.clone(),
            attempt_id: turn_id.to_string(),
            epoch: validation_epoch,
            requirement_id: requirement.requirement_id.clone(),
            validator_id: requirement.validator_id.clone(),
            validator_version: requirement
                .validator_version
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            arguments_sha256: crate::CasStore::hash_of(
                requirement.arguments.to_string().as_bytes(),
            ),
            input_sha256: input_sha256.to_string(),
            environment_id: environment_id.clone(),
            changeset_sha256: Some(changeset_sha256.clone()),
            detail,
            subject_sha256: subjects,
            verdict,
            evidence_refs,
            review_result: None,
            started_at: requirement_started,
            completed_at: chrono::Utc::now().to_rfc3339(),
        });
    }
    required_count += 1;
    let mut subjects = std::collections::HashMap::new();
    for (path, hash) in pending_hashes {
        subjects.insert(
            format!("workspace-path:{path}"),
            hash.clone()
                .unwrap_or_else(crate::verification::workspace_path_absence_sha256),
        );
    }
    let coverage_detail = if coverage_ok {
        passed_count += 1;
        Some(
            "宿主已确认所有候选变更路径均被必需验证范围覆盖，且所有源码路径均有行为命令覆盖"
                .to_string(),
        )
    } else {
        Some(format!(
            "候选变更不在必需验证范围内；遗漏路径={}，未行为验证源码路径={}",
            missing_paths.join(","),
            missing_behavior_paths.join(",")
        ))
    };
    plan_receipts.push(ValidationReceiptV1 {
        receipt_id: format!("single-coverage-{}", uuid::Uuid::new_v4()),
        task_id: session.id.clone(),
        attempt_id: turn_id.to_string(),
        epoch: validation_epoch,
        requirement_id: "host-change-scope-coverage".to_string(),
        validator_id: "host-change-scope-coverage-v1".to_string(),
        validator_version: "1".to_string(),
        arguments_sha256: crate::CasStore::hash_of(plan.plan_id.as_bytes()),
        input_sha256: input_sha256.to_string(),
        environment_id,
        changeset_sha256: Some(changeset_sha256),
        detail: coverage_detail,
        subject_sha256: subjects,
        verdict: if coverage_ok {
            ValidationVerdictV1::Passed
        } else {
            ValidationVerdictV1::Unverified
        },
        evidence_refs: vec![verification_plan_evidence_ref],
        review_result: None,
        started_at: started_at.clone(),
        completed_at: chrono::Utc::now().to_rfc3339(),
    });
    // Never accept using the validation-time cache alone. This separate pass
    // rehashes each unique workspace path after all requirements have run.
    let mut final_snapshot = crate::workspace_snapshot::WorkspaceSnapshotBatch::new(Some(root));
    for receipt in &mut plan_receipts {
        if receipt.verdict == ValidationVerdictV1::Passed
            && !final_snapshot.subjects_match(&receipt.subject_sha256)
        {
            receipt.verdict = ValidationVerdictV1::Stale;
            receipt.detail = Some("Single 最终工作区偏离本轮验证观察，收据已失效".into());
            receipt.completed_at = chrono::Utc::now().to_rfc3339();
            let required = receipt.requirement_id == "host-change-scope-coverage"
                || required_requirements
                    .iter()
                    .any(|requirement| requirement.requirement_id == receipt.requirement_id);
            if required {
                passed_count = passed_count.saturating_sub(1);
                stale_evidence = true;
            }
        }
    }
    session.validation_receipts.extend(plan_receipts);

    let status = base_evidence(required_count, passed_count, failed_count, stale_evidence);
    if status == owo_agent_protocol::CompletionStatusV1::Accepted {
        let mut latest_receipt_by_path = std::collections::BTreeMap::new();
        for (index, execution) in session.execution_receipts.iter().enumerate() {
            for path in &execution.changed_files {
                latest_receipt_by_path.insert(path.replace('\\', "/"), index);
            }
        }
        let accepted_receipt_id = first_passed_receipt_id.unwrap_or_default();
        for (index, execution) in session.execution_receipts.iter_mut().enumerate() {
            let paths = execution
                .changed_files
                .iter()
                .map(|path| path.replace('\\', "/"))
                .collect::<Vec<_>>();
            if execution.status != "executed" || paths.is_empty() {
                continue;
            }
            let all_paths_are_latest = paths.iter().all(|path| {
                pending_hashes.contains_key(path)
                    && latest_receipt_by_path.get(path) == Some(&index)
            });
            if all_paths_are_latest {
                execution.status = "accepted".to_string();
                execution.validation_receipt_id = Some(accepted_receipt_id.clone());
            } else if paths.iter().any(|path| {
                latest_receipt_by_path
                    .get(path)
                    .is_some_and(|latest_index| latest_index != &index)
            }) {
                execution.status = "stale".to_string();
            }
        }
    }
    status
}

#[cfg(test)]
mod single_verification_plan_tests {
    use super::single_manual_acceptance::{
        manual_acceptance_answer_verdict, request_single_manual_acceptance,
        SINGLE_MANUAL_ACCEPT_OPTION, SINGLE_MANUAL_REJECT_OPTION,
    };
    use super::{
        assess_single_turn_completion, single_validation_retry_feedback, CommandExecutionReceipt,
        TurnEvent,
    };
    use crate::plan::{
        VerificationPlanV1, VerificationRequirementV1, VerificationResourcesV1, VerificationScopeV1,
    };
    use crate::session::{ExecutionReceipt, Session};
    use std::collections::HashMap;

    fn plan(validator_id: &str, path: &str, arguments: serde_json::Value) -> VerificationPlanV1 {
        VerificationPlanV1 {
            plan_id: "single-task-plan".to_string(),
            requirements: vec![VerificationRequirementV1 {
                requirement_id: "req-user-visible".to_string(),
                covers_requirement_ids: vec!["user-request:req-user-visible".to_string()],
                validator_id: validator_id.to_string(),
                validator_version: Some("1".to_string()),
                scope: VerificationScopeV1::WorkspacePaths {
                    relative_paths: vec![path.to_string()],
                },
                arguments,
                required: true,
                resources: VerificationResourcesV1 {
                    cpu_slots: 1,
                    memory_mb: 16,
                    exclusive_workspace: false,
                    timeout_ms: 10_000,
                },
            }],
        }
    }

    fn add_write(session: &mut Session, turn_id: &str, relative: &str, hash: &str) {
        session.execution_receipts.push(ExecutionReceipt {
            receipt_id: format!("exec-{turn_id}"),
            tool: "write_file".to_string(),
            turn_id: turn_id.to_string(),
            changed_files: vec![relative.to_string()],
            snapshot_keys: HashMap::new(),
            before_hashes: HashMap::from([(relative.to_string(), None)]),
            after_hashes: HashMap::from([(relative.to_string(), Some(hash.to_string()))]),
            diff_sha256: "diff-hash".to_string(),
            created_at: "2026-10-04T00:00:00Z".to_string(),
            status: "executed".to_string(),
            validation_receipt_id: None,
        });
    }

    #[test]
    fn source_changes_need_a_registered_behavior_check_covering_the_changed_source() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src").join("lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(
            &source,
            "pub fn ready() -> bool { true }
",
        )
        .unwrap();
        let hash = crate::CasStore::hash_of(&std::fs::read(&source).unwrap());
        let mut session = Session::new(workspace.path(), "mock", None);
        add_write(&mut session, "turn-source", "src/lib.rs", &hash);
        let prompt = "实现 ready 检查";
        session.single_verification_plan = Some(plan(
            "workspace-file-exists-v1",
            "src/lib.rs",
            serde_json::json!({}),
        ));
        session.single_verification_plan_input_sha256 =
            Some(crate::CasStore::hash_of(prompt.as_bytes()));
        session.single_verification_plan_turn_id = Some("turn-source".to_string());

        let status = assess_single_turn_completion(
            &mut session,
            prompt,
            "turn-source",
            &[],
            false,
            Some("已实现并验证。"),
        );
        assert_eq!(status, owo_agent_protocol::CompletionStatusV1::Unverified);
        assert!(session.validation_receipts.iter().any(|receipt| {
            receipt.requirement_id == "host-change-scope-coverage"
                && receipt.verdict == crate::plan::ValidationVerdictV1::Unverified
        }));
    }

    #[test]
    fn generic_command_success_without_a_request_bound_plan_is_only_candidate() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src").join("lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(
            &source,
            "pub fn ready() -> bool { true }
",
        )
        .unwrap();
        let hash = crate::CasStore::hash_of(&std::fs::read(&source).unwrap());
        let mut session = Session::new(workspace.path(), "mock", None);
        add_write(&mut session, "turn-unplanned", "src/lib.rs", &hash);
        let command = "cargo test -p owo-agent-core";
        session.single_verification_plan = Some(plan(
            "workspace-command-success-v1",
            "src/lib.rs",
            serde_json::json!({"command":command}),
        ));
        session.single_verification_plan_input_sha256 =
            Some(crate::CasStore::hash_of("实现 ready 检查".as_bytes()));
        session.single_verification_plan_turn_id = Some("older-turn".to_string());
        let events = vec![TurnEvent::ToolResult {
            id: "test-command".to_string(),
            tool: "run_command".to_string(),
            ok: true,
            error: None,
            preview: None,
            outputs: Vec::new(),
            command_receipt: Some(CommandExecutionReceipt {
                command_sha256: crate::CasStore::hash_of(command.as_bytes()),
                exit_code: 0,
                result_sha256: "result-hash".to_string(),
                duration_ms: Some(100),
                workspace_hashes_complete: true,
                validator_id: Some("workspace-command-success-v1".to_string()),
                validator_version: Some("1".to_string()),
                workspace_hashes_before: std::collections::BTreeMap::from([(
                    "src/lib.rs".to_string(),
                    Some(hash.clone()),
                )]),
                workspace_hashes: std::collections::BTreeMap::from([(
                    "src/lib.rs".to_string(),
                    Some(hash),
                )]),
            }),
        }];

        let status = assess_single_turn_completion(
            &mut session,
            "实现 ready 检查",
            "turn-unplanned",
            &events,
            false,
            Some("已实现并运行测试。"),
        );
        assert_eq!(status, owo_agent_protocol::CompletionStatusV1::Candidate);
        assert!(session.validation_receipts.is_empty());
    }

    #[test]
    fn uncovered_change_receipt_is_included_in_single_repair_feedback() {
        let workspace = tempfile::tempdir().unwrap();
        let checked = workspace.path().join("README.md");
        std::fs::write(&checked, "present").unwrap();
        let mut session = Session::new(workspace.path(), "mock", None);
        add_write(
            &mut session,
            "turn-uncovered",
            "src/uncovered.rs",
            "candidate-hash",
        );
        let prompt = "实现功能 req-user-visible";
        session.single_verification_plan = Some(plan(
            "workspace-file-exists-v1",
            "README.md",
            serde_json::json!({}),
        ));
        session.single_verification_plan_input_sha256 =
            Some(crate::CasStore::hash_of(prompt.as_bytes()));
        session.single_verification_plan_turn_id = Some("turn-uncovered".to_string());

        let status = assess_single_turn_completion(
            &mut session,
            prompt,
            "turn-uncovered",
            &[],
            false,
            Some("实现完成。"),
        );
        assert_eq!(status, owo_agent_protocol::CompletionStatusV1::Unverified);
        let (_, feedback) = single_validation_retry_feedback(&session, "turn-uncovered").unwrap();
        assert!(feedback.contains("host-change-scope-coverage"));
        assert!(feedback.contains("src/uncovered.rs"));
    }

    #[test]
    fn request_bound_plan_runs_for_a_verification_only_turn() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src").join("lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "pub fn ready() -> bool { true }\n").unwrap();
        let hash = crate::CasStore::hash_of(&std::fs::read(&source).unwrap());
        let prompt = "运行验证 req-user-visible";
        let turn_id = "verification-only-turn";
        let command = "cargo test -p owo-agent-core";
        let mut session = Session::new(workspace.path(), "mock", None);
        session.single_verification_plan = Some(plan(
            "workspace-command-success-v1",
            "src/lib.rs",
            serde_json::json!({"command":command}),
        ));
        session.single_verification_plan_input_sha256 =
            Some(crate::CasStore::hash_of(prompt.as_bytes()));
        session.single_verification_plan_turn_id = Some(turn_id.to_string());
        let events = vec![TurnEvent::ToolResult {
            id: "test-command".to_string(),
            tool: "run_command".to_string(),
            ok: true,
            error: None,
            preview: None,
            outputs: Vec::new(),
            command_receipt: Some(CommandExecutionReceipt {
                command_sha256: crate::CasStore::hash_of(command.trim().as_bytes()),
                exit_code: 0,
                result_sha256: "result-hash".to_string(),
                duration_ms: Some(100),
                workspace_hashes_complete: true,
                validator_id: Some("workspace-command-success-v1".to_string()),
                validator_version: Some("1".to_string()),
                workspace_hashes_before: std::collections::BTreeMap::from([(
                    "src/lib.rs".to_string(),
                    Some(hash.clone()),
                )]),
                workspace_hashes: std::collections::BTreeMap::from([(
                    "src/lib.rs".to_string(),
                    Some(hash),
                )]),
            }),
        }];

        let status = assess_single_turn_completion(
            &mut session,
            prompt,
            turn_id,
            &events,
            false,
            Some("验证命令通过。"),
        );
        assert_eq!(status, owo_agent_protocol::CompletionStatusV1::Accepted);
        assert!(session.validation_receipts.iter().any(|receipt| {
            receipt.requirement_id == "req-user-visible"
                && receipt.verdict == crate::plan::ValidationVerdictV1::Passed
        }));
        assert!(session.validation_receipts.iter().any(|receipt| {
            receipt.requirement_id == "host-change-scope-coverage"
                && receipt.verdict == crate::plan::ValidationVerdictV1::Passed
        }));
    }

    #[test]
    fn prior_unaccepted_code_does_not_reclassify_a_later_normal_reply() {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = Session::new(workspace.path(), "mock", None);
        add_write(
            &mut session,
            "previous-turn",
            "src/lib.rs",
            "previous-source-hash",
        );

        let status = assess_single_turn_completion(
            &mut session,
            "解释一下所有权",
            "current-chat-turn",
            &[],
            false,
            Some("Rust 所有权用于管理值的生命周期。"),
        );
        assert_eq!(
            status,
            owo_agent_protocol::CompletionStatusV1::ResponseComplete
        );
        assert!(session.validation_receipts.is_empty());
    }

    #[test]
    fn manual_acceptance_requires_the_current_question_and_exact_option() {
        let question_id = "question-current";
        let accepted = crate::question::QuestionAnswer {
            question_id: question_id.to_string(),
            answer: SINGLE_MANUAL_ACCEPT_OPTION.to_string(),
        };
        let declined = crate::question::QuestionAnswer {
            question_id: question_id.to_string(),
            answer: SINGLE_MANUAL_REJECT_OPTION.to_string(),
        };
        let stale_question = crate::question::QuestionAnswer {
            question_id: "question-old".to_string(),
            answer: SINGLE_MANUAL_ACCEPT_OPTION.to_string(),
        };
        assert_eq!(
            manual_acceptance_answer_verdict(Some(&accepted), question_id),
            crate::plan::ValidationVerdictV1::ManualAccepted
        );
        assert_eq!(
            manual_acceptance_answer_verdict(Some(&declined), question_id),
            crate::plan::ValidationVerdictV1::Failed
        );
        assert_eq!(
            manual_acceptance_answer_verdict(Some(&stale_question), question_id),
            crate::plan::ValidationVerdictV1::Unverified
        );
        let free_form = crate::question::QuestionAnswer {
            question_id: question_id.to_string(),
            answer: "yes".to_string(),
        };
        assert_eq!(
            manual_acceptance_answer_verdict(Some(&free_form), question_id),
            crate::plan::ValidationVerdictV1::Unverified
        );
    }

    #[tokio::test]
    async fn manual_acceptance_receipt_binds_the_candidate_snapshot() {
        struct FixedQuestioner {
            answer: String,
            mutate_path: Option<std::path::PathBuf>,
        }

        #[async_trait::async_trait]
        impl crate::question::Questioner for FixedQuestioner {
            async fn ask(
                &self,
                question: &crate::question::UserQuestion,
            ) -> Option<crate::question::QuestionAnswer> {
                if let Some(path) = &self.mutate_path {
                    std::fs::write(path, "changed while waiting").unwrap();
                }
                Some(crate::question::QuestionAnswer {
                    question_id: question.question_id.clone(),
                    answer: self.answer.clone(),
                })
            }
        }

        async fn run_case(
            mutate: bool,
        ) -> (
            owo_agent_protocol::CompletionStatusV1,
            crate::plan::ValidationVerdictV1,
            String,
            String,
            Vec<String>,
            bool,
            bool,
        ) {
            let workspace = tempfile::tempdir().unwrap();
            let source = workspace.path().join("src").join("main.rs");
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(&source, "fn main() {}\n").unwrap();
            let hash = crate::CasStore::hash_of(&std::fs::read(&source).unwrap());
            let mut session = Session::new(workspace.path(), "mock", None);
            add_write(&mut session, "turn-prior", "src/main.rs", &hash);
            let plan = VerificationPlanV1 {
                plan_id: "manual-plan".to_string(),
                requirements: vec![VerificationRequirementV1 {
                    requirement_id: "manual-user-requirement".to_string(),
                    covers_requirement_ids: vec!["user-request:实现可用功能".to_string()],
                    validator_id: crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID
                        .to_string(),
                    validator_version: Some("1".to_string()),
                    scope: VerificationScopeV1::Manual,
                    arguments: serde_json::json!({}),
                    required: true,
                    resources: VerificationResourcesV1::default(),
                }],
            };
            let prompt = "实现可用功能";
            session.single_verification_plan = Some(plan.clone());
            session.single_verification_plan_input_sha256 =
                Some(crate::CasStore::hash_of(prompt.as_bytes()));
            session.single_verification_plan_turn_id = Some("turn-manual".to_string());
            let initial_status = assess_single_turn_completion(
                &mut session,
                prompt,
                "turn-manual",
                &[],
                false,
                Some("候选版本已准备验收。"),
            );
            assert_eq!(
                initial_status,
                owo_agent_protocol::CompletionStatusV1::Unverified
            );
            let questioner = FixedQuestioner {
                answer: SINGLE_MANUAL_ACCEPT_OPTION.to_string(),
                mutate_path: mutate.then(|| source.clone()),
            };
            let abort = std::sync::atomic::AtomicBool::new(false);
            let status = request_single_manual_acceptance(
                &mut session,
                &plan,
                "turn-manual",
                Some(&questioner),
                &abort,
            )
            .await;
            let review_candidate_present =
                super::single_review::accepted_candidate_paths(&session, "turn-manual")
                    .contains_key("src/main.rs");
            let notice_is_explicit =
                super::single_manual_acceptance::completion_notice(&session, "turn-manual")
                    .is_some_and(|notice| notice.contains("不等同于自动行为测试通过"));
            let manual_receipt = session
                .validation_receipts
                .iter()
                .rev()
                .find(|receipt| {
                    receipt.validator_id == crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID
                })
                .expect("manual acceptance receipt");
            (
                status,
                manual_receipt.verdict,
                manual_receipt.subject_sha256["workspace-path:src/main.rs"].clone(),
                session.execution_receipts[0].status.clone(),
                manual_receipt.evidence_refs.clone(),
                review_candidate_present,
                notice_is_explicit,
            )
        }

        let (
            accepted_status,
            accepted_verdict,
            accepted_hash,
            execution_status,
            evidence_refs,
            review_candidate_present,
            notice_is_explicit,
        ) = run_case(false).await;
        assert_eq!(
            accepted_status,
            owo_agent_protocol::CompletionStatusV1::Accepted
        );
        assert_eq!(
            accepted_verdict,
            crate::plan::ValidationVerdictV1::ManualAccepted
        );
        assert_eq!(accepted_hash, crate::CasStore::hash_of(b"fn main() {}\n"));
        assert_eq!(execution_status, "accepted");
        assert!(review_candidate_present);
        assert!(notice_is_explicit);
        assert!(evidence_refs
            .iter()
            .any(|reference| reference.starts_with("manual-question:")));
        assert!(evidence_refs
            .iter()
            .any(|reference| reference.starts_with("user-answer-sha256:")));

        let (stale_status, stale_verdict, _, _, _, _, _) = run_case(true).await;
        assert_eq!(
            stale_status,
            owo_agent_protocol::CompletionStatusV1::Unverified
        );
        assert_eq!(stale_verdict, crate::plan::ValidationVerdictV1::Stale);
    }

    #[test]
    fn missing_current_plan_returns_stable_feedback_for_written_candidate() {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = Session::new(workspace.path(), "mock", None);
        add_write(&mut session, "turn-unplanned", "src/lib.rs", "source-hash");

        let first = super::single_missing_verification_plan_feedback(&session, "turn-unplanned");
        let second = super::single_missing_verification_plan_feedback(&session, "turn-unplanned");
        assert_eq!(first, second);
        let (fingerprint, feedback) = first.unwrap();
        assert!(!fingerprint.is_empty());
        assert!(feedback.contains("verification_plan"));
        assert!(feedback.contains("src/lib.rs"));
        assert!(feedback.contains("source-hash"));
        assert!(super::single_missing_verification_plan_feedback(&session, "other-turn").is_none());
    }

    #[test]
    fn planned_static_check_binds_receipt_to_exact_changed_file_hash() {
        let workspace = tempfile::tempdir().unwrap();
        let doc = workspace.path().join("README.md");
        std::fs::write(
            &doc,
            "用户要求：包含 hello
hello
",
        )
        .unwrap();
        let hash = crate::CasStore::hash_of(&std::fs::read(&doc).unwrap());
        let mut session = Session::new(workspace.path(), "mock", None);
        add_write(&mut session, "turn-doc", "README.md", &hash);
        let prompt = "创建说明并包含 hello";
        session.single_verification_plan = Some(plan(
            "workspace-file-contains-v1",
            "README.md",
            serde_json::json!({"text":"hello"}),
        ));
        session.single_verification_plan_input_sha256 =
            Some(crate::CasStore::hash_of(prompt.as_bytes()));
        session.single_verification_plan_turn_id = Some("turn-doc".to_string());

        let status = assess_single_turn_completion(
            &mut session,
            prompt,
            "turn-doc",
            &[],
            false,
            Some("已完成说明。"),
        );
        assert_eq!(status, owo_agent_protocol::CompletionStatusV1::Accepted);
        assert_eq!(session.validation_receipts.len(), 2);
        assert_eq!(
            session.validation_receipts[0].verdict,
            crate::plan::ValidationVerdictV1::Passed
        );
        assert_eq!(
            session.validation_receipts[0].subject_sha256["workspace-path:README.md"],
            hash
        );
        assert_eq!(
            session.validation_receipts[1].requirement_id,
            "host-change-scope-coverage"
        );
        assert_eq!(
            session.validation_receipts[1].verdict,
            crate::plan::ValidationVerdictV1::Passed
        );
        assert!(session.validation_receipts[1].evidence_refs[0]
            .starts_with("verification-plan-sha256:"));
        assert_eq!(session.execution_receipts[0].status, "accepted");
    }

    #[test]
    fn prior_output_identity_is_not_stale_merely_because_it_is_not_a_workspace_path() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new(root.path(), "mock", None);
        session
            .validation_receipts
            .push(crate::plan::ValidationReceiptV1 {
                receipt_id: "output-receipt".into(),
                task_id: session.id.clone(),
                attempt_id: "old-turn".into(),
                epoch: 1,
                requirement_id: "output".into(),
                validator_id: "artifact-output-non-empty-v1".into(),
                validator_version: "1".into(),
                arguments_sha256: crate::CasStore::hash_of(b"{}"),
                input_sha256: "input".into(),
                environment_id: "env".into(),
                changeset_sha256: None,
                verdict: crate::plan::ValidationVerdictV1::Passed,
                subject_sha256: HashMap::from([(
                    "step-output".into(),
                    crate::CasStore::hash_of(b"ready"),
                )]),
                detail: None,
                evidence_refs: Vec::new(),
                review_result: None,
                started_at: "now".into(),
                completed_at: "now".into(),
            });
        let _ = assess_single_turn_completion(
            &mut session,
            "hello",
            "new-turn",
            &[],
            false,
            Some("hello"),
        );
        assert_eq!(
            session.validation_receipts[0].verdict,
            crate::plan::ValidationVerdictV1::Passed
        );
    }
}
