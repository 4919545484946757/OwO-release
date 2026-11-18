//! Live paired Single/Team suite comparison. Ignored by default; requires isolated paths and provider configuration.
use owo_agent_product_eval::product_eval::single_agent::SingleAgentExecutor;
use owo_agent_product_eval::product_eval::{
    build_live_provider, build_paired_report_json, load_suite, split_paired_mode_reports,
    validate_suite, AgentMode, CaseExecutor, EvalCategory, ExecContext, InputFixture, MatrixRunner,
    PairedReportOptions, ProductEvalCase, RawExecOutcome, RunOptions, RunStatus,
};
use owo_agent_product_eval::workswarm_executor::{WorkSwarmExecutor, WorkSwarmExecutorConfig};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

struct PairedDispatchExecutor {
    single: Arc<dyn CaseExecutor>,
    team: Arc<dyn CaseExecutor>,
}

#[async_trait::async_trait]
impl CaseExecutor for PairedDispatchExecutor {
    async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome {
        match ctx.mode {
            AgentMode::Single => self.single.execute(ctx).await,
            AgentMode::Multi => self.team.execute(ctx).await,
        }
    }
}

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("missing {name}")))
}

fn required_nonempty(name: &str) -> String {
    let value = std::env::var(name)
        .unwrap_or_else(|_| panic!("missing {name}; this value is required for reproducibility"))
        .trim()
        .to_string();
    assert!(!value.is_empty(), "{name} cannot be empty");
    value
}

fn optional_positive_u32(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("{name} must be a positive integer"))
        })
        .map(|value| {
            assert!(value > 0, "{name} must be greater than zero");
            value
        })
}

fn optional_positive_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("{name} must be a positive integer"))
        })
        .map(|value| {
            assert!(value > 0, "{name} must be greater than zero");
            value
        })
}

fn truthy_env(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

fn options(
    modes: Vec<AgentMode>,
    endpoint_sha256: &str,
    batch_label: &str,
    repetitions: u32,
    case_filter: Option<String>,
) -> RunOptions {
    RunOptions {
        modes,
        reps_override: Some(repetitions),
        only: case_filter,
        fresh: true,
        batch_label: Some(batch_label.to_string()),
        provider_endpoint_sha256: Some(endpoint_sha256.to_string()),
        tags: vec![
            "paired-live-v1".to_string(),
            format!("repetitions={repetitions}"),
        ],
        ..RunOptions::default()
    }
}

const MINIMUM_DIRECTIONAL_PAIRED_CELLS: usize = 30;
const MINIMUM_DIRECTIONAL_TASKS: usize = 3;
const MINIMUM_DIRECTIONAL_CATEGORIES: usize = 2;
const MINIMUM_POLICY_CASE_CLUSTERS: usize = 30;

fn directional_evidence_floor(
    aligned: bool,
    paired_cells: usize,
    selected_tasks: usize,
    selected_categories: usize,
) -> bool {
    aligned
        && paired_cells >= MINIMUM_DIRECTIONAL_PAIRED_CELLS
        && selected_tasks >= MINIMUM_DIRECTIONAL_TASKS
        && selected_categories >= MINIMUM_DIRECTIONAL_CATEGORIES
}

fn policy_sample_sufficient(
    independent_task_content_clusters: usize,
    single_case_clusters: usize,
    team_case_clusters: usize,
) -> bool {
    independent_task_content_clusters >= MINIMUM_POLICY_CASE_CLUSTERS
        && single_case_clusters >= MINIMUM_POLICY_CASE_CLUSTERS
        && team_case_clusters >= MINIMUM_POLICY_CASE_CLUSTERS
}

/// Count task content, not labels: changing only an ID/title must not manufacture
/// an independent sample. Repetitions stay on one content fingerprint/cluster.
fn task_content_fingerprint(case: &ProductEvalCase) -> String {
    let content = serde_json::json!({
        "category": case.category,
        "instruction": case.instruction,
        "inputs": case.inputs,
    });
    let bytes = serde_json::to_vec(&content).expect("serialize task content fingerprint");
    format!("{:x}", Sha256::digest(bytes))
}

fn duplicate_task_content_ids(cases: &[&ProductEvalCase]) -> Vec<Vec<String>> {
    let mut ids_by_fingerprint = std::collections::BTreeMap::<String, Vec<String>>::new();
    for case in cases {
        ids_by_fingerprint
            .entry(task_content_fingerprint(case))
            .or_default()
            .push(case.id.clone());
    }
    ids_by_fingerprint
        .into_values()
        .filter(|ids| ids.len() > 1)
        .collect()
}

fn paired_execution_complete(
    aligned: bool,
    valid_complete_pairing: bool,
    paired_cells: usize,
    planned_paired_cells: usize,
) -> bool {
    aligned
        && valid_complete_pairing
        && planned_paired_cells > 0
        && paired_cells == planned_paired_cells
}

fn save_json(path: PathBuf, value: &impl serde::Serialize) {
    use std::io::Write;

    let contents = serde_json::to_vec_pretty(value).expect("serialize paired evaluation report");
    let parent = path.parent().expect("paired report path has parent");
    std::fs::create_dir_all(parent)
        .unwrap_or_else(|error| panic!("create {} failed: {error}", parent.display()));
    let temp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("report.json"),
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&contents)?;
        file.sync_all()?;
        std::fs::rename(&temp, &path)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp);
        panic!("atomic write {} failed: {error}", path.display());
    }
}

