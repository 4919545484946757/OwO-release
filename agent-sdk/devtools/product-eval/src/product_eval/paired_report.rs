//! Paired Single/Team report schema, alignment and per-cell comparison aggregation.

use super::{
    aggregate_metrics, aggregate_per_case, complete_optional_cost_sum, complete_optional_u64_sum,
    err, now_rfc3339, quality_of, wilson_interval, AgentMode, ProductEvalError, ProductEvalReport,
    ProductEvalRun, RunStatus, CI95_Z, SUFFICIENT_SAMPLE_SIZE,
};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// 配对对照报告（第二路交付第三路：PairedStats 兼容 JSON）
// ---------------------------------------------------------------------------

/// 配对对照报告 schema 版本（对齐 team_benefit 的读取契约）。
pub const PAIRED_REPORT_SCHEMA_VERSION: u32 = 1;

/// 配对报告绑定参数：四元组（model/template/task_set/strategy_version）。
/// strategy_version 由三路冻结；本路负责如实记录。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PairedReportOptions {
    pub model: Option<String>,
    pub template: Option<String>,
    pub task_set: Option<String>,
    pub strategy_version: String,
}

/// 单侧快照 JSON（字段对齐 `team_benefit::ModeStatSnapshot` + quality）。
pub(super) fn paired_snapshot_json(mode: AgentMode, runs: &[&ProductEvalRun]) -> serde_json::Value {
    let total = runs.len();
    let independent_case_clusters = runs
        .iter()
        .map(|run| run.key.case_id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let passed = runs
        .iter()
        .filter(|run| run.status == RunStatus::Passed)
        .count();
    let (low, high) = wilson_interval(passed, total, CI95_Z);
    let walls: Vec<u64> = runs.iter().map(|run| run.wall_ms).collect();
    let mean_wall = if total == 0 {
        0.0
    } else {
        walls.iter().sum::<u64>() as f64 / total as f64
    };
    let mean_calls = if total == 0 {
        0.0
    } else {
        runs.iter().map(|r| r.model_calls as f64).sum::<f64>() / total as f64
    };
    let mean_tool_calls = if total > 0 && runs.iter().all(|run| run.tool_calls.is_some()) {
        Some(
            runs.iter()
                .filter_map(|run| run.tool_calls)
                .map(f64::from)
                .sum::<f64>()
                / total as f64,
        )
    } else {
        None
    };
    let tokens = complete_optional_u64_sum(runs.iter().map(|run| run.total_tokens));
    let cost = complete_optional_cost_sum(runs.iter().map(|run| run.cost_usd));
    let owned: Vec<ProductEvalRun> = runs.iter().map(|r| (*r).clone()).collect();
    serde_json::json!({
        "mode": mode.as_str(),
        "runs_total": total,
        "independent_case_clusters": independent_case_clusters,
        "passed": passed,
        "success_rate": if total == 0 { 0.0 } else { passed as f64 / total as f64 },
        "ci95_low": low,
        "ci95_high": high,
        "mean_wall_ms": mean_wall,
        "mean_model_calls": mean_calls,
        "mean_tool_calls": mean_tool_calls,
        "total_tokens": tokens,
        "total_cost_usd": cost,
        "quality": quality_of(&owned),
        "sample_sufficient": independent_case_clusters >= SUFFICIENT_SAMPLE_SIZE,
    })
}

/// Case-cluster percentile bootstrap. Repetitions for one task remain in the same
/// cluster so repeated runs are not treated as independent task samples.
pub(super) fn paired_case_cluster_bootstrap_ci(
    task_means: &[f64],
    pairing_is_complete: bool,
    configuration_aligned: bool,
) -> serde_json::Value {
    const RESAMPLES: usize = 5_000;
    if !pairing_is_complete || !configuration_aligned {
        return serde_json::json!({
            "available": false,
            "reason": if !pairing_is_complete {
                "incomplete_or_duplicate_pairing"
            } else {
                "configuration_mismatch"
            },
            "independent_case_clusters": task_means.len(),
        });
    }
    if task_means.len() < 3 {
        return serde_json::json!({
            "available": false,
            "reason": "requires_at_least_3_independent_cases",
            "independent_case_clusters": task_means.len(),
        });
    }

    let mut seed = 0xcbf29ce484222325_u64;
    for value in task_means {
        seed ^= value.to_bits();
        seed = seed.wrapping_mul(0x100000001b3);
    }
    if seed == 0 {
        seed = 0x9e3779b97f4a7c15;
    }
    let mut draws = Vec::with_capacity(RESAMPLES);
    for _ in 0..RESAMPLES {
        let mut total = 0.0;
        for _ in 0..task_means.len() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            total += task_means[(seed % task_means.len() as u64) as usize];
        }
        draws.push(total / task_means.len() as f64);
    }
    draws.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let percentile = |p: f64| {
        let rank = p * (draws.len() - 1) as f64;
        let lower = rank.floor() as usize;
        let upper = rank.ceil() as usize;
        let fraction = rank - lower as f64;
        draws[lower] * (1.0 - fraction) + draws[upper] * fraction
    };
    serde_json::json!({
        "available": true,
        "method": "deterministic case-cluster percentile bootstrap",
        "confidence_level": 0.95,
        "resamples": RESAMPLES,
        "independent_case_clusters": task_means.len(),
        "lower": percentile(0.025),
        "upper": percentile(0.975),
    })
}

