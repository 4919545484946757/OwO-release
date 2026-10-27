use super::*;
use crate::workflow_revision_store::{WorkflowRevisionStore, WorkflowStoreError};
use rusqlite::Connection;
use serde_json::json;
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("owo-proposal-tx-{}", uuid::Uuid::new_v4())))
    }
    fn open(&self) -> WorkflowRevisionStore {
        WorkflowRevisionStore::open(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn proposal(id: &str) -> TeamTemplateProposal {
    serde_json::from_value(json!({
        "proposal_id":id,"source_team_id":"team","created_at":"fixture",
        "template":{"template_id":"flow","name":"Candidate","mode":"team","origin":"user",
        "created_at":"fixture","applicability":"fixture","roles":[{"node_id":"node-a","role":"author","tools":["read_file"]}]}
    })).unwrap()
}
fn adopt(
    store: &mut WorkflowRevisionStore,
    p: &WorkflowProposalRecord,
) -> Result<WorkflowRevision, WorkflowStoreError> {
    store.adopt_reviewed_proposal(
        &p.proposal.proposal_id,
        &p.proposal_hash,
        &p.proposal.template,
        None,
    )
}
#[test]
fn adoption_rolls_back_both_records_if_the_status_write_fails() {
    let f = Fixture::new();
    let mut store = f.open();
    let p = store.save_proposal_record(&proposal("p")).unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_adopt BEFORE UPDATE ON workflow_proposals BEGIN SELECT RAISE(ABORT,'injected disk write failure'); END;").unwrap();
    assert!(adopt(&mut store, &p).is_err());
    drop(store);
    let mut store = f.open();
    assert!(store.get("flow").unwrap().is_none());
    assert!(store.history("flow", None, 100).unwrap().is_empty());
    assert_eq!(
        store
            .get_proposal_record("p")
            .unwrap()
            .unwrap()
            .proposal
            .status,
        TeamTemplateProposalStatus::Proposed
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_adopt")
        .unwrap();
    let saved = adopt(&mut store, &p).unwrap();
    assert_eq!(saved.revision, 1);
    drop(store);
    let mut store = f.open();
    let adopted = store.get_proposal_record("p").unwrap().unwrap();
    assert_eq!(adopted.adopted_revision, Some(1));
    assert_eq!(adopted.proposal.status, TeamTemplateProposalStatus::Adopted);
    let mut newer = proposal("unused").template;
    newer.name = "Later edit".into();
    store.save_user(&newer, Some(1)).unwrap();
    let replay = adopt(&mut store, &adopted).unwrap();
    assert_eq!(replay.revision, 1);
    assert_eq!(replay.template.unwrap().name, "Candidate");
    assert_eq!(store.get("flow").unwrap().unwrap().revision, 2);
    store.delete_user("flow", Some(2)).unwrap();
    assert!(matches!(
        adopt(&mut store, &adopted),
        Err(WorkflowStoreError::ProposalState(_))
    ));
    assert!(store.get("flow").unwrap().unwrap().template.is_none());
}
#[test]
fn proposal_base_is_frozen_and_stale_adoption_cannot_overwrite_newer_work() {
    let f = Fixture::new();
    let mut store = f.open();
    let original = proposal("p").template;
    store.save_user(&original, None).unwrap();
    let p = store.save_proposal_record(&proposal("p")).unwrap();
    assert_eq!(p.base_revision, 1);
    let mut edited = original.clone();
    edited.name = "Concurrent edit".into();
    store.save_user(&edited, Some(1)).unwrap();
    assert!(matches!(
        adopt(&mut store, &p),
        Err(WorkflowStoreError::Conflict {
            expected: 1,
            current: 2
        })
    ));
    assert!(matches!(
        store.adopt_reviewed_proposal("p", &p.proposal_hash, &p.proposal.template, Some(2)),
        Err(WorkflowStoreError::ProposalState(_))
    ));
    assert_eq!(
        store.get("flow").unwrap().unwrap().template.unwrap().name,
        "Concurrent edit"
    );
    assert_eq!(
        store
            .get_proposal_record("p")
            .unwrap()
            .unwrap()
            .proposal
            .status,
        TeamTemplateProposalStatus::Proposed
    );
    let fresh = store.save_proposal_record(&proposal("fresh")).unwrap();
    assert_eq!(adopt(&mut store, &fresh).unwrap().revision, 3);
}
#[test]
fn proposal_identity_payload_and_terminal_status_are_immutable() {
    let f = Fixture::new();
    let mut store = f.open();
    let input = proposal("p");
    let p = store.save_proposal_record(&input).unwrap();
    assert!(matches!(
        store.adopt_reviewed_proposal("p", "wrong", &input.template, None),
        Err(WorkflowStoreError::ProposalState(_))
    ));
    assert!(store.get("flow").unwrap().is_none());
    let mut changed = input.clone();
    changed.template.name = "Changed without review".into();
    assert!(store.save_proposal_record(&changed).is_err());
    store.reject_proposal_record("p").unwrap();
    store.save_proposal_record(&input).unwrap();
    assert_eq!(
        store
            .get_proposal_record("p")
            .unwrap()
            .unwrap()
            .proposal
            .status,
        TeamTemplateProposalStatus::Rejected
    );
    assert!(adopt(&mut store, &p).is_err());
    for id in ["../outside", "bad/name", " bad", "bad\u{0000}name"] {
        assert!(store.save_proposal_record(&proposal(id)).is_err());
        assert!(store.get_proposal_record(id).is_err());
    }
    assert_eq!(store.list_proposal_records().unwrap().len(), 1);
}
#[test]
fn proposal_adopt_and_reject_race_has_exactly_one_winner() {
    let f = Fixture::new();
    let p = f.open().save_proposal_record(&proposal("p")).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|i| {
            let root = f.0.clone();
            let barrier = barrier.clone();
            let p = p.clone();
            std::thread::spawn(move || {
                let mut store = WorkflowRevisionStore::open(&root).unwrap();
                barrier.wait();
                if i == 0 {
                    adopt(&mut store, &p).map(|_| ())
                } else {
                    store.reject_proposal_record("p")
                }
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(WorkflowStoreError::ProposalState(_))))
            .count(),
        1
    );
    let store = f.open();
    let record = store.get_proposal_record("p").unwrap().unwrap();
    assert_eq!(
        store.get("flow").unwrap().is_some(),
        record.proposal.status == TeamTemplateProposalStatus::Adopted
    );
}
#[test]
fn builtin_protection_and_candidate_origin_are_checked_inside_adoption() {
    let f = Fixture::new();
    let mut store = f.open();
    let mut builtin = proposal("builtin").template;
    builtin.origin = None;
    store.save(&builtin, None).unwrap();
    let p = store.save_proposal_record(&proposal("p")).unwrap();
    assert!(matches!(
        adopt(&mut store, &p),
        Err(WorkflowStoreError::Protected(_))
    ));
    assert_eq!(store.get("flow").unwrap().unwrap().revision, 1);
    let mut wrong_target = p.proposal.template.clone();
    wrong_target.template_id = "different".into();
    assert!(matches!(
        store.adopt_reviewed_proposal("p", &p.proposal_hash, &wrong_target, None),
        Err(WorkflowStoreError::Invalid(_))
    ));
}
#[test]
fn install_is_atomic_non_destructive_and_requires_review_of_deletions() {
    let f = Fixture::new();
    let original = proposal("p").template;
    drop(f.open());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let root = f.0.clone();
            let barrier = barrier.clone();
            let original = original.clone();
            std::thread::spawn(move || {
                let mut store = WorkflowRevisionStore::open(&root).unwrap();
                barrier.wait();
                store.install_if_absent(&original, None).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.1).count(), 1);
    assert!(results.iter().all(|r| r.0.revision == 1));
    let mut store = f.open();
    let mut replacement = original.clone();
    replacement.name = "Clobber".into();
    assert!(!store.install_if_absent(&replacement, Some(0)).unwrap().1);
    assert_eq!(
        store.get("flow").unwrap().unwrap().template.unwrap().name,
        "Candidate"
    );
    store.delete_user("flow", Some(1)).unwrap();
    assert!(matches!(
        store.install_if_absent(&original, None),
        Err(WorkflowStoreError::Conflict { current: 2, .. })
    ));
    assert!(store.get("flow").unwrap().unwrap().template.is_none());
    assert_eq!(
        store
            .install_if_absent(&original, Some(2))
            .unwrap()
            .0
            .revision,
        3
    );
}
#[test]
fn legacy_proposals_migrate_once_transactionally_without_guessing_their_base() {
    let f = Fixture::new();
    let dir = f.0.join("proposals");
    std::fs::create_dir_all(&dir).unwrap();
    let raw = serde_json::to_vec(&proposal("p")).unwrap();
    std::fs::write(dir.join("p.json"), &raw).unwrap();
    std::fs::write(dir.join("z-bad.json"), b"{bad").unwrap();
    assert!(WorkflowRevisionStore::open(&f.0).is_err());
    let sql = Connection::open(f.0.join("workflow-templates.sqlite3")).unwrap();
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM workflow_proposals", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    drop(sql);
    std::fs::remove_file(dir.join("z-bad.json")).unwrap();
    let mut store = f.open();
    let p = store.get_proposal_record("p").unwrap().unwrap();
    assert!(p.legacy_imported);
    assert_eq!(p.base_revision, 0);
    store.save_user(&proposal("p").template, None).unwrap();
    assert!(matches!(
        adopt(&mut store, &p),
        Err(WorkflowStoreError::Conflict { current: 1, .. })
    ));
    drop(store);
    assert_eq!(std::fs::read(dir.join("p.json")).unwrap(), raw);
    std::fs::write(dir.join("p.json"), b"{changed legacy backup").unwrap();
    assert_eq!(f.open().list_proposal_records().unwrap().len(), 1);
}
#[test]
fn legacy_adopted_proposal_does_not_invent_an_adopted_revision() {
    let f = Fixture::new();
    let dir = f.0.join("proposals");
    std::fs::create_dir_all(&dir).unwrap();
    let mut input = proposal("p");
    input.status = TeamTemplateProposalStatus::Adopted;
    std::fs::write(dir.join("p.json"), serde_json::to_vec(&input).unwrap()).unwrap();
    let mut store = f.open();
    store.save_user(&input.template, None).unwrap();
    let p = store.get_proposal_record("p").unwrap().unwrap();
    assert_eq!(p.adopted_revision, None);
    assert!(matches!(
        adopt(&mut store, &p),
        Err(WorkflowStoreError::ProposalState(_))
    ));
    assert_eq!(store.history("flow", None, 100).unwrap().len(), 1);
}
