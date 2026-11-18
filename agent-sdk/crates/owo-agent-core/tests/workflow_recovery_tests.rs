//! Engine contract tests: fresh engine objects, persisted state round trips,
//! real isolated files. These are not process/UI E2E evidence.
use owo_agent_core::skill_health::SkillHealthStore;
use owo_agent_core::workflow::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Default)]
struct Journal {
    state: Mutex<Option<Value>>,
    claims: Mutex<HashSet<usize>>,
    fail_completion: AtomicBool,
}
impl WorkflowRecoveryStore for Journal {
    fn load(&self) -> Result<Option<WorkflowRecoveryState>, String> {
        self.state
            .lock()
            .unwrap()
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| e.to_string())
    }
    fn save(&self, state: &WorkflowRecoveryState) -> Result<(), String> {
        if state.completed_effects() > 0 && self.fail_completion.swap(false, Ordering::SeqCst) {
            return Err("injected durability failure after effect".into());
        }
        *self.state.lock().unwrap() = Some(serde_json::to_value(state).unwrap());
        Ok(())
    }
    fn claim_effect(&self, sequence: usize, _step: &Value) -> Result<(), String> {
        if !self.claims.lock().unwrap().insert(sequence) {
            return Err("already claimed".into());
        }
        Ok(())
    }
}
struct CancelApproval(Arc<AtomicBool>);
#[async_trait::async_trait]
impl HumanApprover for CancelApproval {
    async fn request(&self, _prompt: &str) -> Approval {
        self.0.store(true, Ordering::SeqCst);
        Approval::Rejected
    }
}
fn flow(steps: Value) -> WorkflowDefinition {
    serde_json::from_value(json!({
        "id":"recovery","name":"recovery","version":1,
        "permissions":[{"scope":"fs.write","mode":"allow"}],
        "steps":steps,"max_steps":100
    }))
    .unwrap()
}
fn append(id: &str, value: &str) -> Value {
    json!({"kind":"act","id":id,"scope":"fs.write","spec":{"action":"append_file","target":"effects.txt","value":value}})
}
fn engine(
    definition: WorkflowDefinition,
    children: HashMap<String, WorkflowDefinition>,
    root: &std::path::Path,
    backend: MockBackend,
    approver: Box<dyn HumanApprover>,
    journal: Arc<Journal>,
    cancel: Arc<AtomicBool>,
) -> Result<WorkflowEngine, String> {
    WorkflowEngine::new(
        definition,
        children,
        Box::new(backend),
        approver,
        SkillHealthStore::new(None),
        root.to_path_buf(),
    )
    .with_cancel_flag(cancel)
    .with_recovery(journal)
}