/// Matched per-cell statistics; positive wall deltas mean Team took longer.
pub(super) fn paired_cell_deltas_json(
    single_runs: &[&ProductEvalRun],
    multi_runs: &[&ProductEvalRun],
    include_cell_details: bool,
    configuration_aligned: bool,
) -> serde_json::Value {
    use std::collections::{BTreeMap, BTreeSet};

    fn by_key<'a>(
        runs: &[&'a ProductEvalRun],
        mode: AgentMode,
    ) -> (BTreeMap<(String, u32), &'a ProductEvalRun>, bool) {
        let mut rows = BTreeMap::new();
        let mut duplicates = false;
        for run in runs.iter().filter(|run| run.key.agent_mode == mode) {
            let key = (run.key.case_id.clone(), run.key.repetition);
            if rows.insert(key, *run).is_some() {
                duplicates = true;
            }
        }
        (rows, duplicates)
    }
    let (single, single_duplicates) = by_key(single_runs, AgentMode::Single);
    let (multi, multi_duplicates) = by_key(multi_runs, AgentMode::Multi);
    let single_keys = single.keys().cloned().collect::<BTreeSet<_>>();
    let multi_keys = multi.keys().cloned().collect::<BTreeSet<_>>();
    let keys_equal = !single_keys.is_empty() && single_keys == multi_keys;
    let duplicate_cells = single_duplicates || multi_duplicates;
    let mut wall_deltas = Vec::<f64>::new();
    let mut both_passed_wall_deltas = Vec::<f64>::new();
    let mut call_deltas = Vec::<f64>::new();
    let mut token_deltas = Vec::<f64>::new();
    let mut cost_deltas = Vec::<f64>::new();
    let mut token_usage_known_pairs = 0usize;
    let mut cost_known_pairs = 0usize;
    let mut quality_deltas = Vec::<f64>::new();
    let mut single_only_pass = 0usize;
    let mut team_only_pass = 0usize;
    let mut both_pass = 0usize;
    let mut neither_pass = 0usize;
    let mut success_deltas_by_case = BTreeMap::<String, Vec<f64>>::new();
    let mut wall_deltas_by_case = BTreeMap::<String, Vec<f64>>::new();
    let mut cells = Vec::new();
    let mut paired_cells = 0usize;

    for key in single_keys.intersection(&multi_keys) {
        paired_cells += 1;
        let left = single[key];
        let right = multi[key];
        let wall_delta = right.wall_ms as f64 - left.wall_ms as f64;
        let success_delta = (if right.status == RunStatus::Passed {
            1.0
        } else {
            0.0
        }) - (if left.status == RunStatus::Passed {
            1.0
        } else {
            0.0
        });
        success_deltas_by_case
            .entry(key.0.clone())
            .or_default()
            .push(success_delta);
        wall_deltas_by_case
            .entry(key.0.clone())
            .or_default()
            .push(wall_delta);
        let call_delta = right.model_calls as f64 - left.model_calls as f64;
        wall_deltas.push(wall_delta);
        call_deltas.push(call_delta);
        if let (Some(single_tokens), Some(team_tokens)) = (left.total_tokens, right.total_tokens) {
            token_deltas.push(team_tokens as f64 - single_tokens as f64);
            token_usage_known_pairs += 1;
        }
        if let (Some(single_cost), Some(team_cost)) = (left.cost_usd, right.cost_usd) {
            if single_cost.is_finite()
                && single_cost >= 0.0
                && team_cost.is_finite()
                && team_cost >= 0.0
            {
                cost_deltas.push(team_cost - single_cost);
                cost_known_pairs += 1;
            }
        }
        if left.checker_total > 0 && right.checker_total > 0 {
            let single_quality = left.checker_passed as f64 / left.checker_total as f64;
            let team_quality = right.checker_passed as f64 / right.checker_total as f64;
            quality_deltas.push(team_quality - single_quality);
        }
        match (
            left.status == RunStatus::Passed,
            right.status == RunStatus::Passed,
        ) {
            (true, true) => {
                both_pass += 1;
                both_passed_wall_deltas.push(wall_delta);
            }
            (true, false) => single_only_pass += 1,
            (false, true) => team_only_pass += 1,
            (false, false) => neither_pass += 1,
        }
        if include_cell_details {
            cells.push(serde_json::json!({
            "case_id": key.0.clone(),
            "repetition": key.1,
            "single_status": left.status,
            "team_status": right.status,
            "team_minus_single_wall_ms": wall_delta,
            "team_minus_single_model_calls": call_delta,
            "single_total_tokens": left.total_tokens,
            "team_total_tokens": right.total_tokens,
            "team_minus_single_total_tokens": match (left.total_tokens, right.total_tokens) {
                (Some(single_tokens), Some(team_tokens)) => Some(team_tokens as f64 - single_tokens as f64),
                _ => None,
            },
            "single_cost_usd": left.cost_usd,
            "team_cost_usd": right.cost_usd,
            "team_minus_single_cost_usd": match (left.cost_usd, right.cost_usd) {
                (Some(single_cost), Some(team_cost)) => Some(team_cost - single_cost),
                _ => None,
            },
            "single_checker_quality": if left.checker_total > 0 {
                Some(left.checker_passed as f64 / left.checker_total as f64)
            } else {
                None
            },
            "team_checker_quality": if right.checker_total > 0 {
                Some(right.checker_passed as f64 / right.checker_total as f64)
            } else {
                None
            },
            }));
        }
    }

    let task_means = |by_case: &BTreeMap<String, Vec<f64>>| {
        by_case
            .values()
            .map(|values| values.iter().sum::<f64>() / values.len() as f64)
            .collect::<Vec<_>>()
    };
    let task_success_deltas = task_means(&success_deltas_by_case);
    let task_wall_deltas = task_means(&wall_deltas_by_case);
    let task_mean_value = |values: &[f64]| {
        if values.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!(values.iter().sum::<f64>() / values.len() as f64)
        }
    };
    let complete_pairing = keys_equal && !duplicate_cells;
    let token_usage_complete =
        complete_pairing && paired_cells > 0 && token_usage_known_pairs == paired_cells;
    let cost_complete = complete_pairing && paired_cells > 0 && cost_known_pairs == paired_cells;
    if !token_usage_complete {
        token_deltas.clear();
    }
    if !cost_complete {
        cost_deltas.clear();
    }
    let quantiles = |values: &[f64]| {
        if values.is_empty() {
            return serde_json::json!({"mean": null, "median": null, "p95": null});
        }
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let mut sorted = values.to_vec();
        sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        let percentile = |p: f64| {
            if sorted.len() == 1 {
                return sorted[0];
            }
            let rank = p * (sorted.len() - 1) as f64;
            let lo = rank.floor() as usize;
            let hi = rank.ceil() as usize;
            let fraction = rank - lo as f64;
            sorted[lo] * (1.0 - fraction) + sorted[hi] * fraction
        };
        serde_json::json!({
            "mean": mean,
            "median": percentile(0.5),
            "p95": percentile(0.95),
        })
    };
    serde_json::json!({
        "paired_cells": paired_cells,
        "keys_equal": keys_equal,
        "duplicate_cells": duplicate_cells,
        "valid_complete_pairing": complete_pairing,
        "token_usage_coverage": {
            "known_pairs": token_usage_known_pairs,
            "paired_cells": paired_cells,
            "complete": token_usage_complete,
        },
        "cost_coverage": {
            "known_pairs": cost_known_pairs,
            "paired_cells": paired_cells,
            "complete": cost_complete,
        },
        "single_only_pass": single_only_pass,
        "team_only_pass": team_only_pass,
        "both_pass": both_pass,
        "neither_pass": neither_pass,
        "net_success_rate_delta": if paired_cells == 0 {
            serde_json::Value::Null
        } else {
            serde_json::json!((team_only_pass as f64 - single_only_pass as f64) / paired_cells as f64)
        },
        "team_minus_single_wall_ms_all": quantiles(&wall_deltas),
        "team_minus_single_wall_ms_both_passed": quantiles(&both_passed_wall_deltas),
        "team_minus_single_model_calls": quantiles(&call_deltas),
        "team_minus_single_total_tokens": quantiles(&token_deltas),
        "team_minus_single_cost_usd": quantiles(&cost_deltas),
        "team_minus_single_checker_quality": quantiles(&quality_deltas),
        "uncertainty": {
            "estimand": "equal-weight mean of per-case paired differences",
            "configuration_aligned": configuration_aligned,
            "success_rate_delta_task_balanced_mean": task_mean_value(&task_success_deltas),
            "wall_ms_delta_task_balanced_mean": task_mean_value(&task_wall_deltas),
            "success_rate_delta_case_cluster_bootstrap_95_ci": paired_case_cluster_bootstrap_ci(
                &task_success_deltas,
                complete_pairing,
                configuration_aligned,
            ),
            "wall_ms_delta_case_cluster_bootstrap_95_ci": paired_case_cluster_bootstrap_ci(
                &task_wall_deltas,
                complete_pairing,
                configuration_aligned,
            ),
            "warning": "Repeated runs are clustered by case_id; intervals describe between-case sampling uncertainty and do not establish causal advantage."
        },
        "cell_deltas": if include_cell_details {
            serde_json::Value::Array(cells)
        } else {
            serde_json::Value::Null
        },
    })
}

