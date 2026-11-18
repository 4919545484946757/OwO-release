//! Durable admission must survive process loss without orphaned or duplicate tasks.
use owo_agent_core::task_runtime::{CreateTaskRequest, TaskRuntime, TaskStore};
use serde_json::json;
use std::sync::{Arc, Barrier};

#[test]
fn admission_survives_store_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("tasks.db");
    let fingerprint = json!({"sha256": "synthetic"});
    let first_id;
    {
        let runtime = TaskRuntime::new(TaskStore::open(&db).unwrap());
        let (first, duplicate) = runtime
            .create_task_idempotent(
                CreateTaskRequest::new("synthetic"),
                "bridge:key",
                &fingerprint,
            )
            .unwrap();
        assert!(!duplicate);
        first_id = first.task_id.to_string();
    }
    let runtime = TaskRuntime::new(TaskStore::open(&db).unwrap());
    let (replay, duplicate) = runtime
        .create_task_idempotent(
            CreateTaskRequest::new("synthetic"),
            "bridge:key",
            &fingerprint,
        )
        .unwrap();
    assert!(duplicate);
    assert_eq!(replay.task_id.as_str(), first_id);
    let events = runtime.events(&first_id, 0, 200).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "task.created");
    assert_eq!(events[1].event_type, "task.queued");
}

#[test]
fn changed_content_does_not_reuse_admission_key() {
    let runtime = TaskRuntime::new(TaskStore::open_in_memory().unwrap());
    runtime
        .create_task_idempotent(
            CreateTaskRequest::new("first"),
            "bridge:key",
            &json!({"sha256":"first"}),
        )
        .unwrap();
    let error = runtime
        .create_task_idempotent(
            CreateTaskRequest::new("different"),
            "bridge:key",
            &json!({"sha256":"different"}),
        )
        .unwrap_err();
    assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    assert_eq!(runtime.list_tasks(None, 100, 0).unwrap().len(), 1);
}

#[test]
fn two_connections_admit_only_one_task() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("tasks.db");
    let a = TaskRuntime::new(TaskStore::open(&db).unwrap());
    let b = TaskRuntime::new(TaskStore::open(&db).unwrap());
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [a, b]
        .into_iter()
        .map(|runtime| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                runtime
                    .create_task_idempotent(
                        CreateTaskRequest::new("synthetic"),
                        "bridge:key",
                        &json!({"sha256":"same"}),
                    )
                    .unwrap()
            })
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outcomes[0].0.task_id, outcomes[1].0.task_id);
    assert_ne!(outcomes[0].1, outcomes[1].1);
    let runtime = TaskRuntime::new(TaskStore::open(&db).unwrap());
    assert_eq!(runtime.list_tasks(None, 100, 0).unwrap().len(), 1);
    assert_eq!(
        runtime
            .events(outcomes[0].0.task_id.as_str(), 0, 100)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn admission_claim_failure_rolls_back_task_and_events() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("tasks.db");
    let runtime = TaskRuntime::new(TaskStore::open(&db).unwrap());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_admission BEFORE INSERT ON task_idempotency_keys
         BEGIN SELECT RAISE(ABORT, 'injected admission failure'); END;",
    )
    .unwrap();
    let result = runtime.create_task_idempotent(
        CreateTaskRequest::new("synthetic"),
        "bridge:key",
        &json!({"sha256":"same"}),
    );
    assert!(result.is_err());
    for table in ["tasks", "task_events", "task_idempotency_keys"] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table} must roll back");
    }
    conn.execute_batch("DROP TRIGGER fail_admission").unwrap();
    let (_, duplicate) = runtime
        .create_task_idempotent(
            CreateTaskRequest::new("synthetic"),
            "bridge:key",
            &json!({"sha256":"same"}),
        )
        .unwrap();
    assert!(!duplicate);
}