#[tokio::test]
async fn nested_control_flow_restores_original_context_and_each_invocation_once() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("work");
    std::fs::create_dir(&root).unwrap();
    let journal = Arc::new(Journal::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let child = flow(
        json!([append("child-write","Y"),{"kind":"human_approve","id":"approval","prompt":"continue"}]),
    );
    let children = HashMap::from([("child".into(), child)]);
    let definition = flow(json!([
        {"kind":"sense","id":"flag","spec":{"target":"flag"}},
        {"kind":"loop","id":"loop","max_iter":2,"body":[
            {"kind":"cond","id":"branch","expr":"flag == true","then":[append("branch-write","X")],"otherwise":[append("wrong","WRONG")]},
            {"kind":"subflow","id":"child-call","flow":"child","args":{}}
        ]},
        {"kind":"assert","id":"summary","expr":"exists(child-call)"},
    ]));
    let mut backend = MockBackend::new(root.clone());
    backend.sense_results.insert("flag".into(), json!(true));
    let mut first = engine(
        definition.clone(),
        children.clone(),
        &root,
        backend,
        Box::new(CancelApproval(cancel.clone())),
        journal.clone(),
        cancel,
    )
    .unwrap();
    assert_eq!(first.run().await.unwrap().state, WorkflowState::Aborted);
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "XY"
    );
    let mut backend = MockBackend::new(root.clone());
    backend.sense_results.insert("flag".into(), json!(false));
    let mut resumed = engine(
        definition.clone(),
        children.clone(),
        &root,
        backend,
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let completed = resumed.run().await.unwrap();
    assert_eq!(completed.metrics.tool_calls, 2);
    assert_eq!(completed.metrics.reused_tool_calls, 3);
    assert_eq!(completed.metrics.reserved_task_tool_calls, 5);
    assert_eq!(completed.state, WorkflowState::Succeeded);
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "XYXY"
    );
    assert_eq!(journal.claims.lock().unwrap().len(), 4);
    // A fresh engine also recovers an already completed run without inputs.
    let mut again = engine(
        definition,
        children,
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: false }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(
        again.run().await.unwrap().steps.len(),
        completed.steps.len()
    );
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "XYXY"
    );
    assert_eq!(journal.claims.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn uncertain_effect_and_orphan_claim_never_repeat_input() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let definition = flow(json!([append("once", "ONCE")]));
    let journal = Arc::new(Journal::default());
    journal.fail_completion.store(true, Ordering::SeqCst);
    let mut first = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(first
        .run()
        .await
        .unwrap_err()
        .contains(WORKFLOW_RECOVERY_REQUIRED));
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "ONCE"
    );
    let retry = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal,
        Arc::new(AtomicBool::new(false)),
    );
    assert!(retry.err().unwrap().contains(WORKFLOW_RECOVERY_REQUIRED));
    let orphan = Arc::new(Journal::default());
    orphan.claims.lock().unwrap().insert(0);
    let mut rejected = engine(
        definition,
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        orphan,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(rejected
        .run()
        .await
        .unwrap_err()
        .contains(WORKFLOW_RECOVERY_REQUIRED));
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "ONCE"
    );
}

#[tokio::test]
async fn changed_definition_and_failed_rollback_are_not_replayed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("work");
    std::fs::create_dir(&root).unwrap();
    let journal = Arc::new(Journal::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let definition = flow(json!([
        append("before","A"),{"kind":"rollback_point","id":"snapshot"},
        append("after","B"),{"kind":"human_approve","id":"approval","prompt":"continue"},
        {"kind":"assert","id":"failure","expr":"false"}
    ]));
    let mut first = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(CancelApproval(cancel.clone())),
        journal.clone(),
        cancel,
    )
    .unwrap();
    assert_eq!(first.run().await.unwrap().state, WorkflowState::Aborted);
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "AB"
    );
    let mut changed = definition.clone();
    changed.name = "changed".into();
    let rejected = engine(
        changed,
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(rejected.err().unwrap().contains(WORKFLOW_RECOVERY_REQUIRED));
    let mut resumed = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let failed = resumed.run().await.unwrap();
    assert_eq!(failed.state, WorkflowState::Failed);
    assert_eq!(failed.rollback_to.as_deref(), Some("snapshot"));
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "A"
    );
    let rejected = engine(
        definition,
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal,
        Arc::new(AtomicBool::new(false)),
    );
    assert!(rejected.err().unwrap().contains(WORKFLOW_RECOVERY_REQUIRED));
}

#[tokio::test]
async fn permission_denial_still_precedes_durable_effect_claim() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let journal = Arc::new(Journal::default());
    let mut definition = flow(json!([append("denied", "NO")]));
    definition.permissions.clear();
    let mut run = engine(
        definition,
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(run.run().await.unwrap().state, WorkflowState::Failed);
    assert!(!root.join("effects.txt").exists());
    assert!(journal.claims.lock().unwrap().is_empty());
}

