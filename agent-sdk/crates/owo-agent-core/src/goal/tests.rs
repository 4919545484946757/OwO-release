use super::*;

#[test]
fn first_attempts_do_not_consume_retry_budget() {
    let mut plan = Plan::new("p-retry-budget", "g-retry-budget");
    plan.add_step(StepSpec::new("step-first", "worker"));
    plan.add_step(StepSpec::new("step-retried", "worker"));
    let mut runner = GoalRunner::new(
        Goal::new(
            "g-retry-budget",
            "count only executions after the first as retries",
        ),
        plan,
        RunnerConfig::default(),
    );
    let mut failed = Vec::new();

    for (step_id, attempts) in [("step-first", 1), ("step-retried", 3)] {
        runner
            .merge_step_outcome(
                Some(Ok(StepOutcome {
                    step_id: step_id.to_string(),
                    attempts,
                    result: StepResult::Ok {
                        output: format!("{step_id} completed"),
                        attempt_id: format!("{step_id}-attempt"),
                        validation_receipts: Vec::new(),
                        epoch: 1,
                    },
                })),
                &mut failed,
            )
            .unwrap();
    }

    assert_eq!(runner.state.steps_taken, 4);
    assert_eq!(runner.state.total_retries, 2);
    assert!(failed.is_empty());
}

#[test]
fn shared_completion_record_survives_goal_run_persistence() {
    let plan = Plan::new("completion-plan", "completion-goal");
    let mut state = GoalRunState::new(Goal::new("completion-goal", "persist acceptance"), plan);
    state.completion_record = Some(crate::completion::build_completion_record(
        "completion-goal",
        &state.run_id,
        owo_agent_protocol::CompletionStatusV1::Accepted,
        vec!["validation-receipt-1".to_string()],
        Some("candidate-sha256".to_string()),
    ));
    let directory = tempfile::tempdir().unwrap();
    state.persist(directory.path()).unwrap();
    let loaded = GoalRunState::load(directory.path(), &state.run_id).unwrap();
    assert_eq!(loaded.completion_record, state.completion_record);
}

#[test]
fn failed_goal_completion_retains_observed_candidate_version() {
    let goal = Goal::new("g-failed-candidate", "failed candidate identity");
    let mut plan = Plan::new("p-failed-candidate", "g-failed-candidate");
    plan.add_step(crate::plan::StepSpec::new("step-candidate", "worker"));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    let record = runner.state.records.get_mut("step-candidate").unwrap();
    record.status = StepStatus::Succeeded;
    record.attempt_id = Some("attempt-candidate".to_string());
    record.output = Some("candidate output".to_string());

    runner
        .fail_goal_with_status(
            "required verification failed".to_string(),
            owo_agent_protocol::CompletionStatusV1::Unverified,
        )
        .unwrap();

    let expected = crate::completion::hash_candidate_version(&serde_json::json!({
        "accepted_step_outputs": [{
            "step_id": "step-candidate",
            "attempt_id": "attempt-candidate",
            "output_sha256": crate::cas_store::CasStore::hash_of(b"candidate output"),
        }],
        "workspace_paths": std::collections::BTreeMap::<String, String>::new(),
    }))
    .unwrap();
    let completion = runner.state.completion_record.as_ref().unwrap();
    assert_eq!(
        completion.status,
        owo_agent_protocol::CompletionStatusV1::Unverified
    );
    assert_eq!(
        completion.candidate_version_sha256.as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn aborted_goal_completion_retains_observed_candidate_version() {
    let goal = Goal::new("g-aborted-candidate", "aborted candidate identity");
    let mut plan = Plan::new("p-aborted-candidate", "g-aborted-candidate");
    plan.add_step(crate::plan::StepSpec::new("step-candidate", "worker"));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    let record = runner.state.records.get_mut("step-candidate").unwrap();
    record.status = StepStatus::Succeeded;
    record.attempt_id = Some("attempt-candidate".to_string());
    record.output = Some("candidate output".to_string());

    runner.abort();

    let expected = crate::completion::hash_candidate_version(&serde_json::json!({
        "accepted_step_outputs": [{
            "step_id": "step-candidate",
            "attempt_id": "attempt-candidate",
            "output_sha256": crate::cas_store::CasStore::hash_of(b"candidate output"),
        }],
        "workspace_paths": std::collections::BTreeMap::<String, String>::new(),
    }))
    .unwrap();
    let completion = runner.state.completion_record.as_ref().unwrap();
    assert_eq!(
        completion.status,
        owo_agent_protocol::CompletionStatusV1::Aborted
    );
    assert_eq!(
        completion.candidate_version_sha256.as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn goal_status_machine_transitions() {
    let mut goal = Goal::new("g1", "测试目标");
    assert_eq!(goal.status, GoalStatus::Pending);
    goal.transition(GoalStatus::Planning);
    goal.transition(GoalStatus::Running);
    goal.transition(GoalStatus::Verifying);
    goal.transition(GoalStatus::Succeeded);
    assert!(goal.status.is_terminal());
    assert!(!GoalStatus::Running.is_terminal());
}

#[test]
fn run_state_serde_roundtrip() {
    let plan = Plan::new("p1", "g1");
    let state = GoalRunState::new(Goal::new("g1", "目标"), plan);
    let json = serde_json::to_string(&state).unwrap();
    let restored: GoalRunState = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.run_id, state.run_id);
    assert_eq!(restored.records.len(), 0);

    let mut legacy =
        serde_json::from_value::<serde_json::Value>(serde_json::to_value(&state).unwrap()).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("validation_receipts");
    legacy.as_object_mut().unwrap().remove("delivery_issues");
    legacy.as_object_mut().unwrap().remove("completion_record");
    legacy["goal"]
        .as_object_mut()
        .unwrap()
        .remove("verification_plan");
    let restored_legacy: GoalRunState = serde_json::from_value(legacy).unwrap();
    assert!(restored_legacy.validation_receipts.is_empty());
    assert!(restored_legacy.delivery_issues.is_empty());
    assert!(restored_legacy.goal.verification_plan.is_none());
}

#[test]
fn goal_verification_persists_receipt_bound_to_accepted_output() {
    let mut goal = Goal::new("g-receipt", "验证目标收据");
    goal.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "goal-plan-v1".to_string(),
        requirements: vec![crate::verification::requirement_for_spec(
            "goal-ready",
            &crate::plan::VerificationSpec::OutputContains("ready".to_string()),
        )],
    });
    let mut plan = Plan::new("p-receipt", "g-receipt");
    plan.steps
        .push(owo_agent_contracts::plan::StepSpec::new("step-1", "worker"));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.state.records.insert(
        "step-1".to_string(),
        StepRecord {
            step_id: "step-1".to_string(),
            status: StepStatus::Succeeded,
            attempts: 1,
            attempt_id: Some("attempt-1".to_string()),
            output: Some("service ready".to_string()),
            error: None,
            skip_reason: None,
            phase_epoch: Some(3),
            validation_receipts: Vec::new(),
        },
    );

    runner
        .state
        .validation_receipts
        .push(crate::plan::ValidationReceiptV1 {
            receipt_id: "prior-failed".to_string(),
            task_id: "g-receipt".to_string(),
            attempt_id: "old-attempt".to_string(),
            epoch: 1,
            requirement_id: "old-requirement".to_string(),
            validator_id: "old-validator".to_string(),
            validator_version: "1".to_string(),
            arguments_sha256: "old-arguments".to_string(),
            input_sha256: "old-input".to_string(),
            environment_id: "old-environment".to_string(),
            changeset_sha256: None,
            detail: Some("prior attempt failed".to_string()),
            subject_sha256: std::collections::HashMap::new(),
            verdict: crate::plan::ValidationVerdictV1::Failed,
            evidence_refs: Vec::new(),
            review_result: None,
            started_at: "old".to_string(),
            completed_at: "old".to_string(),
        });

    assert_eq!(runner.verify_goal().unwrap(), GoalStatus::Succeeded);
    let receipt = runner
        .state
        .validation_receipts
        .iter()
        .find(|receipt| {
            receipt.requirement_id == "goal-ready"
                && receipt.verdict == crate::plan::ValidationVerdictV1::Passed
        })
        .unwrap();
    assert_eq!(receipt.task_id, "g-receipt");
    assert_eq!(receipt.requirement_id, "goal-ready");
    assert_eq!(receipt.validator_id, "artifact-output-contains-v1");
    assert_eq!(receipt.verdict, crate::plan::ValidationVerdictV1::Passed);
    assert_eq!(
        receipt.input_sha256,
        crate::cas_store::CasStore::hash_of(b"service ready")
    );
    assert_eq!(
        receipt.evidence_refs,
        vec!["goal-step:step-1:attempt:attempt-1"]
    );

    let completion = runner.state.completion_record.as_ref().unwrap();
    assert_eq!(
        completion.evidence_receipt_ids,
        vec![receipt.receipt_id.clone()]
    );

    let encoded = serde_json::to_vec(&runner.state).unwrap();
    let restored: GoalRunState = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        restored.validation_receipts,
        runner.state.validation_receipts
    );
}

