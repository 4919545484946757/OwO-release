//! AI draft state and template adoption share the canonical workflow database.
//! No model calls, permission grants or workflow execution belong in this module.
use crate::workflow_revision_store::{
    append, current, expected_revision, user_precondition, WorkflowRevision, WorkflowRevisionStore,
    WorkflowStoreError,
};
use owo_agent_protocol::TeamTemplate;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::path::Path;

/// 落盘的一份草稿（候选）：模板 + 校验结果 + 生成上下文。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDraft {
    /// Host-only CAS identity; never accepted from model JSON or exposed as template revision.
    #[serde(skip)]
    pub storage_revision: u64,
    pub draft_id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_of: Option<String>,
    /// 迭代来源工作流 id（`based_on`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopted_revision: Option<u64>,
    /// 实际注入提示词的运行证据段落（诊断/审计：让用户看到模型看到了什么）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_evidence: Option<String>,
    /// 采纳后的验证结果（沿用基线目标再跑一次的结果；未验证为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_outcome: Option<Value>,
    pub template: TeamTemplate,
    /// 校验问题（空 = 可直接采纳）。
    pub errors: Vec<String>,
    /// 模型原始回复片段（诊断；解析失败时尤其有用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_excerpt: Option<String>,
    pub model: String,
    pub created_at: String,
}

const LIMIT: usize = 2 * 1024 * 1024;
const MIGRATION_KEY: &str = "legacy_ai_drafts_imported_v1";
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS workflow_drafts (
 draft_id TEXT PRIMARY KEY,
 storage_revision INTEGER NOT NULL CHECK(storage_revision > 0),
 document_json TEXT
);";

