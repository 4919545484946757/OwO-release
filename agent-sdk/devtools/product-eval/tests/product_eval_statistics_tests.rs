//! R1 live 基线统计的边界测试（第一路）：
//! Wilson 置信区间与分位数的 0 样本 / 全成功 / 全失败 / 样本不足边界，
//! 单/多对照启用条件（成功率 +5% / 质量 +10% / 耗时 −30%）与 JSON 契约形状。

use owo_agent_product_eval::product_eval::{
    aggregate_metrics, compare_mode_statistics, compare_report_statistics, format_mode_comparison,
    format_mode_statistics, mode_statistics, percentile, report_pairing_guardrail,
    report_statistics, wilson_interval, CI95_Z, SUFFICIENT_SAMPLE_SIZE,
};
use owo_agent_product_eval::product_eval::{
    AgentMode, EvalCategory, MatrixKey, ProductEvalRun, RunStatus,
};

fn dummy_run(case_id: &str, mode: AgentMode, status: RunStatus, wall_ms: u64) -> ProductEvalRun {
    ProductEvalRun {
        key: MatrixKey::new(case_id.to_string(), mode, 0),
        category: EvalCategory::Code,
        status,
        wall_ms,
        executor_wall_ms: wall_ms,
        validation_wall_ms: 0,
        delivery_gate_wall_ms: 0,
        model_calls: 2,
        tool_calls: Some(4),
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: Some(100),
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

// ---------------------------------------------------------------------------
// Wilson 置信区间边界
// ---------------------------------------------------------------------------

#[test]
fn wilson_zero_samples_returns_degenerate_interval() {
    assert_eq!(wilson_interval(0, 0, CI95_Z), (0.0, 0.0));
    // 0/1 与 1/1 单样本：区间必须覆盖点估计且受限在 [0,1]。
    let (lo, hi) = wilson_interval(0, 1, CI95_Z);
    assert!(lo <= 0.0 && hi > 0.0 && hi < 1.0, "({lo}, {hi})");
    let (lo, hi) = wilson_interval(1, 1, CI95_Z);
    assert!(lo > 0.0 && lo < 1.0 && hi >= 1.0, "({lo}, {hi})");
}

#[test]
fn wilson_all_success_keeps_lower_bound_below_one() {
    let (lo, hi) = wilson_interval(30, 30, CI95_Z);
    assert!((hi - 1.0).abs() < 1e-9, "全成功上界应达到 1.0：{hi}");
    assert!(
        (0.88..0.91).contains(&lo),
        "n=30 全成功下界应约 0.89（Wilson 上限公式），实际 {lo}"
    );
    // 区间必须包含点估计 1.0。
    assert!(lo <= 1.0 && hi >= 1.0);
}

#[test]
fn wilson_all_failure_keeps_upper_bound_above_zero() {
    let (lo, hi) = wilson_interval(0, 30, CI95_Z);
    assert!(lo.abs() < 1e-9, "全失败下界应为 0：{lo}");
    assert!(
        (0.09..0.13).contains(&hi),
        "n=30 全失败上界应约 0.11，实际 {hi}"
    );
    assert!(lo <= 0.0 && hi >= 0.0);
}

#[test]
fn wilson_interval_contains_point_estimate_and_shrinks_with_n() {
    for n in [5usize, 30, 100, 400] {
        let (lo, hi) = wilson_interval(n / 2, n, CI95_Z);
        assert!(lo <= 0.5 && hi >= 0.5, "n={n}: ({lo}, {hi}) 不含点估计");
        assert!(hi - lo < 1.0);
    }
    // 样本越大区间越窄。
    let width = |n: usize| {
        let (lo, hi) = wilson_interval(n / 2, n, CI95_Z);
        hi - lo
    };
    assert!(width(30) > width(300), "大样本区间应更窄");
}

// ---------------------------------------------------------------------------
// 分位数边界
// ---------------------------------------------------------------------------

#[test]
fn percentile_empty_and_single() {
    assert_eq!(percentile(&[], 50.0), None);
    assert_eq!(percentile(&[42], 50.0), Some(42.0));
    assert_eq!(percentile(&[42], 95.0), Some(42.0));
}

#[test]
fn percentile_interpolates_within_sorted_values() {
    let values: Vec<u64> = vec![30, 10, 20];
    assert_eq!(percentile(&values, 50.0), Some(20.0));
    // 1..=100 的 p95 = 95.05（线性插值）。
    let ramp: Vec<u64> = (1..=100).collect();
    let p95 = percentile(&ramp, 95.0).unwrap();
    assert!((94.0..=96.0).contains(&p95), "p95={p95}");
    let p50 = percentile(&ramp, 50.0).unwrap();
    assert!((50.0..=50.5).contains(&p50), "p50={p50}");
}

// ---------------------------------------------------------------------------
// 拓扑统计边界
// ---------------------------------------------------------------------------

#[test]
fn mode_statistics_zero_samples_is_all_zero() {
    let stats = mode_statistics(&[], AgentMode::Single);
    assert_eq!(stats.runs_total, 0);
    assert_eq!(stats.passed, 0);
    assert_eq!(stats.success_rate, 0.0);
    assert_eq!(stats.ci95_low, 0.0);
    assert_eq!(stats.ci95_high, 0.0);
    assert_eq!(stats.p50_wall_ms, None);
    assert_eq!(stats.p95_wall_ms, None);
    assert!(!stats.sample_sufficient);
}

#[test]
fn mode_statistics_all_success_and_all_failure() {
    let all_pass: Vec<ProductEvalRun> = (0..30)
        .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Passed, 1000))
        .collect();
    let stats = mode_statistics(&all_pass, AgentMode::Single);
    assert_eq!(stats.success_rate, 1.0);
    assert!(stats.ci95_high >= 1.0);
    assert!(stats.ci95_low < 1.0 && stats.ci95_low > 0.85);
    assert_eq!(stats.p50_wall_ms, Some(1000.0));
    assert_eq!(stats.p95_wall_ms, Some(1000.0));
    assert_eq!(stats.total_tokens, Some(3000));
    assert_eq!(stats.mean_tool_calls, Some(4.0));
    assert_eq!(aggregate_metrics(&all_pass).total_tool_calls, Some(120));
    assert!(stats.sample_sufficient, "n=30 应达到充分样本阈值");

    let all_fail: Vec<ProductEvalRun> = (0..30)
        .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Failed, 500))
        .collect();
    let stats = mode_statistics(&all_fail, AgentMode::Single);
    assert_eq!(stats.success_rate, 0.0);
    assert!(stats.ci95_low <= 0.0);
    assert!(stats.ci95_high > 0.0 && stats.ci95_high < 0.2);
    // 分母含失败：p50/p95 仍统计失败单元格的耗时。
    assert_eq!(stats.p50_wall_ms, Some(500.0));
}