#[test]
fn prior_attempt_receipt_cannot_satisfy_the_current_step_requirement() {
    let output = "same output from a retried attempt";
    let mut step = owo_agent_contracts::plan::StepSpec::new("step-1", "worker");
    let requirement = crate::verification::requirement_for_spec(
        "step-check",
        &crate::plan::VerificationSpec::OutputContains("same output".to_string()),
    );
    step.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "step-plan".to_string(),
        requirements: vec![requirement.clone()],
    });
    let mut plan = Plan::new("p-attempt-bound", "g-attempt-bound");
    plan.steps.push(step);
    let mut runner = GoalRunner::new(
        Goal::new("g-attempt-bound", "当前 attempt 必须有自己的验收收据"),
        plan,
        RunnerConfig::default(),
    );
    let output_sha256 = crate::cas_store::CasStore::hash_of(output.as_bytes());
    let arguments_sha256 =
        crate::cas_store::CasStore::hash_of(&serde_json::to_vec(&requirement.arguments).unwrap());
    runner.state.records.insert(
        "step-1".to_string(),
        StepRecord {
            step_id: "step-1".to_string(),
            status: StepStatus::Succeeded,
            attempts: 2,
            attempt_id: Some("attempt-current".to_string()),
            output: Some(output.to_string()),
            error: None,
            skip_reason: None,
            phase_epoch: Some(4),
            validation_receipts: vec![crate::plan::ValidationReceiptV1 {
                receipt_id: "receipt-prior-attempt".to_string(),
                task_id: "step-1".to_string(),
                attempt_id: "attempt-prior".to_string(),
                epoch: 3,
                requirement_id: requirement.requirement_id.clone(),
                validator_id: requirement.validator_id.clone(),
                validator_version: requirement.validator_version.clone().unwrap(),
                arguments_sha256,
                input_sha256: "prior-input".to_string(),
                environment_id: "test".to_string(),
                changeset_sha256: None,
                detail: None,
                subject_sha256: std::collections::HashMap::from([(
                    "step-output".to_string(),
                    output_sha256,
                )]),
                verdict: crate::plan::ValidationVerdictV1::Passed,
                evidence_refs: Vec::new(),
                review_result: None,
                started_at: "t1".to_string(),
                completed_at: "t1".to_string(),
            }],
        },
    );

    assert_eq!(runner.verify_goal().unwrap(), GoalStatus::Failed);
    assert_eq!(
        runner.state.completion_record.unwrap().status,
        owo_agent_protocol::CompletionStatusV1::Unverified
    );
}

#[test]
fn unsupported_goal_verifier_is_recorded_and_cannot_succeed() {
    let mut goal = Goal::new("g-unsupported", "未知验证器不得通过");
    let mut requirement = crate::verification::requirement_for_spec(
        "goal-custom",
        &crate::plan::VerificationSpec::OutputNonEmpty,
    );
    requirement.validator_id = "custom-behavior-check".to_string();
    goal.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "goal-plan-unsupported".to_string(),
        requirements: vec![requirement],
    });
    let mut runner = GoalRunner::new(
        goal,
        Plan::new("p-unsupported", "g-unsupported"),
        RunnerConfig::default(),
    );
    runner.state.records.insert(
        "step-1".to_string(),
        StepRecord {
            step_id: "step-1".to_string(),
            status: StepStatus::Succeeded,
            attempts: 1,
            attempt_id: Some("attempt-1".to_string()),
            output: Some("looks good".to_string()),
            error: None,
            skip_reason: None,
            phase_epoch: Some(1),
            validation_receipts: Vec::new(),
        },
    );

    assert_eq!(runner.verify_goal().unwrap(), GoalStatus::Failed);
    assert_eq!(runner.state.validation_receipts.len(), 1);
    assert_eq!(
        runner.state.validation_receipts[0].verdict,
        crate::plan::ValidationVerdictV1::Unsupported
    );
    assert_eq!(
        runner.state.completion_record.as_ref().unwrap().status,
        owo_agent_protocol::CompletionStatusV1::Unverified
    );
}

