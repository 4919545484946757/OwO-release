use super::*;
use crate::project_space_store::SqliteProjectSpaceStore;
fn state() -> GoalRunState {
    let mut plan = Plan::new("plan", "goal");
    let mut a = StepSpec::new("a", "m-writer");
    a.input = json!({"prompt":"PRIVATE_PROMPT","assigned_task_id":"assigned-a"});
    let mut b = StepSpec::new("b", "m-human");
    b.depends_on = vec!["a".into()];
    plan.add_step(a);
    plan.add_step(b);
    let mut state = GoalRunState::new(Goal::new("goal", "PRIVATE_GOAL"), plan);
    state.run_id = "team".into();
    state
}
fn roles() -> Vec<RoleSpec> {
    let mut writer = RoleSpec::agent("writer");
    writer.node_id = Some("actual-node".into());
    let mut human = RoleSpec::agent("human");
    human.assignee = "human".into();
    vec![writer, human]
}
fn new_host(dir: &Path) -> TeamCoordinator {
    TeamCoordinator::new(
        Arc::new(SqliteProjectSpaceStore::open(&dir.join("space.db")).unwrap()),
        Arc::new(TeamTemplateRegistry::new(dir.join("templates"))),
        CasStore::new(dir.join("cas")).unwrap(),
        dir.join("runs"),
    )
}
async fn register(host: &TeamCoordinator) {
    let team:TeamRun=serde_json::from_value(json!({"team_id":"team","goal_id":null,"mode":"team","members":[],
 "project_space_id":null,"task_graph_ref":null,"template_id":null,"shared_context_refs":[],"budget":{},
 "human_policy":null,"strategy_decision":null,"status":"created",
 "created_at":"2026-10-25T00:00:00Z","updated_at":"2026-10-25T00:00:00Z"})).unwrap();
    host.store.save_team_run(&team).await.unwrap();
}
#[test]
fn committed_views_keep_real_binding_waits_and_repeated_state_transitions() {
    let mut old = state();
    old.team_task_events = Some(next_outbox(None, &old, &roles(), 0, true).unwrap());
    let initial = old.team_task_events.as_ref().unwrap();
    assert_eq!(initial.pending.len(), 2);
    assert_eq!(
        initial.pending[0].event.node_id.as_deref(),
        Some("actual-node")
    );
    assert_eq!(
        initial.pending[0].event.task_id.as_deref(),
        Some("assigned-a")
    );
    assert_eq!(
        initial.pending[1].event.data["task"]["waiting_for"],
        json!(["a"])
    );
    assert_eq!(
        initial.pending[1].event.data["task"]["waiting_reason"],
        "dependencies"
    );
    assert!(!serde_json::to_string(initial).unwrap().contains("PRIVATE"));
    assert_eq!(
        next_outbox(Some(&old), &old, &roles(), 0, false)
            .unwrap()
            .next_seq,
        3
    );
    let mut next = old.clone();
    {
        let a = next.records.get_mut("a").unwrap();
        a.status = StepStatus::Running;
        a.attempt_id = Some("attempt-1".into());
        a.phase_epoch = Some(7);
    }
    let running = next_outbox(Some(&old), &next, &roles(), 2, false).unwrap();
    assert_eq!(running.pending.len(), 1);
    assert_eq!(running.pending[0].source_seq, 3);
    assert_eq!(running.pending[0].cause_source_seq, Some(1));
    assert_eq!(running.pending[0].event.event_type, "task.running");
    next.team_task_events = Some(running);
    let mut failed = next.clone();
    failed.records.get_mut("a").unwrap().status = StepStatus::Failed;
    failed.records.get_mut("a").unwrap().error = Some("错误".repeat(500));
    let failure = next_outbox(Some(&next), &failed, &roles(), 3, false).unwrap();
    assert_eq!(failure.pending[0].event.event_type, "task.failed");
    assert_eq!(
        failure.pending[0].event.data["task"]["error_complete"],
        false
    );
    failed.team_task_events = Some(failure);
    let mut reset = failed.clone();
    reset.records.get_mut("a").unwrap().status = StepStatus::Pending;
    reset.records.get_mut("a").unwrap().attempt_id = None;
    let retry = next_outbox(Some(&failed), &reset, &roles(), 4, false).unwrap();
    assert_eq!(retry.pending[0].event.event_type, "task.pending");
    assert_eq!(retry.pending[0].cause_source_seq, Some(4));
    assert_eq!(retry.pending[0].source_seq, 5);
}
#[test]
fn task_metadata_round_trips_in_checkpoint_but_is_excluded_from_public_state_serialization() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = state();
    state.team_task_events = Some(next_outbox(None, &state, &roles(), 0, true).unwrap());
    assert!(serde_json::to_value(&state)
        .unwrap()
        .get("team_task_events")
        .is_none());
    state.persist(temp.path()).unwrap();
    let restored = GoalRunState::load(temp.path(), "team").unwrap();
    assert!(restored.team_task_events.unwrap().history_complete);
}
#[test]
fn missing_pending_identity_and_cursor_ahead_are_explicit_corruption() {
    let state = state();
    let mut out = next_outbox(None, &state, &roles(), 0, true).unwrap();
    assert!(validate_outbox(&out, 3).is_err());
    out.pending.remove(0);
    assert!(validate_outbox(&out, 0).is_err());
    let mut legacy = state.clone();
    legacy.team_task_events = None;
    assert!(next_outbox(Some(&legacy), &legacy, &roles(), 1, false).is_err());
    let mut newer = state.clone();
    newer.records.get_mut("a").unwrap().status = StepStatus::Failed;
    let out = next_outbox(Some(&legacy), &newer, &roles(), 0, false).unwrap();
    assert!(!out.history_complete);
    assert_eq!(out.pending.len(), 1);
    assert_eq!(out.pending[0].event.event_type, "task.failed");
}
#[tokio::test]
async fn checkpoint_commit_survives_failed_journal_then_restart_drains_once() {
    let temp = tempfile::tempdir().unwrap();
    let host = new_host(temp.path());
    register(&host).await;
    let mut state = state();
    state.team_task_events = Some(next_outbox(None, &state, &roles(), 0, true).unwrap());
    state.persist(&host.run_dir).unwrap();
    host.relay_task_events("team", 128).unwrap();
    let conn = rusqlite::Connection::open(temp.path().join("space.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_task BEFORE INSERT ON run_events WHEN NEW.event_type LIKE 'task.%' BEGIN SELECT RAISE(ABORT,'injected journal failure'); END;").unwrap();
    state.records.get_mut("a").unwrap().status = StepStatus::Running;
    state.records.get_mut("a").unwrap().attempt_id = Some("attempt".into());
    host.persist_state(&state).unwrap();
    let committed = host.load_run_state("team").unwrap();
    assert_eq!(committed.records["a"].status, StepStatus::Running);
    assert_eq!(host.store.task_event_cursor("team").unwrap(), 2);
    assert!(host.relay_task_events("team", 64).is_err());
    conn.execute_batch("DROP TRIGGER fail_task").unwrap();
    drop(conn);
    drop(host);
    let restored = new_host(temp.path());
    let (more, complete) = restored.relay_task_events("team", 1).unwrap();
    assert!(!more);
    assert_eq!(complete, Some(true));
    assert_eq!(restored.store.task_event_cursor("team").unwrap(), 3);
    let before = restored.run_events_after("team", 0, 128).unwrap().events;
    restored.relay_task_events("team", 64).unwrap();
    assert_eq!(
        restored.run_events_after("team", 0, 128).unwrap().events,
        before
    );
    // A later checkpoint compacts only the prefix acknowledged in SQL.
    let state = restored.load_run_state("team").unwrap();
    restored.persist_state(&state).unwrap();
    assert!(restored
        .load_run_state("team")
        .unwrap()
        .team_task_events
        .unwrap()
        .pending
        .is_empty());
}
#[tokio::test]
async fn source_checkpoint_failure_does_not_publish_task_success() {
    let temp = tempfile::tempdir().unwrap();
    let host = new_host(temp.path());
    register(&host).await;
    let state = state();
    host.persist_state(&state).unwrap();
    let before = host.run_events_after("team", 0, 128).unwrap().events;
    let path = host.run_dir.join("team.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep"), "original").unwrap();
    let mut next = state;
    next.records.get_mut("a").unwrap().status = StepStatus::Succeeded;
    assert!(host.persist_state(&next).is_err());
    assert_eq!(
        host.run_events_after("team", 0, 128).unwrap().events,
        before
    );
    assert_eq!(
        std::fs::read_to_string(path.join("keep")).unwrap(),
        "original"
    );
}

