use super::*;
use owo_agent_protocol::{ProjectSpaceStatus, TeamRunStatus};
use std::sync::{Arc, Barrier};

fn store() -> SqliteProjectSpaceStore {
    SqliteProjectSpaceStore::from_connection(Connection::open_in_memory().unwrap()).unwrap()
}
fn team(id: &str) -> TeamRun {
    serde_json::from_value(json!({
        "team_id":id,"goal_id":"SECRET_GOAL","mode":"team","members":[],
        "project_space_id":"project","task_graph_ref":null,"template_id":null,
        "shared_context_refs":[],"budget":{},"human_policy":null,"strategy_decision":null,
        "status":"created","created_at":"2026-10-25T00:00:00Z","updated_at":"2026-10-25T00:00:00Z"
    }))
    .unwrap()
}
fn space() -> ProjectSpace {
    serde_json::from_value(json!({
        "project_id":"project","goal_id":null,"team_id":"team","tasks":[],"artifacts":[],
        "decisions":[],"approvals":[],"discussions":[],"activity_stream":[],
        "delivery_manifest_ref":null,"rework_tasks":[],"version":1,"status":"active",
        "created_at":"2026-10-25T00:00:00Z","updated_at":"2026-10-25T00:00:00Z"
    }))
    .unwrap()
}
fn artifact() -> Artifact {
    serde_json::from_value(json!({
        "artifact_id":"art","kind":"output","version":1,"producer":"worker",
        "content_ref":"cas://sha256:payload","schema_ref":null,"source_refs":[],
        "review_state":"draft","supersedes_artifact_id":null,"created_at":"2026-10-25T00:00:00Z",
        "team_id":"team","task_id":"task","attempt_id":"attempt"
    }))
    .unwrap()
}
fn handoff() -> HandoffRecord {
    serde_json::from_value(json!({
        "handoff_id":"handoff","from_member":"writer","to_member":"reviewer",
        "completed_summary":"PRIVATE_SUMMARY","output_artifact_refs":["art"],
        "created_at":"2026-10-25T00:00:00Z"
    }))
    .unwrap()
}
fn candidate(run: &str, key: &str) -> NewRunEvent {
    NewRunEvent::new(run, key.into(), "tool.started", json!({"name":"read_file"}))
}

#[tokio::test]
async fn domain_records_and_journal_commit_together_with_real_links() {
    let store = store();
    let mut team = team("team");
    store.save_team_run(&team).await.unwrap();
    store.save_team_run(&team).await.unwrap(); // unchanged snapshots do not fabricate transitions
    store.save_project_space(&space()).await.unwrap();
    let art = artifact();
    store.save_artifact(&art, "project").await.unwrap();
    store.save_artifact(&art, "project").await.unwrap();
    store.save_handoff(&handoff(), "project").await.unwrap();
    let mut changed = handoff();
    changed.to_member = "other".into();
    store.save_handoff(&changed, "project").await.unwrap();
    store.save_handoff(&handoff(), "project").await.unwrap(); // A -> B -> A is a real new event
    team.status = TeamRunStatus::Running;
    store.save_team_run(&team).await.unwrap();
    let page = store.run_events_after("team", 0, 256).unwrap();
    assert_eq!(page.events.len(), 6);
    assert!(page.history_complete);
    assert_eq!(page.events[1].task_id.as_deref(), Some("task"));
    assert_eq!(page.events[1].attempt.as_deref(), Some("attempt"));
    assert_eq!(page.events[1].artifact_refs, ["art"]);
    assert_eq!(page.events[2].artifact_refs, ["art"]);
    assert_eq!(page.events[5].data["previous_status"], "created");
    let raw = serde_json::to_string(&page).unwrap();
    assert!(!raw.contains("SECRET_GOAL"));
    assert!(!raw.contains("PRIVATE_SUMMARY"));
}