#[test]
fn goal_with_executed_steps_and_no_validation_plan_is_not_accepted() {
    let mut plan = Plan::new("p-no-plan", "g-no-plan");
    plan.add_step(crate::plan::StepSpec::new("step-1", "worker"));
    let mut runner = GoalRunner::new(
        Goal::new("g-no-plan", "缺少宿主验收计划的目标"),
        plan,
        RunnerConfig::default(),
    );
    runner.state.records.insert(
        "step-1".to_string(),
        StepRecord {
            step_id: "step-1".to_string(),
            status: StepStatus::Succeeded,
            attempts: 1,
            attempt_id: Some("attempt-1".to_string()),
            output: Some("implementation candidate".to_string()),
            error: None,
            skip_reason: None,
            phase_epoch: Some(1),
            validation_receipts: Vec::new(),
        },
    );

    assert_eq!(runner.verify_goal().unwrap(), GoalStatus::Failed);
    assert!(runner
        .state
        .goal
        .error
        .as_deref()
        .is_some_and(|error| error.contains("共享完成条件")));
    assert_eq!(
        runner.state.completion_record.as_ref().unwrap().status,
        owo_agent_protocol::CompletionStatusV1::Candidate
    );
}

struct NamedDispatchGateWorker {
    name: &'static str,
    started: tokio::sync::mpsc::UnboundedSender<String>,
    release: Option<std::sync::Arc<tokio::sync::Semaphore>>,
}

#[async_trait::async_trait]
impl Worker for NamedDispatchGateWorker {
    fn name(&self) -> &str {
        self.name
    }

    async fn run(&self, _input: &serde_json::Value) -> Result<String, String> {
        self.started
            .send(self.name.to_string())
            .map_err(|_| "scheduler test receiver closed".to_string())?;
        if let Some(release) = &self.release {
            let _permit = release
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| "scheduler test gate closed".to_string())?;
        }
        Ok(format!("{} completed", self.name))
    }
}

#[tokio::test]
async fn ready_critical_path_continuation_preempts_unrelated_fifo_backlog() {
    let goal = Goal::new(
        "g-critical-schedule",
        "prioritize critical dependency chain",
    );
    let mut plan = Plan::new("p-critical-schedule", "g-critical-schedule");
    plan.add_step(crate::plan::StepSpec::new("a", "dispatch-a"));
    plan.add_step(crate::plan::StepSpec::new("b", "dispatch-b"));
    plan.add_step(crate::plan::StepSpec::new("c", "dispatch-c"));
    let mut a2 = crate::plan::StepSpec::new("a2", "dispatch-a2");
    a2.depends_on = vec!["a".to_string()];
    plan.add_step(a2);
    let mut a3 = crate::plan::StepSpec::new("a3", "dispatch-a3");
    a3.depends_on = vec!["a2".to_string()];
    plan.add_step(a3);

    let workers = WorkerRegistry::new();
    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
    let release_a = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let release_b = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    for (name, release) in [
        ("dispatch-a", Some(std::sync::Arc::clone(&release_a))),
        ("dispatch-b", Some(std::sync::Arc::clone(&release_b))),
        ("dispatch-c", None),
        ("dispatch-a2", None),
        ("dispatch-a3", None),
    ] {
        workers.register(std::sync::Arc::new(NamedDispatchGateWorker {
            name,
            started: started_tx.clone(),
            release,
        }));
    }
    drop(started_tx);

    let config = RunnerConfig {
        max_parallel: 2,
        ..RunnerConfig::default()
    };
    let mut runner = GoalRunner::new(goal, plan, config);
    // This test verifies ready-frontier ordering only; product acceptance is covered
    // by the outer Team DeliveryGate and has separate completion-contract tests.
    runner.defer_goal_acceptance_to_delivery_gate();
    let driver = tokio::spawn(async move {
        let status = runner.run(&workers).await;
        (status, runner.state)
    });

    let initial = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let mut names = std::collections::HashSet::new();
        while !(names.contains("dispatch-a") && names.contains("dispatch-b")) {
            names.insert(
                started_rx
                    .recv()
                    .await
                    .expect("initial workers should start"),
            );
        }
    })
    .await;
    release_a.add_permits(1);
    if initial.is_err() {
        release_b.add_permits(1);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), driver).await;
        panic!("critical root and independent worker must fill both slots");
    }

    let next = tokio::time::timeout(std::time::Duration::from_secs(3), started_rx.recv()).await;
    release_b.add_permits(1);
    let next = next
        .expect("a completed worker must release a scheduler slot")
        .expect("scheduler event channel remains open");
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), driver)
        .await
        .expect("all scheduled steps should complete")
        .expect("GoalRunner task should not panic");

    assert_eq!(
        next, "dispatch-a2",
        "critical continuation must outrank queued C"
    );
    let (status, state) = outcome;
    assert_eq!(
        status.unwrap(),
        GoalStatus::Succeeded,
        "goal_error={:?}; step_records={:?}",
        state.goal.error,
        state.records
    );
}

struct FixedOutputWorker(&'static str);

#[async_trait::async_trait]
impl Worker for FixedOutputWorker {
    fn name(&self) -> &str {
        "fixed-output"
    }

    async fn run(&self, _input: &serde_json::Value) -> Result<String, String> {
        Ok(self.0.to_string())
    }
}

struct AlwaysFailWorker(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[async_trait::async_trait]
impl Worker for AlwaysFailWorker {
    fn name(&self) -> &str {
        "always-fail"
    }

    async fn run(&self, _input: &serde_json::Value) -> Result<String, String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err("expected worker failure".to_string())
    }
}

#[tokio::test]
async fn zero_retry_budget_allows_first_execution_at_exact_step_limit() {
    let mut goal = Goal::new("g-budget-exact", "finish exactly at the execution budget");
    goal.budget.max_steps = 1;
    goal.budget.max_total_retries = 0;
    let mut plan = Plan::new("p-budget-exact", "g-budget-exact");
    plan.add_step(StepSpec::new("step-1", "fixed-output"));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("accepted candidate")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.defer_goal_acceptance_to_delivery_gate();

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Succeeded);
    assert_eq!(runner.state.steps_taken, 1);
    assert_eq!(runner.state.total_retries, 0);
}