#[test]
fn human_waiting_and_bounded_skip_previews_come_from_committed_state() {
    let old = state();
    let mut next = old.clone();
    next.records.get_mut("a").unwrap().status = StepStatus::Succeeded;
    let mut ledger = old.clone();
    ledger.team_task_events = Some(next_outbox(None, &old, &roles(), 0, true).unwrap());
    let out = next_outbox(Some(&ledger), &next, &roles(), 2, false).unwrap();
    let human = out
        .pending
        .iter()
        .find(|e| e.event.task_id.as_deref() == Some("b"))
        .unwrap();
    assert_eq!(human.event.data["task"]["assignee"], "human");
    assert_eq!(human.event.data["task"]["waiting_reason"], "human_input");
    assert_eq!(human.event.data["task"]["waiting_for"], json!([]));
    next.team_task_events = Some(out);
    let mut aborted = next.clone();
    aborted.records.get_mut("b").unwrap().status = StepStatus::Aborted;
    aborted.records.get_mut("b").unwrap().skip_reason = Some("reason".repeat(500));
    let out = next_outbox(Some(&next), &aborted, &roles(), 2, false).unwrap();
    let skipped = out.pending.last().unwrap();
    assert_eq!(skipped.event.event_type, "task.aborted");
    assert_eq!(skipped.event.data["task"]["skip_reason_complete"], false);
    assert_eq!(skipped.event.data["task"]["waiting_reason"], Value::Null);
}

