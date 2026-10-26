use super::*;
use serde_json::json;
fn binding() -> TeamTurnBinding {
    TeamTurnBinding {
        run_id: "team".into(),
        node_id: Some("stable-node".into()),
        task_id: "task".into(),
        attempt: Some("attempt".into()),
    }
}
fn open(path: &Path) -> SqliteSessionStore {
    SqliteSessionStore::open(path).unwrap()
}

#[test]
fn message_and_delivery_index_failure_roll_back_sequence_and_payload() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp.path().join("index.db"));
    store.conn.lock().unwrap().execute_batch(
        "CREATE TRIGGER fail_delivery BEFORE INSERT ON team_turn_delivery BEGIN SELECT RAISE(ABORT,'injected'); END;"
    ).unwrap();
    let result = store.append_team_turn_event(
        &binding(),
        "team-worker-one",
        "writer",
        &SseEvent::Final {
            text: "result".into(),
        },
    );
    assert!(result.is_err());
    assert!(store
        .turn_events_after("team-worker-one", None, 0, 10)
        .unwrap()
        .is_empty());
    assert!(store
        .pending_team_turn_events("team", 10)
        .unwrap()
        .is_empty());
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_delivery")
        .unwrap();
    let record = store
        .append_team_turn_event(
            &binding(),
            "team-worker-one",
            "writer",
            &SseEvent::Final {
                text: "result".into(),
            },
        )
        .unwrap();
    assert_eq!(record.seq, 1);
    assert_eq!(
        store.pending_team_turn_events("team", 10).unwrap()[0].session_seq,
        1
    );
}

#[test]
fn pending_delivery_survives_restart_and_acknowledgement_is_run_scoped() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.db");
    let store = open(&path);
    store
        .append_team_turn_event(
            &binding(),
            "team-worker-one",
            "writer",
            &SseEvent::TokenDelta {
                delta: "公开消息".into(),
            },
        )
        .unwrap();
    drop(store);
    let store = open(&path);
    let row = store
        .pending_team_turn_events("team", 10)
        .unwrap()
        .remove(0);
    assert_eq!(row.binding, binding());
    assert!(store
        .acknowledge_team_turn_event("other", row.delivery_id)
        .is_err());
    store
        .acknowledge_team_turn_event("team", row.delivery_id)
        .unwrap();
    store
        .acknowledge_team_turn_event("team", row.delivery_id)
        .unwrap();
    assert!(store
        .pending_team_turn_events("team", 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .turn_events_after("team-worker-one", None, 0, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn tool_result_links_real_start_across_ack_and_records_actual_artifact_ids() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp.path().join("index.db"));
    let start = store
        .append_team_turn_event(
            &binding(),
            "team-worker-one",
            "writer",
            &SseEvent::ToolUse {
                id: "call".into(),
                tool: "image".into(),
                args: json!({}),
            },
        )
        .unwrap();
    let delivery = store.pending_team_turn_events("team", 1).unwrap().remove(0);
    store
        .acknowledge_team_turn_event("team", delivery.delivery_id)
        .unwrap();
    let output = owo_agent_protocol::ToolOutputArtifact {
        artifact_id: "image-ref".into(),
        kind: "image".into(),
        mime_type: "image/png".into(),
        bytes: 7,
        sha256: "hash".into(),
        path: "private-path".into(),
    };
    store
        .append_team_turn_event(
            &binding(),
            "team-worker-one",
            "writer",
            &SseEvent::ToolResult {
                id: "call".into(),
                tool: "image".into(),
                ok: true,
                error: None,
                preview: None,
                outputs: vec![output],
            },
        )
        .unwrap();
    let row = store
        .pending_team_turn_events("team", 10)
        .unwrap()
        .remove(0);
    assert_eq!(row.cause_seq, Some(start.seq));
    assert_eq!(row.tool_call_id.as_deref(), Some("call"));
    assert_eq!(row.artifact_refs, ["image-ref"]);
    assert!(row.artifact_refs_complete);
}

#[test]
fn public_binding_rejects_foreign_sessions_and_private_reasoning_without_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp.path().join("index.db"));
    assert!(store
        .append_team_turn_event(
            &binding(),
            "other-worker-one",
            "writer",
            &SseEvent::Final { text: "x".into() }
        )
        .is_err());
    assert!(store
        .append_team_turn_event(
            &binding(),
            "team-worker-one",
            "writer",
            &SseEvent::ReasoningDelta {
                delta: "private".into()
            }
        )
        .is_err());
    assert!(store
        .pending_team_turn_events("team", 10)
        .unwrap()
        .is_empty());
    assert!(store
        .turn_events_after("team-worker-one", None, 0, 10)
        .unwrap()
        .is_empty());
}

