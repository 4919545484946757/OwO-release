use super::*;
use serde_json::json;
use std::sync::{Arc, Barrier};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("owo-draft-tx-{}", uuid::Uuid::new_v4())))
    }
    fn legacy(&self) -> std::path::PathBuf {
        self.0.join("legacy-drafts")
    }
    fn open(&self) -> WorkflowRevisionStore {
        let mut store = WorkflowRevisionStore::open(&self.0).unwrap();
        store.migrate_legacy_drafts(&self.legacy()).unwrap();
        store
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn draft(id: &str) -> WorkflowDraft {
    serde_json::from_value(json!({
        "draft_id":id,"description":"Local candidate","errors":[],"model":"fixture","created_at":"fixture",
        "template":{"template_id":"flow","name":"Candidate","mode":"team","origin":"user",
            "created_at":"fixture","applicability":"fixture",
            "roles":[{"node_id":"node-a","role":"author","tools":["read_file"]}]}
    })).unwrap()
}
fn adopt(
    store: &mut WorkflowRevisionStore,
    draft: &WorkflowDraft,
) -> Result<(WorkflowDraft, WorkflowRevision), WorkflowStoreError> {
    store.adopt_reviewed_draft(draft, &draft.template, None, true)
}

#[test]
fn atomic_adoption_rolls_back_both_documents_and_survives_reopen() {
    let f = Fixture::new();
    let mut store = f.open();
    let original = store.save_draft_record(&draft("d")).unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_draft_adopt BEFORE UPDATE ON workflow_drafts BEGIN SELECT RAISE(ABORT,'injected draft write failure'); END;").unwrap();
    assert!(matches!(
        adopt(&mut store, &original),
        Err(WorkflowStoreError::Sql(_))
    ));
    drop(store);
    let mut store = f.open();
    assert!(store.get("flow").unwrap().is_none());
    assert!(store.history("flow", None, 100).unwrap().is_empty());
    assert!(store
        .get_draft_record("d")
        .unwrap()
        .unwrap()
        .adopted_revision
        .is_none());
    store
        .connection
        .execute_batch("DROP TRIGGER fail_draft_adopt")
        .unwrap();
    let (adopted, saved) = adopt(&mut store, &original).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(adopted.adopted_revision, Some(1));
    assert_eq!(adopted.storage_revision, 2);
    drop(store);
    let mut store = f.open();
    let persisted = store.get_draft_record("d").unwrap().unwrap();
    assert_eq!(persisted.adopted_revision, Some(1));
    assert_eq!(adopt(&mut store, &original).unwrap().1.revision, 1);
    assert_eq!(store.history("flow", None, 100).unwrap().len(), 1);
    // A subsequent edit is not overwritten by retrying this old draft.
    let mut newer = persisted.template.clone();
    newer.name = "Later edit".into();
    store.save_user(&newer, Some(1)).unwrap();
    assert!(matches!(
        adopt(&mut store, &persisted),
        Err(WorkflowStoreError::RevisionRequired { current: 2 })
    ));
    assert_eq!(
        store.get("flow").unwrap().unwrap().template.unwrap().name,
        "Later edit"
    );
    store.delete_user("flow", Some(2)).unwrap();
    assert!(adopt(&mut store, &persisted).is_err());
    assert!(store.get("flow").unwrap().unwrap().template.is_none());
}

#[test]
fn concurrent_same_adoptions_commit_once_and_return_the_same_revision() {
    let f = Fixture::new();
    let mut store = f.open();
    let reviewed = store.save_draft_record(&draft("d")).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let mut connection = f.open();
            let reviewed = reviewed.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adopt(&mut connection, &reviewed)
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap().unwrap().1.revision, 1);
    }
    assert_eq!(store.history("flow", None, 100).unwrap().len(), 1);
    assert_eq!(
        store
            .get_draft_record("d")
            .unwrap()
            .unwrap()
            .storage_revision,
        2
    );
}

#[test]
fn different_concurrent_candidates_have_one_winner_and_do_not_rebase() {
    let f = Fixture::new();
    let mut store = f.open();
    let reviewed = store.save_draft_record(&draft("d")).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|i| {
            let mut connection = f.open();
            let reviewed = reviewed.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut candidate = reviewed.template.clone();
                candidate.name = format!("Candidate {i}");
                barrier.wait();
                connection.adopt_reviewed_draft(&reviewed, &candidate, None, true)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(WorkflowStoreError::DraftConflict { .. })))
            .count(),
        1
    );
    let record = store.get_draft_record("d").unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&record.template).unwrap(),
        serde_json::to_value(store.get("flow").unwrap().unwrap().template).unwrap()
    );
    assert_eq!(store.history("flow", None, 100).unwrap().len(), 1);
}

