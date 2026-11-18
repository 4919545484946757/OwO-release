//! Review the full verified scope, including unchanged tests and host command proof.
use crate::agent::TurnEvent;
use crate::plan::{ValidationVerdictV1, VerificationScopeV1};
use crate::session::Session;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) struct ReviewEvidence {
    pub paths: BTreeMap<String, String>,
    pub proof: Value,
}

pub(super) fn prepare(
    session: &Session,
    turn_id: &str,
    input_sha256: &str,
    candidates: &BTreeMap<String, String>,
    events: &[TurnEvent],
) -> Result<ReviewEvidence, String> {
    let mut paths = candidates.clone();
    let mut validations = Vec::new();
    if let Some(plan) = &session.single_verification_plan {
        for requirement in plan
            .requirements
            .iter()
            .filter(|requirement| requirement.required)
        {
            let Some(receipt) = session.validation_receipts.iter().rev().find(|receipt| {
                receipt.task_id == session.id
                    && receipt.attempt_id == turn_id
                    && receipt.input_sha256 == input_sha256
                    && receipt.requirement_id == requirement.requirement_id
            }) else {
                continue;
            };
            if !matches!(
                receipt.verdict,
                ValidationVerdictV1::Passed | ValidationVerdictV1::ManualAccepted
            ) {
                continue;
            }
            validations.push(receipt);
            if let VerificationScopeV1::WorkspacePaths { relative_paths } = &requirement.scope {
                for path in relative_paths {
                    let normalized = path.replace('\\', "/");
                    let hash = receipt
                        .subject_sha256
                        .get(&format!("workspace-path:{normalized}"))
                        .ok_or_else(|| format!("验证收据缺少评审范围的哈希：{normalized}"))?;
                    if paths
                        .get(&normalized)
                        .is_some_and(|expected| expected != hash)
                    {
                        return Err(format!("评审证据与候选版本冲突：{normalized}"));
                    }
                    paths.insert(normalized, hash.clone());
                }
            }
        }
    }
    let executions = session
        .execution_receipts
        .iter()
        .filter(|receipt| receipt.turn_id == turn_id)
        .take(129)
        .collect::<Vec<_>>();
    let commands = events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ToolResult {
                command_receipt: Some(receipt),
                ..
            } => Some(receipt),
            _ => None,
        })
        .take(129)
        .collect::<Vec<_>>();
    if executions.len() > 128 || commands.len() > 128 {
        return Err("评审宿主证据条数超过上限 128".into());
    }
    let proof = json!({"changed_files":candidates.keys().collect::<Vec<_>>(),
        "validation_receipts":validations,"execution_receipts":executions,
        "host_command_receipts":commands});
    if serde_json::to_vec(&proof)
        .map_err(|error| error.to_string())?
        .len()
        > 64 * 1024
    {
        return Err("评审宿主证据超过 64 KiB 上限".into());
    }
    Ok(ReviewEvidence { paths, proof })
}