#[tokio::test]
#[ignore = "Live paired suite comparison; requires explicit repetition/batch, isolated output, and configured provider"]
async fn live_single_vs_team_paired_suite() {
    assert!(
        !truthy_env("OWO_PRODUCT_EVAL_UNBOUNDED_CALLS"),
        "paired evidence requires the shared per-task model-call budget; unset OWO_PRODUCT_EVAL_UNBOUNDED_CALLS"
    );
    let suite_path = required_path("OWO_BLOG_BENCHMARK_SUITE");
    let out_root = required_path("OWO_BLOG_BENCHMARK_OUT");
    let batch_label = required_nonempty("OWO_PAIRED_BATCH_LABEL");
    let repetitions = optional_positive_u32("OWO_PAIRED_REPETITIONS").unwrap_or_else(|| {
        panic!("missing OWO_PAIRED_REPETITIONS; choose the planned sample count")
    });
    let case_filter = std::env::var("OWO_PAIRED_CASE_FILTER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let team_turn_cap = optional_positive_usize("OWO_PAIRED_TEAM_TURNS").unwrap_or(8);
    let team_retry_cap = std::env::var("OWO_PAIRED_TEAM_RETRIES")
        .ok()
        .map(|value| {
            value
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("OWO_PAIRED_TEAM_RETRIES must be an integer"))
        })
        .unwrap_or(1);
    let suite = load_suite(&suite_path).expect("load benchmark suite");
    let suite_validation = validate_suite(&suite);
    assert!(
        suite_validation.all_ok,
        "benchmark suite failed static/checker validation before provider invocation: {}",
        serde_json::to_string_pretty(&suite_validation)
            .unwrap_or_else(|_| "<serialize failed>".to_string())
    );
    let selected = owo_agent_product_eval::product_eval::filter_cases(
        &suite,
        &RunOptions {
            only: case_filter.clone(),
            ..RunOptions::default()
        },
    );
    assert!(
        !selected.is_empty(),
        "case filter matched no benchmark tasks"
    );
    let selected_refs = selected.iter().collect::<Vec<_>>();
    let duplicate_content = duplicate_task_content_ids(&selected_refs);
    assert!(
        duplicate_content.is_empty(),
        "case IDs do not prove independent evidence; duplicate category/instruction/fixture content found: {duplicate_content:?}"
    );
    let independent_task_content_clusters = selected
        .iter()
        .map(|case| task_content_fingerprint(case))
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    assert_eq!(
        independent_task_content_clusters,
        selected.len(),
        "every selected case must have unique task content"
    );
    assert_eq!(
        repetitions % 2,
        0,
        "paired repetitions must be even so each two-run block counterbalances AB/BA order"
    );
    let planned_categories = selected
        .iter()
        .map(|case| case.category.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let planned_pairs = selected.len().saturating_mul(repetitions as usize);
    assert!(
        selected.len() >= 3 && planned_categories.len() >= 2 && planned_pairs >= 30,
        "refusing live comparison below evidence floor: selected_tasks={}, categories={}, planned_pairs={} (need >=3 tasks, >=2 categories, >=30 paired cells)",
        selected.len(),
        planned_categories.len(),
        planned_pairs
    );
    assert!(
        !out_root.exists(),
        "output root must be new; refusing to overwrite it"
    );

    let model_override = std::env::var("OWO_PAIRED_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let (provider, model, endpoint_sha256) =
        build_live_provider(model_override.as_deref()).expect("build configured provider");
    println!(
        "paired live suite: label={batch_label}, model={model}, tasks={}, repetitions={repetitions}, shared per-task model-call/time budget, Team worker_turn_cap={team_turn_cap}, Team retries={team_retry_cap}",
        selected.len()
    );
    let cancel = Arc::new(AtomicBool::new(false));

    let mut team_executor = WorkSwarmExecutor::new(
        Arc::clone(&provider),
        model.clone(),
        out_root.join("team-runs"),
    );
    team_executor.config = WorkSwarmExecutorConfig {
        max_turns_per_worker: team_turn_cap,
        max_retries_on_failure: team_retry_cap,
        selection: owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam,
    };
    let paired_executor = PairedDispatchExecutor {
        single: Arc::new(SingleAgentExecutor::new(
            Arc::clone(&provider),
            model.clone(),
        )),
        team: Arc::new(team_executor),
    };
    let matrix_options = options(
        vec![AgentMode::Single, AgentMode::Multi],
        &endpoint_sha256,
        &batch_label,
        repetitions,
        case_filter,
    );
    let matrix = MatrixRunner::new(suite.clone(), out_root.join("matrix"))
        .run(
            Arc::new(paired_executor),
            "live-paired",
            Some(model.clone()),
            &matrix_options,
            cancel,
        )
        .await
        .expect("paired run");
    let (single, team) = split_paired_mode_reports(&matrix).expect("split paired run report");

    let pair_options = PairedReportOptions {
        model: Some(model.clone()),
        template: None,
        task_set: Some(suite.suite.name.clone()),
        strategy_version: format!(
            "team-force-v1;worker-turn-cap={team_turn_cap};retries={team_retry_cap}"
        ),
    };
    let mut paired = build_paired_report_json(&single, &team, &pair_options, None);
    let alignment = paired
        .pointer("/run_alignment/configuration_aligned")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let selected_categories = selected
        .iter()
        .map(|case| case.category.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let paired_cells = paired
        .pointer("/run_alignment/paired_cells")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let planned_paired_cells = selected.len().saturating_mul(repetitions as usize);
    let valid_complete_pairing = paired
        .pointer("/run_alignment/valid_complete_pairing")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let single_case_clusters = paired
        .pointer("/pairs/0/single/independent_case_clusters")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let team_case_clusters = paired
        .pointer("/pairs/0/multi/independent_case_clusters")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let execution_complete = paired_execution_complete(
        alignment,
        valid_complete_pairing,
        paired_cells,
        planned_paired_cells,
    );
    let directional_floor_met = directional_evidence_floor(
        alignment,
        paired_cells,
        independent_task_content_clusters,
        selected_categories.len(),
    );
    let policy_sample_sufficient = policy_sample_sufficient(
        independent_task_content_clusters,
        single_case_clusters,
        team_case_clusters,
    );
    if let Some(object) = paired.as_object_mut() {
        object.insert(
            "evaluation_contract".to_string(),
            serde_json::json!({
                "batch_label": batch_label.clone(),
                "repetitions_per_selected_task": repetitions,
                "selected_task_ids": selected.iter().map(|case| case.id.clone()).collect::<Vec<_>>(),
                "suite_hash": matrix.suite_hash.clone(),
                "run_contract_sha256": matrix.run_contract_sha256.clone(),
                "provider_endpoint_sha256": endpoint_sha256.clone(),
                "execution_order": "within each task, two-repetition blocks use a batch/task-derived randomized AB/BA order",
                "order_counterbalanced": true,
                "order_randomized": true,
                "order_seed_sha256": format!("{:x}", Sha256::digest(batch_label.as_bytes())),
                "order_assignment": "lowest SHA-256 bit of batch_label + case_id + pair_block selects block orientation; next repetition reverses it",
                "causal_comparison_eligible": false,
                "causal_limit": "external provider contention and model-service drift remain; task-clustered 95% bootstrap intervals describe task-sample uncertainty but do not establish a causal advantage",
                "sample_assessment": {
                    "paired_cells": paired_cells,
                    "planned_paired_cells": planned_paired_cells,
                    "minimum_paired_cells_for_directional_report": MINIMUM_DIRECTIONAL_PAIRED_CELLS,
                    "paired_execution_complete": execution_complete,
                    "selected_task_count": selected.len(),
                    "selected_independent_task_content_clusters": independent_task_content_clusters,
                    "independence_fingerprint": "sha256(category,instruction,inputs); ID/title/repetition do not create a sample",
                    "minimum_distinct_tasks_for_directional_report": MINIMUM_DIRECTIONAL_TASKS,
                    "selected_task_categories": selected_categories,
                    "minimum_categories_for_directional_report": MINIMUM_DIRECTIONAL_CATEGORIES,
                    "directional_evidence_floor_met": directional_floor_met,
                    "single_independent_case_clusters": single_case_clusters,
                    "team_independent_case_clusters": team_case_clusters,
                    "minimum_independent_case_clusters_for_policy": MINIMUM_POLICY_CASE_CLUSTERS,
                    "policy_sample_sufficient": policy_sample_sufficient,
                    "policy_sample_note": "repetitions of one task do not create independent task clusters",
                },
                "shared_budget": {
                    "model_calls": "case.max_model_calls",
                    "wall_time": "case.timeout_secs",
                    "unbounded_single_override": false,
                },
                "single_profile": {
                    "topology": "single-agent",
                    "model_call_budget": "shared per-task budget",
                },
                "team_profile": {
                    "topology": "forced-team",
                    "global_model_call_budget": "shared per-task budget",
                    "worker_turn_cap": team_turn_cap,
                    "retry_cap": team_retry_cap,
                }
            }),
        );
    }
    save_json(out_root.join("matrix-report.json"), &matrix);
    save_json(out_root.join("single-report.json"), &single);
    save_json(out_root.join("team-report.json"), &team);
    save_json(out_root.join("paired-report.json"), &paired);

    for (label, report) in [("single", &single), ("team", &team)] {
        for run in &report.runs {
            println!(
                "[{label}] case={} repetition={} status={:?} wall_ms={} executor_wall_ms={} validation_wall_ms={} delivery_gate_wall_ms={} calls={} tokens={:?} artifacts={} checkers={}/{} retries={} error={:?}",
                run.key.case_id,
                run.key.repetition,
                run.status,
                run.wall_ms,
                run.executor_wall_ms,
                run.validation_wall_ms,
                run.delivery_gate_wall_ms,
                run.model_calls,
                run.total_tokens,
                run.artifact_refs.len(),
                run.checker_passed,
                run.checker_total,
                run.retries,
                run.error,
            );
        }
    }
    assert!(
        alignment,
        "Single/Team pairing failed alignment; inspect {}",
        out_root.join("paired-report.json").display()
    );
    assert!(
        execution_complete,
        "paired run did not cover every planned task/repetition cell; inspect {}",
        out_root.join("paired-report.json").display()
    );
    assert!(
        directional_floor_met,
        "paired directional coverage is below its minimum; inspect {}",
        out_root.join("paired-report.json").display()
    );
    let error_count = single
        .runs
        .iter()
        .chain(team.runs.iter())
        .filter(|run| run.status == RunStatus::Error)
        .count();
    println!(
        "paired report saved; aligned={alignment}, single_cells={}, team_cells={}, errors={error_count}",
        single.runs.len(),
        team.runs.len()
    );
}

#[cfg(test)]
mod evidence_floor_tests {
    use super::{
        directional_evidence_floor, paired_execution_complete, policy_sample_sufficient,
        EvalCategory, InputFixture, ProductEvalCase,
    };
    use std::path::PathBuf;

    #[test]
    fn v2_benchmark_suite_has_thirty_independent_code_cases_and_cross_category_coverage() {
        let suite_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/v2/suite.json");
        let suite = super::load_suite(&suite_path).expect("load v2 independent benchmark suite");
        let validation = super::validate_suite(&suite);
        assert!(
            validation.all_ok,
            "v2 benchmark suite static validation failed: {}",
            serde_json::to_string_pretty(&validation).expect("serialize suite diagnostics")
        );
        assert_eq!(suite.cases.len(), 31);
        assert_eq!(
            suite
                .cases
                .iter()
                .filter(|case| case.category.as_str() == "code")
                .count(),
            30
        );
        assert_eq!(
            suite
                .cases
                .iter()
                .filter(|case| case.category.as_str() == "research")
                .count(),
            1
        );
        let selected = suite.cases.iter().collect::<Vec<_>>();
        assert!(
            super::duplicate_task_content_ids(&selected).is_empty(),
            "v2 suite must not count relabeled task content as independent"
        );
        let fingerprints = suite
            .cases
            .iter()
            .map(super::task_content_fingerprint)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(fingerprints.len(), 31);
    }

    #[tokio::test]
    async fn v2_code_reference_solutions_pass_the_real_sandbox_command_checks() {
        let suite_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/v2/suite.json");
        let suite = super::load_suite(&suite_path).expect("load v2 independent benchmark suite");
        for case in suite
            .cases
            .iter()
            .filter(|case| case.category.as_str() == "code")
        {
            let sandbox = std::env::temp_dir().join(format!(
                "owo-product-eval-v2-reference-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&sandbox).expect("create isolated reference sandbox");
            for fixture in &case.inputs {
                let path = sandbox.join(&fixture.path);
                std::fs::create_dir_all(path.parent().expect("fixture parent"))
                    .expect("create fixture directory");
                std::fs::write(path, &fixture.content).expect("write task fixture");
            }
            for (relative, content) in &case.reference_outputs {
                let path = sandbox.join(relative);
                std::fs::create_dir_all(path.parent().expect("reference parent"))
                    .expect("create reference directory");
                std::fs::write(path, content).expect("write reference output");
            }
            for checker in &case.checkers {
                if matches!(
                    checker,
                    owo_agent_product_eval::product_eval::ArtifactChecker::CommandCheck { .. }
                ) {
                    owo_agent_product_eval::product_eval::evaluate_command_check(checker, &sandbox)
                        .await
                        .unwrap_or_else(|error| {
                            panic!("reference solution failed {}: {error}", case.id)
                        });
                }
            }
            std::fs::remove_dir_all(&sandbox).expect("remove isolated reference sandbox");
        }
    }

    #[test]
    fn repeated_cells_do_not_count_as_independent_policy_samples() {
        assert!(directional_evidence_floor(true, 30, 3, 2));
        assert!(!policy_sample_sufficient(3, 30, 30));
        assert!(policy_sample_sufficient(30, 30, 30));
    }

    fn case(id: &str, title: &str, instruction: &str, fixture: &str) -> ProductEvalCase {
        ProductEvalCase {
            schema_version: 1,
            id: id.to_string(),
            category: EvalCategory::Code,
            title: title.to_string(),
            instruction: instruction.to_string(),
            inputs: vec![InputFixture {
                path: "src/input.py".to_string(),
                content: fixture.to_string(),
            }],
            allow_read: vec!["src/**".to_string()],
            allow_write: vec!["src/**".to_string(), "out/**".to_string()],
            expected_artifacts: vec!["out/report.md".to_string()],
            checkers: Vec::new(),
            reference_outputs: std::collections::BTreeMap::new(),
            timeout_secs: None,
            max_model_calls: None,
            repetitions: None,
            allow_commands: Vec::new(),
            failure_classes: Vec::new(),
            expected_side_effects: Vec::new(),
            fault_injection: None,
        }
    }

    #[test]
    fn changing_labels_does_not_create_an_independent_task_cluster() {
        let original = case(
            "case-a",
            "First title",
            "Fix the parser",
            "def parse(): return 1",
        );
        let relabeled = case(
            "case-b",
            "Renamed title",
            "Fix the parser",
            "def parse(): return 1",
        );
        let changed_fixture = case(
            "case-c",
            "Other title",
            "Fix the parser",
            "def parse(): return 2",
        );
        assert_eq!(
            super::task_content_fingerprint(&original),
            super::task_content_fingerprint(&relabeled)
        );
        assert_ne!(
            super::task_content_fingerprint(&original),
            super::task_content_fingerprint(&changed_fixture)
        );
        assert_eq!(
            super::duplicate_task_content_ids(&[&original, &relabeled]),
            vec![vec!["case-a".to_string(), "case-b".to_string()]]
        );
        assert!(super::duplicate_task_content_ids(&[&original, &changed_fixture]).is_empty());
    }

    #[test]
    fn report_writer_commits_complete_json_without_temp_residue() {
        let root =
            std::env::temp_dir().join(format!("owo-paired-report-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).expect("create isolated report test directory");
        let path = root.join("paired-report.json");
        super::save_json(
            path.clone(),
            &serde_json::json!({"complete": true, "cells": 40}),
        );
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read committed report"))
                .expect("complete JSON document");
        assert_eq!(report["complete"], true);
        assert_eq!(report["cells"], 40);
        assert_eq!(
            std::fs::read_dir(&root)
                .expect("list report directory")
                .count(),
            1
        );
        std::fs::remove_file(&path).expect("remove test report");
        std::fs::remove_dir(&root).expect("remove empty test directory");
    }

    #[test]
    fn execution_coverage_requires_every_planned_pair_once() {
        assert!(paired_execution_complete(true, true, 40, 40));
        assert!(!paired_execution_complete(true, true, 39, 40));
        assert!(!paired_execution_complete(true, false, 40, 40));
        assert!(!paired_execution_complete(false, true, 40, 40));
    }
}