#[tokio::test]
async fn journal_write_failure_rolls_back_team_artifact_handoff_and_delivery() {
    let store = store();
    let mut team = team("team");
    store.save_team_run(&team).await.unwrap();
    store.save_project_space(&space()).await.unwrap();
    store.conn.lock().unwrap().execute_batch(
        "CREATE TRIGGER fail_journal BEFORE INSERT ON run_events BEGIN SELECT RAISE(ABORT,'injected'); END;"
    ).unwrap();
    team.status = TeamRunStatus::Running;
    assert!(store.save_team_run(&team).await.is_err());
    assert_eq!(
        store.get_team_run("team").await.unwrap().status,
        TeamRunStatus::Created
    );
    assert!(store.save_artifact(&artifact(), "project").await.is_err());
    assert!(store.get_artifact("art").await.is_err());
    assert!(store.save_handoff(&handoff(), "project").await.is_err());
    assert!(store.get_handoff("handoff").await.is_err());
    let mut space = space();
    space.status = ProjectSpaceStatus::Completed;
    team.status = TeamRunStatus::Succeeded;
    assert!(store.commit_team_delivery(&team, &space).await.is_err());
    assert_eq!(
        store.get_team_run("team").await.unwrap().status,
        TeamRunStatus::Created
    );
    assert_eq!(
        store.get_project_space("project").await.unwrap().status,
        ProjectSpaceStatus::Active
    );
    assert_eq!(
        store.run_events_after("team", 0, 256).unwrap().events.len(),
        1
    );
}

#[tokio::test]
async fn cursor_and_causation_are_run_scoped_and_identity_is_immutable() {
    let store = store();
    for id in ["one", "two"] {
        store.save_team_run(&team(id)).await.unwrap();
    }
    let cause = store
        .run_events_after("one", 0, 1)
        .unwrap()
        .events
        .remove(0);
    let mut input = candidate("one", "call");
    input.node_id = Some("node".into());
    input.task_id = Some("task".into());
    input.attempt = Some("attempt".into());
    input.tool_call_id = Some("call".into());
    input.causation_id = Some(format!("run:one:0{}", cause.seq));
    assert!(store.append_run_event(&input).is_err());
    input.causation_id = Some(cause.event_id);
    let first = store.append_run_event(&input).unwrap();
    assert_eq!(store.append_run_event(&input).unwrap(), first);
    input.data = json!({"changed":true});
    assert!(store.append_run_event(&input).is_err());
    input.run_id = "two".into();
    assert!(store.append_run_event(&input).is_err());
    assert!(store.run_events_after("two", first.seq, 1).is_err());
    assert!(store.run_events_after("one", u64::MAX, 1).is_err());
    assert!(store.run_events_after("one", 0, 0).is_err());
    assert!(store.run_events_after("one", 0, 257).is_err());
    assert!(store.run_events_after("absent", 0, 1).is_err());
    let mut oversized = candidate("one", "oversize");
    oversized.data = json!({"data":"X".repeat(MAX_RUN_EVENT_BYTES)});
    assert!(store.append_run_event(&oversized).is_err());
    let mut missing = candidate("one", "missing");
    missing.causation_id = Some("run:one:999999".into());
    assert!(store.append_run_event(&missing).is_err());
}

