use owo_agent_core::audit::AuditLog;
use owo_agent_core::computer_task::{ComputerTask, ComputerTaskRegistry, TaskState};
use owo_agent_core::computer_use::{
    run_approved_task_on, run_approved_task_on_with_receipts, ActionReceiptStage,
    ActionVerification, TaskGoal, TaskSurface,
};
use serde_json::{json, Value};

struct DelayedSurface {
    observations: usize,
    actions: usize,
    appears_at: usize,
}

#[async_trait::async_trait]
impl TaskSurface for DelayedSurface {
    fn app(&self) -> String {
        "notepad".into()
    }
    async fn ocr(&mut self) -> Result<Value, String> {
        self.observations += 1;
        let mut lines = vec![json!({"text":"anchor","x":10,"y":10,"width":100,"height":20})];
        if self.observations >= self.appears_at {
            lines.push(json!({"text":"visible result","x":10,"y":40,"width":100,"height":20}));
        }
        Ok(json!({"lines":lines}))
    }
    async fn click(&mut self, _: i32, _: i32) -> Result<(), String> {
        self.actions += 1;
        Ok(())
    }
    async fn type_text(&mut self, _: &str) -> Result<(), String> {
        unreachable!("this task permits only clicking")
    }
    async fn key(&mut self, _: &str) -> Result<(), String> {
        unreachable!("this task permits only clicking")
    }
    async fn launch(&mut self, _: &str) -> Result<(), String> {
        unreachable!("this task permits only clicking")
    }
}

fn approved_task() -> (ComputerTaskRegistry, Vec<TaskGoal>) {
    let registry = ComputerTaskRegistry::new();
    registry
        .create(ComputerTask {
            id: "delayed".into(),
            target_app: "notepad".into(),
            description: "delayed visual result".into(),
            allowed_actions: vec!["desktop_click".into()],
            max_duration_ms: 60_000,
            state: TaskState::Pending,
            created_at: "2026-11-04T00:00:00Z".into(),
            fuse_reason: None,
        })
        .unwrap();
    registry.approve("delayed").unwrap();
    let goals = vec![TaskGoal {
        anchor_text: "anchor".into(),
        action: "click".into(),
        value: String::new(),
        verify_text: Some("visible result".into()),
    }];
    (registry, goals)
}

#[tokio::test]
async fn valid_but_stale_first_frame_is_reobserved_without_replaying_action() {
    let (registry, goals) = approved_task();
    let mut surface = DelayedSurface {
        observations: 0,
        actions: 0,
        appears_at: 4,
    };
    let report = run_approved_task_on(
        &registry,
        &mut AuditLog::default(),
        "s",
        "delayed",
        &goals,
        &mut surface,
    )
    .await
    .expect("a valid stale frame must not stop bounded visual verification");
    assert_eq!(surface.actions, 1);
    assert_eq!(
        report.receipts[0].verification,
        ActionVerification::Verified
    );
    assert_ne!(
        report.receipts[0].before_observation_ref, report.receipts[0].after_observation_ref,
        "the receipt must reference the frame containing the verified result"
    );
}

#[tokio::test]
async fn receipt_storage_failure_pauses_before_observation_or_another_action() {
    let (registry, mut goals) = approved_task();
    goals.push(goals[0].clone());
    let mut surface = DelayedSurface {
        observations: 0,
        actions: 0,
        appears_at: 4,
    };
    let mut stages = Vec::new();
    let mut sink = |stage, receipt: &owo_agent_core::computer_use::ActionExecutionReceipt| {
        stages.push(stage);
        assert!(receipt.after_observation_ref.is_none());
        assert_eq!(receipt.verification, ActionVerification::NotVerified);
        Err("simulated durable store failure".to_string())
    };
    let error = run_approved_task_on_with_receipts(
        &registry,
        &mut AuditLog::default(),
        "s",
        "delayed",
        &goals,
        &mut surface,
        &mut sink,
    )
    .await
    .unwrap_err();
    assert!(error.contains("ACTION_RECEIPT_STORE_FAILED"), "{error}");
    assert_eq!(stages, vec![ActionReceiptStage::Execution]);
    assert_eq!(surface.actions, 1);
    assert_eq!(
        surface.observations, 1,
        "execution must be stored before post-action OCR"
    );
    assert_eq!(registry.get("delayed").unwrap().state, TaskState::Paused);
}