/// Check the two report sides before presenting aggregate numbers as a matched pair.
/// This validates report/suite/model/batch and (case_id, repetition) alignment; the
/// evaluator binary revision still needs an external freeze binding.
fn paired_run_alignment(
    single: &ProductEvalReport,
    multi: &ProductEvalReport,
    opts: &PairedReportOptions,
) -> serde_json::Value {
    use std::collections::{BTreeMap, BTreeSet};

    let mut reasons = Vec::new();
    if single.suite_hash.trim().is_empty() || single.suite_hash != multi.suite_hash {
        reasons.push("single/multi suite_hash 缺失或不一致".to_string());
    }
    if single
        .run_contract_sha256
        .as_deref()
        .is_none_or(str::is_empty)
        || single.run_contract_sha256 != multi.run_contract_sha256
    {
        reasons.push("single/multi 生效任务、权限、检查器或预算指纹缺失/不一致".to_string());
    }
    if single.execution != "live-agent" || multi.execution != "live-workswarm" {
        reasons.push(format!(
            "执行器不匹配：要求 Single=live-agent、Team=live-workswarm，实际为 Single={}、Team={}",
            single.execution, multi.execution
        ));
    }
    if single
        .evaluator_binary_sha256
        .as_deref()
        .is_none_or(str::is_empty)
        || single.evaluator_binary_sha256 != multi.evaluator_binary_sha256
    {
        reasons.push("Single/Team 评测器二进制身份缺失或不一致".to_string());
    }
    if single
        .provider_endpoint_sha256
        .as_deref()
        .is_none_or(str::is_empty)
        || single.provider_endpoint_sha256 != multi.provider_endpoint_sha256
    {
        reasons.push("Single/Team 模型服务端点身份缺失或不一致".to_string());
    }
    if single.model.as_deref().is_none_or(str::is_empty)
        || single.model != multi.model
        || single.model != opts.model
    {
        reasons.push("single/multi/绑定项 model 缺失或不一致".to_string());
    }
    if single.batch_label.as_deref().is_none_or(str::is_empty)
        || single.batch_label != multi.batch_label
    {
        reasons.push("single/multi batch_label 缺失或不一致".to_string());
    }

    let single_rows = single
        .runs
        .iter()
        .filter(|run| run.key.agent_mode == AgentMode::Single)
        .collect::<Vec<_>>();
    let multi_rows = multi
        .runs
        .iter()
        .filter(|run| run.key.agent_mode == AgentMode::Multi)
        .collect::<Vec<_>>();
    if single_rows.len() != single.runs.len() || multi_rows.len() != multi.runs.len() {
        reasons.push("报告包含不属于该侧的 agent_mode 记录".to_string());
    }
    if single_rows
        .iter()
        .any(|run| run.key.case_id.trim().is_empty())
        || multi_rows
            .iter()
            .any(|run| run.key.case_id.trim().is_empty())
    {
        reasons.push("配对矩阵包含空白 case_id".to_string());
    }
    let single_keys = single_rows
        .iter()
        .map(|run| ((run.key.case_id.clone(), run.key.repetition), *run))
        .collect::<Vec<_>>();
    let multi_keys = multi_rows
        .iter()
        .map(|run| ((run.key.case_id.clone(), run.key.repetition), *run))
        .collect::<Vec<_>>();
    let single_set = single_keys
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<BTreeSet<_>>();
    let multi_set = multi_keys
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<BTreeSet<_>>();
    let duplicates_exist =
        single_set.len() != single_keys.len() || multi_set.len() != multi_keys.len();
    if duplicates_exist {
        reasons.push("存在重复的 (case_id, repetition) 矩阵单元".to_string());
    }
    if single_set.is_empty() || single_set != multi_set {
        reasons.push("single/multi 任务与重复编号集合不一致或为空".to_string());
    }

    let single_by_key = single_keys.into_iter().collect::<BTreeMap<_, _>>();
    let multi_by_key = multi_keys.into_iter().collect::<BTreeMap<_, _>>();
    let mut paired_keys = 0usize;
    let mut run_models_match = true;
    for key in single_set.intersection(&multi_set) {
        paired_keys += 1;
        let left = single_by_key.get(key).and_then(|run| run.model.as_deref());
        let right = multi_by_key.get(key).and_then(|run| run.model.as_deref());
        if left.is_none() || left != right || left != opts.model.as_deref() {
            run_models_match = false;
        }
    }
    if !run_models_match {
        reasons.push("配对运行的有效模型缺失或不一致".to_string());
    }
    let pending_side_mismatch = single
        .pending
        .iter()
        .any(|key| key.agent_mode != AgentMode::Single)
        || multi
            .pending
            .iter()
            .any(|key| key.agent_mode != AgentMode::Multi);
    if pending_side_mismatch {
        reasons.push("pending 列表包含不属于该报告侧的 agent_mode".to_string());
    }
    if !single.pending.is_empty() || !multi.pending.is_empty() {
        reasons.push("配对矩阵仍有未执行单元".to_string());
    }
    serde_json::json!({
        "configuration_aligned": reasons.is_empty(),
        "reasons": reasons,
        "paired_cells": paired_keys,
        "single_cells": single_rows.len(),
        "multi_cells": multi_rows.len(),
        "evaluator_revision_binding": "not present in ProductEvalReport; verify from freeze/git metadata",
    })
}

