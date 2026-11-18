//! ProductEval（V1-R1）集成测试：
//! 校验器、检查器语义、权限范围、dry 参考回放、单/多对照一致性、
//! journal 续跑与失败保留、live 生成器（Stub Provider）、报告与对照。

use async_trait::async_trait;
use owo_agent_core::gateway::{ChatMessage, ModelOutput, ModelProvider, TokenUsage};
use owo_agent_core::tools::ToolSpec;
use owo_agent_product_eval::product_eval::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// 构造工具
// ---------------------------------------------------------------------------

fn bundle(cases: Vec<ProductEvalCase>) -> SuiteBundle {
    SuiteBundle {
        dir: std::env::temp_dir(),
        suite: ProductEvalSuite {
            schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
            name: "test-suite".to_string(),
            description: String::new(),
            defaults: SuiteDefaults::default(),
            tasks: cases
                .iter()
                .map(|c| format!("tasks/{}.json", c.id))
                .collect(),
        },
        cases,
    }
}

fn make_case(id: &str, category: EvalCategory) -> ProductEvalCase {
    ProductEvalCase {
        schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
        id: id.to_string(),
        category,
        title: format!("任务 {id}"),
        instruction: "完成并输出报告。".to_string(),
        inputs: vec![InputFixture {
            path: "src/a.txt".to_string(),
            content: "alpha".to_string(),
        }],
        allow_read: vec!["src/**".to_string()],
        allow_write: vec!["out/**".to_string()],
        expected_artifacts: vec!["out/report.md".to_string()],
        checkers: vec![
            ArtifactChecker::Contains {
                path: "out/report.md".to_string(),
                text: "结论".to_string(),
            },
            ArtifactChecker::Regex {
                path: "out/report.md".to_string(),
                pattern: r"^# ".to_string(),
            },
            ArtifactChecker::LineCountMin {
                path: "out/report.md".to_string(),
                min_lines: 2,
            },
        ],
        reference_outputs: BTreeMap::from([(
            "out/report.md".to_string(),
            "# 报告\n结论 ok\n".to_string(),
        )]),
        timeout_secs: None,
        max_model_calls: None,
        repetitions: None,
        allow_commands: Vec::new(),
        failure_classes: Vec::new(),
        expected_side_effects: Vec::new(),
        fault_injection: None,
    }
}

fn make_json_case(id: &str) -> ProductEvalCase {
    ProductEvalCase {
        schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
        id: id.to_string(),
        category: EvalCategory::Document,
        title: format!("任务 {id}"),
        instruction: "抽取结构化字段。".to_string(),
        inputs: vec![InputFixture {
            path: "notes/meeting.md".to_string(),
            content: "owner: 张三".to_string(),
        }],
        allow_read: vec!["notes/**".to_string()],
        allow_write: vec!["out/**".to_string()],
        expected_artifacts: vec!["out/extract.json".to_string()],
        checkers: vec![ArtifactChecker::JsonFieldEquals {
            path: "out/extract.json".to_string(),
            field: "owner".to_string(),
            expected: serde_json::json!("张三"),
        }],
        reference_outputs: BTreeMap::from([(
            "out/extract.json".to_string(),
            "{\"owner\":\"张三\",\"count\":2}".to_string(),
        )]),
        timeout_secs: None,
        max_model_calls: None,
        repetitions: None,
        allow_commands: Vec::new(),
        failure_classes: Vec::new(),
        expected_side_effects: Vec::new(),
        fault_injection: None,
    }
}

fn dummy_run(key: MatrixKey, status: RunStatus) -> ProductEvalRun {
    ProductEvalRun {
        key,
        category: EvalCategory::Code,
        status,
        wall_ms: 10,
        executor_wall_ms: 10,
        validation_wall_ms: 0,
        delivery_gate_wall_ms: 0,
        model_calls: 0,
        tool_calls: None,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        cost_usd: None,
        failed_steps: vec![],
        retries: 0,
        cancellations: 0,
        artifact_refs: vec![],
        tool_log: vec![],
        checker_passed: 0,
        checker_total: 0,
        sandbox_rel: None,
        artifact_snapshot_rel: None,
        model: None,
        started_at: "2026-01-01T00:00:00Z".to_string(),
        finished_at: "2026-01-01T00:00:01Z".to_string(),
        error: None,
        external_evidence: None,
    }
}

fn temp_out(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("pe-tests-{tag}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// 测试用选项：固定 1 次重复（套件默认 20 次会让矩阵膨胀）。
fn opts1(modes: Vec<AgentMode>) -> RunOptions {
    RunOptions {
        modes,
        reps_override: Some(1),
        ..Default::default()
    }
}

async fn run_matrix(
    runner: &MatrixRunner,
    executor: Arc<dyn CaseExecutor>,
    execution: &str,
    opts: &RunOptions,
    cancel: Arc<AtomicBool>,
) -> ProductEvalReport {
    runner
        .run(executor, execution, None, opts, cancel)
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// 脚本化执行器：按 case id 决定成败
// ---------------------------------------------------------------------------

struct ScriptedExecutor {
    /// 这些 case 一律产出"坏"内容（检查器失败）。
    fail_cases: Vec<String>,
    /// 写完后额外 sleep（测超时用）。
    sleep_ms: u64,
}

impl ScriptedExecutor {
    fn passing() -> Arc<Self> {
        Arc::new(Self {
            fail_cases: vec![],
            sleep_ms: 0,
        })
    }
}

#[async_trait]
impl CaseExecutor for ScriptedExecutor {
    async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome {
        if self.sleep_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
        }
        let artifact = ctx.case.expected_artifacts[0].clone();
        let content = if self.fail_cases.contains(&ctx.case.id) {
            "# 报告\n（缺少关键词）\n".to_string()
        } else {
            "# 报告\n结论 ok\n".to_string()
        };
        let mut outcome = RawExecOutcome::default();
        if let Err(error) = ctx.write_file(&artifact, &content) {
            outcome.error = Some(error);
        }
        outcome
    }
}

// ---------------------------------------------------------------------------
// 队列 Provider：live 生成器的确定性替身
// ---------------------------------------------------------------------------

struct QueueProvider {
    replies: Mutex<Vec<String>>,
    calls: AtomicU32,
}

impl QueueProvider {
    fn new(replies: Vec<&str>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().map(str::to_string).collect()),
            calls: AtomicU32::new(0),
        })
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ModelProvider for QueueProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut queue = self.replies.lock().unwrap();
        if queue.is_empty() {
            return Err("队列耗尽：脚本回复不足".to_string());
        }
        Ok(ModelOutput::Text(queue.remove(0)))
    }

    fn usage_snapshot(&self) -> TokenUsage {
        TokenUsage::default()
    }
}

fn file_block(path: &str, content: &str) -> String {
    format!("=== FILE: {path} ===\n{content}\n=== END FILE ===")
}

// ---------------------------------------------------------------------------
// 1) 路径与权限范围
// ---------------------------------------------------------------------------

#[test]
fn sanitize_and_scope_semantics() {
    assert_eq!(sanitize_rel_path("a/b/c.md").unwrap(), "a/b/c.md");
    assert_eq!(sanitize_rel_path("a\\b\\c.md").unwrap(), "a/b/c.md");
    assert_eq!(sanitize_rel_path("./a//b/./").unwrap(), "a/b");
    assert!(sanitize_rel_path("/abs").is_err());
    assert!(sanitize_rel_path("a/../..").is_err());
    assert!(sanitize_rel_path("C:/x").is_err());
    assert!(sanitize_rel_path("").is_err());

    // 目录前缀语义
    assert!(scope_matches("out", "out/report.md"));
    assert!(!scope_matches("out", "out2/report.md"));
    // 段通配
    assert!(scope_matches("out/*.md", "out/report.md"));
    assert!(!scope_matches("out/*.md", "out/sub/report.md"));
    // ** 跨段
    assert!(scope_matches("src/**", "src/a/b/c.txt"));
    assert!(scope_matches("notes/??/x.md", "notes/ab/x.md"));
    assert!(!scope_matches("notes/??/x.md", "notes/abc/x.md"));
    // in_scope 聚合
    let rules = vec!["out/**".to_string(), "tmp".to_string()];
    assert!(in_scope(&rules, "out/a.md"));
    assert!(in_scope(&rules, "tmp/b.md"));
    assert!(!in_scope(&rules, "etc/passwd"));
}

// ---------------------------------------------------------------------------
// 2) 检查器语义
// ---------------------------------------------------------------------------

