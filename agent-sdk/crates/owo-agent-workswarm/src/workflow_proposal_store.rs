//! Immutable proposal documents; adoption and template CAS commit in one transaction.
use crate::workflow_revision_store::{
    append, current, expected_revision, user_precondition, WorkflowRevision, WorkflowRevisionStore,
    WorkflowStoreError,
};
use owo_agent_protocol::{TeamTemplate, TeamTemplateProposal, TeamTemplateProposalStatus};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub(super) const PROPOSAL_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS workflow_proposals (
 proposal_id TEXT PRIMARY KEY, proposal_json TEXT NOT NULL,
 base_revision INTEGER NOT NULL CHECK(base_revision >= 0),
 adopted_revision INTEGER, legacy_imported INTEGER NOT NULL DEFAULT 0
);";
const LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowProposalRecord {
    #[serde(flatten)]
    pub proposal: TeamTemplateProposal,
    pub base_revision: u64,
    pub adopted_revision: Option<u64>,
    pub legacy_imported: bool,
    pub proposal_hash: String,
}
fn hash(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}
fn validate_id(id: &str) -> Result<(), WorkflowStoreError> {
    if id.is_empty()
        || id.len() > 512
        || id.trim() != id
        || id.contains("..")
        || id
            .chars()
            .any(|ch| ch.is_control() || "/\\:*?\"<>|".contains(ch))
    {
        Err(WorkflowStoreError::Invalid("invalid proposal id".into()))
    } else {
        Ok(())
    }
}
fn encode(proposal: &TeamTemplateProposal) -> Result<String, WorkflowStoreError> {
    validate_id(&proposal.proposal_id)?;
    let raw = serde_json::to_string(proposal)?;
    if raw.len() > LIMIT {
        return Err(WorkflowStoreError::Invalid("proposal exceeds 2 MiB".into()));
    }
    Ok(raw)
}
fn record(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowProposalRecord> {
    let raw: String = row.get(0)?;
    let proposal = serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(WorkflowProposalRecord {
        proposal,
        base_revision: row.get(1)?,
        adopted_revision: row.get(2)?,
        legacy_imported: row.get(3)?,
        proposal_hash: hash(&raw),
    })
}
fn get(
    connection: &Connection,
    id: &str,
) -> Result<Option<WorkflowProposalRecord>, WorkflowStoreError> {
    validate_id(id)?;
    Ok(connection.query_row("SELECT proposal_json,base_revision,adopted_revision,legacy_imported FROM workflow_proposals WHERE proposal_id=?1", [id], record).optional()?)
}
pub(super) fn migrate_proposals(
    connection: &mut Connection,
    root: &Path,
) -> Result<(), WorkflowStoreError> {
    connection.execute_batch(PROPOSAL_SCHEMA)?;
    let done: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM workflow_store_meta WHERE key='legacy_proposals_imported_v1')",
        [],
        |row| row.get(0),
    )?;
    if done {
        return Ok(());
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let done: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM workflow_store_meta WHERE key='legacy_proposals_imported_v1')",
        [],
        |row| row.get(0),
    )?;
    if !done {
        let dir = root.join("proposals");
        if dir.exists() {
            let mut entries = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                if !entry.file_type()?.is_file() {
                    return Err(WorkflowStoreError::Invalid(
                        "legacy proposal is not a regular file".into(),
                    ));
                }
                let mut bytes = Vec::new();
                std::fs::File::open(&path)?
                    .take((LIMIT + 1) as u64)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > LIMIT {
                    return Err(WorkflowStoreError::Invalid(
                        "legacy proposal exceeds 2 MiB".into(),
                    ));
                }
                let proposal: TeamTemplateProposal = serde_json::from_slice(&bytes)?;
                validate_id(&proposal.proposal_id)?;
                if path.file_stem().and_then(|s| s.to_str()) != Some(&proposal.proposal_id) {
                    return Err(WorkflowStoreError::Invalid(
                        "legacy proposal filename/id mismatch".into(),
                    ));
                }
                let raw = encode(&proposal)?;
                // Old JSON has no trustworthy base revision or adoption identity.
                // Never infer the newest workflow as its base or silently overwrite it.
                tx.execute("INSERT OR IGNORE INTO workflow_proposals(proposal_id,proposal_json,base_revision,legacy_imported) VALUES(?1,?2,0,1)",
                    params![proposal.proposal_id, raw])?;
            }
        }
        tx.execute(
            "INSERT INTO workflow_store_meta(key,value) VALUES('legacy_proposals_imported_v1','1')",
            [],
        )?;
    }
    tx.commit()?;
    Ok(())
}
impl WorkflowRevisionStore {
    pub fn get_proposal_record(
        &self,
        id: &str,
    ) -> Result<Option<WorkflowProposalRecord>, WorkflowStoreError> {
        get(&self.connection, id)
    }
    pub fn list_proposal_records(&self) -> Result<Vec<WorkflowProposalRecord>, WorkflowStoreError> {
        let mut statement = self.connection.prepare("SELECT proposal_json,base_revision,adopted_revision,legacy_imported FROM workflow_proposals ORDER BY proposal_id")?;
        let mut records = statement
            .query_map([], record)?
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by(|a, b| b.proposal.created_at.cmp(&a.proposal.created_at));
        Ok(records)
    }
    pub fn save_proposal_record(
        &mut self,
        proposal: &TeamTemplateProposal,
    ) -> Result<WorkflowProposalRecord, WorkflowStoreError> {
        let raw = encode(proposal)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = get(&tx, &proposal.proposal_id)? {
            let mut replay = proposal.clone();
            replay.status = existing.proposal.status;
            if encode(&replay)? != encode(&existing.proposal)? {
                return Err(WorkflowStoreError::ProposalState(
                    "proposal is immutable; create a new proposal id".into(),
                ));
            }
            tx.commit()?;
            return Ok(existing);
        }
        if proposal.status != TeamTemplateProposalStatus::Proposed {
            return Err(WorkflowStoreError::ProposalState(
                "a new proposal must be proposed".into(),
            ));
        }
        let base = current(&tx, &proposal.template.template_id)?
            .map(|r| r.revision)
            .unwrap_or(0);
        tx.execute("INSERT INTO workflow_proposals(proposal_id,proposal_json,base_revision) VALUES(?1,?2,?3)",
            params![proposal.proposal_id, raw, base])?;
        let result = get(&tx, &proposal.proposal_id)?.expect("inserted proposal");
        tx.commit()?;
        Ok(result)
    }
    /// The shared core service validates the candidate and binds its source proposal hash.
    /// This trusted state API neither grants permission nor executes the workflow.
    pub fn adopt_reviewed_proposal(
        &mut self,
        id: &str,
        source_hash: &str,
        template: &TeamTemplate,
        expected_base: Option<u64>,
    ) -> Result<WorkflowRevision, WorkflowStoreError> {
        validate_id(id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut proposal = get(&tx, id)?
            .ok_or_else(|| WorkflowStoreError::NotFound(format!("提案 {id} 不存在")))?;
        let existing = current(&tx, &proposal.proposal.template.template_id)?;
        match proposal.proposal.status {
            TeamTemplateProposalStatus::Rejected => {
                return Err(WorkflowStoreError::ProposalState(format!(
                    "提案 {id} 已被拒绝，不能采纳"
                )))
            }
            TeamTemplateProposalStatus::Adopted => {
                if existing.as_ref().is_none_or(|r| r.template.is_none()) {
                    return Err(WorkflowStoreError::ProposalState(
                        "已采纳模板已删除，不会重建；请创建新提案".into(),
                    ));
                }
                let revision = proposal.adopted_revision.ok_or_else(|| WorkflowStoreError::ProposalState(
                    "legacy adopted proposal has no verified revision; inspect workflow history".into()))?;
                let result = tx.query_row(
                    "SELECT template_id,revision,committed_at,content_hash,template_json FROM workflow_revisions WHERE template_id=?1 AND revision=?2",
                    params![proposal.proposal.template.template_id, revision], crate::workflow_revision_store::decode)?;
                tx.commit()?;
                return Ok(result);
            }
            TeamTemplateProposalStatus::Proposed => {}
        }
        if proposal.proposal_hash != source_hash {
            return Err(WorkflowStoreError::ProposalState(
                "proposal changed since validation".into(),
            ));
        }
        if expected_base.is_some_and(|expected| expected != proposal.base_revision) {
            return Err(WorkflowStoreError::ProposalState(
                "proposal base_revision is frozen; create and review a new proposal".into(),
            ));
        }
        if template.template_id != proposal.proposal.template.template_id
            || !matches!(template.origin.as_deref(), Some("user") | Some("community"))
        {
            return Err(WorkflowStoreError::Invalid(
                "invalid reviewed proposal target/origin".into(),
            ));
        }
        user_precondition(existing.as_ref(), Some(proposal.base_revision))?;
        let base = expected_revision(existing.as_ref(), Some(proposal.base_revision))?;
        let raw = serde_json::to_string(template)?;
        if raw.len() > LIMIT {
            return Err(WorkflowStoreError::Invalid("workflow exceeds 2 MiB".into()));
        }
        let digest = hash(&raw);
        let result =
            if let Some(record) = existing.filter(|r| r.content_hash.as_deref() == Some(&digest)) {
                record
            } else {
                let next = base
                    .checked_add(1)
                    .filter(|n| *n <= i64::MAX as u64)
                    .ok_or_else(|| WorkflowStoreError::Invalid("revision exhausted".into()))?;
                append(&tx, &template.template_id, next, Some(template))?
            };
        proposal.proposal.status = TeamTemplateProposalStatus::Adopted;
        tx.execute("UPDATE workflow_proposals SET proposal_json=?2,adopted_revision=?3 WHERE proposal_id=?1",
            params![id, encode(&proposal.proposal)?, result.revision])?;
        tx.commit()?;
        Ok(result)
    }
    pub fn reject_proposal_record(&mut self, id: &str) -> Result<(), WorkflowStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut proposal = get(&tx, id)?
            .ok_or_else(|| WorkflowStoreError::NotFound(format!("提案 {id} 不存在")))?;
        if proposal.proposal.status == TeamTemplateProposalStatus::Adopted {
            return Err(WorkflowStoreError::ProposalState(format!(
                "提案 {id} 已采纳，不能拒绝"
            )));
        }
        proposal.proposal.status = TeamTemplateProposalStatus::Rejected;
        tx.execute(
            "UPDATE workflow_proposals SET proposal_json=?2 WHERE proposal_id=?1",
            params![id, encode(&proposal.proposal)?],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "workflow_proposal_store_tests.rs"]
mod tests;
