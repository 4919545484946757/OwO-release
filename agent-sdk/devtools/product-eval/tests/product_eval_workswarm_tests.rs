//! WorkSwarm 真实 TeamRun 评测适配器 · 集成测试（走 MatrixRunner 完整链路）。
//!
//! 验证：multi 模式经 MatrixRunner 与单 Agent 使用**同一份**任务定义、输入
//! fixture、权限范围、预算与检查器；journal/报告记录 workswarm 语义字段
//! （retries = 局部 Retry 次数、model_calls = 全员模型调用之和）。
//! 本文件全部使用脚本化 Provider（零网络、确定性）；真实 GLM 对照见
//! scratch-eval-runs 的 live 驱动记录。

use async_trait::async_trait;
use owo_agent_core::gateway::{ChatMessage, ModelOutput, ModelProvider};
use owo_agent_core::team_strategy::TeamSelectionMode;
use owo_agent_product_eval::product_eval::{
    AgentMode, ArtifactChecker, CaseExecutor, EvalCategory, InputFixture, MatrixRunner,
    ProductEvalCase, ProductEvalSuite, ReferenceDryExecutor, RunOptions, RunStatus, SuiteBundle,
    SuiteDefaults, PRODUCT_EVAL_SCHEMA_VERSION,
};
use owo_agent_product_eval::workswarm_executor::{WorkSwarmExecutor, WorkSwarmExecutorConfig};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// 脚本化 Provider（按序回放；先计数后执行 = 已计费口径）
// ---------------------------------------------------------------------------

struct ScriptedProvider {
    outputs: Mutex<VecDeque<String>>,
    calls: AtomicU32,
}

impl ScriptedProvider {
    fn new(outputs: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(
                outputs
                    .iter()
                    .map(|text| (*text).to_string())
                    .collect::<VecDeque<_>>(),
            ),
            calls: AtomicU32::new(0),
        })
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[owo_agent_core::tools::ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let text = self
            .outputs
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front())
            .ok_or_else(|| "脚本输出耗尽".to_string())?;
        Ok(ModelOutput::Text(scripted_response_for_prompt(
            text, _messages,
        )))
    }
}

// ---------------------------------------------------------------------------
// 套件/任务构造（字段公开，直接构造；不需要临时 JSON 文件）
// ---------------------------------------------------------------------------

const ARTIFACT: &str = "out/report.md";
const LEADER_FINAL: &str =
    "# 最终交付\n交付完成：关键结论 A 已核验。\n## 结论\n采纳草稿并修正措辞。";

// —— WorkerOutputV1 契约信封（R4：worker 必须返回结构化 JSON，正文在 artifact.content）——
// 手写 const JSON（r###：正文含 `"##` 序列，需三重 # 终止）；content 内 \n 为
// JSON 转义 = 真实换行，与 LEADER_FINAL 字面一致。
// 十期 · 三路：Document 分类 multi 评测走 document-delivery-v1 模板
//（drafter → content_reviewer → finalizer）；评审结论按共享 WorkerOutputV1 契约放 summary，
// Coordinator 再将其记录为 review Artifact，最终交付选择排除评审角色。

const DRAFTER_CONTRACT: &str = r###"{"status":"done","summary":"初稿完成","artifact":{"kind":"draft","format":"markdown","content":"## 草稿\n关键结论 A 的初稿，结构完整，待评审。"},"evidence":[],"open_issues":[]}"###;
const CONTENT_REVIEWER_CONTRACT: &str = r###"{"status":"done","summary":"评审通过","review_result":{"verdict":"approved","findings":[]},"evidence":[],"open_issues":[]}"###;
const FINALIZER_CONTRACT: &str = r###"{"status":"done","summary":"最终交付","artifact":{"kind":"final","format":"markdown","content":"# 最终交付\n交付完成：关键结论 A 已核验。\n## 结论\n采纳草稿并修正措辞。"},"evidence":[],"open_issues":[]}"###;

fn scripted_response_for_prompt(text: String, messages: &[ChatMessage]) -> String {
    if text != CONTENT_REVIEWER_CONTRACT {
        return text;
    }
    let Some(prompt) = messages
        .iter()
        .filter_map(|message| message.content.as_deref())
        .find(|prompt| prompt.contains("宿主绑定的评审清单（只读）："))
    else {
        return text;
    };
    let marker = "宿主绑定的评审清单（只读）：";
    let Some(manifest_text) = prompt
        .split_once(marker)
        .and_then(|(_, rest)| rest.split_once("。逐项审查").map(|(manifest, _)| manifest))
    else {
        return text;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(manifest_text) else {
        return text;
    };
    let requirement_ids = manifest
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artifact| {
            artifact
                .get("review_requirements")
                .and_then(serde_json::Value::as_array)
        })
        .flatten()
        .filter_map(|requirement| {
            requirement
                .get("requirement_id")
                .and_then(serde_json::Value::as_str)
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "status": "done",
        "summary": "评审通过",
        "review_result": {
            "verdict": "approved",
            "reviewed_requirement_ids": requirement_ids,
            "findings": []
        },
        "evidence": [],
        "open_issues": []
    })
    .to_string()
}

