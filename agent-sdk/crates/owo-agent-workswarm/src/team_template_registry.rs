pub use crate::workflow_draft_store::{validate_draft_id, WorkflowDraft};
pub use crate::workflow_proposal_store::WorkflowProposalRecord;
pub use crate::workflow_revision_store::{
    WorkflowRevision, WorkflowRevisionStore, WorkflowStoreError,
};
use owo_agent_protocol::{TeamMode, TeamTemplate, TeamTemplateProposal};
use std::path::{Path, PathBuf};
/// One template registry: transactional revisions + proposals (never executed automatically).
/// Legacy templates/*.json are imported once and retained untouched; SQLite is canonical afterward.
#[derive(Debug, Clone)]
pub struct TeamTemplateRegistry {
    dir: PathBuf,
}

impl TeamTemplateRegistry {
    /// All Daemon entry points use the same historical on-disk layout.
    /// `new` remains available for standalone coordinators and test registries.
    /// Keep the nested templates directory as the retained legacy migration source.
    pub fn for_workswarm_root(root: &Path) -> Self {
        Self::new(root.join("templates"))
    }

    pub fn new(root: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(root.join("templates"));
        let _ = std::fs::create_dir_all(root.join("proposals"));
        Self { dir: root }
    }

    /// Drafts use the same database as template revisions; no secondary writer.
    pub fn workflow_draft_store(
        &self,
        legacy_dir: &Path,
    ) -> Result<WorkflowRevisionStore, WorkflowStoreError> {
        let mut store = WorkflowRevisionStore::open(&self.dir)?;
        store.migrate_legacy_drafts(legacy_dir)?;
        Ok(store)
    }

    // -- 模板 --