#[tokio::test]
async fn exhausted_retry_budget_blocks_only_the_extra_worker_call() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut goal = Goal::new("g-budget-retry", "retry budget only limits retries");
    goal.budget.max_total_retries = 0;
    let mut plan = Plan::new("p-budget-retry", "g-budget-retry");
    let mut step = StepSpec::new("step-1", "always-fail");
    step.retries = 3;
    plan.add_step(step);
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(AlwaysFailWorker(
        std::sync::Arc::clone(&calls),
    )));
    let mut runner = GoalRunner::new(
        goal,
        plan,
        RunnerConfig {
            allow_replan: false,
            ..RunnerConfig::default()
        },
    );

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Failed);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(runner.state.steps_taken, 1);
    assert_eq!(runner.state.total_retries, 0);
}

struct CountingSuccessWorker(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[async_trait::async_trait]
impl Worker for CountingSuccessWorker {
    fn name(&self) -> &str {
        "counting-success"
    }

    async fn run(&self, _input: &serde_json::Value) -> Result<String, String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok("completed".to_string())
    }
}

#[tokio::test]
async fn worker_call_is_blocked_when_durable_admission_is_rejected() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(CountingSuccessWorker(Arc::clone(
        &calls,
    ))));
    let step = StepSpec::new("step-admission", "counting-success");
    let (admission_tx, mut admission_rx) = attempt_admission_channel();
    let runtime = StepRuntime {
        execution_budget: Arc::new(ExecutionBudget::new(0, 0, GoalBudget::default())),
        aborted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        critic: None,
        blackboard: None,
        bb_writer: None,
        use_worker_pool: false,
        worker_pool: None,
        capabilities: None,
        capability_requirement: None,
        transport: None,
        leases: None,
        bindings: Vec::new(),
        run_id: "run-admission".to_string(),
        cancels: DispatchCancelRegistry::default(),
        workspace_verification_root: None,
        workspace_command_verifier: None,
        attempt_admission: Some(admission_tx),
    };
    let execution = tokio::spawn(super::run_step_attempts(
        workers,
        step,
        runtime,
        "attempt-admission".to_string(),
        1,
    ));

    let admission = admission_rx.recv().await.expect("worker admission");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    admission.respond(Err("checkpoint write failed".to_string()));
    let outcome = execution.await.unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(outcome.attempts, 0);
    assert!(matches!(outcome.result, StepResult::Budget { .. }));
}

#[tokio::test]
async fn worker_call_stays_blocked_when_abort_races_durable_admission_ack() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(CountingSuccessWorker(Arc::clone(
        &calls,
    ))));
    let step = StepSpec::new("step-cancel-race", "counting-success");
    let (admission_tx, mut admission_rx) = attempt_admission_channel();
    let aborted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let runtime = StepRuntime {
        execution_budget: Arc::new(ExecutionBudget::new(0, 0, GoalBudget::default())),
        aborted: Arc::clone(&aborted),
        critic: None,
        blackboard: None,
        bb_writer: None,
        use_worker_pool: false,
        worker_pool: None,
        capabilities: None,
        capability_requirement: None,
        transport: None,
        leases: None,
        bindings: Vec::new(),
        run_id: "run-cancel-race".to_string(),
        cancels: DispatchCancelRegistry::default(),
        workspace_verification_root: None,
        workspace_command_verifier: None,
        attempt_admission: Some(admission_tx),
    };
    let execution = tokio::spawn(super::run_step_attempts(
        workers,
        step,
        runtime,
        "attempt-cancel-race".to_string(),
        1,
    ));

    let admission = admission_rx.recv().await.expect("worker admission");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    // Cancellation lands after the request reached the durable host but before its
    // acknowledgement wakes the Worker task: this is the race the shared flag closes.
    aborted.store(true, std::sync::atomic::Ordering::SeqCst);
    admission.respond(Ok(()));
    let outcome = execution.await.unwrap();

    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(outcome.attempts, 0);
    assert!(matches!(outcome.result, StepResult::Retried { .. }));
}

#[tokio::test]
async fn parallel_dispatch_reserves_global_step_budget_before_worker_calls() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut goal = Goal::new(
        "g-budget-parallel",
        "parallel dispatch respects the shared budget",
    );
    goal.budget.max_steps = 2;
    let mut plan = Plan::new("p-budget-parallel", "g-budget-parallel");
    plan.add_step(StepSpec::new("step-a", "counting-success"));
    plan.add_step(StepSpec::new("step-b", "counting-success"));
    plan.add_step(StepSpec::new("step-c", "counting-success"));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(CountingSuccessWorker(
        std::sync::Arc::clone(&calls),
    )));
    let mut runner = GoalRunner::new(
        goal,
        plan,
        RunnerConfig {
            max_parallel: 3,
            allow_replan: false,
            ..RunnerConfig::default()
        },
    );
    runner.defer_goal_acceptance_to_delivery_gate();

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Failed);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(runner.state.steps_taken, 2);
}

#[tokio::test]
async fn team_phase_budget_seed_includes_prior_phase_usage() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut goal = Goal::new("g-budget-phase-seed", "phase budget is run-global");
    goal.budget.max_steps = 1;
    let mut plan = Plan::new("p-budget-phase-seed", "g-budget-phase-seed");
    plan.add_step(StepSpec::new("step-1", "counting-success"));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(CountingSuccessWorker(
        std::sync::Arc::clone(&calls),
    )));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.seed_execution_budget_usage(1, 0);
    runner.defer_goal_acceptance_to_delivery_gate();

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Failed);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn checkpoint_failure_is_reported_instead_of_returning_goal_success() {
    let root = tempfile::tempdir().unwrap();
    let blocked_path = root.path().join("not-a-directory");
    std::fs::write(&blocked_path, b"block checkpoint directory").unwrap();

    let mut plan = Plan::new("p-checkpoint-failure", "g-checkpoint-failure");
    plan.add_step(StepSpec::new("step-1", "fixed-output"));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("candidate")));
    let mut runner = GoalRunner::new(
        Goal::new(
            "g-checkpoint-failure",
            "checkpoint failure cannot report success",
        ),
        plan,
        RunnerConfig {
            persist_dir: Some(blocked_path),
            ..RunnerConfig::default()
        },
    );
    runner.defer_goal_acceptance_to_delivery_gate();

    let result = runner.run(&workers).await;
    assert!(result
        .as_ref()
        .err()
        .is_some_and(|error| error.contains("检查点写入失败")));
    assert_eq!(runner.state.goal.status, GoalStatus::Succeeded);
}