fn ws_case(id: &str) -> ProductEvalCase {
    ProductEvalCase {
        schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
        id: id.to_string(),
        category: EvalCategory::Document,
        title: "WorkSwarm 集成冒烟".to_string(),
        instruction: "依据输入材料产出最终交付物正文。".to_string(),
        inputs: vec![InputFixture {
            path: "inputs/brief.md".to_string(),
            content: "输入材料：关键结论 A。".to_string(),
        }],
        allow_read: vec!["inputs/**".to_string()],
        allow_write: vec!["out/**".to_string()],
        expected_artifacts: vec![ARTIFACT.to_string()],
        checkers: vec![
            ArtifactChecker::Exists {
                path: ARTIFACT.to_string(),
            },
            ArtifactChecker::Contains {
                path: ARTIFACT.to_string(),
                text: "交付完成".to_string(),
            },
            ArtifactChecker::LineCountMin {
                path: ARTIFACT.to_string(),
                min_lines: 2,
            },
        ],
        reference_outputs: BTreeMapCase::reference_outputs(),
        timeout_secs: Some(30),
        max_model_calls: Some(12),
        repetitions: Some(1),
        allow_commands: Vec::new(),
        failure_classes: Vec::new(),
        expected_side_effects: Vec::new(),
        fault_injection: None,
    }
}

/// dry 对照所需的参考输出（ReferenceDryExecutor 原样回放）。
struct BTreeMapCase;

impl BTreeMapCase {
    fn reference_outputs() -> std::collections::BTreeMap<String, String> {
        let mut map = std::collections::BTreeMap::new();
        map.insert(ARTIFACT.to_string(), LEADER_FINAL.to_string());
        map
    }
}

fn bundle_with(case: ProductEvalCase, dir: &Path) -> SuiteBundle {
    SuiteBundle {
        dir: dir.to_path_buf(),
        suite: ProductEvalSuite {
            schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
            name: "ws-it-suite".to_string(),
            description: "WorkSwarm 适配器集成测试套件".to_string(),
            defaults: SuiteDefaults {
                repetitions: 1,
                timeout_secs: 30,
                max_model_calls: 12,
            },
            tasks: Vec::new(),
        },
        cases: vec![case],
    }
}