#[test]
fn verification_and_adoption_race_cannot_erase_each_other() {
    let f = Fixture::new();
    let mut store = f.open();
    let reviewed = store.save_draft_record(&draft("d")).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let mut writer = f.open();
    let mut adopter = f.open();
    let mut verification = reviewed.clone();
    verification.verify_outcome = Some(json!({"status":"failed"}));
    let b = barrier.clone();
    let save = std::thread::spawn(move || {
        b.wait();
        writer.save_draft_record(&verification)
    });
    let d = reviewed.clone();
    let run = std::thread::spawn(move || {
        barrier.wait();
        adopt(&mut adopter, &d)
    });
    let saved = save.join().unwrap();
    let adopted = run.join().unwrap();
    assert_ne!(saved.is_ok(), adopted.is_ok());
    assert!(
        matches!(saved, Err(WorkflowStoreError::DraftConflict { .. }))
            || matches!(adopted, Err(WorkflowStoreError::DraftConflict { .. }))
    );
    // Caller can explicitly reload and retry; no hidden overwrite or automatic rebasing.
    let current = store.get_draft_record("d").unwrap().unwrap();
    let (mut current, _) = adopt(&mut store, &current).unwrap();
    current.verify_outcome = Some(json!({"status":"succeeded"}));
    let current = store.save_draft_record(&current).unwrap();
    assert_eq!(current.adopted_revision, Some(1));
    assert_eq!(current.verify_outcome.unwrap()["status"], "succeeded");
    assert_eq!(store.history("flow", None, 100).unwrap().len(), 1);
}

#[test]
fn discard_and_adoption_race_has_one_winner_and_cannot_resurrect() {
    let f = Fixture::new();
    let mut store = f.open();
    let reviewed = store.save_draft_record(&draft("d")).unwrap();
    let mut deleter = f.open();
    let mut adopter = f.open();
    let barrier = Arc::new(Barrier::new(2));
    let b = barrier.clone();
    let d = reviewed.clone();
    let deleted = std::thread::spawn(move || {
        b.wait();
        deleter.delete_draft_record("d", d.storage_revision)
    });
    let adopted = std::thread::spawn(move || {
        barrier.wait();
        adopt(&mut adopter, &reviewed)
    });
    let deleted = deleted.join().unwrap();
    let adopted = adopted.join().unwrap();
    assert_ne!(deleted.is_ok(), adopted.is_ok());
    if deleted.is_ok() {
        assert!(matches!(adopted, Err(WorkflowStoreError::NotFound(_))));
        assert!(store.get("flow").unwrap().is_none());
    } else {
        assert!(matches!(
            deleted,
            Err(WorkflowStoreError::DraftConflict { .. })
        ));
        let adopted = store.get_draft_record("d").unwrap().unwrap();
        store
            .delete_draft_record("d", adopted.storage_revision)
            .unwrap();
        assert_eq!(store.get("flow").unwrap().unwrap().revision, 1);
    }
    assert!(matches!(
        store.save_draft_record(&draft("d")),
        Err(WorkflowStoreError::DraftConflict { .. })
    ));
    assert!(store.get_draft_record("d").unwrap().is_none());
}

#[test]
fn adoption_checks_frozen_base_and_protected_origin_inside_transaction() {
    let f = Fixture::new();
    let mut store = f.open();
    let mut d = draft("d");
    store.save_user(&d.template, None).unwrap();
    d.base_template_id = Some("flow".into());
    d.base_revision = Some(1);
    let reviewed = store.save_draft_record(&d).unwrap();
    let mut newer = d.template.clone();
    newer.name = "Human update".into();
    store.save_user(&newer, Some(1)).unwrap();
    assert!(matches!(
        adopt(&mut store, &reviewed),
        Err(WorkflowStoreError::Conflict {
            expected: 1,
            current: 2
        })
    ));
    assert!(store
        .get_draft_record("d")
        .unwrap()
        .unwrap()
        .adopted_revision
        .is_none());
    let mut builtin = newer.clone();
    builtin.origin = None;
    store.save(&builtin, Some(2)).unwrap();
    assert!(matches!(
        store.adopt_reviewed_draft(&reviewed, &reviewed.template, Some(3), false),
        Err(WorkflowStoreError::Protected(_))
    ));
    // A user-edited target is independent of the old base, but still uses destination CAS.
    let mut copy = reviewed.template.clone();
    copy.template_id = "copy".into();
    let (saved, revision) = store
        .adopt_reviewed_draft(&reviewed, &copy, None, true)
        .unwrap();
    assert_eq!(revision.revision, 1);
    assert_eq!(saved.template.template_id, "copy");
    assert_eq!(store.get("flow").unwrap().unwrap().revision, 3);
}