#[tokio::test]
async fn recovery_omits_private_inputs_but_preserves_string_decisions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("work");
    std::fs::create_dir(&root).unwrap();
    let sample = "SYNTHETIC_PRIVATE_SAMPLE_3847";
    let credential = "FAKE_API_CREDENTIAL_FOR_TEST_ONLY_9364";
    let payload = "SYNTHETIC_PRIVATE_WRITE_2834";
    let prompt = "SYNTHETIC_PRIVATE_APPROVAL_3749";
    let message = "SYNTHETIC_PRIVATE_MESSAGE_9248";
    let comparison = format!("captured == \"{sample}\"");
    let definition = flow(json!([
        {"kind":"sense","id":"captured","spec":{"target":"sample"}},
        {"kind":"sense","id":"private-object","spec":{"target":"object"}},
        {"kind":"cond","id":"branch","expr":comparison,"then":[append("first",payload)],"otherwise":[append("wrong","WRONG")]},
        {"kind":"human_approve","id":"wait","prompt":prompt},
        {"kind":"assert","id":"after-resume","expr":comparison},
        {"kind":"notify","id":"notify","message":message}
    ]));
    let journal = Arc::new(Journal::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let mut backend = MockBackend::new(root.clone());
    backend.sense_results.insert("sample".into(), json!(sample));
    backend.sense_results.insert(
        "object".into(),
        json!({"api_key":credential,"clipboard":sample}),
    );
    let mut first = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        backend,
        Box::new(CancelApproval(cancel.clone())),
        journal.clone(),
        cancel,
    )
    .unwrap();
    let aborted = first.run().await.unwrap();
    assert_eq!(aborted.state, WorkflowState::Aborted);
    assert_eq!(aborted.metrics.tool_calls, 3);
    let snapshot = journal.state.lock().unwrap().clone().unwrap();
    assert_eq!(snapshot["version"], 2);
    assert!(snapshot.get("definition").is_none());
    for private in [sample, credential, payload, prompt, message] {
        assert!(
            !snapshot.to_string().contains(private),
            "recovery journal copied private source/output"
        );
        assert!(
            !serde_json::to_string(&aborted.steps)
                .unwrap()
                .contains(private),
            "outcome copied raw sampled data"
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        payload
    );
    // No read results are configured. Re-reading a completed sense would fail.
    let mut resumed = engine(
        definition,
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let completed = resumed.run().await.unwrap();
    assert_eq!(completed.state, WorkflowState::Succeeded);
    assert_eq!(completed.metrics.tool_calls, 1);
    assert_eq!(completed.metrics.reused_tool_calls, 3);
    assert_eq!(completed.metrics.reserved_task_tool_calls, 4);
    assert_eq!(resumed.ctx().get("captured"), Some(&Value::Null));
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        payload
    );
    for private in [sample, credential, payload, prompt, message] {
        assert!(!journal
            .state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .to_string()
            .contains(private));
        assert!(!serde_json::to_string(&completed.steps)
            .unwrap()
            .contains(private));
    }
}

#[tokio::test]
async fn restored_numeric_predicates_cover_every_comparison_operator() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let definition = flow(json!([
        {"kind":"sense","id":"number","spec":{"target":"number"}},
        {"kind":"human_approve","id":"wait","prompt":"continue"},
        {"kind":"assert","id":"eq","expr":"number == 42"},
        {"kind":"assert","id":"ne","expr":"number != 43"},
        {"kind":"assert","id":"gt","expr":"number > 41"},
        {"kind":"assert","id":"ge","expr":"number >= 42"},
        {"kind":"assert","id":"lt","expr":"number < 43"},
        {"kind":"assert","id":"le","expr":"number <= 42"}
    ]));
    let journal = Arc::new(Journal::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let mut backend = MockBackend::new(root.clone());
    backend.sense_results.insert("number".into(), json!(42));
    let mut first = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        backend,
        Box::new(CancelApproval(cancel.clone())),
        journal.clone(),
        cancel,
    )
    .unwrap();
    assert_eq!(first.run().await.unwrap().state, WorkflowState::Aborted);
    let mut resumed = engine(
        definition,
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let completed = resumed.run().await.unwrap();
    assert_eq!(completed.state, WorkflowState::Succeeded);
    assert_eq!(completed.metrics.tool_calls, 0);
    assert_eq!(completed.metrics.reused_tool_calls, 1);
}