#[tokio::test]
async fn team_phase_can_defer_aggregate_acceptance_to_delivery_gate() {
    let goal = Goal::new("g-team-phase", "team phase");
    let mut plan = Plan::new("p-team-phase", "g-team-phase");
    plan.add_step(crate::plan::StepSpec::new("step-1", "fixed-output"));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("candidate output")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.defer_goal_acceptance_to_delivery_gate();

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Succeeded);
    assert_eq!(runner.state.goal.status, GoalStatus::Succeeded);
    assert!(
        runner.state.completion_record.is_none(),
        "an internal Team phase is not a final delivery acceptance"
    );
}

#[tokio::test]
async fn typed_step_plan_executes_and_persists_attempt_bound_receipt() {
    let goal = Goal::new("g-step-receipt", "typed step verification");
    let mut plan = Plan::new("p-step-receipt", "g-step-receipt");
    let mut step = crate::plan::StepSpec::new("step-1", "fixed-output");
    step.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "step-plan-v1".to_string(),
        requirements: vec![crate::verification::requirement_for_spec(
            "contains-ready",
            &crate::plan::VerificationSpec::OutputContains("ready".to_string()),
        )],
    });
    plan.add_step(step);
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("service ready")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Succeeded);
    let record = runner.state.records.get("step-1").unwrap();
    assert_eq!(record.status, StepStatus::Succeeded);
    assert_eq!(record.output.as_deref(), Some("service ready"));
    assert!(record.attempt_id.is_some());
    assert_eq!(
        record.validation_receipts[0].attempt_id,
        record.attempt_id.as_deref().unwrap()
    );
    assert_eq!(
        Some(record.validation_receipts[0].epoch),
        record.phase_epoch
    );
    assert_eq!(record.validation_receipts.len(), 1);
    assert_eq!(
        record.validation_receipts[0].verdict,
        crate::plan::ValidationVerdictV1::Passed
    );
    assert_eq!(
        record.validation_receipts[0]
            .subject_sha256
            .get("step-output"),
        Some(&crate::cas_store::CasStore::hash_of(b"service ready"))
    );
}

#[tokio::test]
async fn host_command_verifier_receipt_is_consumed_by_goal_step_and_bound_to_attempt() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    let source = root.path().join("src/lib.rs");
    std::fs::write(&source, "pub fn ready() {}\n").unwrap();
    let workspace_hash = crate::cas_store::CasStore::hash_of(&std::fs::read(&source).unwrap());

    let goal = Goal::new("g-command-receipt", "consume host behavior receipt");
    let mut plan = Plan::new("p-command-receipt", "g-command-receipt");
    let mut step = crate::plan::StepSpec::new("step-command", "fixed-output");
    step.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "command-plan".to_string(),
        requirements: vec![crate::plan::VerificationRequirementV1 {
            requirement_id: "behavior".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-command-success-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["src/lib.rs".to_string()],
            },
            arguments: serde_json::json!({"command":"cargo test -p owo-agent-core"}),
            required: true,
            resources: crate::plan::VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 16,
                exclusive_workspace: false,
                timeout_ms: 10_000,
            },
        }],
    });
    plan.add_step(step);
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("candidate")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.attach_workspace_verification_root(root.path().to_path_buf());
    let expected_workspace_hash = workspace_hash.clone();
    runner.attach_workspace_command_verifier(move |step_id, attempt_id, requirement| {
        assert_eq!(step_id, "step-command");
        assert_eq!(requirement.requirement_id, "behavior");
        crate::goal::HostCommandValidationV1 {
            verdict: crate::plan::ValidationVerdictV1::Passed,
            detail: None,
            subject_sha256: std::collections::BTreeMap::from([(
                "workspace-path:src/lib.rs".to_string(),
                expected_workspace_hash.clone(),
            )]),
            evidence_ref: Some(format!(
                "command-result:sha256:{}",
                crate::cas_store::CasStore::hash_of(attempt_id.as_bytes())
            )),
        }
    });

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Succeeded);
    let record = runner.state.records.get("step-command").unwrap();
    let receipt = record.validation_receipts.first().unwrap();
    assert_eq!(receipt.attempt_id, record.attempt_id.as_deref().unwrap());
    assert_eq!(receipt.verdict, crate::plan::ValidationVerdictV1::Passed);
    assert_eq!(
        receipt.subject_sha256.get("workspace-path:src/lib.rs"),
        Some(&workspace_hash)
    );
    assert!(receipt.evidence_refs.iter().any(|reference| reference
        == &format!(
            "command-result:sha256:{}",
            crate::cas_store::CasStore::hash_of(receipt.attempt_id.as_bytes())
        )));
}

#[test]
fn passed_host_command_receipt_requires_exact_final_source_and_command_evidence() {
    let requirement = crate::plan::VerificationRequirementV1 {
        requirement_id: "behavior".to_string(),
        covers_requirement_ids: Vec::new(),
        validator_id: "workspace-command-success-v1".to_string(),
        validator_version: Some("1".to_string()),
        scope: crate::plan::VerificationScopeV1::WorkspacePaths {
            relative_paths: vec!["src/lib.rs".to_string()],
        },
        arguments: serde_json::json!({"command":"cargo test -p owo-agent-core"}),
        required: true,
        resources: crate::plan::VerificationResourcesV1 {
            cpu_slots: 1,
            memory_mb: 16,
            exclusive_workspace: false,
            timeout_ms: 10_000,
        },
    };
    let valid = crate::goal::HostCommandValidationV1 {
        verdict: crate::plan::ValidationVerdictV1::Passed,
        detail: None,
        subject_sha256: std::collections::BTreeMap::from([(
            "workspace-path:src/lib.rs".to_string(),
            "a".repeat(64),
        )]),
        evidence_ref: Some(format!("command-result:sha256:{}", "b".repeat(64))),
    };
    assert!(super::validate_host_command_validation(&requirement, &valid).is_ok());

    let mut missing_evidence = valid.clone();
    missing_evidence.evidence_ref = None;
    assert!(super::validate_host_command_validation(&requirement, &missing_evidence).is_err());

    let mut missing_path = valid.clone();
    missing_path.subject_sha256.clear();
    assert!(super::validate_host_command_validation(&requirement, &missing_path).is_err());

    let mut extra_path = valid.clone();
    extra_path
        .subject_sha256
        .insert("workspace-path:src/other.rs".to_string(), "c".repeat(64));
    assert!(super::validate_host_command_validation(&requirement, &extra_path).is_err());

    let mut malformed_hash = valid;
    malformed_hash.subject_sha256.insert(
        "workspace-path:src/lib.rs".to_string(),
        "not-a-hash".to_string(),
    );
    assert!(super::validate_host_command_validation(&requirement, &malformed_hash).is_err());
}