#[tokio::test]
async fn moving_delivery_cursor_reloads_checkpoint_without_false_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let host = new_host(temp.path());
    register(&host).await;
    let mut state = state();
    state.team_task_events = Some(next_outbox(None, &state, &roles(), 0, true).unwrap());
    state.persist(&host.run_dir).unwrap();
    let mut reads = 0;
    let (cursor, out) = host
        .task_delivery_snapshot("team", || {
            reads += 1;
            let projected = host.load_run_state("team")?.team_task_events;
            if reads == 1 {
                // Publication advances SQL after this reader captured the old file.
                host.relay_task_events("team", 128)?;
                let mut next = host.load_run_state("team")?;
                next.records.get_mut("a").unwrap().status = StepStatus::Running;
                host.persist_state(&next)?;
            }
            Ok(projected)
        })
        .unwrap();
    assert_eq!(reads, 2);
    assert_eq!(cursor, 3);
    validate_outbox(&out.unwrap(), cursor).unwrap();
    assert_eq!(
        host.relay_task_events("team", 128).unwrap(),
        (false, Some(true))
    );
}

#[tokio::test]
async fn recovered_task_backlog_is_drained_in_bounded_prefixes_without_gaps() {
    let temp = tempfile::tempdir().unwrap();
    let host = new_host(temp.path());
    register(&host).await;
    let mut plan = Plan::new("many", "goal");
    for i in 0..130 {
        plan.add_step(StepSpec::new(format!("step-{i:03}"), "worker"));
    }
    let mut state = GoalRunState::new(Goal::new("goal", "backlog"), plan);
    state.run_id = "team".into();
    state.team_task_events = Some(next_outbox(None, &state, &[], 0, true).unwrap());
    state.persist(&host.run_dir).unwrap();
    assert_eq!(
        host.relay_task_events("team", 64).unwrap(),
        (true, Some(true))
    );
    assert_eq!(host.store.task_event_cursor("team").unwrap(), 64);
    drop(host);
    let host = new_host(temp.path());
    assert_eq!(
        host.relay_task_events("team", 64).unwrap(),
        (true, Some(true))
    );
    assert_eq!(host.store.task_event_cursor("team").unwrap(), 128);
    assert_eq!(
        host.relay_task_events("team", 64).unwrap(),
        (false, Some(true))
    );
    assert_eq!(host.store.task_event_cursor("team").unwrap(), 130);
    let page = host.run_events_after("team", 0, 256).unwrap();
    let tasks = page
        .events
        .iter()
        .filter(|e| e.event_type == "task.created")
        .collect::<Vec<_>>();
    assert_eq!(tasks.len(), 130);
    for (index, event) in tasks.iter().enumerate() {
        assert_eq!(event.data["source_seq"], (index + 1) as u64);
        assert_eq!(
            event.task_id.as_deref(),
            Some(format!("step-{index:03}").as_str())
        );
    }
    assert_eq!(
        host.relay_task_events("team", 64).unwrap(),
        (false, Some(true))
    );
    assert_eq!(
        host.run_events_after("team", 0, 256).unwrap().events,
        page.events
    );
}