#[test]
fn mode_statistics_small_sample_is_flagged_insufficient() {
    let small: Vec<ProductEvalRun> = (0..5)
        .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Passed, 100))
        .collect();
    let stats = mode_statistics(&small, AgentMode::Single);
    assert!(stats.runs_total < SUFFICIENT_SAMPLE_SIZE);
    assert!(!stats.sample_sufficient);
    // 拓扑过滤：Single 快照不受 multi 记录影响。
    let mut mixed = small.clone();
    mixed.push(dummy_run("c", AgentMode::Multi, RunStatus::Failed, 10));
    let single = mode_statistics(&mixed, AgentMode::Single);
    assert_eq!(single.runs_total, 5);
    assert_eq!(single.success_rate, 1.0);
}

#[test]
fn tool_call_statistics_preserve_unknown_legacy_values() {
    let mut runs = vec![
        dummy_run("c", AgentMode::Single, RunStatus::Passed, 100),
        dummy_run("c", AgentMode::Single, RunStatus::Passed, 100),
    ];
    runs[1].tool_calls = None;
    assert_eq!(aggregate_metrics(&runs).total_tool_calls, None);
    assert_eq!(
        mode_statistics(&runs, AgentMode::Single).mean_tool_calls,
        None
    );
}

#[test]
fn tool_call_comparison_is_reported_but_does_not_override_quality_and_speed_gates() {
    let single = mode_statistics(
        &(0..SUFFICIENT_SAMPLE_SIZE)
            .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Passed, 1000))
            .collect::<Vec<_>>(),
        AgentMode::Single,
    );
    let mut team_runs: Vec<ProductEvalRun> = (0..SUFFICIENT_SAMPLE_SIZE)
        .map(|_| dummy_run("c", AgentMode::Multi, RunStatus::Passed, 1000))
        .collect();
    for run in &mut team_runs {
        run.tool_calls = Some(2);
    }
    let team = mode_statistics(&team_runs, AgentMode::Multi);
    let comparison = compare_mode_statistics(&single, &team);
    assert_eq!(comparison.multi_tool_calls_rel_change, Some(-0.5));
    assert!(!comparison.enabled);
}

