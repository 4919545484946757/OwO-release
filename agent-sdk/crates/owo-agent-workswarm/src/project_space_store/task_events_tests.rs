use super::*;
use serde_json::json;
fn store() -> SqliteProjectSpaceStore {
    SqliteProjectSpaceStore::from_connection(Connection::open_in_memory().unwrap()).unwrap()
}
fn team() -> TeamRun {
    serde_json::from_value(json!({
 "team_id":"team","goal_id":null,"mode":"team","members":[],"project_space_id":null,
 "task_graph_ref":null,"template_id":null,"shared_context_refs":[],"budget":{},"human_policy":null,
 "strategy_decision":null,"status":"created","created_at":"2026-10-25T00:00:00Z","updated_at":"2026-10-25T00:00:00Z"
})).unwrap()
}
fn event(seq: u64, cause: Option<u64>) -> TaskRunEvent {
    let mut event = NewRunEvent::new(
        "team",
        task_source_key("team", seq),
        "task.running",
        json!({"status":"running","source_seq":seq,"step_id":"step"}),
    );
    event.task_id = Some("task".into());
    event.attempt = Some("attempt".into());
    TaskRunEvent {
        source_seq: seq,
        cause_source_seq: cause,
        event,
    }
}
#[tokio::test]
async fn task_publication_and_cursor_roll_back_together_then_recover_idempotently() {
    let store = store();
    store.save_team_run(&team()).await.unwrap();
    store.conn.lock().unwrap().execute_batch(
  "CREATE TRIGGER fail_task_ack BEFORE INSERT ON run_task_delivery_cursor BEGIN SELECT RAISE(ABORT,'injected cursor fault'); END;"
 ).unwrap();
    let batch = vec![event(1, None), event(2, Some(1))];
    assert!(store.publish_task_events("team", &batch).is_err());
    assert_eq!(store.task_event_cursor("team").unwrap(), 0);
    assert_eq!(
        store.run_events_after("team", 0, 128).unwrap().events.len(),
        1
    );
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_task_ack")
        .unwrap();
    assert_eq!(store.publish_task_events("team", &batch).unwrap(), 2);
    let page = store.run_events_after("team", 0, 128).unwrap();
    assert_eq!(page.events.len(), 3);
    assert_eq!(
        page.events[2].causation_id,
        Some(page.events[1].event_id.clone())
    );
    assert_eq!(store.publish_task_events("team", &batch).unwrap(), 2);
    assert_eq!(
        store.run_events_after("team", 0, 128).unwrap().events.len(),
        3
    );
    let mut changed = batch;
    changed[0].event.data["status"] = json!("forged");
    assert!(store.publish_task_events("team", &changed).is_err());
    assert_eq!(store.task_event_cursor("team").unwrap(), 2);
}
#[tokio::test]
async fn task_source_gaps_foreign_identity_and_forward_causes_do_not_advance_cursor() {
    let store = store();
    store.save_team_run(&team()).await.unwrap();
    for candidate in [event(2, None), event(1, Some(1)), {
        let mut x = event(1, None);
        x.event.run_id = "other".into();
        x
    }] {
        assert!(store.publish_task_events("team", &[candidate]).is_err());
        assert_eq!(store.task_event_cursor("team").unwrap(), 0);
    }
    let mut foreign = event(2, Some(1));
    foreign.event.data["step_id"] = json!("other-step");
    assert!(store
        .publish_task_events("team", &[event(1, None), foreign])
        .is_err());
    assert_eq!(store.task_event_cursor("team").unwrap(), 0);
    let mut oversized = event(2, Some(1));
    oversized.event.data = json!({"step_id":"step","large":"x".repeat(20000)});
    let error = store
        .publish_task_events("team", &[event(1, None), oversized])
        .unwrap_err();
    assert!(error.to_string().contains("16 KiB"), "{error}");
    assert_eq!(store.task_event_cursor("team").unwrap(), 0);
    assert_eq!(
        store.run_events_after("team", 0, 128).unwrap().events.len(),
        1
    );
}
#[tokio::test]
async fn task_cursor_survives_restart_and_competing_replay_has_one_canonical_identity() {
    let path = std::env::temp_dir().join(format!("owo-task-journal-{}.db", uuid::Uuid::new_v4()));
    let store = SqliteProjectSpaceStore::open(&path).unwrap();
    store.save_team_run(&team()).await.unwrap();
    drop(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let store = SqliteProjectSpaceStore::open(&path).unwrap();
            barrier.wait();
            store
                .publish_task_events("team", &[event(1, None), event(2, Some(1))])
                .unwrap()
        }));
    }
    for thread in threads {
        assert_eq!(thread.join().unwrap(), 2);
    }
    let store = SqliteProjectSpaceStore::open(&path).unwrap();
    assert_eq!(store.task_event_cursor("team").unwrap(), 2);
    assert_eq!(
        store.run_events_after("team", 0, 128).unwrap().events.len(),
        3
    );
    drop(store);
    std::fs::remove_file(path).unwrap();
}
