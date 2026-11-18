//! Fail-closed external driver gate contract tests. These are not formal runs.

use async_trait::async_trait;
use owo_agent_product_eval::external_drivers::ExternalDriverRegistry;
use owo_agent_product_eval::product_eval::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn bundle(cases: Vec<ProductEvalCase>) -> SuiteBundle {
    SuiteBundle {
        dir: std::env::temp_dir(),
        suite: ProductEvalSuite {
            schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
            name: "external-driver-contract".to_string(),
            description: String::new(),
            defaults: SuiteDefaults {
                repetitions: 1,
                ..SuiteDefaults::default()
            },
            tasks: cases
                .iter()
                .map(|c| format!("tasks/{}.json", c.id))
                .collect(),
        },
        cases,
    }
}

fn case(id: &str, category: EvalCategory, fault: Option<FaultInjection>) -> ProductEvalCase {
    ProductEvalCase {
        schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
        id: id.to_string(),
        category,
        title: id.to_string(),
        instruction: "must be a real external execution".to_string(),
        inputs: vec![],
        allow_read: vec!["inputs/**".to_string()],
        allow_write: vec!["out/**".to_string()],
        expected_artifacts: vec!["out/report.md".to_string()],
        checkers: vec![],
        reference_outputs: BTreeMap::from([(
            "out/report.md".to_string(),
            "# 报告\n结论 ok\n".to_string(),
        )]),
        timeout_secs: Some(30),
        max_model_calls: Some(4),
        repetitions: Some(1),
        allow_commands: vec![],
        failure_classes: vec![],
        expected_side_effects: vec![],
        fault_injection: fault,
    }
}

fn opts() -> RunOptions {
    RunOptions {
        modes: vec![AgentMode::Single],
        reps_override: Some(1),
        ..Default::default()
    }
}