#[test]
fn draft_diagnostics_cannot_forge_adoption_or_overwrite_a_stale_version() {
    let f = Fixture::new();
    let mut store = f.open();
    let reviewed = store.save_draft_record(&draft("d")).unwrap();
    let mut stale = reviewed.clone();
    stale.verify_outcome = Some(json!({"status":"failed"}));
    let (mut adopted, _) = adopt(&mut store, &reviewed).unwrap();
    assert!(matches!(
        store.save_draft_record(&stale),
        Err(WorkflowStoreError::DraftConflict { .. })
    ));
    adopted.adopted_revision = None;
    assert!(matches!(
        store.save_draft_record(&adopted),
        Err(WorkflowStoreError::Invalid(_))
    ));
    adopted.adopted_revision = Some(1);
    adopted.template.name = "Forged".into();
    assert!(matches!(
        store.save_draft_record(&adopted),
        Err(WorkflowStoreError::Invalid(_))
    ));
    let mut forged = draft("new");
    forged.adopted_revision = Some(1);
    assert!(matches!(
        store.save_draft_record(&forged),
        Err(WorkflowStoreError::Invalid(_))
    ));
    let current = store.get_draft_record("d").unwrap().unwrap();
    store
        .delete_draft_record("d", current.storage_revision)
        .unwrap();
    assert!(store.save_draft_record(&current).is_err());
    assert!(store.get_draft_record("d").unwrap().is_none());
}

#[test]
fn legacy_migration_is_atomic_preserves_backups_and_does_not_revive_deleted_drafts() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.legacy()).unwrap();
    let a = serde_json::to_string(&draft("a")).unwrap();
    std::fs::write(f.legacy().join("a.json"), &a).unwrap();
    std::fs::write(f.legacy().join("b.json"), "{broken").unwrap();
    let mut store = WorkflowRevisionStore::open(&f.0).unwrap();
    assert!(store.migrate_legacy_drafts(&f.legacy()).is_err());
    assert!(store.list_draft_records().unwrap().is_empty());
    std::fs::write(
        f.legacy().join("b.json"),
        serde_json::to_vec(&draft("b")).unwrap(),
    )
    .unwrap();
    store.migrate_legacy_drafts(&f.legacy()).unwrap();
    assert_eq!(store.list_draft_records().unwrap().len(), 2);
    let imported = store.get_draft_record("a").unwrap().unwrap();
    assert_eq!(imported.storage_revision, 1);
    assert!(imported.base_revision.is_none());
    store.delete_draft_record("a", 1).unwrap();
    drop(store);
    let store = f.open();
    assert!(store.get_draft_record("a").unwrap().is_none());
    assert_eq!(
        std::fs::read_to_string(f.legacy().join("a.json")).unwrap(),
        a
    );
    assert_eq!(store.list_draft_records().unwrap().len(), 1);
}

#[test]
fn legacy_identity_size_and_document_errors_fail_closed_and_can_be_retried() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.legacy()).unwrap();
    let path = f.legacy().join("d.json");
    let mut store = WorkflowRevisionStore::open(&f.0).unwrap();
    for raw in [
        serde_json::to_vec(&draft("other")).unwrap(),
        vec![b' '; LIMIT + 1],
        b"{}".to_vec(),
    ] {
        std::fs::write(&path, raw).unwrap();
        assert!(store.migrate_legacy_drafts(&f.legacy()).is_err());
        assert!(store.list_draft_records().unwrap().is_empty());
    }
    let mut legacy = draft("d");
    legacy.adopted_revision = Some(17);
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    store.migrate_legacy_drafts(&f.legacy()).unwrap();
    let legacy = store.get_draft_record("d").unwrap().unwrap();
    assert_eq!(legacy.adopted_revision, Some(17));
    // An unverifiable historical claim cannot be treated as a successful replay.
    let mut current = legacy.template.clone();
    current.name = "Unrelated current".into();
    store.save_user(&current, None).unwrap();
    assert!(adopt(&mut store, &legacy).is_err());
    assert_eq!(
        store.get("flow").unwrap().unwrap().template.unwrap().name,
        "Unrelated current"
    );
}

#[test]
fn corrupt_canonical_rows_and_invalid_ids_do_not_fall_back_to_files() {
    let f = Fixture::new();
    let mut store = f.open();
    store.save_draft_record(&draft("d")).unwrap();
    store
        .connection
        .execute(
            "UPDATE workflow_drafts SET document_json='{broken' WHERE draft_id='d'",
            [],
        )
        .unwrap();
    assert!(store.get_draft_record("d").is_err());
    assert!(store.list_draft_records().is_err());
    assert!(adopt(&mut store, &draft("d")).is_err());
    for id in ["../bad", "a/b", "a\\b", "a:b", " id", "id ", "id\n"] {
        assert!(store.get_draft_record(id).is_err());
    }
    let mut oversized = draft("big");
    oversized.description = "x".repeat(LIMIT);
    assert!(store.save_draft_record(&oversized).is_err());
    assert!(store.get_draft_record("big").unwrap().is_none());
}