    /// Compatibility writer for trusted installation/legacy callers. New user edits use CAS.
    pub fn save_template(&self, template: &TeamTemplate) -> std::io::Result<()> {
        self.save_template_checked(template, None)
            .map(|_| ())
            .map_err(std::io::Error::other)
    }
    pub fn save_template_checked(
        &self,
        template: &TeamTemplate,
        expected_revision: Option<u64>,
    ) -> Result<WorkflowRevision, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.save(template, expected_revision)
    }
    pub fn save_user_template(
        &self,
        template: &TeamTemplate,
        expected_revision: Option<u64>,
    ) -> Result<WorkflowRevision, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.save_user(template, expected_revision)
    }
    pub fn delete_user_template(
        &self,
        id: &str,
        expected_revision: Option<u64>,
    ) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.delete_user(id, expected_revision)
    }
    pub fn get_revision(&self, id: &str) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.get(id)
    }
    pub fn list_revisions(&self) -> Result<Vec<WorkflowRevision>, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.list()
    }
    pub fn revision_history(
        &self,
        id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<WorkflowRevision>, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.history(id, before, limit)
    }
    pub fn get_template(&self, id: &str) -> Option<TeamTemplate> {
        self.get_revision(id)
            .ok()
            .flatten()
            .and_then(|record| record.template)
    }
    pub fn list_templates(&self) -> Vec<TeamTemplate> {
        self.list_revisions()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| record.template)
            .collect()
    }

    /// Select the document and its identity from one canonical list snapshot.
    /// Storage errors remain errors; callers must not silently fall back to a default workflow.
    pub fn find_match_revision(
        &self,
        mode: TeamMode,
        objective: &str,
    ) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
        let objective_lower = objective.to_lowercase();
        let mut matched: Vec<WorkflowRevision> = self
            .list_revisions()?
            .into_iter()
            .filter(|record| {
                record.template.as_ref().is_some_and(|template| {
                    template.mode == mode
                        && template
                            .applicability
                            .split([' ', '，', ',', '、', '/', '\n', '\t'])
                            .filter(|token| token.chars().count() >= 2)
                            .any(|token| objective_lower.contains(&token.to_lowercase()))
                })
            })
            .collect();
        matched.sort_by(|a, b| {
            b.template
                .as_ref()
                .map(|t| &t.created_at)
                .cmp(&a.template.as_ref().map(|t| &t.created_at))
        });
        Ok(matched.into_iter().next())
    }

    /// Trusted compatibility reader. Runtime creation uses the checked revision selector above.
    pub fn find_match(&self, mode: TeamMode, objective: &str) -> Option<TeamTemplate> {
        self.find_match_revision(mode, objective)
            .ok()
            .flatten()
            .and_then(|record| record.template)
    }

    // -- 提案（只提案，不自动启用） --

    pub fn save_proposal(&self, p: &TeamTemplateProposal) -> std::io::Result<()> {
        self.save_proposal_checked(p)
            .map(|_| ())
            .map_err(std::io::Error::other)
    }
    pub fn save_proposal_checked(
        &self,
        p: &TeamTemplateProposal,
    ) -> Result<WorkflowProposalRecord, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.save_proposal_record(p)
    }
    pub fn get_proposal_record(
        &self,
        id: &str,
    ) -> Result<Option<WorkflowProposalRecord>, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.get_proposal_record(id)
    }
    pub fn list_proposal_records(&self) -> Result<Vec<WorkflowProposalRecord>, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.list_proposal_records()
    }
    pub fn get_proposal(&self, id: &str) -> Option<TeamTemplateProposal> {
        self.get_proposal_record(id)
            .ok()
            .flatten()
            .map(|r| r.proposal)
    }
    pub fn list_proposals(&self) -> Vec<TeamTemplateProposal> {
        self.list_proposal_records()
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.proposal)
            .collect()
    }
    pub fn install_if_absent(
        &self,
        template: &TeamTemplate,
        expected_revision: Option<u64>,
    ) -> Result<(WorkflowRevision, bool), WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.install_if_absent(template, expected_revision)
    }
    pub fn adopt_reviewed_proposal(
        &self,
        id: &str,
        hash: &str,
        template: &TeamTemplate,
        expected_revision: Option<u64>,
    ) -> Result<WorkflowRevision, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.adopt_reviewed_proposal(
            id,
            hash,
            template,
            expected_revision,
        )
    }
    pub fn reject_proposal_checked(&self, id: &str) -> Result<(), WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.reject_proposal_record(id)
    }

    /// 删除模板：返回 `Ok(true)` 表示确实删掉了一个（`Ok(false)` = 本来就不存在）。
    ///
    /// A retained tombstone replaces the active document; history and legacy backups remain.
    /// Callers still enforce the builtin/user origin policy.
    pub fn delete_template(&self, id: &str) -> std::io::Result<bool> {
        // Preserve the existing public error contract even though ids no longer become paths.
        if id.trim().is_empty()
            || id.contains("..")
            || id.contains(['/', '\u{005c}', ':', '*', '?', '"', '<', '>', '|'])
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("非法模板 id：{id}"),
            ));
        }
        self.delete_template_checked(id, None)
            .map(|record| record.is_some())
            .map_err(std::io::Error::other)
    }
    pub fn delete_template_checked(
        &self,
        id: &str,
        expected_revision: Option<u64>,
    ) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
        WorkflowRevisionStore::open(&self.dir)?.delete(id, expected_revision)
    }

    /// Trusted compatibility entry point. User-facing routes use the shared core validator.
    pub fn adopt_proposal(&self, proposal_id: &str) -> Result<TeamTemplate, String> {
        let record = self
            .get_proposal_record(proposal_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("提案 {proposal_id} 不存在"))?;
        let mut candidate = record.proposal.template.clone();
        if candidate.origin.is_none() {
            candidate.origin = Some("user".into());
        }
        let candidate = crate::workflow_patch::with_stable_node_ids(&candidate)?;
        self.adopt_reviewed_proposal(proposal_id, &record.proposal_hash, &candidate, None)
            .map_err(|e| e.to_string())?
            .template
            .ok_or_else(|| "adopted revision has no template".into())
    }

    /// Reject and adoption serialize through the same database transaction.
    pub fn reject_proposal(&self, proposal_id: &str) -> Result<(), String> {
        self.reject_proposal_checked(proposal_id)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_reopen_uses_one_canonical_revision_store_and_retains_history_after_delete() {
        let dir =
            std::env::temp_dir().join(format!("owo-template-registry-{}", uuid::Uuid::new_v4()));
        let registry = TeamTemplateRegistry::for_workswarm_root(&dir);
        let template:TeamTemplate=serde_json::from_value(serde_json::json!({"template_id":"flow","name":"First","applicability":"fixture","roles":[]})).unwrap();
        let record = registry.save_template_checked(&template, Some(0)).unwrap();
        assert_eq!(record.revision, 1);
        let reopened = TeamTemplateRegistry::for_workswarm_root(&dir);
        assert_eq!(reopened.list_templates().len(), 1);
        assert_eq!(reopened.get_template("flow").unwrap().name, "First");
        assert!(reopened.delete_template_checked("flow", Some(0)).is_err());
        assert!(reopened.delete_template("flow").unwrap());
        assert!(TeamTemplateRegistry::for_workswarm_root(&dir)
            .get_template("flow")
            .is_none());
        assert_eq!(
            reopened.revision_history("flow", None, 100).unwrap().len(),
            2
        );
        assert!(reopened.get_template("../outside").is_none());
        assert_eq!(
            reopened.delete_template("../outside").unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        drop(registry);
        drop(reopened);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn registry_adopted_proposal_replay_does_not_resurrect_a_deleted_template() {
        let dir =
            std::env::temp_dir().join(format!("owo-template-replay-{}", uuid::Uuid::new_v4()));
        let registry = TeamTemplateRegistry::new(dir.clone());
        let proposal:TeamTemplateProposal=serde_json::from_value(serde_json::json!({"proposal_id":"proposal","source_team_id":"team","template":{"template_id":"flow","name":"Flow","applicability":"fixture","roles":[]},"evidence":[],"created_at":"fixture"})).unwrap();
        registry.save_proposal(&proposal).unwrap();
        registry.adopt_proposal("proposal").unwrap();
        registry.adopt_proposal("proposal").unwrap();
        registry.delete_template("flow").unwrap();
        assert!(registry
            .adopt_proposal("proposal")
            .unwrap_err()
            .contains("不会重建"));
        assert!(registry.get_template("flow").is_none());
        assert_eq!(
            registry.revision_history("flow", None, 100).unwrap().len(),
            2
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
