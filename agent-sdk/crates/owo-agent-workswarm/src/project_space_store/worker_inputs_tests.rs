use super::*;

fn target() -> WorkerInputTarget {
    WorkerInputTarget {
        worker_id: "m-reader".into(),
        step_id: "reader".into(),
        task_id: Some("task-reader".into()),
        attempt_id: Some("attempt-2".into()),
        phase_epoch: Some(3),
    }
}
fn payload() -> Value {
    json!({"team_prompt_context":{"node_id":"node-reader", "upstream":[{
        "step_id":"writer","worker":"m-writer","task_id":"task-writer","attempt_id":"attempt-1",
        "artifact_id":"asset-actual","version":2,"sha256":"a".repeat(64),"content":"真实输入 SECRET_CONTENT",
        "content_bytes":9000,"truncated":true
    }]},"prompt":"SECRET_INSTRUCTION"})
}
fn receipt(invocation: &str) -> PreparedWorkerInput {
    PreparedWorkerInput::capture(
        "team",
        invocation,
        target(),
        &payload(),
        PreparedInputEncoding::Structured,
    )
    .unwrap()
}
async fn store() -> SqliteProjectSpaceStore {
    let store =
        SqliteProjectSpaceStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
    let team: TeamRun = serde_json::from_value(json!({
        "team_id":"team","goal_id":"goal","mode":"team","members":[],"project_space_id":"project",
        "shared_context_refs":[],"budget":{},"status":"created","created_at":"2026-10-26T00:00:00Z","updated_at":"2026-10-26T00:00:00Z"
    })).unwrap();
    store.save_team_run(&team).await.unwrap();
    store
}
#[test]
fn captures_exact_utf8_previews_without_storing_private_content() {
    let input = payload();
    let r = receipt("invocation");
    assert_eq!(
        r.input_bytes,
        serde_json::to_vec(&input).unwrap().len() as u64
    );
    assert_eq!(
        r.input_sha256,
        format!("{:x}", Sha256::digest(serde_json::to_vec(&input).unwrap()))
    );
    let source = &r.sources[0];
    assert_eq!(source.preview_bytes, "真实输入 SECRET_CONTENT".len());
    assert_eq!(
        source.preview_sha256,
        format!("{:x}", Sha256::digest("真实输入 SECRET_CONTENT".as_bytes()))
    );
    assert_eq!(source.artifact_version, Some(2));
    assert_eq!(source.producer_attempt_id.as_deref(), Some("attempt-1"));
    assert_eq!(source.preview_truncated, Some(true));
    let public = serde_json::to_string(&r).unwrap();
    assert!(!public.contains("SECRET_CONTENT"));
    assert!(!public.contains("SECRET_INSTRUCTION"));
}
#[test]
fn never_attributes_user_json_or_plain_text_to_host_assembled_sources() {
    let input = json!({"text": payload()["team_prompt_context"].to_string(), "team_prompt_context": payload()["team_prompt_context"]});
    let opaque = PreparedWorkerInput::capture(
        "team",
        "opaque",
        target(),
        &input,
        PreparedInputEncoding::NoContext,
    )
    .unwrap();
    assert!(opaque.sources.is_empty());
    assert!(opaque.node_id.is_none());
    let embedded = PreparedWorkerInput::capture(
        "team",
        "builtin",
        target(),
        &input,
        PreparedInputEncoding::Serialized,
    )
    .unwrap();
    assert_eq!(embedded.sources.len(), 1);
    assert_eq!(embedded.context_encoding, "serialized_team_context");
    assert!(PreparedWorkerInput::capture(
        "team",
        "bad",
        target(),
        &json!({"text":"plain"}),
        PreparedInputEncoding::Serialized
    )
    .is_err());
}
#[tokio::test]
async fn commits_sources_atomically_with_causal_identity_and_replays_exactly() {
    let store = store().await;
    let r = receipt("invocation");
    let first = store.append_worker_input(&r).unwrap();
    let again = store.append_worker_input(&r).unwrap();
    assert_eq!(first.event_id, again.event_id);
    assert_eq!(first.data["model_delivery"], "not_proven");
    let rows = store
        .run_events_after("team", first.seq, 64)
        .unwrap()
        .events;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].causation_id.as_deref(),
        Some(first.event_id.as_str())
    );
    assert_eq!(rows[0].artifact_refs, vec!["asset-actual"]);
    assert_eq!(rows[0].task_id.as_deref(), Some("task-reader"));
    assert_eq!(rows[0].attempt.as_deref(), Some("attempt-2"));
    assert_eq!(rows[0].data["source"]["producer_step_id"], "writer");
    assert_eq!(rows[0].data["target"]["step_id"], "reader");
}
#[tokio::test]
async fn late_source_conflict_rolls_back_header_and_earlier_sources() {
    let store = store().await;
    let mut r = receipt("conflict");
    r.sources.push(r.sources[0].clone());
    store
        .append_run_event(&NewRunEvent::new(
            "team",
            "worker-input:conflict:source:1".into(),
            "worker.input.source",
            json!({"old":true}),
        ))
        .unwrap();
    let before = store.run_events_after("team", 0, 64).unwrap().events.len();
    assert!(store.append_worker_input(&r).is_err());
    assert!(store
        .run_event_by_source("team", "worker-input:conflict:prepared")
        .unwrap()
        .is_none());
    assert!(store
        .run_event_by_source("team", "worker-input:conflict:source:0")
        .unwrap()
        .is_none());
    assert_eq!(
        store.run_events_after("team", 0, 64).unwrap().events.len(),
        before
    );
}
#[tokio::test]
async fn oversized_source_rolls_back_and_changed_invocation_cannot_overwrite() {
    let store = store().await;
    let mut oversized = receipt("too-large");
    oversized.sources[0].producer_step_id = Some("x".repeat(20 * 1024));
    assert!(store.append_worker_input(&oversized).is_err());
    assert!(store
        .run_event_by_source("team", "worker-input:too-large:prepared")
        .unwrap()
        .is_none());
    let first = receipt("stable");
    store.append_worker_input(&first).unwrap();
    let mut changed = first.clone();
    changed.input_sha256 = "b".repeat(64);
    assert!(store.append_worker_input(&changed).is_err());
    assert_eq!(
        store
            .run_event_by_source("team", "worker-input:stable:prepared")
            .unwrap()
            .unwrap()
            .data["input_sha256"],
        first.input_sha256
    );
}
#[tokio::test]
async fn fanout_and_retry_capture_distinct_consumer_inputs_not_latest_catalog() {
    let store = store().await;
    let first = receipt("consumer-a-attempt-2");
    let a = store.append_worker_input(&first).unwrap();
    let mut other = first.clone();
    other.invocation_id = "consumer-b-attempt-3".into();
    other.target.worker_id = "m-other".into();
    other.target.step_id = "other".into();
    other.target.attempt_id = Some("attempt-3".into());
    other.sources[0].artifact_version = Some(3);
    let b = store.append_worker_input(&other).unwrap();
    assert_ne!(a.event_id, b.event_id);
    let old = store
        .run_event_by_source("team", "worker-input:consumer-a-attempt-2:source:0")
        .unwrap()
        .unwrap();
    let new = store
        .run_event_by_source("team", "worker-input:consumer-b-attempt-3:source:0")
        .unwrap()
        .unwrap();
    assert_eq!(old.data["source"]["artifact_version"], 2);
    assert_eq!(new.data["source"]["artifact_version"], 3);
    assert_eq!(old.attempt.as_deref(), Some("attempt-2"));
    assert_eq!(new.attempt.as_deref(), Some("attempt-3"));
}