fn out_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("pe-driver-{tag}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct CheckerOnlyExecutor;

#[async_trait]
impl CaseExecutor for CheckerOnlyExecutor {
    async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome {
        let _ = ctx.write_file("out/report.md", "# 报告\n结论 ok\n");
        RawExecOutcome::default()
    }
}

#[tokio::test]
async fn reference_replay_and_checker_only_cannot_pass_gated_cases() {
    for exec in ["dry-reference", "custom-checker-only"] {
        let mut desktop = case("desktop", EvalCategory::Desktop, None);
        desktop.checkers = vec![ArtifactChecker::Contains {
            path: "out/report.md".to_string(),
            text: "结论".to_string(),
        }];
        let mut long = case(
            "long",
            EvalCategory::LongHorizon,
            Some(FaultInjection {
                kind: "restart".to_string(),
                after_step: Some(2),
                note: String::new(),
            }),
        );
        long.checkers = desktop.checkers.clone();
        let out = out_dir("deny");
        let runner = MatrixRunner::new(bundle(vec![desktop, long]), &out);
        let executor: Arc<dyn CaseExecutor> = if exec == "dry-reference" {
            Arc::new(ReferenceDryExecutor)
        } else {
            Arc::new(CheckerOnlyExecutor)
        };
        let report = runner
            .run(
                executor,
                exec,
                None,
                &opts(),
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap();
        assert_eq!(report.runs.len(), 2);
        for run in report.runs {
            assert_eq!(run.status, RunStatus::Error, "{:?}", run);
            assert!(run
                .error
                .as_deref()
                .unwrap_or_default()
                .starts_with("EVAL_"));
            assert!(run.external_evidence.is_none());
        }
        let _ = std::fs::remove_dir_all(&out);
    }
}

#[tokio::test]
async fn zero_checker_gated_case_still_fails_closed() {
    let out = out_dir("zero-checkers");
    let runner = MatrixRunner::new(
        bundle(vec![case("desktop", EvalCategory::Desktop, None)]),
        &out,
    );
    let report = runner
        .run(
            Arc::new(CheckerOnlyExecutor),
            "custom-checker-only",
            None,
            &opts(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.runs[0].status, RunStatus::Error);
    assert_eq!(
        report.runs[0].error.as_deref(),
        Some("EVAL_REAL_DESKTOP_DRIVER_REQUIRED")
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn prevalidated_checkers_cannot_bypass_external_gate() {
    struct PrevalidatedExecutor;
    #[async_trait]
    impl CaseExecutor for PrevalidatedExecutor {
        async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome {
            ctx.mark_checkers_prevalidated_for_test(1, 1);
            let _ = ctx.write_file("out/report.md", "# 报告\n结论 ok\n");
            RawExecOutcome::default()
        }
    }
    let mut desktop = case("desktop", EvalCategory::Desktop, None);
    desktop.checkers = vec![ArtifactChecker::Contains {
        path: "out/report.md".to_string(),
        text: "结论".to_string(),
    }];
    let out = out_dir("prevalidated");
    let runner = MatrixRunner::new(bundle(vec![desktop]), &out);
    let report = runner
        .run(
            Arc::new(PrevalidatedExecutor),
            "custom",
            None,
            &opts(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.runs[0].status, RunStatus::Error);
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn stale_evidence_cannot_pass_a_registered_gate() {
    let registered = ExternalDriverRegistry::new(
        Some(std::env::current_exe().unwrap()),
        Some(std::env::current_exe().unwrap()),
        None,
    )
    .unwrap();
    let out = out_dir("stale");
    let runner = MatrixRunner::new(
        bundle(vec![case("desktop", EvalCategory::Desktop, None)]),
        &out,
    )
    .with_external_drivers(registered);
    let opts = RunOptions {
        repair_attempts: 1,
        ..opts()
    };
    let report = runner
        .run(
            Arc::new(CheckerOnlyExecutor),
            "custom",
            None,
            &opts,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.runs[0].status, RunStatus::Error);
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn historical_journal_rows_without_evidence_remain_parseable() {
    let out = out_dir("history");
    let runner = MatrixRunner::new(bundle(vec![case("code", EvalCategory::Code, None)]), &out);
    std::fs::create_dir_all(&out).unwrap();
    let row = r#"{"key":{"case_id":"code","agent_mode":"single","repetition":0},"category":"code","status":"passed","wall_ms":1,"model_calls":0,"prompt_tokens":null,"completion_tokens":null,"total_tokens":null,"cost_usd":null,"failed_steps":[],"retries":0,"cancellations":0,"artifact_refs":[],"model":null,"started_at":"t","finished_at":"t","error":null}"#;
    std::fs::write(out.join("state.jsonl"), format!("{row}\n")).unwrap();
    let report = runner
        .run(
            Arc::new(CheckerOnlyExecutor),
            "custom",
            None,
            &RunOptions {
                fresh: true,
                ..opts()
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.runs.len(), 1);
    assert!(report.runs[0].external_evidence.is_none());
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn non_long_horizon_case_with_fault_injection_is_denied() {
    let out = out_dir("third-arm");
    let fault = Some(FaultInjection {
        kind: "transient-error".to_string(),
        after_step: Some(3),
        note: String::new(),
    });
    let runner = MatrixRunner::new(
        bundle(vec![case("code-fault", EvalCategory::Code, fault)]),
        &out,
    );
    let report = runner
        .run(
            Arc::new(CheckerOnlyExecutor),
            "custom",
            None,
            &opts(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.runs[0].status, RunStatus::Error);
    assert_eq!(
        report.runs[0].error.as_deref(),
        Some("EVAL_FAULT_INJECTION_DRIVER_REQUIRED")
    );
    let _ = std::fs::remove_dir_all(&out);
}

/// Positive end-to-end proof: a registered real external executable runs,
/// returns runner-bound sandbox artifacts and writes a complete evidence
/// bundle. The runner must inject artifacts through allow_write, compute the
/// authoritative file hashes, validate the exact matrix attempt, and pass.
#[tokio::test]
async fn registered_real_driver_produces_bound_evidence_and_passes() {
    let out = out_dir("positive");
    let config = out.join("drivers").join("driver-config.json");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "{}").unwrap();

    // A real, standalone executable (python.exe) plus a helper script passed via
    // driver argv. The registry identity includes both file bytes.
    let helper = out.join("drivers").join("positive-driver.py");
    std::fs::write(
        &helper,
        r##"import json, pathlib, sys, hashlib
request = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
d = pathlib.Path(request["evidence_dir"])
(d / "screenshots").mkdir(parents=True, exist_ok=True)
(d / "windows-actions.json").write_text(json.dumps({"actions": ["real-window-observe"]}), encoding="utf-8")
(d / "independent-observations.json").write_text(json.dumps({"observations": ["real-independent-check"]}), encoding="utf-8")
(d / "screenshots" / "manifest.json").write_text(json.dumps({"screenshots": ["real-desktop.png"]}), encoding="utf-8")
report = {
  "schema_version": 1,
  "driver_kind": "real_desktop",
  "driver_id": request["driver_id"],
  "driver_identity_sha256": request["driver_identity_sha256"],
  "case_id": request["case_id"],
  "case_contract_sha256": request["case_contract_sha256"],
  "execution": request["execution"],
  "agent_mode": request["agent_mode"],
  "repetition": request["repetition"],
  "batch_label": request["batch_label"],
  "repair_index": request["repair_index"],
  "repair_limit": request["repair_limit"],
  "nonce": request["nonce"],
  "status": "passed",
  "real_execution": True,
  "fault_kind": request["fault_kind"],
  "fault_after_step": request["fault_after_step"],
  "action_receipt_count": 1,
  "independent_observation_count": 1,
  "human_interventions": 0,
  "artifacts": [{"path": "out/report.md", "content": "# 报告\n结论 ok\n"}],
  "files": [],
  "evidence_dir": request["evidence_dir"],
}
print(json.dumps(report))
"##,
    )
    .unwrap();

    // Windows executable registry expects a single file. Use a .cmd wrapper;
    // its bytes are hashed and it invokes python with the helper script.
    let wrapper = out.join("drivers").join("positive-driver.cmd");
    let python = std::env::var("PYTHON_EXE").unwrap_or_else(|_| "python".to_string());
    std::fs::write(
        &wrapper,
        format!(
            "@echo off\r\n\"{}\" -I \"{}\" %*\r\n",
            python,
            helper.display()
        ),
    )
    .unwrap();

    let desktop = case("desktop", EvalCategory::Desktop, None);
    let registry = ExternalDriverRegistry::new(Some(wrapper), None, Some(config)).unwrap();
    let runner = MatrixRunner::new(bundle(vec![desktop]), &out).with_external_drivers(registry);
    let report = runner
        .run(
            Arc::new(CheckerOnlyExecutor),
            "custom-real-driver",
            None,
            &opts(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.runs.len(), 1, "{:?}", report.runs);
    assert_eq!(
        report.runs[0].status,
        RunStatus::Passed,
        "{:?}",
        report.runs[0]
    );
    assert!(report.runs[0].external_evidence.is_some());
    let _ = std::fs::remove_dir_all(&out);
}