fn fresh_out(tag: &str) -> PathBuf {
    let out = std::env::temp_dir().join(format!(
        "ws-eval-it-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&out).unwrap();
    out
}

fn workswarm_executor(root: &Path, provider: Arc<ScriptedProvider>) -> WorkSwarmExecutor {
    let mut executor = WorkSwarmExecutor::new(
        provider as Arc<dyn ModelProvider>,
        "scripted-model",
        root.join("teams"),
    );
    executor.config = WorkSwarmExecutorConfig {
        max_turns_per_worker: 6,
        max_retries_on_failure: 1,
        // 本文件脚本化的是完整三角色流水线（producer → critic → leader）：
        // 显式 ForceTeam 保持既有语义（auto 模式的自适应裁剪见 team_strategy_tests）。
        selection: TeamSelectionMode::ForceTeam,
    };
    executor
}

fn run_report(report: &owo_agent_product_eval::product_eval::ProductEvalReport) {
    for run in &report.runs {
        println!(
            "cell {} → {:?} wall={}ms calls={} retries={} artifacts={:?} failed={:?}",
            run.key,
            run.status,
            run.wall_ms,
            run.model_calls,
            run.retries,
            run.artifact_refs,
            run.failed_steps
        );
    }
}

// ---------------------------------------------------------------------------
// 用例
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_mode_matrix_cell_passes_with_workswarm_semantics() {
    let root = fresh_out("multi-pass");
    let provider = ScriptedProvider::new(&[
        DRAFTER_CONTRACT,
        CONTENT_REVIEWER_CONTRACT,
        FINALIZER_CONTRACT,
    ]);
    let case = ws_case("ws-multi-pass");
    let runner = MatrixRunner::new(bundle_with(case, &root), root.join("out"));
    let report = runner
        .run(
            Arc::new(workswarm_executor(&root, provider.clone())) as Arc<dyn CaseExecutor>,
            "workswarm-it",
            Some("scripted-model".to_string()),
            &RunOptions {
                modes: vec![AgentMode::Multi],
                fresh: true,
                ..RunOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    run_report(&report);

    assert_eq!(report.runs.len(), 1);
    let run = &report.runs[0];
    assert_eq!(run.key.agent_mode, AgentMode::Multi);
    assert_eq!(run.status, RunStatus::Passed, "run = {run:?}");
    assert_eq!(run.artifact_refs, vec![ARTIFACT.to_string()]);
    assert_eq!(
        run.model_calls, 3,
        "drafter+content_reviewer+finalizer 各一次"
    );
    assert_eq!(run.retries, 0);
    assert_eq!(run.cancellations, 0);
    assert!(run.error.is_none());
    assert_eq!(report.metrics.passed, 1);
    assert_eq!(provider.calls(), 3);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn same_runner_parity_between_dry_single_and_workswarm_multi() {
    let root = fresh_out("parity");
    let case = ws_case("ws-parity");

    // 同一 runner：single 模式用 dry 参考回放（零模型），multi 用 WorkSwarm 真实 TeamRun。
    let dry_runner = MatrixRunner::new(bundle_with(case.clone(), &root), root.join("out-dry"));
    let dry_report = dry_runner
        .run(
            Arc::new(ReferenceDryExecutor) as Arc<dyn CaseExecutor>,
            "workswarm-it-dry",
            None,
            &RunOptions {
                modes: vec![AgentMode::Single],
                fresh: true,
                ..RunOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();

    let provider = ScriptedProvider::new(&[
        DRAFTER_CONTRACT,
        CONTENT_REVIEWER_CONTRACT,
        FINALIZER_CONTRACT,
    ]);
    let ws_runner = MatrixRunner::new(bundle_with(case, &root), root.join("out-ws"));
    let ws_report = ws_runner
        .run(
            Arc::new(workswarm_executor(&root, provider.clone())) as Arc<dyn CaseExecutor>,
            "workswarm-it-ws",
            Some("scripted-model".to_string()),
            &RunOptions {
                modes: vec![AgentMode::Multi],
                fresh: true,
                ..RunOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    run_report(&dry_report);
    run_report(&ws_report);

    let dry_run = &dry_report.runs[0];
    let ws_run = &ws_report.runs[0];
    // 同任务/同输入/同权限/同预算/同检查器：状态与产物口径一致（成功率之外不做优劣判定）。
    assert_eq!(dry_run.status, RunStatus::Passed);
    assert_eq!(ws_run.status, RunStatus::Passed);
    assert_eq!(dry_run.artifact_refs, ws_run.artifact_refs);
    assert!(dry_run.failed_steps.is_empty());
    assert!(ws_run.failed_steps.is_empty());
    // 差异只如实记录：workswarm 全员模型调用 ≥ 单代理。
    assert!(ws_run.model_calls >= dry_run.model_calls);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn preset_cancel_at_matrix_level_records_zero_executed_cells() {
    let root = fresh_out("matrix-cancel");
    let provider = ScriptedProvider::new(&[]);
    let case = ws_case("ws-matrix-cancel");
    let runner = MatrixRunner::new(bundle_with(case, &root), root.join("out"));
    let report = runner
        .run(
            Arc::new(workswarm_executor(&root, provider.clone())) as Arc<dyn CaseExecutor>,
            "workswarm-it",
            Some("scripted-model".to_string()),
            &RunOptions {
                modes: vec![AgentMode::Multi],
                fresh: true,
                ..RunOptions::default()
            },
            Arc::new(AtomicBool::new(true)),
        )
        .await
        .unwrap();
    run_report(&report);

    assert!(report.runs.is_empty(), "取消后不得产生已执行单元格");
    assert_eq!(report.metrics.runs_total, 0);
    assert_eq!(report.pending.len(), 1, "未完成单元格进入 pending");
    assert_eq!(provider.calls(), 0, "取消后零计费");
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn retry_semantics_are_persisted_in_journal_run_record() {
    let root = fresh_out("journal-retry");
    // 首次产出为坏契约触发定向修复 + 局部 Retry；随后 3 次成功调用。
    let provider = ScriptedProvider::new(&[
        // 第 1 次尝试：契约 JSON 但 artifact.content 为空 → 角色校验不过 → 一次定向修复；
        // 修复输出仍是自由文本 → 仍不合规 → output_contract_invalid（worker Err，2 次调用）。
        "{\"status\":\"done\",\"summary\":\"x\",\"artifact\":{\"kind\":\"document\",\"format\":\"markdown\",\"content\":\"\"}}",
        "自由文本修复失败样例",
        // 局部 retry 重跑本步骤：3 次契约调用成功（drafter + content_reviewer + finalizer）。
        DRAFTER_CONTRACT,
        CONTENT_REVIEWER_CONTRACT,
        FINALIZER_CONTRACT,
    ]);
    let case = ws_case("ws-journal-retry");
    let runner = MatrixRunner::new(bundle_with(case, &root), root.join("out"));
    let report = runner
        .run(
            Arc::new(workswarm_executor(&root, provider.clone())) as Arc<dyn CaseExecutor>,
            "workswarm-it",
            Some("scripted-model".to_string()),
            &RunOptions {
                modes: vec![AgentMode::Multi],
                fresh: true,
                ..RunOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    run_report(&report);

    let run = &report.runs[0];
    assert_eq!(run.status, RunStatus::Passed, "run = {run:?}");
    assert_eq!(run.retries, 1, "局部 Retry 次数必须入 journal");
    assert_eq!(
        run.model_calls, 5,
        "失败尝试 2 次（turn+repair）+ 成功重跑 3 次"
    );
    let _ = std::fs::remove_dir_all(&root);
}