#[tokio::test]
async fn reassigned_task_id_retains_real_step_causation_without_poisoning_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let host = new_host(temp.path());
    register(&host).await;
    let mut state = state();
    host.persist_state(&state).unwrap();
    let original = host
        .run_events_after("team", 0, 128)
        .unwrap()
        .events
        .into_iter()
        .find(|e| e.task_id.as_deref() == Some("assigned-a"))
        .unwrap();
    state.plan.steps[0].input["assigned_task_id"] = json!("reassigned-a");
    host.persist_state(&state).unwrap();
    assert_eq!(
        host.relay_task_events("team", 128).unwrap(),
        (false, Some(false))
    );
    let next = host
        .run_events_after("team", 0, 128)
        .unwrap()
        .events
        .into_iter()
        .find(|e| e.task_id.as_deref() == Some("reassigned-a"))
        .unwrap();
    assert_eq!(
        next.causation_id.as_deref(),
        Some(original.event_id.as_str())
    );
    assert_eq!(next.data["previous_task_id"], "assigned-a");
    assert_eq!(next.data["step_id"], "a");
    state.records.get_mut("a").unwrap().status = StepStatus::Running;
    state.records.get_mut("a").unwrap().attempt_id = Some("new-attempt".into());
    host.persist_state(&state).unwrap();
    assert_eq!(host.store.task_event_cursor("team").unwrap(), 4);
}

#[tokio::test]
async fn legacy_without_checkpoint_is_incomplete_but_loss_after_publication_is_corrupt() {
    let temp = tempfile::tempdir().unwrap();
    let host = new_host(temp.path());
    register(&host).await;
    assert_eq!(
        host.relay_task_events("team", 128).unwrap(),
        (false, Some(false))
    );
    assert_eq!(
        host.run_events_after("team", 0, 128).unwrap().events.len(),
        1
    );
    host.persist_state(&state()).unwrap();
    assert_eq!(host.store.task_event_cursor("team").unwrap(), 2);
    std::fs::remove_file(host.run_dir.join("team.json")).unwrap();
    assert!(matches!(
        host.relay_task_events("team", 128),
        Err(WorkSwarmError::CorruptState(_))
    ));
}