#[test]
fn byte_limited_reads_do_not_copy_large_payloads_and_clear_removes_delivery_index() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp.path().join("index.db"));
    let record = store
        .append_team_turn_event(
            &binding(),
            "team-worker-one",
            "writer",
            &SseEvent::Final {
                text: "图".repeat(5000),
            },
        )
        .unwrap();
    assert!(store
        .turn_event_json_bounded("team-worker-one", record.seq, 8192)
        .unwrap()
        .is_none());
    assert!(store
        .turn_event_json_bounded("team-worker-one", record.seq, 20000)
        .unwrap()
        .is_some());
    assert!(store.pending_team_turn_events("team", 258).is_err());
    store.clear_all().unwrap();
    assert!(store
        .pending_team_turn_events("team", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn session_removal_rolls_back_source_and_delivery_index_on_delete_failure() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp.path().join("index.db"));
    let mut session = store.create(temp.path(), "model", None).unwrap();
    session.id = "team-worker-one".into();
    store.save(&session).unwrap();
    store
        .append_team_turn_event(
            &binding(),
            &session.id,
            "writer",
            &SseEvent::Final { text: "x".into() },
        )
        .unwrap();
    store.conn.lock().unwrap().execute_batch(
        "CREATE TRIGGER fail_event_delete BEFORE DELETE ON turn_events BEGIN SELECT RAISE(ABORT,'injected'); END;"
    ).unwrap();
    assert!(store.remove(&session.id).is_err());
    assert_eq!(store.pending_team_turn_events("team", 10).unwrap().len(), 1);
    assert_eq!(
        store
            .turn_events_after(&session.id, None, 0, 10)
            .unwrap()
            .len(),
        1
    );
    assert!(store.exists(&session.id).unwrap());
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_event_delete")
        .unwrap();
    store.remove(&session.id).unwrap();
    assert!(store
        .pending_team_turn_events("team", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn conversation_index_skips_global_and_caught_up_sessions_but_revisits_late_events() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp.path().join("index.db"));
    let mut session = store.create(temp.path(), "model", None).unwrap();
    let mut cursors = std::collections::BTreeMap::new();
    for i in 0..80 {
        session.id = format!("team-worker-{i:03}");
        store.save(&session).unwrap();
        store
            .append_turn_event(&session.id, "writer", &SseEvent::Final { text: "x".into() })
            .unwrap();
    }
    session.id = "other-worker-foreign".into();
    store.save(&session).unwrap();
    store
        .append_turn_event(
            &session.id,
            "secret",
            &SseEvent::Final {
                text: "secret".into(),
            },
        )
        .unwrap();
    assert_eq!(
        store
            .pending_conversation_sessions("team", &cursors, None, 17)
            .unwrap()
            .len(),
        17
    );
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let page = store
            .pending_conversation_sessions("team", &cursors, None, 17)
            .unwrap();
        if page.is_empty() {
            break;
        }
        for id in page {
            assert!(seen.insert(id.clone()));
            cursors.insert(id, 1);
        }
    }
    assert_eq!(seen.len(), 80);
    store
        .append_turn_event(
            "team-worker-000",
            "writer",
            &SseEvent::Final {
                text: "late".into(),
            },
        )
        .unwrap();
    assert_eq!(
        store
            .pending_conversation_sessions("team", &cursors, None, 17)
            .unwrap(),
        vec!["team-worker-000"]
    );
    cursors.insert("other-worker-foreign".into(), 1);
    assert!(store
        .pending_conversation_sessions("team", &cursors, None, 17)
        .is_err());
    assert!(store
        .pending_conversation_sessions("team", &Default::default(), None, 0)
        .is_err());
    assert!(store
        .pending_conversation_sessions("team", &Default::default(), None, 65)
        .is_err());
}

#[test]
fn conversation_index_literal_prefix_handles_wildcards_and_quoted_session_ids() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp.path().join("index.db"));
    let mut session = store.create(temp.path(), "model", None).unwrap();
    session.id = "team_%中文-worker-node.with_underscore".into();
    store.save(&session).unwrap();
    store
        .append_turn_event(&session.id, "writer", &SseEvent::Final { text: "x".into() })
        .unwrap();
    let cursor = [(session.id.clone(), 1)].into_iter().collect();
    assert!(store
        .pending_conversation_sessions("team_%中文", &cursor, None, 17)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .pending_conversation_sessions("team_%中文", &Default::default(), None, 17)
            .unwrap(),
        vec![session.id]
    );
    assert!(store
        .pending_conversation_sessions("team_", &Default::default(), None, 17)
        .unwrap()
        .is_empty());
}

#[test]
fn conversation_rotation_does_not_starve_later_members_and_wraps() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp.path().join("index.db"));
    let mut session = store.create(temp.path(), "model", None).unwrap();
    for i in 0..40 {
        session.id = format!("team-worker-{i:03}");
        store.save(&session).unwrap();
        store
            .append_turn_event(&session.id, "writer", &SseEvent::Final { text: "x".into() })
            .unwrap();
    }
    let cursors = Default::default();
    let mut after = None;
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..3 {
        let page = store
            .pending_conversation_sessions("team", &cursors, after.as_deref(), 17)
            .unwrap();
        for id in page.iter().take(16) {
            seen.insert(id.clone());
        }
        after = page.iter().take(16).next_back().cloned();
    }
    assert_eq!(seen.len(), 40);
    let wrapped = store
        .pending_conversation_sessions("team", &cursors, after.as_deref(), 17)
        .unwrap();
    assert_eq!(wrapped[0], "team-worker-000");
}