#[tokio::test]
async fn malformed_passed_host_command_receipt_cannot_succeed_a_goal_step() {
    let root = tempfile::tempdir().unwrap();
    let goal = Goal::new("g-command-malformed", "reject malformed host evidence");
    let mut plan = Plan::new("p-command-malformed", "g-command-malformed");
    let mut step = crate::plan::StepSpec::new("step-command", "fixed-output");
    step.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "command-plan".to_string(),
        requirements: vec![crate::plan::VerificationRequirementV1 {
            requirement_id: "behavior".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-command-success-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["src/lib.rs".to_string()],
            },
            arguments: serde_json::json!({"command":"cargo test -p owo-agent-core"}),
            required: true,
            resources: crate::plan::VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 16,
                exclusive_workspace: false,
                timeout_ms: 10_000,
            },
        }],
    });
    plan.add_step(step);
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("candidate")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.attach_workspace_verification_root(root.path().to_path_buf());
    runner.attach_workspace_command_verifier(|_, _, _| crate::goal::HostCommandValidationV1 {
        verdict: crate::plan::ValidationVerdictV1::Passed,
        detail: Some("forged pass without command evidence".to_string()),
        subject_sha256: std::collections::BTreeMap::from([(
            "workspace-path:src/lib.rs".to_string(),
            "a".repeat(64),
        )]),
        evidence_ref: None,
    });

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Failed);
    let receipt = &runner.state.records["step-command"].validation_receipts[0];
    assert_eq!(
        receipt.verdict,
        crate::plan::ValidationVerdictV1::Unverified
    );
    assert_eq!(
        runner.state.completion_record.as_ref().unwrap().status,
        owo_agent_protocol::CompletionStatusV1::Unverified
    );
    assert!(receipt
        .evidence_refs
        .iter()
        .all(|reference| { !reference.starts_with("command-result:") }));
}

#[tokio::test]
async fn workspace_command_requirement_stays_unsupported_without_host_receipt_resolver() {
    let goal = Goal::new("g-command-no-host", "require trusted command evidence");
    let mut plan = Plan::new("p-command-no-host", "g-command-no-host");
    let mut step = crate::plan::StepSpec::new("step-command", "fixed-output");
    step.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "command-plan".to_string(),
        requirements: vec![crate::plan::VerificationRequirementV1 {
            requirement_id: "behavior".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-command-success-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["src/lib.rs".to_string()],
            },
            arguments: serde_json::json!({"command":"cargo test -p owo-agent-core"}),
            required: true,
            resources: crate::plan::VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 16,
                exclusive_workspace: false,
                timeout_ms: 10_000,
            },
        }],
    });
    plan.add_step(step);
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("candidate")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Failed);
    assert_eq!(
        runner.state.records["step-command"].validation_receipts[0].verdict,
        crate::plan::ValidationVerdictV1::Unverified
    );
    assert_eq!(
        runner.state.completion_record.as_ref().unwrap().status,
        owo_agent_protocol::CompletionStatusV1::Unverified
    );
}

struct MutateWorkspaceWorker(std::path::PathBuf);

#[async_trait::async_trait]
impl Worker for MutateWorkspaceWorker {
    fn name(&self) -> &str {
        "mutate-workspace"
    }

    async fn run(&self, _input: &serde_json::Value) -> Result<String, String> {
        std::fs::write(&self.0, "pub fn changed_after_validation() {}\n")
            .map_err(|error| error.to_string())?;
        Ok("target ok".to_string())
    }
}

#[tokio::test]
async fn goal_cannot_accept_a_workspace_receipt_after_a_later_step_mutates_its_file() {
    let root = std::env::temp_dir().join(format!(
        "owo-goal-final-snapshot-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("src")).unwrap();
    let source = root.join("src/lib.rs");
    std::fs::write(&source, "pub fn ready() {}\n").unwrap();

    let mut goal = Goal::new("g-final-snapshot", "reject stale workspace evidence");
    goal.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "final-snapshot-goal-plan".to_string(),
        requirements: vec![crate::verification::requirement_for_spec(
            "goal-output",
            &crate::plan::VerificationSpec::OutputContains("target ok".to_string()),
        )],
    });
    let mut plan = Plan::new("p-final-snapshot", "g-final-snapshot");
    plan.add_step(workspace_plan_step("step-workspace", "ready"));
    let mut mutate = crate::plan::StepSpec::new("step-mutate", "mutate-workspace");
    mutate.depends_on = vec!["step-workspace".to_string()];
    mutate.verify = Some(crate::plan::VerificationSpec::OutputNonEmpty);
    plan.add_step(mutate);

    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("target ok")));
    workers.register(std::sync::Arc::new(MutateWorkspaceWorker(source)));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.attach_workspace_verification_root(root.clone());

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Failed);
    let stale = runner.state.records["step-workspace"]
        .validation_receipts
        .iter()
        .find(|receipt| receipt.requirement_id == "step-workspace:file-ready")
        .unwrap();
    assert_eq!(stale.verdict, crate::plan::ValidationVerdictV1::Stale);
    assert!(runner
        .state
        .goal
        .error
        .as_deref()
        .is_some_and(|error| { error.contains("偏离通过验证的快照") }));
    assert_eq!(
        runner.state.completion_record.as_ref().unwrap().status,
        owo_agent_protocol::CompletionStatusV1::Unverified
    );
    std::fs::remove_dir_all(root).unwrap();
}

struct FeedbackAwareWorker;

#[async_trait::async_trait]
impl Worker for FeedbackAwareWorker {
    fn name(&self) -> &str {
        "feedback-worker"
    }

    async fn run(&self, input: &serde_json::Value) -> Result<String, String> {
        if input.get("_critic_feedback").is_some() {
            Ok("service ready".to_string())
        } else {
            Ok("service pending".to_string())
        }
    }
}