/// Split a matrix report containing exactly the Single and Multi sides into aligned
/// reports that can be consumed by the paired-report builder.
pub fn split_paired_mode_reports(
    report: &ProductEvalReport,
) -> Result<(ProductEvalReport, ProductEvalReport), ProductEvalError> {
    let split = |mode: AgentMode, execution: &str| {
        let mut side = report.clone();
        side.execution = execution.to_string();
        side.runs.retain(|run| run.key.agent_mode == mode);
        side.pending.retain(|key| key.agent_mode == mode);
        side.metrics = aggregate_metrics(&side.runs);
        side.per_case = aggregate_per_case(&side.runs);
        side
    };
    let single = split(AgentMode::Single, "live-agent");
    let multi = split(AgentMode::Multi, "live-workswarm");
    if single.runs.is_empty() || multi.runs.is_empty() {
        return err("配对拆分要求 Single 与 Multi 都至少有一个运行单元");
    }
    Ok((single, multi))
}

/// 生成三路可直接读取的配对对照报告 JSON（一个包里含全部任务组）。
///
/// 分组：`overall` / 分类 `code|research|document` / 每 `case_id`；
/// 每组含 single/multi 快照（样本数、成功率、质量、耗时）与绑定四元组，
/// 同时保留两侧报告摘要（suite_hash/批次/生成时间）供三路追溯。
pub fn build_paired_report_json(
    single: &ProductEvalReport,
    multi: &ProductEvalReport,
    opts: &PairedReportOptions,
    generated_at: Option<&str>,
) -> serde_json::Value {
    use std::collections::BTreeSet;

    let generated_at = generated_at.unwrap_or(&now_rfc3339()).to_string();
    let bindings = serde_json::json!({
        "model": opts.model.clone(),
        "template": opts.template.clone(),
        "task_set": opts.task_set.clone(),
        "strategy_version": opts.strategy_version,
    });
    let run_alignment = paired_run_alignment(single, multi, opts);
    let configuration_aligned = run_alignment
        .get("configuration_aligned")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let mut pairs: Vec<serde_json::Value> = Vec::new();

    let mut categories: BTreeSet<String> = BTreeSet::new();
    let mut case_ids: BTreeSet<String> = BTreeSet::new();
    for run in single.runs.iter().chain(multi.runs.iter()) {
        categories.insert(run.category.as_str().to_string());
        case_ids.insert(run.key.case_id.clone());
    }

    let mut push_group = |label: &str, filter: &dyn Fn(&ProductEvalRun) -> bool| {
        let single_runs: Vec<&ProductEvalRun> = single
            .runs
            .iter()
            .filter(|r| r.key.agent_mode == AgentMode::Single && filter(r))
            .collect();
        let multi_runs: Vec<&ProductEvalRun> = multi
            .runs
            .iter()
            .filter(|r| r.key.agent_mode == AgentMode::Multi && filter(r))
            .collect();
        if single_runs.is_empty() && multi_runs.is_empty() {
            return;
        }
        pairs.push(serde_json::json!({
            "task_group": label,
            "single": paired_snapshot_json(AgentMode::Single, &single_runs),
            "multi": paired_snapshot_json(AgentMode::Multi, &multi_runs),
            "matched_comparison": paired_cell_deltas_json(
                &single_runs,
                &multi_runs,
                label == "overall",
                configuration_aligned,
            ),
            "bindings": bindings.clone(),
            "generated_at": generated_at,
        }));
    };

    push_group("overall", &|_| true);
    for category in &categories {
        let wanted = category.clone();
        push_group(category, &move |r| r.category.as_str() == wanted);
    }
    for case_id in &case_ids {
        let wanted = case_id.clone();
        push_group(case_id, &move |r| r.key.case_id == wanted);
    }

    serde_json::json!({
        "schema_version": PAIRED_REPORT_SCHEMA_VERSION,
        "generated_at": generated_at,
        "bindings": bindings,
        "run_alignment": run_alignment,
        "single_report": {
            "suite_name": single.suite_name,
            "suite_hash": single.suite_hash,
            "execution": single.execution,
            "run_contract_sha256": single.run_contract_sha256,
            "evaluator_binary_sha256": single.evaluator_binary_sha256,
            "provider_endpoint_sha256": single.provider_endpoint_sha256,
            "batch_label": single.batch_label,
            "generated_at": single.generated_at,
            "metrics": single.metrics,
        },
        "multi_report": {
            "suite_name": multi.suite_name,
            "suite_hash": multi.suite_hash,
            "execution": multi.execution,
            "run_contract_sha256": multi.run_contract_sha256,
            "evaluator_binary_sha256": multi.evaluator_binary_sha256,
            "provider_endpoint_sha256": multi.provider_endpoint_sha256,
            "batch_label": multi.batch_label,
            "generated_at": multi.generated_at,
            "metrics": multi.metrics,
        },
        "pairs": pairs,
    })
}

