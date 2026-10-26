//! Transactional revisions for TeamTemplate documents. No execution or permission decisions.
//! Legacy JSON is imported once, in one transaction, and retained untouched as a backup.
use chrono::Utc;
use owo_agent_protocol::TeamTemplate;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum WorkflowStoreError {
    #[error("workflow revision conflict: expected {expected}, current {current}")]
    Conflict { expected: u64, current: u64 },
    #[error("workflow draft conflict: expected {expected}, current {current}")]
    DraftConflict { expected: u64, current: u64 },
    #[error("expected_revision is required; current revision is {current}")]
    RevisionRequired { current: u64 },
    #[error("builtin workflow cannot be changed: {0}")]
    Protected(String),
    #[error("workflow storage: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("workflow storage: {0}")]
    Io(#[from] std::io::Error),
    #[error("workflow document: {0}")]
    Json(#[from] serde_json::Error),
    #[error("workflow document: {0}")]
    Invalid(String),
    #[error("workflow proposal not found: {0}")]
    NotFound(String),
    #[error("workflow proposal state: {0}")]
    ProposalState(String),
}

/// `revision` is host state, independent of a template's community `version`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRevision {
    pub template_id: String,
    pub revision: u64,
    pub committed_at: String,
    pub content_hash: Option<String>,
    /// A null document is a retained deletion, never a request to reload legacy JSON.
    pub template: Option<TeamTemplate>,
}

pub struct WorkflowRevisionStore {
    pub(super) connection: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS workflow_heads (
 template_id TEXT PRIMARY KEY, revision INTEGER NOT NULL CHECK(revision > 0)
);
CREATE TABLE IF NOT EXISTS workflow_revisions (
 template_id TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision > 0),
 committed_at TEXT NOT NULL, content_hash TEXT, template_json TEXT,
 PRIMARY KEY(template_id,revision)
);
CREATE TABLE IF NOT EXISTS workflow_store_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";
const ROW: &str = "r.template_id,r.revision,r.committed_at,r.content_hash,r.template_json";

fn invalid(message: impl Into<String>) -> WorkflowStoreError {
    WorkflowStoreError::Invalid(message.into())
}
pub(super) fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRevision> {
    let raw: Option<String> = row.get(4)?;
    let template = raw
        .map(|raw| serde_json::from_str(&raw))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(WorkflowRevision {
        template_id: row.get(0)?,
        revision: row.get(1)?,
        committed_at: row.get(2)?,
        content_hash: row.get(3)?,
        template,
    })
}
pub(super) fn current(
    connection: &Connection,
    id: &str,
) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
    Ok(connection.query_row(
        &format!("SELECT {ROW} FROM workflow_heads h JOIN workflow_revisions r ON r.template_id=h.template_id AND r.revision=h.revision WHERE h.template_id=?1"),
        [id], decode,
    ).optional()?)
}
pub(super) fn append(
    connection: &Connection,
    id: &str,
    revision: u64,
    template: Option<&TeamTemplate>,
) -> Result<WorkflowRevision, WorkflowStoreError> {
    let raw = template.map(serde_json::to_string).transpose()?;
    let hash = raw
        .as_ref()
        .map(|raw| format!("{:x}", Sha256::digest(raw.as_bytes())));
    let at = Utc::now().to_rfc3339();
    connection.execute("INSERT INTO workflow_revisions(template_id,revision,committed_at,content_hash,template_json) VALUES(?1,?2,?3,?4,?5)", params![id,revision,at,hash,raw])?;
    connection.execute("INSERT INTO workflow_heads(template_id,revision) VALUES(?1,?2) ON CONFLICT(template_id) DO UPDATE SET revision=excluded.revision", params![id,revision])?;
    Ok(WorkflowRevision {
        template_id: id.into(),
        revision,
        committed_at: at,
        content_hash: hash,
        template: template.cloned(),
    })
}
pub(super) fn expected_revision(
    current: Option<&WorkflowRevision>,
    expected: Option<u64>,
) -> Result<u64, WorkflowStoreError> {
    let actual = current.map(|record| record.revision).unwrap_or(0);
    if let Some(expected) = expected {
        if expected != actual {
            return Err(WorkflowStoreError::Conflict {
                expected,
                current: actual,
            });
        }
    }
    Ok(actual)
}

pub(super) fn user_precondition(
    record: Option<&WorkflowRevision>,
    expected: Option<u64>,
) -> Result<(), WorkflowStoreError> {
    if let Some(record) = record {
        if record.template.as_ref().is_some_and(|template| {
            !matches!(template.origin.as_deref(), Some("user") | Some("community"))
        }) {
            return Err(WorkflowStoreError::Protected(record.template_id.clone()));
        }
        if expected.is_none() {
            return Err(WorkflowStoreError::RevisionRequired {
                current: record.revision,
            });
        }
    }
    Ok(())
}