#[tokio::test]
async fn typed_step_verifier_checks_post_critic_output() {
    let goal = Goal::new("g-critic-receipt", "critic output must be verified");
    let mut plan = Plan::new("p-critic-receipt", "g-critic-receipt");
    let mut step = crate::plan::StepSpec::new("step-critic", "feedback-worker");
    step.input = serde_json::json!({"_critic": {"rounds": 2}});
    step.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "critic-output-plan".to_string(),
        requirements: vec![crate::verification::requirement_for_spec(
            "contains-ready",
            &crate::plan::VerificationSpec::OutputContains("ready".to_string()),
        )],
    });
    plan.add_step(step);
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FeedbackAwareWorker));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.attach_critic(crate::critic::CriticConfig::new(
        std::sync::Arc::new(crate::critic::ScriptedCritic::new(vec![
            crate::critic::CriticVerdict::reject(20, vec!["needs repair".to_string()]),
            crate::critic::CriticVerdict::approve(95),
        ])),
        crate::critic::ReadOnlyGate::read_only(),
    ));

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Succeeded);
    let record = runner.state.records.get("step-critic").unwrap();
    assert_eq!(record.output.as_deref(), Some("service ready"));
    assert_eq!(record.validation_receipts.len(), 1);
    assert_eq!(
        record.validation_receipts[0]
            .subject_sha256
            .get("step-output"),
        Some(&crate::cas_store::CasStore::hash_of(b"service ready"))
    );
}

#[tokio::test]
async fn unsupported_step_verifier_records_rejected_receipt() {
    let goal = Goal::new("g-step-unsupported", "unknown validator must fail closed");
    let mut plan = Plan::new("p-step-unsupported", "g-step-unsupported");
    let mut step = crate::plan::StepSpec::new("step-unsupported", "fixed-output");
    let mut requirement = crate::verification::requirement_for_spec(
        "behavior-check",
        &crate::plan::VerificationSpec::OutputNonEmpty,
    );
    requirement.validator_id = "behavior-check-not-registered".to_string();
    step.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "unsupported-step-plan".to_string(),
        requirements: vec![requirement],
    });
    plan.add_step(step);
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("looks good")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Failed);
    let record = runner.state.records.get("step-unsupported").unwrap();
    assert_eq!(record.status, StepStatus::Failed);
    assert!(!record.validation_receipts.is_empty());
    assert!(record
        .validation_receipts
        .iter()
        .all(|receipt| receipt.verdict == crate::plan::ValidationVerdictV1::Unsupported));
}

fn workspace_plan_step(id: &str, expected: &str) -> crate::plan::StepSpec {
    let mut step = crate::plan::StepSpec::new(id, "fixed-output");
    step.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: format!("workspace-{id}"),
        requirements: vec![crate::plan::VerificationRequirementV1 {
            requirement_id: format!("{id}:file-ready"),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-file-contains-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["src/lib.rs".to_string()],
            },
            arguments: serde_json::json!({"text": expected}),
            required: true,
            resources: crate::plan::VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 8,
                exclusive_workspace: false,
                timeout_ms: 5_000,
            },
        }],
    });
    step
}