// ---------------------------------------------------------------------------
// 启用条件与对照边界
// ---------------------------------------------------------------------------

#[test]
fn enablement_success_rate_plus_5pp_rule() {
    let single = mode_statistics(
        &(0..30)
            .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Passed, 1000))
            .collect::<Vec<_>>(),
        AgentMode::Single,
    );
    // multi 全过（diff=+0pp，两个都是 1.0）→ 不满足 +5pp；但相对提升 = 0 也不满足。
    let multi = mode_statistics(
        &(0..30)
            .map(|_| dummy_run("c", AgentMode::Multi, RunStatus::Passed, 1000))
            .collect::<Vec<_>>(),
        AgentMode::Multi,
    );
    let comparison = compare_mode_statistics(&single, &multi);
    assert!(!comparison.enabled, "全绿对全绿不应触发启用条件");

    // single 70% vs multi 95%：diff=+25pp ≥ +5pp → 满足。
    let s: Vec<ProductEvalRun> = (0..30)
        .map(|i| {
            dummy_run(
                "c",
                AgentMode::Single,
                if i < 21 {
                    RunStatus::Passed
                } else {
                    RunStatus::Failed
                },
                1000,
            )
        })
        .collect();
    let m: Vec<ProductEvalRun> = (0..30)
        .map(|i| {
            dummy_run(
                "c",
                AgentMode::Multi,
                if i < 29 {
                    RunStatus::Passed
                } else {
                    RunStatus::Failed
                },
                1000,
            )
        })
        .collect();
    let comparison = compare_mode_statistics(
        &mode_statistics(&s, AgentMode::Single),
        &mode_statistics(&m, AgentMode::Multi),
    );
    assert!(comparison.multi_success_rate_diff >= 0.05);
    assert!(comparison.enabled);
    assert!(comparison.rules[0].satisfied);
}

#[test]
fn enablement_wall_minus_30pct_rule() {
    let single = mode_statistics(
        &(0..30)
            .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Passed, 1000))
            .collect::<Vec<_>>(),
        AgentMode::Single,
    );
    let multi = mode_statistics(
        &(0..30)
            .map(|_| dummy_run("c", AgentMode::Multi, RunStatus::Passed, 600))
            .collect::<Vec<_>>(),
        AgentMode::Multi,
    );
    let comparison = compare_mode_statistics(&single, &multi);
    assert!(
        comparison
            .multi_wall_rel_change
            .is_some_and(|value| value <= -0.30),
        "600 vs 1000 应为 −40%：{:?}",
        comparison.multi_wall_rel_change
    );
    assert!(comparison.enabled);
    assert!(comparison.rules[2].satisfied, "{:?}", comparison.rules);
    // 边界：恰好 −30% 应判满足（<=）。
    let boundary = mode_statistics(
        &(0..30)
            .map(|_| dummy_run("c", AgentMode::Multi, RunStatus::Passed, 700))
            .collect::<Vec<_>>(),
        AgentMode::Multi,
    );
    let comparison = compare_mode_statistics(&single, &boundary);
    assert!(comparison.rules[2].satisfied, "{:?}", comparison.rules[2]);
    // −20% 不满足。
    let not_enough = mode_statistics(
        &(0..30)
            .map(|_| dummy_run("c", AgentMode::Multi, RunStatus::Passed, 800))
            .collect::<Vec<_>>(),
        AgentMode::Multi,
    );
    let comparison = compare_mode_statistics(&single, &not_enough);
    assert!(!comparison.rules[2].satisfied);
}