#[tokio::test]
async fn unchanged_frame_with_existing_expected_text_is_not_verified() {
    let (registry, goals) = approved_task();
    let mut surface = DelayedSurface {
        observations: 0,
        actions: 0,
        appears_at: 1,
    };
    let error = run_approved_task_on(
        &registry,
        &mut AuditLog::default(),
        "s",
        "delayed",
        &goals,
        &mut surface,
    )
    .await
    .expect_err("preexisting text on an unchanged frame cannot prove a click took effect");
    assert!(error.contains("POST_ACTION_VERIFY_FAILED"), "{error}");
    assert_eq!(surface.actions, 1, "verification must not replay the click");
    assert!(
        surface.observations <= 12,
        "observation must remain bounded"
    );
    assert_eq!(registry.get("delayed").unwrap().state, TaskState::Paused);
}

#[tokio::test]
async fn result_that_never_appears_still_fails_and_does_not_replay_action() {
    let (registry, goals) = approved_task();
    let mut surface = DelayedSurface {
        observations: 0,
        actions: 0,
        appears_at: usize::MAX,
    };
    let error = run_approved_task_on(
        &registry,
        &mut AuditLog::default(),
        "s",
        "delayed",
        &goals,
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.contains("POST_ACTION_VERIFY_FAILED"), "{error}");
    assert_eq!(surface.actions, 1);
    assert!(
        surface.observations <= 12,
        "verification must remain bounded"
    );
    assert_eq!(registry.get("delayed").unwrap().state, TaskState::Paused);
}

#[tokio::test]
async fn missing_control_pauses_without_input_or_receipts() {
    let (registry, mut goals) = approved_task();
    goals[0].anchor_text = "absent control".into();
    let mut surface = DelayedSurface {
        observations: 0,
        actions: 0,
        appears_at: usize::MAX,
    };
    let mut receipts = 0;
    let mut sink = |_, _: &owo_agent_core::computer_use::ActionExecutionReceipt| {
        receipts += 1;
        Ok(())
    };
    let error = run_approved_task_on_with_receipts(
        &registry,
        &mut AuditLog::default(),
        "s",
        "delayed",
        &goals,
        &mut surface,
        &mut sink,
    )
    .await
    .unwrap_err();
    assert!(error.contains("TARGET_NOT_FOUND"), "{error}");
    assert_eq!(surface.actions, 0);
    assert_eq!(receipts, 0);
    assert_eq!(registry.get("delayed").unwrap().state, TaskState::Paused);
}

#[test]
fn failure_cleanup_preserves_non_executable_states_and_their_reason() {
    for state in [
        TaskState::Pending,
        TaskState::Approved,
        TaskState::Running,
        TaskState::Paused,
        TaskState::Fused,
        TaskState::Completed,
        TaskState::Rejected,
        TaskState::Cancelled,
    ] {
        let (source, _) = approved_task();
        let mut task = source.get("delayed").unwrap();
        task.state = state;
        task.fuse_reason = Some("original reason".into());
        let registry = ComputerTaskRegistry::new();
        registry.create(task).unwrap();
        registry
            .pause_if_executable("delayed", "execution failed")
            .unwrap();
        let after = registry.get("delayed").unwrap();
        assert_eq!(
            after.state,
            if state.can_execute() {
                TaskState::Paused
            } else {
                state
            }
        );
        assert_eq!(
            after.fuse_reason.as_deref(),
            Some(if state.can_execute() {
                "execution failed"
            } else {
                "original reason"
            })
        );
    }
}