#[tokio::test]
async fn large_interleaved_history_pages_without_duplicates_or_lost_new_events() {
    let store = store();
    for id in ["one", "two"] {
        store.save_team_run(&team(id)).await.unwrap();
    }
    // A single bulk transaction keeps this test fast; production append uses its own atomic transaction.
    {
        let mut conn = store.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for i in 0..10010 {
            append(
                &tx,
                &candidate(
                    if i % 2 == 0 { "one" } else { "two" },
                    &format!("event-{i}"),
                ),
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let mut after = 0;
    let mut ids = Vec::new();
    loop {
        let page = store.run_events_after("one", after, 127).unwrap();
        assert!(page.events.len() <= 127);
        ids.extend(page.events.iter().map(|e| e.seq));
        after = page.next_after;
        if ids.len() == 127 {
            store
                .append_run_event(&candidate("one", "arrived-during-paging"))
                .unwrap();
        }
        if !page.has_more {
            break;
        }
    }
    assert_eq!(ids.len(), 5007);
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    // Add after the last page, then continue from the exact existing cursor.
    let next = store
        .append_run_event(&candidate("one", "arrived-after-page"))
        .unwrap();
    let page = store.run_events_after("one", after, 127).unwrap();
    assert_eq!(page.events, [next]);
    assert!(!page.has_more);
}

#[tokio::test]
async fn legacy_runs_report_missing_history_without_synthesizing_creation() {
    let store = store();
    let team = team("legacy");
    let raw = serde_json::to_string(&team).unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO team_runs(team_id,data_json,created_at,updated_at) VALUES(?1,?2,?3,?3)",
            params!["legacy", raw, team.created_at],
        )
        .unwrap();
    let page = store.run_events_after("legacy", 0, 10).unwrap();
    assert!(!page.history_complete);
    assert!(page.events.is_empty());
    let mut next = team;
    next.status = TeamRunStatus::Running;
    store.save_team_run(&next).await.unwrap();
    let page = store.run_events_after("legacy", 0, 10).unwrap();
    assert!(!page.history_complete);
    assert_eq!(page.events[0].event_type, "team.status_changed");
}

#[tokio::test]
async fn concurrent_connections_replay_one_identity_and_restart_keeps_sequence() {
    let path = std::env::temp_dir().join(format!("owo-run-journal-{}.db", uuid::Uuid::new_v4()));
    let first = SqliteProjectSpaceStore::open(&path).unwrap();
    first.save_team_run(&team("team")).await.unwrap();
    let second = SqliteProjectSpaceStore::open(&path).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let workers = [first, second]
        .into_iter()
        .map(|store| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.append_run_event(&candidate("team", "same")).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = workers
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results[0], results[1]);
    let reopened = SqliteProjectSpaceStore::open(&path).unwrap();
    let next = reopened
        .append_run_event(&candidate("team", "next"))
        .unwrap();
    assert!(next.seq > results[0].seq);
    let page = reopened
        .run_events_after("team", results[0].seq, 10)
        .unwrap();
    assert_eq!(page.events, [next]);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn reverse_history_is_bounded_exclusive_and_unaffected_by_new_tail_records() {
    let store = store();
    for id in ["one", "two"] {
        store.save_team_run(&team(id)).await.unwrap();
    }
    {
        let mut conn = store.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for n in 0..10010 {
            append(
                &tx,
                &candidate(
                    if n % 2 == 0 { "one" } else { "two" },
                    &format!("reverse-{n}"),
                ),
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let mut before = 0;
    let mut chunks = Vec::new();
    let mut expected_high = 0;
    loop {
        let page = store.run_events_before("one", before, 127).unwrap();
        assert!(page.events.len() <= 127);
        assert!(page.events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
        assert!(page
            .events
            .iter()
            .all(|event| event.run_id == "one" && (before == 0 || event.seq < before)));
        if before == 0 {
            expected_high = page.high_watermark;
            store
                .append_run_event(&candidate("one", "late-tail-record"))
                .unwrap();
        }
        chunks.push(
            page.events
                .iter()
                .map(|event| event.seq)
                .collect::<Vec<_>>(),
        );
        before = page.next_before.unwrap();
        if !page.has_more {
            break;
        }
    }
    chunks.reverse();
    let ids = chunks.into_iter().flatten().collect::<Vec<_>>();
    assert_eq!(ids.len(), 5006); // one team-created event plus 5,005 interleaved records
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(ids.last().copied(), Some(expected_high));
    let fresh = store.run_events_before("one", 0, 1).unwrap();
    assert_eq!(
        fresh.events[0],
        store
            .run_event_by_source("one", "late-tail-record")
            .unwrap()
            .unwrap()
    );
    let empty = store.run_events_before("one", ids[0], 127).unwrap();
    assert!(empty.events.is_empty());
    assert!(empty.next_before.is_none());
    assert!(!empty.has_more);
}

#[tokio::test]
async fn reverse_history_validates_run_cursor_and_limits_without_fabricating_legacy_events() {
    let store = store();
    for id in ["one", "two"] {
        store.save_team_run(&team(id)).await.unwrap();
    }
    let foreign = store.run_events_before("two", 0, 1).unwrap().events[0].seq;
    assert!(store.run_events_before("one", foreign, 1).is_err());
    assert!(store.run_events_before("one", u64::MAX, 1).is_err());
    assert!(store.run_events_before("one", 999999, 1).is_err());
    assert!(store.run_events_before("one", 0, 0).is_err());
    assert!(store.run_events_before("one", 0, 257).is_err());
    assert!(store.run_events_before("absent", 0, 1).is_err());
    let conn = store.conn.lock().unwrap();
    conn.execute("DELETE FROM run_events WHERE run_id='one'", [])
        .unwrap();
    drop(conn);
    let empty = store.run_events_before("one", 0, 1).unwrap();
    assert!(!empty.history_complete);
    assert!(empty.events.is_empty());
    assert_eq!(empty.high_watermark, 0);
    assert_eq!(empty.next_before, None);
}