#[tokio::test]
async fn workspace_step_validation_passes_before_success_receipt() {
    let root = std::env::temp_dir().join(format!(
        "owo-goal-workspace-pass-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn ready() {}\n").unwrap();

    let goal = Goal::new("g-workspace-pass", "host workspace validation");
    let mut plan = Plan::new("p-workspace-pass", "g-workspace-pass");
    plan.add_step(workspace_plan_step("step-workspace", "ready"));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("candidate")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.attach_workspace_verification_root(root.clone());

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Succeeded);
    let receipt = &runner.state.records["step-workspace"].validation_receipts[0];
    assert_eq!(receipt.verdict, crate::plan::ValidationVerdictV1::Passed);
    assert_eq!(
        receipt.subject_sha256.get("workspace-path:src/lib.rs"),
        Some(&crate::cas_store::CasStore::hash_of(b"pub fn ready() {}\n"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn goal_workspace_validation_binds_receipt_to_real_file_hash() {
    let root = std::env::temp_dir().join(format!(
        "owo-goal-workspace-goal-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn ready() {}\n").unwrap();

    let mut goal = Goal::new("g-workspace-goal", "host goal workspace validation");
    goal.verification_plan = Some(crate::plan::VerificationPlanV1 {
        plan_id: "goal-workspace-plan".to_string(),
        requirements: vec![crate::plan::VerificationRequirementV1 {
            requirement_id: "goal-source-ready".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-file-contains-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["src/lib.rs".to_string()],
            },
            arguments: serde_json::json!({"text": "ready"}),
            required: true,
            resources: crate::plan::VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 8,
                exclusive_workspace: false,
                timeout_ms: 5_000,
            },
        }],
    });
    let mut plan = Plan::new("p-workspace-goal", "g-workspace-goal");
    plan.add_step(crate::plan::StepSpec::new("step", "fixed-output"));
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(FixedOutputWorker("candidate")));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.attach_workspace_verification_root(root.clone());

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Succeeded);
    let receipt = &runner.state.validation_receipts[0];
    assert_eq!(receipt.verdict, crate::plan::ValidationVerdictV1::Passed);
    assert_eq!(
        receipt.subject_sha256.get("workspace-path:src/lib.rs"),
        Some(&crate::cas_store::CasStore::hash_of(b"pub fn ready() {}\n"))
    );
    assert!(receipt.changeset_sha256.is_some());
    assert!(receipt.environment_id.starts_with("goal-workspace-v1:"));
    let record = runner.state.completion_record.as_ref().unwrap();
    let step_record = &runner.state.records["step"];
    let expected_candidate = crate::completion::hash_candidate_version(&serde_json::json!({
        "accepted_step_outputs": [{
            "step_id": "step",
            "attempt_id": step_record.attempt_id,
            "output_sha256": crate::cas_store::CasStore::hash_of(b"candidate"),
        }],
        "workspace_paths": std::collections::BTreeMap::from([(
            "src/lib.rs",
            crate::cas_store::CasStore::hash_of(b"pub fn ready() {}\n"),
        )]),
    }))
    .unwrap();
    assert_eq!(
        record.candidate_version_sha256.as_deref(),
        Some(expected_candidate.as_str())
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn failed_workspace_validation_keeps_dependent_step_locked() {
    struct CountingWorker {
        name: &'static str,
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl Worker for CountingWorker {
        fn name(&self) -> &str {
            self.name
        }
        async fn run(&self, _input: &serde_json::Value) -> Result<String, String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("candidate".to_string())
        }
    }

    let root = std::env::temp_dir().join(format!(
        "owo-goal-workspace-fail-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn ready() {}\n").unwrap();

    let first_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let dependent_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let goal = Goal::new("g-workspace-fail", "failed validation blocks dependents");
    let mut plan = Plan::new("p-workspace-fail", "g-workspace-fail");
    let mut first = workspace_plan_step("step-first", "not-present");
    first.worker = "counting-first".to_string();
    let mut dependent = crate::plan::StepSpec::new("step-dependent", "counting-dependent");
    dependent.depends_on = vec!["step-first".to_string()];
    plan.add_step(first);
    plan.add_step(dependent);
    let workers = WorkerRegistry::new();
    workers.register(std::sync::Arc::new(CountingWorker {
        name: "counting-first",
        calls: first_calls.clone(),
    }));
    workers.register(std::sync::Arc::new(CountingWorker {
        name: "counting-dependent",
        calls: dependent_calls.clone(),
    }));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    runner.attach_workspace_verification_root(root.clone());

    assert_eq!(runner.run(&workers).await.unwrap(), GoalStatus::Failed);
    assert!(first_calls.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert_eq!(dependent_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(
        runner.state.records["step-first"].status,
        StepStatus::Failed
    );
    assert_eq!(
        runner.state.records["step-dependent"].status,
        StepStatus::Pending
    );
    assert_eq!(
        runner.state.records["step-first"].validation_receipts[0].verdict,
        crate::plan::ValidationVerdictV1::Failed
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn step_final_fence_rejects_file_mutated_after_cached_static_validation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    let source = root.path().join("src/lib.rs");
    std::fs::write(&source, b"ready").unwrap();
    let old_hash = crate::CasStore::hash_of(b"ready");
    let mut step = workspace_plan_step("step-fence", "ready");
    let mut command = step.verification_plan.as_ref().unwrap().requirements[0].clone();
    command.requirement_id = "behavior-mutation".into();
    command.validator_id = "workspace-command-success-v1".into();
    command.arguments = serde_json::json!({"command":"cargo test"});
    step.verification_plan
        .as_mut()
        .unwrap()
        .requirements
        .push(command);
    let rt = StepRuntime {
        execution_budget: Arc::new(ExecutionBudget::new(0, 0, GoalBudget::default())),
        aborted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        critic: None,
        blackboard: None,
        bb_writer: None,
        use_worker_pool: false,
        worker_pool: None,
        capabilities: None,
        capability_requirement: None,
        transport: None,
        leases: None,
        bindings: Vec::new(),
        run_id: "run-fence".into(),
        cancels: DispatchCancelRegistry::default(),
        workspace_verification_root: Some(root.path().to_path_buf()),
        attempt_admission: None,
        workspace_command_verifier: Some(Arc::new(move |_, attempt_id, _| {
            // A host callback is an observable boundary; emulate an external write.
            std::fs::write(&source, b"broken").unwrap();
            HostCommandValidationV1 {
                verdict: crate::plan::ValidationVerdictV1::Passed,
                detail: None,
                subject_sha256: std::collections::BTreeMap::from([(
                    "workspace-path:src/lib.rs".into(),
                    old_hash.clone(),
                )]),
                evidence_ref: Some(format!(
                    "command-result:sha256:{}",
                    crate::CasStore::hash_of(attempt_id.as_bytes())
                )),
            }
        })),
    };
    let (receipts, error) = verify_step_output(
        &step,
        &rt,
        &serde_json::json!({}),
        "attempt-fence",
        1,
        "candidate",
    );
    assert!(error.is_some());
    assert!(receipts
        .iter()
        .any(|receipt| receipt.requirement_id == "step-fence:file-ready"
            && receipt.verdict == crate::plan::ValidationVerdictV1::Stale));
    assert!(receipts
        .iter()
        .all(|receipt| receipt.verdict != crate::plan::ValidationVerdictV1::Passed));
}
#[test]
fn host_abort_signal_is_shared_with_goal_runner() {
    let plan = Plan::new("abort-plan", "abort-goal");
    let mut runner = GoalRunner::new(
        Goal::new("abort-goal", "host cancellation"),
        plan,
        RunnerConfig::default(),
    );
    let host_abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    runner.attach_abort_signal(std::sync::Arc::clone(&host_abort));

    let observed = runner.abort_signal();
    host_abort.store(true, std::sync::atomic::Ordering::SeqCst);

    assert!(std::sync::Arc::ptr_eq(&observed, &host_abort));
    assert!(observed.load(std::sync::atomic::Ordering::SeqCst));
}

struct DeadlineWorker;
#[async_trait::async_trait]
impl Worker for DeadlineWorker {
    fn name(&self) -> &str {
        "deadline-worker"
    }
    async fn run(&self, _input: &serde_json::Value) -> Result<String, String> {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        Ok("must not finish".into())
    }
}
#[tokio::test]
async fn in_flight_worker_is_cancelled_at_goal_deadline() {
    let mut goal = Goal::new("deadline-goal", "enforce in-flight deadline");
    goal.budget.max_duration_secs = 1;
    let mut plan = Plan::new("deadline-plan", "deadline-goal");
    plan.add_step(StepSpec::new("slow", "deadline-worker"));
    let registry = WorkerRegistry::new();
    registry.register(std::sync::Arc::new(DeadlineWorker));
    let mut runner = GoalRunner::new(goal, plan, RunnerConfig::default());
    let aborted = std::sync::Arc::clone(&runner.aborted_flag);
    let task = tokio::spawn(async move {
        let result = runner.run(&registry).await;
        (result, runner.state)
    });
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert!(
        aborted.load(std::sync::atomic::Ordering::SeqCst),
        "deadline must signal cancellation before cleanup completes"
    );
    let (result, state) = tokio::time::timeout(
        CANCELLATION_CLEANUP_GRACE + std::time::Duration::from_secs(5),
        task,
    )
    .await
    .expect("cleanup must remain bounded")
    .unwrap();
    let result = result.unwrap();
    assert_eq!(result, GoalStatus::Failed);
    assert!(state
        .goal
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("预算"));
    assert_ne!(state.records["slow"].status, StepStatus::Succeeded);
}