#[test]
fn quality_regression_blocks_speed_only_enablement() {
    let single_runs: Vec<ProductEvalRun> = (0..30)
        .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Passed, 1000))
        .collect();
    let multi_runs: Vec<ProductEvalRun> = (0..30)
        .map(|index| {
            dummy_run(
                "c",
                AgentMode::Multi,
                if index < 29 {
                    RunStatus::Passed
                } else {
                    RunStatus::Failed
                },
                500,
            )
        })
        .collect();
    let comparison = compare_mode_statistics(
        &mode_statistics(&single_runs, AgentMode::Single),
        &mode_statistics(&multi_runs, AgentMode::Multi),
    );

    assert!(comparison.rules[2].satisfied, "速度收益应单独达到门槛");
    assert!(comparison.multi_success_rate_diff < 0.0);
    assert!(!comparison.enabled, "质量退化不得被速度收益抵消");
    assert!(format_mode_comparison(&comparison).contains("速度收益不抵消质量退化"));
}

#[test]
fn resource_overhead_blocks_speed_only_enablement() {
    let single_runs: Vec<ProductEvalRun> = (0..30)
        .map(|_| {
            dummy_run(
                "resource-budget",
                AgentMode::Single,
                RunStatus::Passed,
                1000,
            )
        })
        .collect();
    let mut team_runs: Vec<ProductEvalRun> = (0..30)
        .map(|_| dummy_run("resource-budget", AgentMode::Multi, RunStatus::Passed, 500))
        .collect();
    for run in &mut team_runs {
        run.total_tokens = Some(300); // 3x token cost despite a 50% wall-clock win.
        run.model_calls = 4; // 2x model requests.
        run.tool_calls = Some(12); // 3x tool operations.
    }
    let comparison = compare_mode_statistics(
        &mode_statistics(&single_runs, AgentMode::Single),
        &mode_statistics(&team_runs, AgentMode::Multi),
    );
    assert!(comparison.rules[2].satisfied, "speed gain is present");
    assert!(
        !comparison.enabled,
        "resource overhead must block the speed-only win"
    );
    assert!(comparison
        .resource_guardrails
        .iter()
        .any(|rule| rule.name == "token_overhead" && !rule.satisfied));
    assert!(comparison
        .resource_guardrails
        .iter()
        .any(|rule| rule.name == "model_call_overhead" && !rule.satisfied));
    assert!(comparison
        .resource_guardrails
        .iter()
        .any(|rule| rule.name == "tool_call_overhead" && !rule.satisfied));
}

#[test]
fn missing_usage_telemetry_blocks_automatic_team_enablement() {
    let single_runs: Vec<ProductEvalRun> = (0..30)
        .map(|_| dummy_run("unknown-usage", AgentMode::Single, RunStatus::Passed, 1000))
        .collect();
    let mut team_runs: Vec<ProductEvalRun> = (0..30)
        .map(|_| dummy_run("unknown-usage", AgentMode::Multi, RunStatus::Passed, 500))
        .collect();
    for run in &mut team_runs {
        run.total_tokens = None;
        run.tool_calls = None;
    }
    let comparison = compare_mode_statistics(
        &mode_statistics(&single_runs, AgentMode::Single),
        &mode_statistics(&team_runs, AgentMode::Multi),
    );
    assert!(
        !comparison.enabled,
        "unknown resource cost is not equivalent to zero"
    );
    assert!(comparison
        .resource_guardrails
        .iter()
        .any(|rule| rule.name == "token_overhead" && !rule.satisfied));
    assert!(comparison
        .resource_guardrails
        .iter()
        .any(|rule| rule.name == "tool_call_overhead" && !rule.satisfied));
}