#[test]
fn checker_semantics_on_map_and_dir() {
    let snapshot = BTreeMap::from([
        ("a.md".to_string(), "# 标题\n结论 ok\n".to_string()),
        ("b.md".to_string(), "普通文本".to_string()),
        ("empty.md".to_string(), String::new()),
        ("c.json".to_string(), "{\"owner\":\"张三\"}".to_string()),
    ]);
    let ok_cases = vec![
        ArtifactChecker::Exists {
            path: "a.md".to_string(),
        },
        ArtifactChecker::Contains {
            path: "a.md".to_string(),
            text: "结论".to_string(),
        },
        ArtifactChecker::NotContains {
            path: "b.md".to_string(),
            text: "禁止词".to_string(),
        },
        ArtifactChecker::Regex {
            path: "a.md".to_string(),
            pattern: r"^# ".to_string(),
        },
        // multi_line 语义：^ 按行锚定，第 2 行行首也应命中。
        ArtifactChecker::Regex {
            path: "a.md".to_string(),
            pattern: "^结论".to_string(),
        },
        ArtifactChecker::LineCountMin {
            path: "a.md".to_string(),
            min_lines: 2,
        },
        ArtifactChecker::JsonFieldEquals {
            path: "c.json".to_string(),
            field: "owner".to_string(),
            expected: serde_json::json!("张三"),
        },
    ];
    for checker in &ok_cases {
        assert!(
            evaluate_checker_on_map(checker, &snapshot).is_ok(),
            "应当通过：{}",
            checker.describe()
        );
    }
    let fail_cases = vec![
        ArtifactChecker::Exists {
            path: "missing.md".to_string(),
        },
        ArtifactChecker::Exists {
            path: "empty.md".to_string(),
        },
        ArtifactChecker::Contains {
            path: "b.md".to_string(),
            text: "结论".to_string(),
        },
        ArtifactChecker::NotContains {
            path: "a.md".to_string(),
            text: "结论".to_string(),
        },
        ArtifactChecker::LineCountMin {
            path: "b.md".to_string(),
            min_lines: 5,
        },
        ArtifactChecker::JsonFieldEquals {
            path: "c.json".to_string(),
            field: "owner".to_string(),
            expected: serde_json::json!("李四"),
        },
    ];
    for checker in &fail_cases {
        assert!(
            evaluate_checker_on_map(checker, &snapshot).is_err(),
            "应当失败：{}",
            checker.describe()
        );
    }
    // 磁盘来源
    let dir = temp_out("checker-dir");
    std::fs::write(dir.join("d.md"), "# t\n结论\n").unwrap();
    assert!(evaluate_checker_on_dir(
        &ArtifactChecker::Contains {
            path: "d.md".to_string(),
            text: "结论".to_string()
        },
        &dir
    )
    .is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3) validate：合法与各类违规
// ---------------------------------------------------------------------------

#[test]
fn validate_accepts_well_formed_cases() {
    let b = bundle(vec![
        make_case("good-code", EvalCategory::Code),
        make_json_case("good-doc"),
    ]);
    let validation = validate_suite(&b);
    assert!(validation.all_ok, "issues={:?}", validation.tasks);
    assert_eq!(validation.ok_count, 2);
}

#[test]
fn validate_rejects_missing_reference_and_scope_and_checker_drift() {
    let mut missing_ref = make_case("missing-ref", EvalCategory::Code);
    missing_ref.reference_outputs.clear();

    let mut out_of_scope = make_case("out-of-scope", EvalCategory::Code);
    out_of_scope.expected_artifacts = vec!["etc/leak.md".to_string()];
    out_of_scope
        .reference_outputs
        .insert("etc/leak.md".to_string(), "# 报告\n结论 ok\n".to_string());

    let mut checker_drift = make_case("checker-drift", EvalCategory::Code);
    checker_drift.checkers.push(ArtifactChecker::Exists {
        path: "elsewhere.md".to_string(),
    });

    let mut bad_regex = make_case("bad-regex", EvalCategory::Code);
    bad_regex.checkers.push(ArtifactChecker::Regex {
        path: "out/report.md".to_string(),
        pattern: "([".to_string(),
    });

    let mut ref_fails = make_case("ref-fails", EvalCategory::Code);
    ref_fails
        .reference_outputs
        .insert("out/report.md".to_string(), "没有关键词的内容".to_string());

    let b = bundle(vec![
        missing_ref,
        out_of_scope,
        checker_drift,
        bad_regex,
        ref_fails,
    ]);
    let validation = validate_suite(&b);
    assert!(!validation.all_ok);
    let joined = validation
        .tasks
        .iter()
        .flat_map(|t| t.issues.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("reference_outputs 缺少"), "{joined}");
    assert!(joined.contains("allow_write"), "{joined}");
    assert!(joined.contains("不在 expected_artifacts"), "{joined}");
    assert!(joined.contains("正则无效"), "{joined}");
    assert!(joined.contains("参考输出未通过检查器"), "{joined}");
}

#[test]
fn validate_rejects_duplicate_ids() {
    let b = bundle(vec![
        make_case("dup", EvalCategory::Code),
        make_case("dup", EvalCategory::Research),
    ]);
    let validation = validate_suite(&b);
    assert!(!validation.all_ok);
    assert!(format_validation(&validation).contains("重复"));
}

// ---------------------------------------------------------------------------
// 4) dry 参考回放 + 续跑不重复
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dry_run_reference_passes_and_resume_skips_completed() {
    let b = bundle(vec![make_case("dry-a", EvalCategory::Code)]);
    let out = temp_out("dry-resume");
    let runner = MatrixRunner::new(b.clone(), &out);
    let opts = opts1(vec![AgentMode::Single]);
    let report = run_matrix(
        &runner,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        no_cancel(),
    )
    .await;
    assert_eq!(report.runs.len(), 1);
    assert_eq!(report.runs[0].status, RunStatus::Passed);
    assert_eq!(report.runs[0].model_calls, 0);
    assert_eq!(report.runs[0].artifact_refs, vec!["out/report.md"]);
    let snapshot_rel = report.runs[0]
        .artifact_snapshot_rel
        .as_ref()
        .expect("成功运行应保留登记产物快照");
    let snapshot = out.join(snapshot_rel);
    assert_eq!(
        std::fs::read_to_string(snapshot.join("out/report.md")).unwrap(),
        b.clone().cases[0].reference_outputs["out/report.md"]
    );
    assert!(snapshot.join("manifest.json").is_file());
    assert!(format_report_summary(&report).contains(snapshot_rel));
    assert!(
        report.runs[0].sandbox_rel.is_none(),
        "成功快照不保留整个临时沙盒"
    );
    let mut legacy_run = serde_json::to_value(&report.runs[0]).unwrap();
    legacy_run
        .as_object_mut()
        .unwrap()
        .remove("artifact_snapshot_rel");
    let legacy_run: ProductEvalRun = serde_json::from_value(legacy_run).unwrap();
    assert!(legacy_run.artifact_snapshot_rel.is_none());
    assert!(report.pending.is_empty());
    assert!(report.metrics.success_rate == 1.0);

    // 重跑同一 out：已完成 case 不重复执行。
    let again = run_matrix(
        &runner,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        no_cancel(),
    )
    .await;
    assert_eq!(again.runs.len(), 1, "续跑不得重复已完成 case");
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn resume_rejects_model_execution_and_batch_label_drift() {
    let b = bundle(vec![make_case("run-identity", EvalCategory::Code)]);
    let out = temp_out("run-identity-contract");
    let opts = RunOptions {
        batch_label: Some("batch-a".into()),
        ..opts1(vec![AgentMode::Single])
    };
    let runner = MatrixRunner::new(b, &out);
    let _ = runner
        .run(
            Arc::new(ReferenceDryExecutor),
            "live-agent",
            Some("model-a".into()),
            &opts,
            no_cancel(),
        )
        .await
        .unwrap();

    let variants = [
        ("live-agent", Some("model-b"), opts.clone(), "模型"),
        ("dry-reference", Some("model-a"), opts.clone(), "执行器"),
        (
            "live-agent",
            Some("model-a"),
            RunOptions {
                batch_label: None,
                ..opts.clone()
            },
            "批次标签",
        ),
    ];
    for (execution, model, changed_opts, reason) in variants {
        let error = runner
            .run(
                Arc::new(ReferenceDryExecutor),
                execution,
                model.map(str::to_string),
                &changed_opts,
                no_cancel(),
            )
            .await
            .expect_err("运行身份变化必须拒绝续跑");
        assert!(error.0.contains(reason), "{}", error.0);
    }
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn resume_rejects_changed_effective_budget_even_when_suite_hash_is_unchanged() {
    let b = bundle(vec![make_case("budget-drift", EvalCategory::Code)]);
    let out = temp_out("budget-contract");
    let opts = opts1(vec![AgentMode::Single]);
    let first_runner = MatrixRunner::new(b.clone(), &out);
    let first = run_matrix(
        &first_runner,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        no_cancel(),
    )
    .await;
    assert_eq!(first.runs.len(), 1);

    let mut changed_bundle = b;
    changed_bundle.suite.defaults.max_model_calls = changed_bundle
        .suite
        .defaults
        .max_model_calls
        .saturating_add(1);
    assert_eq!(
        suite_hash(&first_runner.bundle),
        suite_hash(&changed_bundle),
        "本回归需要覆盖 suite_hash 未包含套件默认预算的场景"
    );
    let changed_runner = MatrixRunner::new(changed_bundle, &out);
    let error = changed_runner
        .run(
            Arc::new(ReferenceDryExecutor),
            "dry-reference",
            None,
            &opts,
            no_cancel(),
        )
        .await
        .expect_err("预算变化必须阻止续跑混合");
    assert!(error.0.contains("指纹"), "{}", error.0);
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn dry_run_single_multi_parity() {
    let b = bundle(vec![make_case("parity-a", EvalCategory::Research)]);
    let out = temp_out("parity");
    let runner = MatrixRunner::new(b, &out);
    let opts = RunOptions {
        reps_override: Some(2),
        ..opts1(AgentMode::all().to_vec())
    };
    let report = run_matrix(
        &runner,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        no_cancel(),
    )
    .await;
    assert_eq!(report.runs.len(), 4);
    assert!(report.runs.iter().all(|r| r.status == RunStatus::Passed));
    let modes: Vec<AgentMode> = report.runs.iter().map(|run| run.key.agent_mode).collect();
    assert_eq!(modes.len(), 4);
    assert_ne!(
        modes[0], modes[1],
        "each paired repetition must contain both topologies"
    );
    assert_eq!(
        modes[0..2]
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>(),
        modes[2..4]
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>(),
        "the partner repetition must reverse the same topology pair"
    );
    assert_eq!(modes[0], modes[3]);
    assert_eq!(modes[1], modes[2]);
    let aggregate_modes: Vec<AgentMode> = report.per_case.iter().map(|r| r.agent_mode).collect();
    assert_eq!(aggregate_modes, vec![AgentMode::Single, AgentMode::Multi]);
    assert!(report.per_case.iter().all(|r| r.success_rate == 1.0));
    let (single, multi) = split_paired_mode_reports(&report).expect("split paired report");
    assert_eq!(single.execution, "live-agent");
    assert_eq!(multi.execution, "live-workswarm");
    assert_eq!(single.runs.len(), 2);
    assert_eq!(multi.runs.len(), 2);
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn matrix_runner_rejects_empty_and_duplicate_modes() {
    let b = bundle(vec![make_case("invalid-modes", EvalCategory::Code)]);
    let out = temp_out("invalid-modes");
    let runner = MatrixRunner::new(b, &out);

    let empty = RunOptions {
        modes: Vec::new(),
        ..RunOptions::default()
    };
    let error = runner
        .run(
            Arc::new(ReferenceDryExecutor),
            "dry-reference",
            None,
            &empty,
            no_cancel(),
        )
        .await
        .expect_err("empty topology list must be rejected");
    assert!(error.0.contains("不能为空"));

    let duplicate = RunOptions {
        modes: vec![AgentMode::Single, AgentMode::Single],
        ..RunOptions::default()
    };
    let error = runner
        .run(
            Arc::new(ReferenceDryExecutor),
            "dry-reference",
            None,
            &duplicate,
            no_cancel(),
        )
        .await
        .expect_err("duplicate topology list must be rejected");
    assert!(error.0.contains("重复"));
    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------------
// 5) 失败保留 + 续跑 + fresh 重置
// ---------------------------------------------------------------------------

#[tokio::test]
async fn failed_runs_retained_and_never_rerun() {
    let b = bundle(vec![
        make_case("keep-a", EvalCategory::Code),
        make_case("keep-b", EvalCategory::Document),
    ]);
    let out = temp_out("retain");
    let runner = MatrixRunner::new(b, &out);
    let opts = opts1(AgentMode::all().to_vec());

    let failing = Arc::new(ScriptedExecutor {
        fail_cases: vec!["keep-b".to_string()],
        sleep_ms: 0,
    });
    let first = run_matrix(&runner, failing, "dry-reference", &opts, no_cancel()).await;
    assert_eq!(first.runs.len(), 4); // 2 case × 2 mode
    assert_eq!(first.metrics.failed, 2); // keep-b × single/multi
    assert!(first.metrics.success_rate < 1.0);

    // 换"全过"执行器续跑：失败记录仍在、不重算、不重跑。
    let second = run_matrix(
        &runner,
        ScriptedExecutor::passing(),
        "dry-reference",
        &opts,
        no_cancel(),
    )
    .await;
    assert_eq!(second.runs.len(), 4, "续跑不得产生新运行");
    assert_eq!(second.metrics.failed, 2, "失败运行必须保留在报告中");
    assert!(second.runs.iter().any(|r| r.status == RunStatus::Failed));
    assert!(second.pending.is_empty());

    // fresh 是唯一允许归零重跑的入口（沿用同样的 reps=1 覆盖）。
    let fresh_opts = RunOptions {
        fresh: true,
        ..opts.clone()
    };
    let third = run_matrix(
        &runner,
        ScriptedExecutor::passing(),
        "dry-reference",
        &fresh_opts,
        no_cancel(),
    )
    .await;
    assert_eq!(third.runs.len(), 4);
    assert_eq!(third.metrics.passed, 4);
    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------------
// 6) 取消：未完成单元格进入 pending，已完成记录保留
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_leaves_pending_and_keeps_journal() {
    let b = bundle(vec![make_case("cancel-a", EvalCategory::Code)]);
    let out = temp_out("cancel");
    let runner = MatrixRunner::new(b, &out);
    let opts = opts1(vec![AgentMode::Single]);
    let report = run_matrix(
        &runner,
        ScriptedExecutor::passing(),
        "dry-reference",
        &opts,
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    assert_eq!(report.runs.len(), 0, "取消后不得有新运行");
    assert_eq!(report.pending.len(), 1, "未完成单元格应出现在 pending");
    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------------
// 7) journal 损坏的容错语义：尾行撕裂容忍、中部损坏拒绝
// ---------------------------------------------------------------------------

#[tokio::test]
async fn torn_journal_tail_is_tolerated_mid_corruption_is_fatal() {
    let b = bundle(vec![
        make_case("jrnl-a", EvalCategory::Code),
        make_case("jrnl-b", EvalCategory::Code),
    ]);
    let opts = opts1(vec![AgentMode::Single]);

    // 尾行撕裂：先由真实 runner 写入有效元数据，再注入中断 journal。
    let out = temp_out("torn-tail");
    let runner = MatrixRunner::new(b.clone(), &out);
    let _ = run_matrix(
        &runner,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    let a_line = serde_json::to_string(&dummy_run(
        MatrixKey::new("jrnl-a", AgentMode::Single, 0),
        RunStatus::Passed,
    ))
    .unwrap();
    let mut torn_journal = format!("{a_line}\n{{\"partial\":\"").into_bytes();
    torn_journal.extend_from_slice(&[0xE4, 0xB8]); // 截断在 UTF-8 多字节字符中间。
    std::fs::write(out.join("state.jsonl"), torn_journal).unwrap();
    let report = run_matrix(
        &runner,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        no_cancel(),
    )
    .await;
    assert_eq!(report.runs.len(), 2, "撕裂尾 + 补跑 1 格 = 2");
    assert_eq!(report.metrics.passed, 2);
    let resumed = run_matrix(
        &runner,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        no_cancel(),
    )
    .await;
    assert_eq!(resumed.runs.len(), 2, "修复后的 journal 必须可再次恢复");
    assert_eq!(resumed.metrics.passed, 2);

    // 中部损坏：拒绝静默丢弃，明确失败。
    let out2 = temp_out("mid-corrupt");
    let runner2 = MatrixRunner::new(b.clone(), &out2);
    let _ = run_matrix(
        &runner2,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    let b_line = serde_json::to_string(&dummy_run(
        MatrixKey::new("jrnl-b", AgentMode::Single, 0),
        RunStatus::Passed,
    ))
    .unwrap();
    std::fs::write(
        out2.join("state.jsonl"),
        format!("{a_line}\n{{broken-middle}}\n{b_line}\n"),
    )
    .unwrap();
    let result = runner2
        .run(
            Arc::new(ReferenceDryExecutor),
            "dry-reference",
            None,
            &opts,
            no_cancel(),
        )
        .await;
    let message = result.expect_err("中部损坏必须报错").0;
    assert!(message.contains("损坏"), "{message}");

    // 损坏但换行完整的末行不是撕裂写入，不能静默截断。
    let out3 = temp_out("complete-corrupt-tail");
    let runner3 = MatrixRunner::new(b, &out3);
    let _ = run_matrix(
        &runner3,
        Arc::new(ReferenceDryExecutor),
        "dry-reference",
        &opts,
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    std::fs::write(
        out3.join("state.jsonl"),
        format!("{a_line}\n{{broken-tail}}\n"),
    )
    .unwrap();
    let result = runner3
        .run(
            Arc::new(ReferenceDryExecutor),
            "dry-reference",
            None,
            &opts,
            no_cancel(),
        )
        .await;
    assert!(result
        .expect_err("换行完整的损坏末行必须报错")
        .0
        .contains("损坏"));

    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(&out2);
    let _ = std::fs::remove_dir_all(&out3);
}

// ---------------------------------------------------------------------------
// 8) live 生成器（Stub Provider）：single / multi / 返工 / 越权写入
// ---------------------------------------------------------------------------

fn live_case(id: &str) -> ProductEvalCase {
    let mut case = make_case(id, EvalCategory::Code);
    case.max_model_calls = Some(8);
    case
}

#[tokio::test]
async fn live_single_mode_generates_and_passes() {
    let b = bundle(vec![live_case("live-s")]);
    let out = temp_out("live-single");
    let runner = MatrixRunner::new(b, &out);
    let provider = QueueProvider::new(vec![&file_block(
        "out/report.md",
        "# 报告\n结论 generated\n",
    )]);
    let executor: Arc<dyn CaseExecutor> = Arc::new(GenerativeExecutor {
        provider: provider.clone(),
        model: "stub-model".to_string(),
    });
    let opts = opts1(vec![AgentMode::Single]);
    let report = run_matrix(&runner, executor, "live-stub", &opts, no_cancel()).await;
    assert_eq!(report.runs.len(), 1);
    assert_eq!(
        report.runs[0].status,
        RunStatus::Passed,
        "{:?}",
        report.runs[0]
    );
    assert_eq!(report.runs[0].model_calls, 1);
    assert_eq!(provider.calls(), 1);
    assert_eq!(report.runs[0].total_tokens, None, "Stub 不报用量 → null");
    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn live_generator_rejects_cases_that_need_external_execution() {
    let mut desktop = live_case("external-desktop");
    desktop.category = EvalCategory::Desktop;
    let mut long_horizon = live_case("external-long-horizon");
    long_horizon.category = EvalCategory::LongHorizon;
    let out = temp_out("live-external-execution");
    let runner = MatrixRunner::new(bundle(vec![desktop, long_horizon]), &out);
    let provider = QueueProvider::new(vec![]);
    let executor = Arc::new(GenerativeExecutor {
        provider: provider.clone(),
        model: "stub-model".to_string(),
    });
    let opts = opts1(vec![AgentMode::Single]);
    let report = run_matrix(&runner, executor, "live-stub", &opts, no_cancel()).await;

    assert_eq!(provider.calls(), 0);
    assert_eq!(report.runs.len(), 2);
    for run in &report.runs {
        let code = match run.category {
            EvalCategory::Desktop => "EVAL_REAL_DESKTOP_DRIVER_REQUIRED",
            EvalCategory::LongHorizon => "EVAL_FAULT_INJECTION_DRIVER_REQUIRED",
            other => panic!("unexpected category: {other:?}"),
        };
        assert_eq!(run.status, RunStatus::Error);
        assert_eq!(run.error.as_deref(), Some(code));
        assert_eq!(run.tool_calls, Some(0));
        assert!(run.artifact_refs.is_empty());
    }
}

#[tokio::test]
async fn live_multi_mode_planner_reviewer_and_rework() {
    let b = bundle(vec![live_case("live-m")]);
    let out = temp_out("live-multi");
    let runner = MatrixRunner::new(b, &out);

    // APPROVED 路径：plan → draft → approved = 3 次调用。
    let provider = QueueProvider::new(vec![
        "1. 读输入 2. 写报告",
        &file_block("out/report.md", "# 报告\n结论 draft\n"),
        "APPROVED",
    ]);
    let executor: Arc<dyn CaseExecutor> = Arc::new(GenerativeExecutor {
        provider: provider.clone(),
        model: "stub-model".to_string(),
    });
    let opts = opts1(vec![AgentMode::Multi]);
    let report = run_matrix(&runner, executor, "live-stub", &opts, no_cancel()).await;
    assert_eq!(report.runs[0].status, RunStatus::Passed);
    assert_eq!(report.runs[0].model_calls, 3);
    assert_eq!(provider.calls(), 3);
    assert_eq!(report.runs[0].retries, 0);
    let _ = std::fs::remove_dir_all(&out);

    // REVISE 路径：草稿不合规 → 返工后通过 = 4 次调用、retries=1。
    let out2 = temp_out("live-multi-revise");
    let runner2 = MatrixRunner::new(bundle(vec![live_case("live-m2")]), &out2);
    let provider2 = QueueProvider::new(vec![
        "plan",
        &file_block("out/report.md", "# 报告\n（缺关键词）\n"),
        "REVISE: 报告缺少结论关键词",
        &file_block("out/report.md", "# 报告\n结论 revised-ok\n"),
    ]);
    let executor2: Arc<dyn CaseExecutor> = Arc::new(GenerativeExecutor {
        provider: provider2.clone(),
        model: "stub-model".to_string(),
    });
    let report2 = run_matrix(&runner2, executor2, "live-stub", &opts, no_cancel()).await;
    assert_eq!(
        report2.runs[0].status,
        RunStatus::Passed,
        "{:?}",
        report2.runs[0]
    );
    assert_eq!(report2.runs[0].model_calls, 4);
    assert_eq!(report2.runs[0].retries, 1);
    let _ = std::fs::remove_dir_all(&out2);
}

#[tokio::test]
async fn live_scope_violation_fails_run() {
    let b = bundle(vec![live_case("live-scope")]);
    let out = temp_out("live-scope");
    let runner = MatrixRunner::new(b, &out);
    let provider = QueueProvider::new(vec![&file_block("outside/evil.md", "# 越权写入\n")]);
    let executor: Arc<dyn CaseExecutor> = Arc::new(GenerativeExecutor {
        provider,
        model: "stub-model".to_string(),
    });
    let opts = opts1(vec![AgentMode::Single]);
    let report = run_matrix(&runner, executor, "live-stub", &opts, no_cancel()).await;
    assert_eq!(report.runs[0].status, RunStatus::Error);
    let run = &report.runs[0];
    assert!(
        run.failed_steps
            .iter()
            .any(|s| s.contains("write_denied:outside/evil.md")),
        "{:?}",
        run.failed_steps
    );
    assert!(run.error.as_deref().unwrap_or("").contains("allow_write"));
    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------------
// 9) 超时 → Timeout 且保留现场
// ---------------------------------------------------------------------------

#[tokio::test]
async fn slow_executor_times_out_and_is_recorded() {
    let mut case = make_case("slow-a", EvalCategory::Code);
    case.timeout_secs = Some(1);
    let b = bundle(vec![case]);
    let out = temp_out("timeout");
    let runner = MatrixRunner::new(b, &out);
    let executor = Arc::new(ScriptedExecutor {
        fail_cases: vec![],
        sleep_ms: 1500,
    });
    let opts = opts1(vec![AgentMode::Single]);
    let report = run_matrix(&runner, executor, "dry-reference", &opts, no_cancel()).await;
    assert_eq!(report.runs[0].status, RunStatus::Timeout);
    assert!(report.runs[0].wall_ms >= 900);
    assert!(report.runs[0]
        .failed_steps
        .iter()
        .any(|s| s.starts_with("timeout:")));
    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------------
// 10) 指标聚合与 compare
// ---------------------------------------------------------------------------

#[test]
fn metrics_aggregation_counts_every_attempt() {
    let runs = vec![
        dummy_run(MatrixKey::new("a", AgentMode::Single, 0), RunStatus::Passed),
        dummy_run(MatrixKey::new("a", AgentMode::Single, 1), RunStatus::Failed),
        dummy_run(MatrixKey::new("a", AgentMode::Single, 2), RunStatus::Error),
        dummy_run(
            MatrixKey::new("a", AgentMode::Single, 3),
            RunStatus::Timeout,
        ),
        dummy_run(
            MatrixKey::new("a", AgentMode::Single, 4),
            RunStatus::Cancelled,
        ),
    ];
    let m = aggregate_metrics(&runs);
    assert_eq!(m.runs_total, 5);
    assert_eq!(m.passed, 1);
    assert_eq!(m.failed, 1);
    assert_eq!(m.errors, 1);
    assert_eq!(m.timeouts, 1);
    assert_eq!(m.cancelled, 1);
    assert!((m.success_rate - 0.2).abs() < 1e-9, "分母必须含全部尝试");
    let per_case = aggregate_per_case(&runs);
    assert_eq!(per_case.len(), 1);
    assert_eq!(per_case[0].runs_total, 5);
}

#[test]
fn compare_reports_flags_regressions_and_mismatched_execution() {
    let report_a = ProductEvalReport {
        schema_version: 1,
        suite_name: "s".into(),
        suite_hash: "h1".into(),
        execution: "dry-reference".into(),
        model: None,
        batch_label: None,
        tags: vec![],
        repair_attempts: 0,
        run_contract_sha256: None,
        evaluator_binary_sha256: None,
        provider_endpoint_sha256: None,
        generated_at: "t1".into(),
        runs: vec![],
        pending: vec![],
        metrics: ProductEvalMetrics::default(),
        per_case: vec![CaseModeMetrics {
            case_id: "x".into(),
            category: EvalCategory::Code,
            agent_mode: AgentMode::Single,
            runs_total: 4,
            passed: 4,
            success_rate: 1.0,
            mean_wall_ms: 10.0,
            mean_model_calls: 1.0,
            mean_tool_calls: None,
            total_tokens: None,
            quality: None,
        }],
        comparison: None,
    };
    let mut report_b = report_a.clone();
    report_b.execution = "dry-reference".into();
    report_b.per_case[0].passed = 2;
    report_b.per_case[0].runs_total = 4;
    report_b.per_case[0].success_rate = 0.5;
    let text = compare_reports(&report_a, &report_b, false);
    assert!(text.contains("回归 1 组"), "{text}");
    assert!(text.contains('↓'), "{text}");
    assert!(!text.contains("execution 不同"), "{text}");

    report_b.execution = "live-generative".into();
    let text2 = compare_reports(&report_a, &report_b, false);
    assert!(text2.contains("execution 不同"), "{text2}");
}

// ---------------------------------------------------------------------------
// 11) 文件块解析
// ---------------------------------------------------------------------------

#[test]
fn parse_file_blocks_handles_fences_and_reports_residuals() {
    // 围栏包裹的完整块 + 未闭合块被后续 FILE 覆盖（残块告警）。
    let text = format!(
        "```\n{}\n```\n=== FILE: c.md ===\n残块内容（未闭合）\n{}",
        file_block("a.md", "内容甲"),
        file_block("b.md", "内容乙")
    );
    let (blocks, warnings) = parse_file_blocks(&text);
    assert_eq!(blocks.len(), 2, "{blocks:?}");
    let a = blocks.iter().find(|(p, _)| p == "a.md").unwrap();
    assert_eq!(a.1.trim(), "内容甲");
    let b = blocks.iter().find(|(p, _)| p == "b.md").unwrap();
    assert_eq!(b.1.trim(), "内容乙");
    assert!(!blocks.iter().any(|(p, _)| p == "c.md"), "残块必须被丢弃");
    assert!(warnings.iter().any(|w| w.contains("嵌套")), "{warnings:?}");
}

// ---------------------------------------------------------------------------
// 12) 十期二路新增能力：json_keys_exact / command_check / 质量分 / 配对报告 / freeze
// ---------------------------------------------------------------------------

#[test]
fn json_keys_exact_enforces_exact_top_level_schema() {
    let checker = ArtifactChecker::JsonKeysExact {
        path: "out/data.json".to_string(),
        keys: vec!["owner".into(), "deadline".into(), "budget".into()],
    };
    let good = BTreeMap::from([(
        "out/data.json".to_string(),
        r#"{"owner":"张三","deadline":"2026-09-05","budget":42000}"#.to_string(),
    )]);
    assert!(evaluate_checker_on_map(&checker, &good).is_ok());
    // 多余字段拒绝。
    let extra = BTreeMap::from([(
        "out/data.json".to_string(),
        r#"{"owner":"张三","deadline":"2026-09-05","budget":1,"note":"多出"}"#.to_string(),
    )]);
    let err = evaluate_checker_on_map(&checker, &extra).expect_err("多余字段必须失败");
    assert!(err.contains("多余"), "{err}");
    // 缺失字段拒绝。
    let missing = BTreeMap::from([(
        "out/data.json".to_string(),
        r#"{"owner":"张三","budget":1}"#.to_string(),
    )]);
    let err = evaluate_checker_on_map(&checker, &missing).expect_err("缺失字段必须失败");
    assert!(err.contains("缺失"), "{err}");
    // 非 JSON 对象拒绝。
    let not_object = BTreeMap::from([("out/data.json".to_string(), "[1,2,3]".to_string())]);
    assert!(evaluate_checker_on_map(&checker, &not_object).is_err());
    // 静态校验：空 keys 报问题。
    let empty_keys = ArtifactChecker::JsonKeysExact {
        path: "out/data.json".to_string(),
        keys: vec![],
    };
    assert!(empty_keys
        .static_issues()
        .iter()
        .any(|issue| issue.contains("keys 为空")));
}

#[tokio::test]
async fn command_check_runs_in_sandbox_and_skips_on_memory_snapshot() {
    let sandbox = temp_out("cmdcheck");
    std::fs::write(sandbox.join("out.txt"), "report-v1: PASS\nline2\n").unwrap();
    // 命令必须是单条简单调用（无内嵌引号，避免 cmd /C 二次解析）。
    std::fs::write(
        sandbox.join("ping.py"),
        "print('CHECK-OK')\nprint('OTHER')\n",
    )
    .unwrap();
    std::fs::write(sandbox.join("fail.py"), "import sys\nsys.exit(3)\n").unwrap();
    let checker = ArtifactChecker::CommandCheck {
        command: "python ping.py".to_string(),
        cwd: None,
        expect_exit: Some(0),
        stdout_contains: vec!["CHECK-OK".to_string()],
        stdout_not_contains: vec!["FAIL".to_string()],
        timeout_secs: 30,
    };
    // 内存快照上静默跳过（不执行，不失败）。
    let snapshot = BTreeMap::new();
    assert!(evaluate_checker_on_map(&checker, &snapshot).is_ok());
    // 真实沙盒上执行并断言 stdout。
    assert!(evaluate_command_check(&checker, &sandbox).await.is_ok());
    // 期望片段缺失 → 失败。
    let bad = ArtifactChecker::CommandCheck {
        command: "python ping.py".to_string(),
        cwd: None,
        expect_exit: Some(0),
        stdout_contains: vec!["NEVER-PRINTED".to_string()],
        stdout_not_contains: vec![],
        timeout_secs: 30,
    };
    let err = evaluate_command_check(&bad, &sandbox)
        .await
        .expect_err("stdout 缺少期望片段必须失败");
    assert!(err.contains("缺少片段"), "{err}");
    // 退出码不符 → 失败。
    let exit_bad = ArtifactChecker::CommandCheck {
        command: "python fail.py".to_string(),
        cwd: None,
        expect_exit: Some(0),
        stdout_contains: vec![],
        stdout_not_contains: vec![],
        timeout_secs: 30,
    };
    let err = evaluate_command_check(&exit_bad, &sandbox)
        .await
        .expect_err("退出码不符必须失败");
    assert!(err.contains("退出码"), "{err}");
    // 静态校验：shell 链式元字符 / cwd 越界拒绝。
    let meta = ArtifactChecker::CommandCheck {
        command: "python a.py; rm -rf /".to_string(),
        cwd: None,
        expect_exit: None,
        stdout_contains: vec![],
        stdout_not_contains: vec![],
        timeout_secs: 30,
    };
    assert!(meta
        .static_issues()
        .iter()
        .any(|issue| issue.contains("元字符")));
    let _ = std::fs::remove_dir_all(&sandbox);
}

#[test]
fn checker_quality_metrics_and_mode_statistics() {
    let mut passed = dummy_run(
        MatrixKey::new(String::from("c"), AgentMode::Single, 0),
        RunStatus::Passed,
    );
    passed.checker_passed = 5;
    passed.checker_total = 6;
    let mut partial = dummy_run(
        MatrixKey::new(String::from("c"), AgentMode::Single, 1),
        RunStatus::Failed,
    );
    partial.checker_passed = 3;
    partial.checker_total = 6;
    let mut no_counts = dummy_run(
        MatrixKey::new(String::from("c"), AgentMode::Single, 2),
        RunStatus::Passed,
    );
    no_counts.checker_total = 0;
    assert_eq!(quality_of(&[passed.clone()]), Some(5.0 / 6.0));
    assert_eq!(
        quality_of(&[passed, partial]),
        Some((5.0 / 6.0 + 3.0 / 6.0) / 2.0)
    );
    assert_eq!(quality_of(&[no_counts]), None);
    // CaseModeMetrics 与模式统计带 quality。
    let runs = vec![
        dummy_run(
            MatrixKey::new(String::from("c"), AgentMode::Single, 0),
            RunStatus::Passed,
        ),
        dummy_run(
            MatrixKey::new(String::from("c"), AgentMode::Multi, 0),
            RunStatus::Passed,
        ),
    ];
    let per = aggregate_per_case(&runs);
    assert_eq!(per.len(), 2);
}

#[test]
fn paired_report_json_matches_route3_paired_stats_contract() {
    let mut a = dummy_run(
        MatrixKey::new(String::from("code-one"), AgentMode::Single, 0),
        RunStatus::Passed,
    );
    a.model = Some("glm-5.3-flash".into());
    a.model_calls = 2;
    a.total_tokens = Some(100);
    a.cost_usd = Some(0.10);
    a.checker_passed = 3;
    a.checker_total = 4;
    let mut b = dummy_run(
        MatrixKey::new(String::from("code-one"), AgentMode::Multi, 0),
        RunStatus::Passed,
    );
    b.model = Some("glm-5.3-flash".into());
    b.wall_ms = 25;
    b.model_calls = 4;
    b.total_tokens = Some(250);
    b.cost_usd = Some(0.16);
    b.checker_passed = 1;
    b.checker_total = 4;
    let single = ProductEvalReport {
        schema_version: 1,
        suite_name: "s".into(),
        suite_hash: "h".into(),
        execution: "live-agent".into(),
        model: Some("glm-5.3-flash".into()),
        batch_label: Some("b1".into()),
        tags: vec!["tag1".into()],
        repair_attempts: 0,
        run_contract_sha256: Some("contract-v1".into()),
        evaluator_binary_sha256: Some("binary-v1".into()),
        provider_endpoint_sha256: Some("endpoint-v1".into()),
        generated_at: "t1".into(),
        runs: vec![a.clone()],
        pending: vec![],
        metrics: aggregate_metrics(std::slice::from_ref(&a)),
        per_case: aggregate_per_case(std::slice::from_ref(&a)),
        comparison: None,
    };
    let multi = ProductEvalReport {
        schema_version: 1,
        suite_name: "s".into(),
        suite_hash: "h".into(),
        execution: "live-workswarm".into(),
        model: Some("glm-5.3-flash".into()),
        batch_label: Some("b1".into()),
        tags: vec!["tag1".into()],
        repair_attempts: 0,
        run_contract_sha256: Some("contract-v1".into()),
        evaluator_binary_sha256: Some("binary-v1".into()),
        provider_endpoint_sha256: Some("endpoint-v1".into()),
        generated_at: "t2".into(),
        runs: vec![b.clone()],
        pending: vec![],
        metrics: aggregate_metrics(std::slice::from_ref(&b)),
        per_case: aggregate_per_case(std::slice::from_ref(&b)),
        comparison: None,
    };
    let opts = PairedReportOptions {
        model: Some("glm-5.3-flash".into()),
        template: Some("default".into()),
        task_set: Some("v1-r1-product-suite".into()),
        strategy_version: "ten-5-independent-case-samples".into(),
    };
    let paired = build_paired_report_json(&single, &multi, &opts, Some("t3"));
    assert_eq!(
        paired
            .get("run_alignment")
            .and_then(|value| value.get("configuration_aligned")),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(
        paired
            .get("run_alignment")
            .and_then(|value| value.get("paired_cells")),
        Some(&serde_json::json!(1))
    );
    let overall = paired["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["task_group"] == "overall")
        .unwrap();
    let matched = &overall["matched_comparison"];
    assert_eq!(matched["valid_complete_pairing"], true);
    assert_eq!(matched["both_pass"], 1);
    assert_eq!(matched["net_success_rate_delta"], 0.0);
    assert_eq!(
        matched["uncertainty"]["success_rate_delta_case_cluster_bootstrap_95_ci"]["available"],
        false
    );
    assert_eq!(
        matched["uncertainty"]["success_rate_delta_case_cluster_bootstrap_95_ci"]["reason"],
        "requires_at_least_3_independent_cases"
    );
    assert_eq!(matched["team_minus_single_wall_ms_all"]["mean"], 15.0);
    assert_eq!(matched["team_minus_single_model_calls"]["mean"], 2.0);
    assert_eq!(matched["team_minus_single_checker_quality"]["mean"], -0.5);
    assert_eq!(matched["team_minus_single_total_tokens"]["mean"], 150.0);
    assert!(
        (matched["team_minus_single_cost_usd"]["mean"]
            .as_f64()
            .unwrap()
            - 0.06)
            .abs()
            < 1e-9
    );
    assert_eq!(matched["cell_deltas"].as_array().unwrap().len(), 1);

    let mut unmatched_multi = multi.clone();
    unmatched_multi.runs[0].key.repetition = 1;
    let unmatched = build_paired_report_json(&single, &unmatched_multi, &opts, Some("t4"));
    assert_eq!(
        unmatched
            .get("run_alignment")
            .and_then(|value| value.get("configuration_aligned")),
        Some(&serde_json::Value::Bool(false))
    );
    let unmatched_overall = unmatched["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["task_group"] == "overall")
        .unwrap();
    assert_eq!(
        unmatched_overall["matched_comparison"]["valid_complete_pairing"],
        false
    );
    assert!(unmatched
        .get("run_alignment")
        .and_then(|value| value.get("reasons"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|reasons| !reasons.is_empty()));

    let mut different_contract_multi = multi.clone();
    different_contract_multi.run_contract_sha256 = Some("different-budget".into());
    let different_contract = build_paired_report_json(
        &single,
        &different_contract_multi,
        &opts,
        Some("t-contract"),
    );
    assert_eq!(
        different_contract["run_alignment"]["configuration_aligned"],
        serde_json::Value::Bool(false)
    );
    let different_contract_overall = different_contract["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["task_group"] == "overall")
        .unwrap();
    assert_eq!(
        different_contract_overall["matched_comparison"]["uncertainty"]
            ["wall_ms_delta_case_cluster_bootstrap_95_ci"]["reason"],
        "configuration_mismatch"
    );
    let mut different_binary_multi = multi.clone();
    different_binary_multi.evaluator_binary_sha256 = Some("binary-v2".into());
    let different_binary =
        build_paired_report_json(&single, &different_binary_multi, &opts, Some("t-binary"));
    assert_eq!(
        different_binary["run_alignment"]["configuration_aligned"],
        serde_json::Value::Bool(false)
    );
    let mut different_endpoint_multi = multi.clone();
    different_endpoint_multi.provider_endpoint_sha256 = Some("endpoint-v2".into());
    let different_endpoint = build_paired_report_json(
        &single,
        &different_endpoint_multi,
        &opts,
        Some("t-endpoint"),
    );
    assert_eq!(
        different_endpoint["run_alignment"]["configuration_aligned"],
        serde_json::Value::Bool(false)
    );

    let mut legacy_multi = multi.clone();
    legacy_multi.evaluator_binary_sha256 = None;
    let legacy_pair =
        build_paired_report_json(&single, &legacy_multi, &opts, Some("t-legacy-binary"));
    assert_eq!(
        legacy_pair["run_alignment"]["configuration_aligned"],
        serde_json::Value::Bool(false)
    );

    let mut dry_single = single.clone();
    dry_single.execution = "dry-reference".into();
    let dry_pair = build_paired_report_json(&dry_single, &multi, &opts, Some("t-dry"));
    assert_eq!(
        dry_pair["run_alignment"]["configuration_aligned"],
        serde_json::Value::Bool(false)
    );

    let mut cross_side_pending = single.clone();
    cross_side_pending
        .pending
        .push(MatrixKey::new("not-single", AgentMode::Multi, 0));
    let pending_report = build_paired_report_json(&cross_side_pending, &multi, &opts, Some("t5"));
    let pending_alignment = pending_report.get("run_alignment").unwrap();
    assert_eq!(
        pending_alignment.get("configuration_aligned"),
        Some(&serde_json::Value::Bool(false))
    );
    assert!(pending_alignment
        .get("reasons")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|reasons| reasons.iter().any(|reason| {
            reason
                .as_str()
                .is_some_and(|reason| reason.contains("不属于该报告侧"))
        })));

    let mut blank_case_single = single.clone();
    blank_case_single.runs[0].key.case_id = "  ".into();
    let blank_case_report = build_paired_report_json(&blank_case_single, &multi, &opts, Some("t6"));
    assert_eq!(
        blank_case_report["run_alignment"]["configuration_aligned"],
        serde_json::Value::Bool(false)
    );

    let pairs = paired.get("pairs").unwrap().as_array().unwrap();
    let overall = pairs
        .iter()
        .find(|p| p.get("task_group").unwrap() == "overall")
        .expect("必须包含 overall 组");
    let s = overall.get("single").unwrap();
    assert_eq!(s.get("mode").unwrap(), "single");
    assert_eq!(s.get("runs_total").unwrap(), 1);
    assert_eq!(s.get("success_rate").unwrap(), 1.0);
    assert_eq!(s.get("mean_wall_ms").unwrap(), 10.0);
    assert_eq!(s.get("sample_sufficient").unwrap(), false);
    assert_eq!(
        s.get("quality").unwrap(),
        &serde_json::json!(0.75),
        "单侧快照应反映该运行的检查器通过率"
    );
    let m = overall.get("multi").unwrap();
    assert_eq!(m.get("mode").unwrap(), "multi");
    assert_eq!(overall.get("generated_at").unwrap(), "t3");
    let bindings = overall.get("bindings").unwrap();
    assert_eq!(bindings.get("model").unwrap(), "glm-5.3-flash");
    assert_eq!(bindings.get("template").unwrap(), "default");
    assert_eq!(bindings.get("task_set").unwrap(), "v1-r1-product-suite");
    assert_eq!(
        bindings.get("strategy_version").unwrap(),
        "ten-5-independent-case-samples"
    );
    // 每 case 一组。
    assert!(pairs
        .iter()
        .any(|p| p.get("task_group").unwrap() == "code-one"));
    // 元信息保留。
    assert_eq!(
        paired
            .get("single_report")
            .unwrap()
            .get("suite_hash")
            .unwrap(),
        "h"
    );
}

#[test]
fn freeze_build_verify_and_drift_detection() {
    // 用真实 v1 套件构造冻结记录（文件级哈希 + 权限哈希 + 版本身份）。
    let suite_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/v1/suite.json");
    let bundle = load_suite(&suite_path).expect("加载真实 v1 套件失败");
    let freeze = build_freeze_json(
        &bundle,
        Some("glm-5.3-flash"),
        Some("https://open.bigmodel.cn/api/paas/v4"),
        Some("abc123def"),
        Some(false),
        Some("2026-09-01T00:00:00Z"),
    )
    .expect("生成 freeze 失败");
    assert_eq!(freeze.get("schema_version").unwrap(), 1);
    let tasks = freeze.get("tasks").unwrap().as_array().unwrap();
    assert_eq!(tasks.len(), 10, "v1 套件应为 10 个任务");
    // 冻结预算：默认 20 reps。
    assert_eq!(
        freeze.get("budget").unwrap().get("single_cells").unwrap(),
        200
    );
    assert_eq!(
        freeze.get("budget").unwrap().get("multi_cells").unwrap(),
        200
    );
    // 与自身一致（同一 git 提交/模型）。
    let issues = verify_freeze(&bundle, &freeze, Some("glm-5.3-flash"));
    assert!(issues.is_empty(), "{issues:?}");
    // 模型漂移检出。
    let issues = verify_freeze(&bundle, &freeze, Some("glm-4.5"));
    assert!(
        issues.iter().any(|issue| issue.contains("模型配置已漂移")),
        "{issues:?}"
    );
    // 任务文件哈希漂移检出：篡改冻结记录中的单个任务 sha256。
    let mut tampered = freeze.clone();
    let tasks = tampered.get_mut("tasks").unwrap().as_array_mut().unwrap();
    tasks[0]["sha256"] = serde_json::Value::String("tampered".to_string());
    let issues = verify_freeze(&bundle, &tampered, Some("glm-5.3-flash"));
    assert!(
        issues.iter().any(|issue| issue.contains("哈希漂移")),
        "{issues:?}"
    );
    // 权限哈希漂移检出。
    let mut tampered_perm = freeze.clone();
    let tasks = tampered_perm
        .get_mut("tasks")
        .unwrap()
        .as_array_mut()
        .unwrap();
    tasks[0]["permissions_sha256"] = serde_json::Value::String("tampered".to_string());
    let issues = verify_freeze(&bundle, &tampered_perm, Some("glm-5.3-flash"));
    assert!(
        issues.iter().any(|issue| issue.contains("权限哈希漂移")),
        "{issues:?}"
    );
    // 外部 freeze 不得指定套件之外的路径，且必须覆盖每个唯一任务文件。
    let mut external_path = freeze.clone();
    external_path["tasks"][0]["file"] = serde_json::Value::String("../../outside.txt".to_string());
    let issues = verify_freeze(&bundle, &external_path, Some("glm-5.3-flash"));
    assert!(
        issues.iter().any(|issue| issue.contains("当前套件之外")),
        "{issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("freeze 缺少任务文件")),
        "{issues:?}"
    );

    let mut duplicate_path = freeze.clone();
    let duplicate_file = duplicate_path["tasks"][1]["file"].clone();
    duplicate_path["tasks"][0]["file"] = duplicate_file;
    let issues = verify_freeze(&bundle, &duplicate_path, Some("glm-5.3-flash"));
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("重复包含任务文件")),
        "{issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("freeze 缺少任务文件")),
        "{issues:?}"
    );

    let mut mismatched_id = freeze.clone();
    mismatched_id["tasks"][0]["id"] = serde_json::Value::String("other-task".to_string());
    let issues = verify_freeze(&bundle, &mismatched_id, Some("glm-5.3-flash"));
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("任务 ID 与文件不匹配")),
        "{issues:?}"
    );

    let mut missing_permission_digest = freeze.clone();
    let removed_permission_digest = missing_permission_digest["tasks"][0]
        .as_object_mut()
        .unwrap()
        .remove("permissions_sha256");
    assert!(removed_permission_digest.is_some());
    let issues = verify_freeze(&bundle, &missing_permission_digest, Some("glm-5.3-flash"));
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("缺少 permissions_sha256")),
        "{issues:?}"
    );

    // 序列化往返可解析。
    let text = serde_json::to_string(&freeze).unwrap();
    let parsed = parse_freeze(&text).expect("freeze.json 解析失败");
    assert_eq!(parsed.get("schema_version").unwrap(), 1);
}

#[tokio::test]
async fn batch_label_enforced_and_tags_recorded() {
    use owo_agent_product_eval::product_eval::SuiteDefaults;
    let defaults = SuiteDefaults {
        repetitions: 1,
        ..Default::default()
    };
    let b = SuiteBundle {
        dir: std::env::temp_dir(),
        suite: ProductEvalSuite {
            schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
            name: "test-suite".to_string(),
            description: String::new(),
            defaults,
            tasks: vec!["tasks/x.json".to_string()],
        },
        cases: vec![make_case("x", EvalCategory::Code)],
    };
    let out = temp_out("batch-label");
    let runner = MatrixRunner::new(b, &out);
    let opts = RunOptions {
        modes: vec![AgentMode::Single],
        reps_override: Some(1),
        only: None,
        category: None,
        fresh: false,
        batch_label: Some("acceptance-01".into()),
        tags: vec!["formal".into(), "b1".into()],
        repair_attempts: 0,
        provider_endpoint_sha256: None,
        desktop_driver: None,
        fault_driver: None,
        driver_config: None,
    };
    let report = run_matrix(
        &runner,
        ScriptedExecutor::passing(),
        "dry-reference",
        &opts,
        no_cancel(),
    )
    .await;
    assert_eq!(report.batch_label.as_deref(), Some("acceptance-01"));
    assert_eq!(report.tags, vec!["formal", "b1"]);
    // 同目录换批次 → 拒绝续跑（修复后重测必须建立新批次）。
    let opts2 = RunOptions {
        modes: vec![AgentMode::Single],
        reps_override: Some(1),
        only: None,
        category: None,
        fresh: false,
        batch_label: Some("acceptance-02".into()),
        tags: vec![],
        repair_attempts: 0,
        provider_endpoint_sha256: None,
        desktop_driver: None,
        fault_driver: None,
        driver_config: None,
    };
    let result = runner
        .run(
            ScriptedExecutor::passing(),
            "dry-reference",
            None,
            &opts2,
            no_cancel(),
        )
        .await;
    let err = result.expect_err("批次不一致必须拒绝续跑");
    assert!(err.0.contains("批次"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn freeze_permissions_hash_is_stable_and_case_sensitive() {
    let mut case = make_case("perm", EvalCategory::Code);
    case.allow_read = vec!["src/**".to_string()];
    case.allow_write = vec!["out/**".to_string()];
    case.allow_commands = vec!["python src/calc.py".to_string()];
    let h1 = permissions_hash(&case);
    case.allow_commands.push("python src/check.py".to_string());
    let h2 = permissions_hash(&case);
    assert_ne!(h1, h2, "权限范围变更必须改变权限哈希");
    let h3 = permissions_hash(&case);
    assert_eq!(h2, h3, "同一权限范围哈希必须稳定");
}

// ---------------------------------------------------------------------------
// 12) 生效参数优先级与过滤
// ---------------------------------------------------------------------------

#[test]
fn effective_values_and_filters() {
    let mut case = make_case("eff-a", EvalCategory::Code);
    case.repetitions = Some(7);
    case.timeout_secs = Some(30);
    case.max_model_calls = Some(3);
    let defaults = SuiteDefaults::default();
    assert_eq!(case.effective_repetitions(&defaults, None), 7);
    assert_eq!(case.effective_repetitions(&defaults, Some(2)), 2);
    assert_eq!(case.effective_timeout_secs(&defaults), 30);
    assert_eq!(case.effective_max_model_calls(&defaults), 3);
    let mut multi_round = make_case("eff-c", EvalCategory::Code);
    multi_round.max_model_calls = Some(64);
    assert_eq!(multi_round.effective_max_model_calls(&defaults), 64);
    let bare = make_case("eff-b", EvalCategory::Research);
    assert_eq!(bare.effective_repetitions(&defaults, None), 20);
    assert_eq!(bare.effective_timeout_secs(&defaults), 180);
    assert_eq!(bare.effective_max_model_calls(&defaults), 6);

    let b = bundle(vec![
        make_case("code-one", EvalCategory::Code),
        make_case("doc-one", EvalCategory::Document),
    ]);
    let only = filter_cases(
        &b,
        &RunOptions {
            only: Some("code".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].id, "code-one");
    let cat = filter_cases(
        &b,
        &RunOptions {
            category: Some(EvalCategory::Document),
            ..Default::default()
        },
    );
    assert_eq!(cat.len(), 1);
    assert_eq!(cat[0].id, "doc-one");
}

// ---------------------------------------------------------------------------
#[test]
fn wallclock_benchmark_accepts_full_model_call_range() {
    let previous = std::env::var("OWO_PRODUCT_EVAL_UNBOUNDED_CALLS").ok();
    std::env::set_var("OWO_PRODUCT_EVAL_UNBOUNDED_CALLS", "1");
    let mut case = make_case("wallclock-budget", EvalCategory::Code);
    case.max_model_calls = Some(u32::MAX);
    assert_eq!(
        case.effective_max_model_calls(&SuiteDefaults::default()),
        u32::MAX
    );
    let issues = validate_case(&case, &mut std::collections::BTreeSet::new());
    assert!(
        issues
            .iter()
            .all(|issue| !issue.contains("max_model_calls")),
        "opt-in timed benchmark should accept the full u32 range: {issues:?}"
    );
    match previous {
        Some(value) => std::env::set_var("OWO_PRODUCT_EVAL_UNBOUNDED_CALLS", value),
        None => std::env::remove_var("OWO_PRODUCT_EVAL_UNBOUNDED_CALLS"),
    }
}

// 13) R1 统计段：模式快照按拓扑过滤 + 对照 JSON 形状（live 基线用）
// ---------------------------------------------------------------------------

#[test]
fn mode_statistics_and_report_statistics_shapes() {
    let runs = vec![
        dummy_run(
            MatrixKey::new(String::from("c"), AgentMode::Single, 0),
            RunStatus::Passed,
        ),
        dummy_run(
            MatrixKey::new(String::from("c"), AgentMode::Single, 1),
            RunStatus::Failed,
        ),
        dummy_run(
            MatrixKey::new(String::from("c"), AgentMode::Multi, 0),
            RunStatus::Passed,
        ),
    ];
    let single = mode_statistics(&runs, AgentMode::Single);
    assert_eq!(single.runs_total, 2);
    assert_eq!(single.passed, 1);
    assert!((single.success_rate - 0.5).abs() < 1e-9);
    assert!(single.ci95_low < 0.5 && single.ci95_high > 0.5);
    let multi = mode_statistics(&runs, AgentMode::Multi);
    assert_eq!(multi.runs_total, 1);
    assert!((multi.success_rate - 1.0).abs() < 1e-9);

    let json = report_statistics(&runs);
    assert_eq!(
        json["comparison"]["multi_success_rate_diff"],
        serde_json::json!(0.5)
    );
    assert_eq!(json["comparison"]["rules"].as_array().unwrap().len(), 3);
    // 渲染段落含统计与启用条件两个小节。
    let text = format_mode_statistics(&runs);
    assert!(text.contains("CI95"));
    assert!(text.contains("多 Agent 启用条件"));
}

// ---------------------------------------------------------------------------
// 验证/修复组：检查器失败反馈 → 第二次尝试
// ---------------------------------------------------------------------------

/// 第一次产出坏内容（检查器失败），收到反馈后第二次产出正确内容。
struct RepairingExecutor {
    attempts: AtomicU32,
    seen_feedback: Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl CaseExecutor for RepairingExecutor {
    async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome {
        let attempt = self.attempts.fetch_add(1, Ordering::Relaxed) + 1;
        let owner = ctx
            .matrix_attempt()
            .expect("matrix runner must identify the attempt");
        assert_eq!(owner.key.case_id, ctx.case.id);
        assert_eq!(owner.key.agent_mode, ctx.mode);
        assert_eq!(owner.key.repetition, 0);
        assert_eq!(owner.repair_index, attempt - 1);
        assert!(owner.repair_index <= owner.repair_limit);
        self.seen_feedback
            .lock()
            .unwrap()
            .push(ctx.repair_feedback().map(str::to_string));
        let artifact = ctx.case.expected_artifacts[0].clone();
        let content = if attempt == 1 {
            "# 报告\n（缺少关键词）\n".to_string()
        } else {
            "# 报告\n结论 ok\n".to_string()
        };
        let mut outcome = RawExecOutcome {
            model_calls: 1,
            usage_known: true,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
            ..RawExecOutcome::default()
        };
        if let Err(error) = ctx.write_file(&artifact, &content) {
            outcome.error = Some(error);
        }
        outcome
    }
}

fn repairing_executor() -> Arc<RepairingExecutor> {
    Arc::new(RepairingExecutor {
        attempts: AtomicU32::new(0),
        seen_feedback: Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn repair_attempts_feed_checker_failures_and_aggregate_usage() {
    let out = temp_out("repair-loop");
    let runner = MatrixRunner::new(bundle(vec![make_case("repair", EvalCategory::Code)]), &out);
    let opts = RunOptions {
        modes: vec![AgentMode::Single],
        reps_override: Some(1),
        fresh: true,
        repair_attempts: 1,
        ..Default::default()
    };
    let executor = repairing_executor();
    let report = run_matrix(&runner, executor.clone(), "repair-test", &opts, no_cancel()).await;

    assert_eq!(report.repair_attempts, 1);
    let run = &report.runs[0];
    assert_eq!(run.status, RunStatus::Passed);
    assert_eq!(run.model_calls, 2, "修复尝试必须计入模型调用");
    assert_eq!(run.total_tokens, Some(30), "两轮用量必须累加");
    assert!(
        run.failed_steps
            .iter()
            .any(|step| step.contains("repair:1")),
        "修复轮次必须留在失败步骤里，不能只保留最终成功"
    );
    let seen = executor.seen_feedback.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].is_none(), "首次尝试不应带修复反馈");
    let feedback = seen[1].as_deref().expect("第二次尝试必须带检查器反馈");
    assert!(feedback.contains("修复请求（第 1 次）"));
    assert!(
        feedback.contains("out/report.md"),
        "反馈必须点名未通过的产物"
    );
}

#[tokio::test]
async fn repair_is_opt_in_and_zero_attempts_keeps_single_run() {
    let out = temp_out("repair-off");
    let runner = MatrixRunner::new(bundle(vec![make_case("repair", EvalCategory::Code)]), &out);
    let opts = RunOptions {
        modes: vec![AgentMode::Single],
        reps_override: Some(1),
        fresh: true,
        ..Default::default()
    };
    let executor = repairing_executor();
    let report = run_matrix(&runner, executor.clone(), "repair-off", &opts, no_cancel()).await;
    assert_eq!(report.repair_attempts, 0);
    assert_eq!(report.runs[0].status, RunStatus::Failed);
    assert_eq!(report.runs[0].model_calls, 1);
    assert_eq!(executor.attempts.load(Ordering::Relaxed), 1);
}