impl WorkflowRevisionStore {
    pub fn open(root: &Path) -> Result<Self, WorkflowStoreError> {
        std::fs::create_dir_all(root)?;
        let mut connection = Connection::open(root.join("workflow-templates.sqlite3"))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(SCHEMA)?;
        let imported: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_store_meta WHERE key='legacy_json_imported_v1')",
            [],
            |row| row.get(0),
        )?;
        if imported {
            crate::workflow_proposal_store::migrate_proposals(&mut connection, root)?;
            return Ok(Self { connection });
        }
        // A second process may win migration while we wait for BEGIN IMMEDIATE; check inside it.
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let imported: Option<String> = transaction
            .query_row(
                "SELECT value FROM workflow_store_meta WHERE key='legacy_json_imported_v1'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if imported.is_none() {
            let dir = root.join("templates");
            if dir.exists() {
                let mut entries = std::fs::read_dir(&dir)?.collect::<Result<Vec<_>, _>>()?;
                entries.sort_by_key(|entry| entry.file_name());
                for entry in entries {
                    let path = entry.path();
                    if path.extension().and_then(|part| part.to_str()) != Some("json") {
                        continue;
                    }
                    if !entry.file_type()?.is_file() {
                        return Err(invalid(format!(
                            "legacy template is not a regular file: {}",
                            path.display()
                        )));
                    }
                    let mut raw = Vec::new();
                    std::fs::File::open(&path)?
                        .take(2 * 1024 * 1024 + 1)
                        .read_to_end(&mut raw)?;
                    if raw.len() > 2 * 1024 * 1024 {
                        return Err(invalid(format!(
                            "legacy template exceeds 2 MiB: {}",
                            path.display()
                        )));
                    }
                    let template: TeamTemplate = serde_json::from_slice(&raw)?;
                    if path.file_stem().and_then(|part| part.to_str())
                        != Some(&template.template_id)
                    {
                        return Err(invalid(format!(
                            "legacy filename does not match template id: {}",
                            path.display()
                        )));
                    }
                    if current(&transaction, &template.template_id)?.is_none() {
                        append(&transaction, &template.template_id, 1, Some(&template))?;
                    }
                }
            }
            transaction.execute("INSERT INTO workflow_store_meta(key,value) VALUES('legacy_json_imported_v1','complete')", [])?;
        }
        transaction.commit()?;
        crate::workflow_proposal_store::migrate_proposals(&mut connection, root)?;
        Ok(Self { connection })
    }

    /// Install is atomic and non-destructive. A retained deletion requires its reviewed revision.
    pub fn install_if_absent(
        &mut self,
        template: &TeamTemplate,
        expected: Option<u64>,
    ) -> Result<(WorkflowRevision, bool), WorkflowStoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = current(&tx, &template.template_id)?;
        if let Some(record) = existing.as_ref().filter(|r| r.template.is_some()) {
            let record = record.clone();
            tx.commit()?;
            return Ok((record, false));
        }
        let base = expected_revision(existing.as_ref(), Some(expected.unwrap_or(0)))?;
        let raw = serde_json::to_string(template)?;
        if template.template_id.is_empty() || raw.len() > 2 * 1024 * 1024 {
            return Err(WorkflowStoreError::Invalid(
                "invalid installation document".into(),
            ));
        }
        let next = base
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| WorkflowStoreError::Invalid("revision exhausted".into()))?;
        let result = append(&tx, &template.template_id, next, Some(template))?;
        tx.commit()?;
        Ok((result, true))
    }

    pub fn get(&self, id: &str) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
        current(&self.connection, id)
    }
    pub fn list(&self) -> Result<Vec<WorkflowRevision>, WorkflowStoreError> {
        let mut statement = self.connection.prepare(&format!("SELECT {ROW} FROM workflow_heads h JOIN workflow_revisions r ON r.template_id=h.template_id AND r.revision=h.revision WHERE r.template_json IS NOT NULL ORDER BY r.template_id"))?;
        let records = statement
            .query_map([], decode)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }
    /// History is paged newest first. The cursor is an exclusive revision number.
    pub fn history(
        &self,
        id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<WorkflowRevision>, WorkflowStoreError> {
        let cursor = before
            .map(i64::try_from)
            .transpose()
            .map_err(|_| invalid("history cursor exceeds storage range"))?
            .unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare(&format!("SELECT {ROW} FROM workflow_revisions r WHERE r.template_id=?1 AND r.revision<?2 ORDER BY r.revision DESC LIMIT ?3"))?;
        let records = statement
            .query_map(params![id, cursor, limit.clamp(1, 100)], decode)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }
    pub fn save(
        &mut self,
        template: &TeamTemplate,
        expected: Option<u64>,
    ) -> Result<WorkflowRevision, WorkflowStoreError> {
        self.save_inner(template, expected, false)
    }
    /// User-facing mutations must state the revision of any existing document or tombstone.
    pub fn save_user(
        &mut self,
        template: &TeamTemplate,
        expected: Option<u64>,
    ) -> Result<WorkflowRevision, WorkflowStoreError> {
        if !matches!(template.origin.as_deref(), Some("user") | Some("community")) {
            return Err(invalid("user workflow origin must be user/community"));
        }
        self.save_inner(template, expected, true)
    }
    fn save_inner(
        &mut self,
        template: &TeamTemplate,
        expected: Option<u64>,
        user_only: bool,
    ) -> Result<WorkflowRevision, WorkflowStoreError> {
        if template.template_id.is_empty() || template.template_id.len() > 512 {
            return Err(invalid("template id is empty or too long"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = current(&transaction, &template.template_id)?;
        if user_only {
            user_precondition(existing.as_ref(), expected)?;
        }
        let revision = expected_revision(existing.as_ref(), expected)?;
        // Retry with the same current document is idempotent, but never bypass a stale CAS.
        let raw = serde_json::to_string(template)?;
        if raw.len() > 2 * 1024 * 1024 {
            return Err(invalid("workflow document exceeds 2 MiB"));
        }
        let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
        if let Some(record) = existing {
            if record.content_hash.as_deref() == Some(&hash) {
                transaction.commit()?;
                return Ok(record);
            }
        }
        let next = revision
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| invalid("revision exhausted"))?;
        let result = append(&transaction, &template.template_id, next, Some(template))?;
        transaction.commit()?;
        Ok(result)
    }
    pub fn delete(
        &mut self,
        id: &str,
        expected: Option<u64>,
    ) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
        self.delete_inner(id, expected, false)
    }
    pub fn delete_user(
        &mut self,
        id: &str,
        expected: Option<u64>,
    ) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
        self.delete_inner(id, expected, true)
    }
    fn delete_inner(
        &mut self,
        id: &str,
        expected: Option<u64>,
        user_only: bool,
    ) -> Result<Option<WorkflowRevision>, WorkflowStoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = current(&transaction, id)?;
        if user_only {
            user_precondition(existing.as_ref(), expected)?;
        }
        let revision = expected_revision(existing.as_ref(), expected)?;
        if existing
            .as_ref()
            .is_none_or(|record| record.template.is_none())
        {
            transaction.commit()?;
            return Ok(None);
        }
        let next = revision
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| invalid("revision exhausted"))?;
        let result = append(&transaction, id, next, None)?;
        transaction.commit()?;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("owo-workflow-revision-{}", uuid::Uuid::new_v4())),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn template(name: &str) -> TeamTemplate {
        serde_json::from_value(serde_json::json!({"template_id":"test-flow","name":name,"mode":"team","applicability":"fixture","roles":[{"node_id":"node-a","role":"Agent","verify":"non_empty"}]})).unwrap()
    }

    #[test]
    fn workflow_revisions_cas_history_and_restart_preserve_exact_document() {
        let fixture = Fixture::new();
        let mut store = WorkflowRevisionStore::open(&fixture.0).unwrap();
        let a = template("First");
        let record = store.save(&a, Some(0)).unwrap();
        assert_eq!(record.revision, 1);
        assert_eq!(record.content_hash.as_deref().unwrap().len(), 64);
        assert_eq!(store.save(&a, Some(1)).unwrap().revision, 1);
        let mut b = template("Second");
        b.version = Some("community-v7".into());
        let record = store.save(&b, Some(1)).unwrap();
        assert_eq!(record.revision, 2);
        assert!(matches!(
            store.save(&a, Some(1)),
            Err(WorkflowStoreError::Conflict {
                expected: 1,
                current: 2
            })
        ));
        drop(store);
        let store = WorkflowRevisionStore::open(&fixture.0).unwrap();
        assert_eq!(
            store
                .get("test-flow")
                .unwrap()
                .unwrap()
                .template
                .unwrap()
                .name,
            "Second"
        );
        let history = store.history("test-flow", None, 1).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].revision, 2);
        let history = store.history("test-flow", Some(2), 1000).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].template.as_ref().unwrap().name, "First");
    }
    #[test]
    fn workflow_revisions_import_once_and_tombstones_never_resurrect_legacy_json() {
        let fixture = Fixture::new();
        let dir = fixture.0.join("templates");
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = dir.join("test-flow.json");
        let raw = serde_json::to_vec(&template("Legacy")).unwrap();
        std::fs::write(&legacy, &raw).unwrap();
        let mut store = WorkflowRevisionStore::open(&fixture.0).unwrap();
        assert_eq!(store.get("test-flow").unwrap().unwrap().revision, 1);
        assert!(store.delete("test-flow", Some(0)).is_err());
        let deleted = store.delete("test-flow", Some(1)).unwrap().unwrap();
        assert_eq!(deleted.revision, 2);
        assert!(deleted.template.is_none());
        drop(store);
        assert_eq!(std::fs::read(&legacy).unwrap(), raw);
        let mut store = WorkflowRevisionStore::open(&fixture.0).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(store.get("test-flow").unwrap().unwrap().template.is_none());
        assert!(matches!(
            store.save(&template("Recreated"), Some(0)),
            Err(WorkflowStoreError::Conflict { current: 2, .. })
        ));
        assert_eq!(
            store
                .save(&template("Recreated"), Some(2))
                .unwrap()
                .revision,
            3
        );
        assert_eq!(store.history("test-flow", None, 100).unwrap().len(), 3);
    }
    #[test]
    fn workflow_revisions_migration_is_all_or_nothing_and_can_retry_after_invalid_source_repair() {
        let fixture = Fixture::new();
        let dir = fixture.0.join("templates");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("test-flow.json"),
            serde_json::to_vec(&template("Valid")).unwrap(),
        )
        .unwrap();
        let broken = dir.join("z-broken.json");
        std::fs::write(&broken, b"{bad").unwrap();
        assert!(WorkflowRevisionStore::open(&fixture.0).is_err());
        let connection = Connection::open(fixture.0.join("workflow-templates.sqlite3")).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM workflow_revisions", [], |r| r
                    .get::<_, usize>(0))
                .unwrap(),
            0
        );
        drop(connection);
        std::fs::remove_file(broken).unwrap();
        let store = WorkflowRevisionStore::open(&fixture.0).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }
    #[test]
    fn workflow_revisions_concurrent_independent_connections_have_one_cas_winner() {
        let fixture = Fixture::new();
        let mut store = WorkflowRevisionStore::open(&fixture.0).unwrap();
        store.save(&template("Base"), Some(0)).unwrap();
        drop(store);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2)
            .map(|i| {
                let root = fixture.0.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut store = WorkflowRevisionStore::open(&root).unwrap();
                    barrier.wait();
                    store.save(&template(&format!("Writer {i}")), Some(1))
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(WorkflowStoreError::Conflict { current: 2, .. })))
                .count(),
            1
        );
        let store = WorkflowRevisionStore::open(&fixture.0).unwrap();
        assert_eq!(store.history("test-flow", None, 100).unwrap().len(), 2);
    }
    #[test]
    fn workflow_user_mutations_require_revision_and_protect_builtin_inside_transaction() {
        let fixture = Fixture::new();
        let mut store = WorkflowRevisionStore::open(&fixture.0).unwrap();
        let mut user = template("User");
        user.origin = Some("user".into());
        store.save_user(&user, None).unwrap();
        user.name = "Changed".into();
        assert!(matches!(
            store.save_user(&user, None),
            Err(WorkflowStoreError::RevisionRequired { current: 1 })
        ));
        assert!(matches!(
            store.delete_user("test-flow", None),
            Err(WorkflowStoreError::RevisionRequired { current: 1 })
        ));
        store.save_user(&user, Some(1)).unwrap();
        assert!(matches!(
            store.save_user(&user, Some(1)),
            Err(WorkflowStoreError::Conflict { current: 2, .. })
        ));
        store.delete_user("test-flow", Some(2)).unwrap();
        assert!(matches!(
            store.save_user(&user, None),
            Err(WorkflowStoreError::RevisionRequired { current: 3 })
        ));
        let builtin = template("Builtin");
        store.save(&builtin, Some(3)).unwrap();
        assert!(matches!(
            store.save_user(&user, Some(4)),
            Err(WorkflowStoreError::Protected(_))
        ));
        assert!(matches!(
            store.delete_user("test-flow", Some(4)),
            Err(WorkflowStoreError::Protected(_))
        ));
        assert_eq!(
            store
                .get("test-flow")
                .unwrap()
                .unwrap()
                .template
                .unwrap()
                .name,
            "Builtin"
        );
        assert_eq!(store.history("test-flow", None, 100).unwrap().len(), 4);
    }
}