#[test]
fn enablement_insufficient_samples_flagged() {
    let small: Vec<ProductEvalRun> = (0..3)
        .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Passed, 1000))
        .collect();
    let small_multi: Vec<ProductEvalRun> = (0..3)
        .map(|_| dummy_run("c", AgentMode::Multi, RunStatus::Failed, 10))
        .collect();
    let comparison = compare_mode_statistics(
        &mode_statistics(&small, AgentMode::Single),
        &mode_statistics(&small_multi, AgentMode::Multi),
    );
    assert!(!comparison.sample_sufficient);
    assert!(!comparison.enabled);

    let fast_multi: Vec<ProductEvalRun> = (0..3)
        .map(|_| dummy_run("c", AgentMode::Multi, RunStatus::Passed, 600))
        .collect();
    let small_speed_only = compare_mode_statistics(
        &mode_statistics(&small, AgentMode::Single),
        &mode_statistics(&fast_multi, AgentMode::Multi),
    );
    assert!(small_speed_only.rules[2].satisfied);
    assert!(!small_speed_only.sample_sufficient);
    assert!(!small_speed_only.enabled, "小样本速度不能作为启用证据");
    // 渲染段落必须显式提示样本不足（UI/API 同样以此为标注依据）。
    let text = format_mode_comparison(&comparison);
    assert!(text.contains("样本不足"), "{text}");
}

#[test]
fn stage_wall_times_are_reported_for_single_and_multi_comparisons() {
    let mut single = dummy_run("stage-times", AgentMode::Single, RunStatus::Passed, 100);
    single.executor_wall_ms = 70;
    single.validation_wall_ms = 30;
    let mut multi = dummy_run("stage-times", AgentMode::Multi, RunStatus::Passed, 160);
    multi.executor_wall_ms = 120;
    multi.validation_wall_ms = 25;
    multi.delivery_gate_wall_ms = 15;

    let aggregate = aggregate_metrics(&[single.clone(), multi.clone()]);
    assert_eq!(aggregate.mean_executor_wall_ms, 95.0);
    assert_eq!(aggregate.mean_validation_wall_ms, 27.5);
    assert_eq!(aggregate.mean_delivery_gate_wall_ms, 7.5);

    let stats = mode_statistics(&[single, multi], AgentMode::Multi);
    assert_eq!(stats.mean_executor_wall_ms, 120.0);
    assert_eq!(stats.mean_validation_wall_ms, 25.0);
    assert_eq!(stats.mean_delivery_gate_wall_ms, 15.0);
}