#[cfg(test)]
mod independent_case_sample_tests {
    use super::*;
    use crate::product_eval::{AgentMode, EvalCategory, MatrixKey};

    fn run(case_id: &str, repetition: u32) -> ProductEvalRun {
        ProductEvalRun {
            key: MatrixKey::new(case_id, AgentMode::Single, repetition),
            category: EvalCategory::Code,
            status: RunStatus::Passed,
            wall_ms: 100,
            executor_wall_ms: 100,
            validation_wall_ms: 0,
            delivery_gate_wall_ms: 0,
            model_calls: 1,
            tool_calls: Some(1),
            prompt_tokens: Some(10),
            completion_tokens: Some(10),
            total_tokens: Some(20),
            cost_usd: None,
            failed_steps: Vec::new(),
            retries: 0,
            cancellations: 0,
            artifact_refs: Vec::new(),
            tool_log: Vec::new(),
            checker_passed: 1,
            checker_total: 1,
            sandbox_rel: None,
            artifact_snapshot_rel: None,
            model: None,
            started_at: String::new(),
            finished_at: String::new(),
            error: None,
            external_evidence: None,
        }
    }

    #[test]
    fn repeated_runs_do_not_count_as_independent_task_samples() {
        let repeated: Vec<_> = (0..30).map(|n| run("same-case", n)).collect();
        let refs: Vec<_> = repeated.iter().collect();
        let snapshot = paired_snapshot_json(AgentMode::Single, &refs);
        assert_eq!(snapshot["runs_total"], 30);
        assert_eq!(snapshot["independent_case_clusters"], 1);
        assert_eq!(snapshot["sample_sufficient"], false);

        let distinct: Vec<_> = (0..30).map(|n| run(&format!("case-{n}"), n)).collect();
        let refs: Vec<_> = distinct.iter().collect();
        let snapshot = paired_snapshot_json(AgentMode::Single, &refs);
        assert_eq!(snapshot["independent_case_clusters"], 30);
        assert_eq!(snapshot["sample_sufficient"], true);
    }
}