pub fn validate_draft_id(id: &str) -> Result<(), WorkflowStoreError> {
    if id.trim() != id
        || id.is_empty()
        || id.chars().count() > 64
        || id.contains("..")
        || id.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        || id.chars().any(char::is_control)
    {
        return Err(WorkflowStoreError::Invalid(
            "draft_id 长度非法或含路径/保留字符".into(),
        ));
    }
    Ok(())
}
fn conflict(expected: u64, actual: u64) -> WorkflowStoreError {
    WorkflowStoreError::DraftConflict {
        expected,
        current: actual,
    }
}
fn encode(draft: &WorkflowDraft) -> Result<String, WorkflowStoreError> {
    validate_draft_id(&draft.draft_id)?;
    let raw = serde_json::to_string(draft)?;
    if raw.len() > LIMIT {
        return Err(WorkflowStoreError::Invalid(
            "workflow draft exceeds 2 MiB".into(),
        ));
    }
    Ok(raw)
}
fn get_row(
    connection: &Connection,
    id: &str,
) -> Result<Option<(u64, Option<String>)>, WorkflowStoreError> {
    Ok(connection
        .query_row(
            "SELECT storage_revision,document_json FROM workflow_drafts WHERE draft_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}
fn decode(id: &str, revision: u64, raw: &str) -> Result<WorkflowDraft, WorkflowStoreError> {
    let mut draft: WorkflowDraft = serde_json::from_str(raw)?;
    validate_draft_id(&draft.draft_id)?;
    if draft.draft_id != id {
        return Err(WorkflowStoreError::Invalid(
            "stored draft identity mismatch".into(),
        ));
    }
    draft.storage_revision = revision;
    Ok(draft)
}
fn get(connection: &Connection, id: &str) -> Result<Option<WorkflowDraft>, WorkflowStoreError> {
    get_row(connection, id)?
        .and_then(|(revision, raw)| raw.map(|raw| (revision, raw)))
        .map(|(revision, raw)| decode(id, revision, &raw))
        .transpose()
}
fn next(revision: u64) -> Result<u64, WorkflowStoreError> {
    revision
        .checked_add(1)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(|| WorkflowStoreError::Invalid("draft revision exhausted".into()))
}

impl WorkflowRevisionStore {
    /// Explicit legacy directory: the historical draft path differs from the registry root.
    /// Import as one batch, retaining originals; after the marker no JSON fallback is allowed.
    pub fn migrate_legacy_drafts(&mut self, legacy_dir: &Path) -> Result<(), WorkflowStoreError> {
        self.connection.execute_batch(SCHEMA)?;
        let imported: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_store_meta WHERE key=?1)",
            [MIGRATION_KEY],
            |row| row.get(0),
        )?;
        if imported {
            return Ok(());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let imported: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_store_meta WHERE key=?1)",
            [MIGRATION_KEY],
            |row| row.get(0),
        )?;
        if !imported {
            match std::fs::read_dir(legacy_dir) {
                Ok(entries) => {
                    let mut entries = entries.collect::<Result<Vec<_>, _>>()?;
                    entries.sort_by_key(|entry| entry.file_name());
                    for entry in entries {
                        let path = entry.path();
                        if path.extension().and_then(|s| s.to_str()) != Some("json") {
                            continue;
                        }
                        if !entry.file_type()?.is_file() {
                            return Err(WorkflowStoreError::Invalid(format!(
                                "legacy draft is not a regular file: {}",
                                path.display()
                            )));
                        }
                        let mut raw = Vec::new();
                        std::fs::File::open(&path)?
                            .take(LIMIT as u64 + 1)
                            .read_to_end(&mut raw)?;
                        if raw.len() > LIMIT {
                            return Err(WorkflowStoreError::Invalid(format!(
                                "legacy draft exceeds 2 MiB: {}",
                                path.display()
                            )));
                        }
                        let draft: WorkflowDraft = serde_json::from_slice(&raw)?;
                        validate_draft_id(&draft.draft_id)?;
                        if path.file_stem().and_then(|s| s.to_str()) != Some(&draft.draft_id) {
                            return Err(WorkflowStoreError::Invalid(format!(
                                "legacy draft filename/identity mismatch: {}",
                                path.display()
                            )));
                        }
                        // Preserve historical adoption claims as recorded. They never supply a
                        // missing base revision or authorize replay against an unrelated head.
                        tx.execute("INSERT INTO workflow_drafts(draft_id,storage_revision,document_json) VALUES(?1,1,?2)",
                            params![draft.draft_id, encode(&draft)?])?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            tx.execute(
                "INSERT INTO workflow_store_meta(key,value) VALUES(?1,'1')",
                [MIGRATION_KEY],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn get_draft_record(&self, id: &str) -> Result<Option<WorkflowDraft>, WorkflowStoreError> {
        validate_draft_id(id)?;
        get(&self.connection, id)
    }
    pub fn list_draft_records(&self) -> Result<Vec<WorkflowDraft>, WorkflowStoreError> {
        let mut stmt=self.connection.prepare(
            "SELECT draft_id,storage_revision,document_json FROM workflow_drafts WHERE document_json IS NOT NULL ORDER BY draft_id")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, revision, raw) = row?;
            decode(&id, revision, &raw)
        })
        .collect()
    }
    /// CAS for creation and diagnostics/verification updates. Adoption fields can only
    /// change through adopt_reviewed_draft, which commits them with the template.
    pub fn save_draft_record(
        &mut self,
        draft: &WorkflowDraft,
    ) -> Result<WorkflowDraft, WorkflowStoreError> {
        let raw = encode(draft)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = get_row(&tx, &draft.draft_id)?;
        let actual = row.as_ref().map(|(r, _)| *r).unwrap_or(0);
        if actual != draft.storage_revision || row.as_ref().is_some_and(|(_, raw)| raw.is_none()) {
            return Err(conflict(draft.storage_revision, actual));
        }
        let previous = row
            .and_then(|(r, raw)| raw.map(|raw| (r, raw)))
            .map(|(r, raw)| decode(&draft.draft_id, r, &raw))
            .transpose()?;
        if previous.as_ref().and_then(|d| d.adopted_revision) != draft.adopted_revision
            || previous.as_ref().is_some_and(|d| {
                d.adopted_revision.is_some()
                    && serde_json::to_value(&d.template).ok()
                        != serde_json::to_value(&draft.template).ok()
            })
        {
            return Err(WorkflowStoreError::Invalid(
                "adoption identity must be updated atomically with the template".into(),
            ));
        }
        let revision = next(actual)?;
        tx.execute("INSERT INTO workflow_drafts(draft_id,storage_revision,document_json) VALUES(?1,?2,?3) ON CONFLICT(draft_id) DO UPDATE SET storage_revision=excluded.storage_revision,document_json=excluded.document_json",
            params![draft.draft_id,revision,raw])?;
        tx.commit()?;
        let mut result = draft.clone();
        result.storage_revision = revision;
        Ok(result)
    }
    pub fn delete_draft_record(
        &mut self,
        id: &str,
        expected: u64,
    ) -> Result<(), WorkflowStoreError> {
        validate_draft_id(id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (actual, raw) = get_row(&tx, id)?
            .ok_or_else(|| WorkflowStoreError::NotFound(format!("草稿不存在：{id}")))?;
        if raw.is_none() {
            return Err(WorkflowStoreError::NotFound(format!("草稿不存在：{id}")));
        }
        if actual != expected {
            return Err(conflict(expected, actual));
        }
        tx.execute(
            "UPDATE workflow_drafts SET storage_revision=?2,document_json=NULL WHERE draft_id=?1",
            params![id, next(actual)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// The caller runs the shared structural validator and live Worker catalog check.
    /// Read/compare/write both documents under one lock; never infer a fresh base.
    pub fn adopt_reviewed_draft(
        &mut self,
        reviewed: &WorkflowDraft,
        template: &TeamTemplate,
        expected: Option<u64>,
        allow_replay: bool,
    ) -> Result<(WorkflowDraft, WorkflowRevision), WorkflowStoreError> {
        validate_draft_id(&reviewed.draft_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut draft = get(&tx, &reviewed.draft_id)?.ok_or_else(|| {
            WorkflowStoreError::NotFound(format!("草稿不存在：{}", reviewed.draft_id))
        })?;
        let existing = current(&tx, &template.template_id)?;
        let same_document =
            serde_json::to_value(&draft.template)? == serde_json::to_value(template)?;
        if allow_replay && same_document && draft.adopted_revision.is_some() {
            if let Some(saved) = existing
                .as_ref()
                .filter(|r| Some(r.revision) == draft.adopted_revision)
            {
                if saved
                    .template
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()?
                    == Some(serde_json::to_value(template)?)
                {
                    let result = saved.clone();
                    tx.commit()?;
                    return Ok((draft, result));
                }
            }
        }
        if draft.storage_revision != reviewed.storage_revision {
            return Err(conflict(reviewed.storage_revision, draft.storage_revision));
        }
        if !matches!(template.origin.as_deref(), Some("user") | Some("community")) {
            return Err(WorkflowStoreError::Invalid(
                "invalid reviewed draft origin".into(),
            ));
        }
        let frozen = if draft.base_template_id.as_deref() == Some(template.template_id.as_str()) {
            draft.base_revision
        } else {
            None
        };
        let expected = expected.or(frozen);
        user_precondition(existing.as_ref(), expected)?;
        let base = expected_revision(existing.as_ref(), expected)?;
        let raw = serde_json::to_string(template)?;
        if raw.len() > LIMIT {
            return Err(WorkflowStoreError::Invalid("workflow exceeds 2 MiB".into()));
        }
        let saved = if let Some(record) = existing.filter(|r| {
            r.template
                .as_ref()
                .is_some_and(|t| serde_json::to_string(t).ok().as_deref() == Some(raw.as_str()))
        }) {
            record
        } else {
            append(&tx, &template.template_id, next(base)?, Some(template))?
        };
        draft.template = template.clone();
        draft.errors.clear();
        draft.adopted_revision = Some(saved.revision);
        draft.storage_revision = next(draft.storage_revision)?;
        tx.execute(
            "UPDATE workflow_drafts SET storage_revision=?2,document_json=?3 WHERE draft_id=?1",
            params![draft.draft_id, draft.storage_revision, encode(&draft)?],
        )?;
        tx.commit()?;
        Ok((draft, saved))
    }
}

#[cfg(test)]
#[path = "workflow_draft_store_tests.rs"]
mod tests;