#[test]
fn statistics_render_and_json_contract_shapes() {
    let runs: Vec<ProductEvalRun> = (0..6)
        .flat_map(|i| {
            vec![
                dummy_run(
                    "c",
                    AgentMode::Single,
                    if i < 5 {
                        RunStatus::Passed
                    } else {
                        RunStatus::Failed
                    },
                    1000 + i,
                ),
                dummy_run("c", AgentMode::Multi, RunStatus::Passed, 700 + i),
            ]
        })
        .collect();
    let mut runs = runs;
    runs[0].executor_wall_ms = 900;
    runs[0].validation_wall_ms = 100;
    runs[1].delivery_gate_wall_ms = 25;
    let text = format_mode_statistics(&runs);
    assert!(text.contains("CI95"), "{text}");
    assert!(text.contains("p50"), "{text}");
    assert!(text.contains("executor="), "{text}");
    assert!(text.contains("validate="), "{text}");
    assert!(text.contains("gate="), "{text}");
    assert!(text.contains("不可相加替代总 wall_ms"), "{text}");
    assert!(text.contains("多 Agent 启用条件"), "{text}");

    let json = report_statistics(&runs);
    let modes = json
        .get("modes")
        .and_then(serde_json::Value::as_array)
        .unwrap();
    assert_eq!(modes.len(), 2);
    assert_eq!(modes[0]["mode"], "single");
    assert_eq!(modes[1]["mode"], "multi");
    assert!(modes[0]["ci95_low"].is_number());
    assert!(modes[0]["p95_wall_ms"].is_number());
    assert!(modes[0]["mean_executor_wall_ms"].is_number());
    assert!(modes[0]["mean_validation_wall_ms"].is_number());
    assert!(modes[0]["mean_delivery_gate_wall_ms"].is_number());
    let comparison = json.get("comparison").unwrap();
    assert!(comparison["multi_success_rate_diff"].is_number());
    assert!(comparison["rules"].is_array());
    assert_eq!(comparison["rules"].as_array().unwrap().len(), 3);
    assert!(comparison["sample_sufficient"].is_boolean());
    // 无样本时 JSON 形状保持不变（0 值 + null 分位数）。
    let empty = report_statistics(&[]);
    assert_eq!(empty["modes"][0]["runs_total"], 0);
    assert!(empty["modes"][0]["p50_wall_ms"].is_null());
}

#[test]
fn checker_quality_regression_blocks_speed_based_auto_team_enablement() {
    let mut single_runs: Vec<ProductEvalRun> = (0..SUFFICIENT_SAMPLE_SIZE)
        .map(|_| dummy_run("c", AgentMode::Single, RunStatus::Passed, 1000))
        .collect();
    let mut team_runs: Vec<ProductEvalRun> = (0..SUFFICIENT_SAMPLE_SIZE)
        .map(|_| dummy_run("c", AgentMode::Multi, RunStatus::Passed, 600))
        .collect();
    for run in &mut single_runs {
        run.checker_passed = 8;
        run.checker_total = 10;
    }
    for run in &mut team_runs {
        run.checker_passed = 7;
        run.checker_total = 10;
    }
    let single = mode_statistics(&single_runs, AgentMode::Single);
    let team = mode_statistics(&team_runs, AgentMode::Multi);
    let comparison = compare_mode_statistics(&single, &team);
    assert!(comparison
        .rules
        .iter()
        .any(|rule| rule.name == "耗时 -30%" && rule.satisfied));
    assert!(!comparison.enabled, "质量下降时不得因提速自动启用 Team");
    assert!(comparison
        .quality_guardrails
        .iter()
        .any(|rule| !rule.satisfied));
}

#[test]
fn report_auto_enablement_requires_complete_single_team_pairs_and_no_pending_cells() {
    let mut runs = Vec::new();
    for repetition in 0..SUFFICIENT_SAMPLE_SIZE as u32 {
        let mut single = dummy_run("c", AgentMode::Single, RunStatus::Passed, 1000);
        single.key.repetition = repetition;
        let mut team = dummy_run("c", AgentMode::Multi, RunStatus::Passed, 600);
        team.key.repetition = repetition;
        runs.extend([single, team]);
    }
    assert!(report_pairing_guardrail(&runs, &[]).satisfied);
    assert!(
        compare_report_statistics(&runs, &[]).enabled,
        "complete paired speed gain may qualify"
    );

    let mut incomplete = runs.clone();
    incomplete.pop();
    let blocked = compare_report_statistics(&incomplete, &[]);
    assert!(!blocked.enabled);
    assert!(blocked
        .alignment_guardrails
        .iter()
        .any(|rule| !rule.satisfied));
    assert!(format_mode_comparison(&blocked).contains("缺少一侧"));

    let pending = vec![MatrixKey::new("next".to_string(), AgentMode::Multi, 0)];
    let blocked = compare_report_statistics(&runs, &pending);
    assert!(!blocked.enabled);
    assert!(blocked
        .alignment_guardrails
        .iter()
        .any(|rule| !rule.satisfied));
}