#[tokio::test]
async fn new_loop_read_overwrites_the_protected_historical_value() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let definition = flow(json!([{"kind":"loop","id":"loop","max_iter":2,"body":[
        {"kind":"sense","id":"reading","spec":{"target":"value"}},
        {"kind":"cond","id":"choice","expr":"reading == 1","then":[append("a","A")],"otherwise":[append("b","B")]},
        {"kind":"human_approve","id":"wait","prompt":"continue"}
    ]}]));
    let journal = Arc::new(Journal::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let mut backend = MockBackend::new(root.clone());
    backend.sense_results.insert("value".into(), json!(1));
    let mut first = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        backend,
        Box::new(CancelApproval(cancel.clone())),
        journal.clone(),
        cancel,
    )
    .unwrap();
    assert_eq!(first.run().await.unwrap().state, WorkflowState::Aborted);
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "A"
    );
    let mut backend = MockBackend::new(root.clone());
    backend.sense_results.insert("value".into(), json!(2));
    let mut resumed = engine(
        definition,
        HashMap::new(),
        &root,
        backend,
        Box::new(AutoApprover { approve: true }),
        journal,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let completed = resumed.run().await.unwrap();
    assert_eq!(completed.state, WorkflowState::Succeeded);
    assert_eq!(completed.metrics.tool_calls, 2);
    assert_eq!(completed.metrics.reused_tool_calls, 2);
    assert_eq!(
        std::fs::read_to_string(root.join("effects.txt")).unwrap(),
        "AB"
    );
}

#[test]
fn fingerprint_is_order_independent_and_does_not_copy_source() {
    let left: Value = serde_json::from_str(r#"{"b":{"d":2,"c":"PRIVATE_SOURCE"},"a":1}"#).unwrap();
    let right: Value = serde_json::from_str(r#"{"a":1,"b":{"c":"PRIVATE_SOURCE","d":2}}"#).unwrap();
    assert_eq!(recovery_fingerprint(&left), recovery_fingerprint(&right));
    assert!(!recovery_fingerprint(&left).contains("PRIVATE_SOURCE"));
    assert_ne!(
        recovery_fingerprint(&left),
        recovery_fingerprint(&json!({"a":2}))
    );
}

#[tokio::test]
async fn legacy_journal_version_is_not_guessed_into_new_budget_state() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let definition = flow(json!([append("write", "NO")]));
    let journal = Arc::new(Journal::default());
    let _first = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    journal.state.lock().unwrap().as_mut().unwrap()["version"] = json!(1);
    let retry = engine(
        definition,
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(retry.err().unwrap().contains(WORKFLOW_RECOVERY_REQUIRED));
    assert!(journal.claims.lock().unwrap().is_empty());
    assert!(!root.join("effects.txt").exists());
}

#[test]
fn equality_does_not_equate_distinct_non_string_values() {
    use std::collections::BTreeMap;
    let cases = [
        (json!(false), "true", false),
        (json!(true), "false", false),
        (json!({"files":["owned"]}), "true", false),
        (json!({"files":["owned"]}), "3", false),
        (json!([]), "false", false),
        (Value::Null, "true", false),
        (json!(false), "false", true),
        (json!(true), "true", true),
        (json!(42), "42", true),
        (json!("42"), "42", true),
        (json!(42), "\"42\"", true),
        (json!("word"), "\"word\"", true),
        (json!("word"), "\"other\"", false),
    ];
    for (value, literal, expected) in cases {
        let context = BTreeMap::from([("value".to_string(), value.clone())]);
        assert_eq!(
            eval_expr(&format!("value == {literal}"), &context).unwrap(),
            expected,
            "wrong equality: {value:?} == {literal}"
        );
        assert_eq!(
            eval_expr(&format!("value != {literal}"), &context).unwrap(),
            !expected,
            "wrong inequality: {value:?} != {literal}"
        );
    }
}

#[tokio::test]
async fn recovery_rejects_predicates_from_pre_fix_evaluator() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let definition = flow(json!([append("write", "NO")]));
    let journal = Arc::new(Journal::default());
    let _first = engine(
        definition.clone(),
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let old_ref = recovery_fingerprint(&json!({"flow":definition,"subflows":{},"work_root":root}));
    journal.state.lock().unwrap().as_mut().unwrap()["definition_ref"] = json!(old_ref);
    let resumed = engine(
        definition,
        HashMap::new(),
        &root,
        MockBackend::new(root.clone()),
        Box::new(AutoApprover { approve: true }),
        journal.clone(),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(resumed.err().unwrap().contains(WORKFLOW_RECOVERY_REQUIRED));
    assert!(journal.claims.lock().unwrap().is_empty());
    assert!(!root.join("effects.txt").exists());
}
