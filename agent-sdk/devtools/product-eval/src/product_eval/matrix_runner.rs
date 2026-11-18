//! Sequential paired matrix execution, durable journaling and resumable report updates.

use super::{
    aggregate_metrics, aggregate_per_case, compare_report_statistics, current_executable_sha256,
    err, estimate_cost_from_env, evaluate_all_on_dir, now_rfc3339, permissions_hash,
    preserve_artifact_snapshot, sanitize_rel_path, suite_hash, unbounded_benchmark_calls,
    write_report, AgentMode, CaseExecutor, EvalCategory, ExecContext, MatrixAttempt, MatrixKey,
    ProductEvalCase, ProductEvalError, ProductEvalReport, ProductEvalRun, RawExecOutcome,
    RunDirMeta, RunStatus, SuiteBundle, PRODUCT_EVAL_SCHEMA_VERSION,
};
use crate::external_drivers::{run_registered_driver, ExternalDriverRegistry};
use owo_agent_core::gateway::TokenUsage;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------

/// 矩阵运行选项。
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// 参与对照的拓扑（默认 single+multi）。
    pub modes: Vec<AgentMode>,
    /// CLI 级重复次数覆盖。
    pub reps_override: Option<u32>,
    /// 只跑 id 包含该子串的任务。
    pub only: Option<String>,
    /// 只跑指定分类。
    pub category: Option<EvalCategory>,
    /// 清空 out 目录重跑（唯一允许"重算"的入口；journal 归零）。
    pub fresh: bool,
    /// 批次标签（写入 meta/报告；同目录批次不一致拒绝续跑）。
    pub batch_label: Option<String>,
    /// Hash of the configured model provider endpoint; never stores the URL.
    pub provider_endpoint_sha256: Option<String>,
    /// 附加标签（溯源用）。
    pub tags: Vec<String>,
    /// 验证/修复组：检查器未通过时最多追加的修复尝试次数（0 = 纯单次执行）。
    pub repair_attempts: u32,
    /// 显式注册驱动；None 时 Desktop/LongHorizon 保持默认 deny。
    pub desktop_driver: Option<PathBuf>,
    pub fault_driver: Option<PathBuf>,
    pub driver_config: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            modes: AgentMode::all().to_vec(),
            reps_override: None,
            only: None,
            category: None,
            fresh: false,
            batch_label: None,
            provider_endpoint_sha256: None,
            tags: Vec::new(),
            repair_attempts: 0,
            desktop_driver: None,
            fault_driver: None,
            driver_config: None,
        }
    }
}

/// 过滤后的任务列表。
pub fn filter_cases(bundle: &SuiteBundle, opts: &RunOptions) -> Vec<ProductEvalCase> {
    bundle
        .cases
        .iter()
        .filter(|case| {
            if let Some(only) = &opts.only {
                if !case.id.contains(only.as_str()) {
                    return false;
                }
            }
            if let Some(category) = &opts.category {
                if case.category != *category {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

/// 产品评测矩阵执行器：顺序执行 (case × mode × repetition)，
/// journal 追加落盘 + 断点续跑 + 全量报告（含失败）。
pub struct MatrixRunner {
    pub bundle: SuiteBundle,
    pub out_dir: PathBuf,
    pub external_drivers: Option<ExternalDriverRegistry>,
}

impl MatrixRunner {
    pub fn new(bundle: SuiteBundle, out_dir: impl Into<PathBuf>) -> Self {
        Self {
            bundle,
            out_dir: out_dir.into(),
            external_drivers: None,
        }
    }

    /// Explicitly register external drivers for fail-closed gated cells.
    pub fn with_external_drivers(mut self, registry: ExternalDriverRegistry) -> Self {
        self.external_drivers = Some(registry);
        self
    }

    fn journal_path(&self) -> PathBuf {
        self.out_dir.join("state.jsonl")
    }

    fn meta_path(&self) -> PathBuf {
        self.out_dir.join("meta.json")
    }

    fn failures_dir(&self) -> PathBuf {
        self.out_dir.join("failures")
    }

    fn planned_matrix(&self, cases: &[ProductEvalCase], opts: &RunOptions) -> Vec<MatrixKey> {
        let mut keys = Vec::new();
        for case in cases {
            let reps = case.effective_repetitions(&self.bundle.suite.defaults, opts.reps_override);
            if opts.modes.is_empty() {
                continue;
            }
            for repetition in 0..reps {
                // For a Single/Multi pair, hash the batch, task, and two-repetition
                // block to choose AB or BA, then reverse it in the partner repetition.
                // This is reproducible, randomized block order with exact balance per
                // complete two-repetition block.
                let paired_modes = opts.modes.len() == 2
                    && opts.modes.contains(&AgentMode::Single)
                    && opts.modes.contains(&AgentMode::Multi);
                if paired_modes {
                    let block = repetition / 2;
                    let batch = opts.batch_label.as_deref().unwrap_or_default();
                    let seed_material = format!("{batch}\\0{}\\0{block}", case.id);
                    let digest = Sha256::digest(seed_material.as_bytes());
                    let block_first = (digest[0] & 1) as usize;
                    let first_index = block_first ^ ((repetition % 2) as usize);
                    let first = AgentMode::all()[first_index];
                    let second = AgentMode::all()[1 - first_index];
                    keys.push(MatrixKey::new(case.id.clone(), first, repetition));
                    keys.push(MatrixKey::new(case.id.clone(), second, repetition));
                } else {
                    let offset = (repetition as usize) % opts.modes.len();
                    for index in 0..opts.modes.len() {
                        let mode = opts.modes[(offset + index) % opts.modes.len()];
                        keys.push(MatrixKey::new(case.id.clone(), mode, repetition));
                    }
                }
            }
        }
        keys
    }

    fn run_contract_sha256(&self, cases: &[ProductEvalCase], opts: &RunOptions) -> String {
        let effective_tasks = cases
            .iter()
            .map(|case| {
                serde_json::json!({
                    "case_id": case.id,
                    "category": case.category.as_str(),
                    "repetitions": case.effective_repetitions(
                        &self.bundle.suite.defaults,
                        opts.reps_override,
                    ),
                    "timeout_secs": case.effective_timeout_secs(&self.bundle.suite.defaults),
                    "max_model_calls": case.effective_max_model_calls(&self.bundle.suite.defaults),
                    "permissions_sha256": permissions_hash(case),
                })
            })
            .collect::<Vec<_>>();
        let contract = serde_json::json!({
            "suite_hash": suite_hash(&self.bundle),
            "suite_defaults": self.bundle.suite.defaults,
            "effective_tasks": effective_tasks,
            // 修复轮次改变执行语义：不同值必须视为不同合同，不能续跑到同一 out 目录。
            "repair_attempts": opts.repair_attempts,
        });
        let bytes = serde_json::to_vec(&contract).unwrap_or_default();
        format!("{:x}", Sha256::digest(bytes))
    }

    fn init_or_verify_out_dir(
        &self,
        model: Option<&str>,
        execution: &str,
        opts: &RunOptions,
        run_contract_sha256: &str,
        evaluator_binary_sha256: &str,
        provider_endpoint_sha256: Option<&str>,
    ) -> Result<(), ProductEvalError> {
        let hash = suite_hash(&self.bundle);
        let fresh = opts.fresh;
        if fresh && self.out_dir.exists() {
            std::fs::remove_dir_all(&self.out_dir).map_err(|e| {
                ProductEvalError(format!("清空 {} 失败：{e}", self.out_dir.display()))
            })?;
        }
        std::fs::create_dir_all(&self.out_dir)
            .map_err(|e| ProductEvalError(format!("创建 {} 失败：{e}", self.out_dir.display())))?;
        std::fs::create_dir_all(self.failures_dir()).map_err(|e| {
            ProductEvalError(format!("创建 {} 失败：{e}", self.failures_dir().display()))
        })?;
        let meta_path = self.meta_path();
        if meta_path.exists() {
            let text = std::fs::read_to_string(&meta_path)
                .map_err(|e| ProductEvalError(format!("读取 meta.json 失败：{e}")))?;
            let meta: RunDirMeta = serde_json::from_str(&text)
                .map_err(|e| ProductEvalError(format!("解析 meta.json 失败：{e}")))?;
            if meta.suite_hash != hash || meta.schema_version != PRODUCT_EVAL_SCHEMA_VERSION {
                return err(format!(
                    "out 目录属于另一套件/版本（suite_hash 不一致）：换 --out 或加 --fresh。目录：{}",
                    self.out_dir.display()
                ));
            }
            if meta.model.as_deref() != model {
                return err(format!(
                    "out 目录模型与本次运行不一致：换 --out 或加 --fresh。目录：{}",
                    self.out_dir.display()
                ));
            }
            if meta.execution.as_deref() != Some(execution) {
                return err(format!(
                    "out 目录执行器与本次运行不一致或元数据缺失：换 --out 或加 --fresh。目录：{}",
                    self.out_dir.display()
                ));
            }
            if meta.batch_label != opts.batch_label {
                return err(format!(
                    "out 目录批次标签与本次运行不一致或元数据缺失：换 --out 或加 --fresh。目录：{}",
                    self.out_dir.display()
                ));
            }
            if meta.run_contract_sha256.as_deref() != Some(run_contract_sha256) {
                return err(format!(
                    "out 目录的生效任务/权限/预算指纹缺失或不一致：换 --out 或加 --fresh。目录：{}",
                    self.out_dir.display()
                ));
            }
            if meta.evaluator_binary_sha256.as_deref() != Some(evaluator_binary_sha256) {
                return err(format!(
                    "out 目录评测器二进制身份缺失或不一致：换 --out 或加 --fresh。目录：{}",
                    self.out_dir.display()
                ));
            }
            let driver_identity = self
                .external_drivers
                .as_ref()
                .map(|registry| registry.identity_sha256());
            if meta.external_driver_identity_sha256.as_deref() != driver_identity.as_deref() {
                return err("out 目录外部驱动身份不一致：换 --out 或加 --fresh。".to_string());
            }
            if meta.provider_endpoint_sha256.as_deref() != provider_endpoint_sha256 {
                return err(format!(
                    "out 目录模型服务端点身份与本次运行不一致：换 --out 或加 --fresh。目录：{}",
                    self.out_dir.display()
                ));
            }
        } else {
            if self.journal_path().exists() {
                return err(format!(
                    "out 目录存在 state.jsonl 但缺少 meta.json（来源不明，拒绝续跑）：换 --out 或 --fresh。目录：{}",
                    self.out_dir.display()
                ));
            }
            let meta = RunDirMeta {
                schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
                suite_name: self.bundle.suite.name.clone(),
                suite_hash: hash,
                model: model.map(str::to_string),
                execution: Some(execution.to_string()),
                batch_label: opts.batch_label.clone(),
                tags: opts.tags.clone(),
                run_contract_sha256: Some(run_contract_sha256.to_string()),
                evaluator_binary_sha256: Some(evaluator_binary_sha256.to_string()),
                provider_endpoint_sha256: provider_endpoint_sha256.map(str::to_string),
                external_driver_identity_sha256: self
                    .external_drivers
                    .as_ref()
                    .map(|registry| registry.identity_sha256()),
                created_at: now_rfc3339(),
            };
            let text = serde_json::to_string_pretty(&meta)
                .map_err(|e| ProductEvalError(format!("序列化 meta.json 失败：{e}")))?;
            std::fs::write(&meta_path, text)
                .map_err(|e| ProductEvalError(format!("写入 meta.json 失败：{e}")))?;
        }
        Ok(())
    }

    fn load_runs(&self) -> Result<Vec<ProductEvalRun>, ProductEvalError> {
        let path = self.journal_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| ProductEvalError(format!("读取 {} 失败：{e}", path.display())))?;
        let lines: Vec<&[u8]> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
        let mut runs = Vec::new();
        let mut valid_prefix_len = 0_u64;
        let mut final_record_missing_newline = false;
        for (index, raw_line) in lines.iter().enumerate() {
            let terminated = raw_line.last() == Some(&b'\n');
            let content_end = raw_line.len() - terminated as usize;
            let mut line = &raw_line[..content_end];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                valid_prefix_len = valid_prefix_len.saturating_add(raw_line.len() as u64);
                continue;
            }
            match serde_json::from_slice::<ProductEvalRun>(line) {
                Ok(run) => {
                    runs.push(run);
                    valid_prefix_len = valid_prefix_len.saturating_add(raw_line.len() as u64);
                    final_record_missing_newline = index + 1 == lines.len() && !terminated;
                }
                Err(parse_error) if index + 1 == lines.len() && !terminated => {
                    // A non-newline-terminated invalid record is the only tolerated
                    // torn tail. Remove it now so future appends cannot turn it into
                    // unrecoverable middle-of-file corruption.
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .open(&path)
                        .map_err(|e| {
                            ProductEvalError(format!(
                                "修复撕裂 journal {} 失败：{e}",
                                path.display()
                            ))
                        })?;
                    file.set_len(valid_prefix_len).map_err(|e| {
                        ProductEvalError(format!("截断撕裂 journal {} 失败：{e}", path.display()))
                    })?;
                    file.sync_all().map_err(|e| {
                        ProductEvalError(format!(
                            "同步修复后的 journal {} 失败：{e}",
                            path.display()
                        ))
                    })?;
                    tracing::warn!(
                        line = index + 1,
                        error = %parse_error,
                        "journal 尾行撕裂，已截断到最后有效记录（对应单元格将重跑）"
                    );
                    break;
                }
                Err(parse_error) => {
                    return err(format!(
                        "state.jsonl 第 {} 行损坏：{parse_error}（拒绝静默丢弃历史记录；如确要重跑请 --fresh）",
                        index + 1
                    ));
                }
            }
        }
        if final_record_missing_newline {
            // A complete JSON record may survive a crash just before its delimiter.
            // Normalize it before a later append so it stays a separate journal row.
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .map_err(|e| {
                    ProductEvalError(format!("修复 journal 分隔符 {} 失败：{e}", path.display()))
                })?;
            file.write_all(b"\n").map_err(|e| {
                ProductEvalError(format!("补齐 journal 分隔符 {} 失败：{e}", path.display()))
            })?;
            file.sync_all().map_err(|e| {
                ProductEvalError(format!("同步 journal 分隔符 {} 失败：{e}", path.display()))
            })?;
        }
        Ok(runs)
    }

    fn append_run(&self, run: &ProductEvalRun) -> Result<(), ProductEvalError> {
        let line = serde_json::to_string(run)
            .map_err(|e| ProductEvalError(format!("序列化运行记录失败：{e}")))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.journal_path())
            .map_err(|e| ProductEvalError(format!("打开 journal 失败：{e}")))?;
        writeln!(file, "{line}")
            .map_err(|e| ProductEvalError(format!("追加 journal 失败：{e}")))?;
        file.flush()
            .map_err(|e| ProductEvalError(format!("刷新 journal 失败：{e}")))?;
        Ok(())
    }

    /// 执行矩阵（断点续跑）。返回聚合了全部历史记录（含失败）的报告。
    pub async fn run(
        &self,
        executor: Arc<dyn CaseExecutor>,
        execution: &str,
        model: Option<String>,
        opts: &RunOptions,
        cancel: Arc<AtomicBool>,
    ) -> Result<ProductEvalReport, ProductEvalError> {
        let cases = filter_cases(&self.bundle, opts);
        if cases.is_empty() {
            return err("过滤条件下没有可执行的任务");
        }
        if opts.modes.is_empty() {
            return err("运行模式列表不能为空");
        }
        if opts
            .modes
            .iter()
            .enumerate()
            .any(|(index, mode)| opts.modes[..index].contains(mode))
        {
            return err("运行模式列表不能包含重复模式");
        }
        let run_contract_sha256 = self.run_contract_sha256(&cases, opts);
        let evaluator_binary_sha256 = current_executable_sha256().ok_or_else(|| {
            ProductEvalError("无法读取当前评测器二进制，拒绝生成无来源绑定的运行报告".to_string())
        })?;
        self.init_or_verify_out_dir(
            model.as_deref(),
            execution,
            opts,
            &run_contract_sha256,
            &evaluator_binary_sha256,
            opts.provider_endpoint_sha256.as_deref(),
        )?;
        let mut runs = self.load_runs()?;
        let completed: std::collections::BTreeSet<MatrixKey> =
            runs.iter().map(|run| run.key.clone()).collect();
        let planned = self.planned_matrix(&cases, opts);
        let pending: Vec<MatrixKey> = planned
            .iter()
            .filter(|key| !completed.contains(key))
            .cloned()
            .collect();

        let defaults = self.bundle.suite.defaults.clone();
        let total_planned = planned.len();
        tracing::info!(
            suite = %self.bundle.suite.name,
            planned = total_planned,
            done = runs.len(),
            todo = pending.len(),
            execution = execution,
            "product-eval 矩阵开始"
        );

        for key in &pending {
            if cancel.load(Ordering::Relaxed) {
                tracing::info!("收到取消信号，停止调度后续单元格（已完成记录保留）");
                break;
            }
            let case = cases
                .iter()
                .find(|case| case.id == key.case_id)
                .ok_or_else(|| ProductEvalError(format!("矩阵键引用未知任务 {}", key.case_id)))?;
            let timeout_secs = case.effective_timeout_secs(&defaults);
            let max_model_calls = case.effective_max_model_calls(&defaults);

            // 沙盒：输入 fixture 预写。
            let sandbox = self.out_dir.join("sandboxes").join(format!(
                "{}-{}",
                key.slug().replace('#', "__"),
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&sandbox)
                .map_err(|e| ProductEvalError(format!("创建沙盒失败：{e}")))?;
            let mut setup_error = None;
            for input in &case.inputs {
                let target = sandbox.join(sanitize_rel_path(&input.path).unwrap_or_default());
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&target, &input.content) {
                    setup_error = Some(format!("写输入 fixture {} 失败：{e}", input.path));
                    break;
                }
            }

            let started_at = now_rfc3339();
            let cell_started = Instant::now();
            let mut execution_wall_ms = None;
            let mut executor_wall_ms = 0_u64;
            let (status, failed_steps, artifact_refs, outcome, checker_passed, checker_total) =
                if let Some(setup_error) = setup_error {
                    (
                        RunStatus::Error,
                        vec![format!("setup:{setup_error}")],
                        Vec::new(),
                        RawExecOutcome::default(),
                        0,
                        0,
                    )
                } else {
                    // 验证/修复组：检查器未通过时，把失败项作为反馈再跑一次，最多
                    // opts.repair_attempts 次。单 Agent 复用同一沙盒（能看到上一轮产物）；
                    // WorkSwarm 每次尝试是新的 TeamRun，反馈经团队目标注入。预算按剩余
                    // 模型调用数收紧，避免修复轮无限放大成本；超时/取消/执行器错误不修复。
                    let mut attempt: u32 = 0;
                    let mut repair_feedback: Option<String> = None;
                    let mut attempt_notes: Vec<String> = Vec::new();
                    let mut artifacts: Vec<String> = Vec::new();
                    let mut total_calls = 0_u32;
                    let mut total_usage = TokenUsage::default();
                    let mut usage_known = true;
                    let mut total_tool_calls: Option<u32> = Some(0);
                    let mut validation_wall_ms = 0_u64;
                    let mut delivery_gate_wall_ms = 0_u64;
                    let mut retries = 0_u32;
                    let mut tool_log: Vec<String> = Vec::new();
                    loop {
                        let attempt_budget = max_model_calls.saturating_sub(total_calls).max(1);
                        let mut ctx = ExecContext::new(
                            case,
                            &sandbox,
                            key.agent_mode,
                            timeout_secs,
                            attempt_budget,
                            Arc::clone(&cancel),
                        );
                        ctx.matrix_attempt = Some(MatrixAttempt {
                            key: &key,
                            execution,
                            batch_label: opts.batch_label.as_deref(),
                            repair_index: attempt,
                            repair_limit: opts.repair_attempts,
                        });
                        ctx.set_external_drivers_registered(self.external_drivers.is_some());
                        if let Some(feedback) = repair_feedback.take() {
                            ctx.set_repair_feedback(feedback);
                        }
                        let execution_started = Instant::now();
                        let future = executor.execute(&mut ctx);
                        let (
                            attempt_status,
                            attempt_steps,
                            attempt_artifacts,
                            mut attempt_outcome,
                            attempt_passed,
                            attempt_total,
                        ) = match tokio::time::timeout(Duration::from_secs(timeout_secs), future)
                            .await
                        {
                            Ok(mut outcome) => {
                                if let Some(registry) = &self.external_drivers {
                                    run_registered_driver(registry, &mut ctx, opts, &mut outcome);
                                }
                                let (artifacts, mut steps) = ctx.take_records();
                                if outcome.aborted {
                                    steps.push("cancelled:收到取消信号".to_string());
                                }
                                let (status, checker_steps, checker_passed, checker_total) =
                                    if outcome.aborted {
                                        (RunStatus::Cancelled, Vec::new(), 0, 0)
                                    } else if let Some(error) = &outcome.error {
                                        steps.push(format!("executor:{error}"));
                                        (RunStatus::Error, Vec::new(), 0, 0)
                                    } else {
                                        // Team 全栈路径在 finalize_success 前已对同一工作区执行
                                        // 宿主检查器，复用通过结果，避免重复运行 command_check。
                                        let checks = ctx.progress().prevalidated_checkers();
                                        let (passed, total, failed) = match checks {
                                            Some((passed, total)) => (passed, total, Vec::new()),
                                            None => {
                                                let validation_started = Instant::now();
                                                let result =
                                                    evaluate_all_on_dir(&case.checkers, &sandbox)
                                                        .await;
                                                outcome.validation_wall_ms =
                                                    outcome.validation_wall_ms.saturating_add(
                                                        validation_started.elapsed().as_millis()
                                                            as u64,
                                                    );
                                                result
                                            }
                                        };
                                        if failed.is_empty() {
                                            (RunStatus::Passed, Vec::new(), passed, total)
                                        } else {
                                            (RunStatus::Failed, failed, passed, total)
                                        }
                                    };
                                steps.extend(checker_steps);
                                (
                                    status,
                                    steps,
                                    artifacts,
                                    outcome,
                                    checker_passed,
                                    checker_total,
                                )
                            }
                            Err(_) => {
                                execution_wall_ms = Some(cell_started.elapsed().as_millis() as u64);
                                let (mut artifacts, mut steps) = ctx.take_records();
                                steps.push(format!("timeout:超过 {timeout_secs}s 上限"));
                                if unbounded_benchmark_calls() {
                                    // Freeze the Team workspace into the eval sandbox at the exact cutoff.
                                    // Preserve live telemetry even though the executor future is cancelled.
                                    match ctx.progress.copy_expected_artifacts(case, &sandbox) {
                                        Ok(partial_artifacts) => {
                                            for artifact in partial_artifacts {
                                                if !artifacts.contains(&artifact) {
                                                    artifacts.push(artifact);
                                                }
                                            }
                                        }
                                        Err(error) => {
                                            steps.push(format!("partial_snapshot:{error}"))
                                        }
                                    }
                                    let mut partial_outcome = ctx.progress.snapshot();
                                    let validation_started = Instant::now();
                                    let (passed, total, failed) =
                                        evaluate_all_on_dir(&case.checkers, &sandbox).await;
                                    partial_outcome.validation_wall_ms =
                                        partial_outcome.validation_wall_ms.saturating_add(
                                            validation_started.elapsed().as_millis() as u64,
                                        );
                                    steps.extend(
                                        failed
                                            .into_iter()
                                            .map(|failure| format!("partial_completion:{failure}")),
                                    );
                                    (
                                        RunStatus::Timeout,
                                        steps,
                                        artifacts,
                                        partial_outcome,
                                        passed,
                                        total,
                                    )
                                } else {
                                    (
                                        RunStatus::Timeout,
                                        steps,
                                        artifacts,
                                        RawExecOutcome::default(),
                                        0,
                                        0,
                                    )
                                }
                            }
                        };
                        executor_wall_ms += execution_started.elapsed().as_millis() as u64;
                        total_calls = total_calls.saturating_add(attempt_outcome.model_calls);
                        total_usage.add(&attempt_outcome.usage);
                        usage_known = usage_known && attempt_outcome.usage_known;
                        total_tool_calls = match (total_tool_calls, attempt_outcome.tool_calls) {
                            (Some(seen), Some(calls)) => Some(seen.saturating_add(calls)),
                            _ => None,
                        };
                        validation_wall_ms =
                            validation_wall_ms.saturating_add(attempt_outcome.validation_wall_ms);
                        delivery_gate_wall_ms = delivery_gate_wall_ms
                            .saturating_add(attempt_outcome.delivery_gate_wall_ms);
                        retries = retries.saturating_add(attempt_outcome.retries);
                        tool_log.append(&mut attempt_outcome.tool_log);
                        for artifact in attempt_artifacts {
                            if !artifacts.contains(&artifact) {
                                artifacts.push(artifact);
                            }
                        }
                        let can_repair = matches!(attempt_status, RunStatus::Failed)
                            && attempt < opts.repair_attempts
                            && !attempt_steps.is_empty()
                            && total_calls < max_model_calls
                            && !cancel.load(Ordering::Relaxed);
                        if can_repair {
                            attempt += 1;
                            attempt_notes.push(format!(
                                "repair:{attempt}（上一轮检查器未通过：{}）",
                                attempt_steps.join("; ")
                            ));
                            let failures = attempt_steps
                                .iter()
                                .map(|step| format!("- {step}"))
                                .collect::<Vec<_>>()
                                .join("\n");
                            repair_feedback = Some(format!(
                                "\n\n## 修复请求（第 {attempt} 次）\n上一轮产物未通过以下检查：\n{failures}\n请修正这些检查项后重新完成；不要删除或降低已有的正确内容。\n"
                            ));
                            continue;
                        }
                        let mut steps = std::mem::take(&mut attempt_notes);
                        steps.extend(attempt_steps);
                        let outcome = RawExecOutcome {
                            aborted: attempt_outcome.aborted,
                            error: attempt_outcome.error.clone(),
                            model_calls: total_calls,
                            tool_calls: total_tool_calls,
                            usage: total_usage,
                            usage_known,
                            validation_wall_ms,
                            delivery_gate_wall_ms,
                            retries,
                            tool_log,
                            external_evidence: attempt_outcome.external_evidence.clone(),
                        };
                        break (
                            attempt_status,
                            steps,
                            artifacts,
                            outcome,
                            attempt_passed,
                            attempt_total,
                        );
                    }
                };
            let mut status = status;
            let mut failed_steps = failed_steps;
            let wall_ms =
                execution_wall_ms.unwrap_or_else(|| cell_started.elapsed().as_millis() as u64);

            let mut run_error = outcome.error.clone();
            if let Some(error) =
                self.validate_external_evidence(case, key, execution, opts, &outcome)
            {
                if matches!(status, RunStatus::Passed) {
                    status = RunStatus::Error;
                    failed_steps.push(format!("external_driver:{error}"));
                    run_error = Some(error);
                }
            }
            let artifact_snapshot_rel = if matches!(status, RunStatus::Passed) {
                match preserve_artifact_snapshot(&sandbox, &self.out_dir, key, &artifact_refs) {
                    Ok(snapshot_rel) => snapshot_rel,
                    Err(snapshot_error) => {
                        status = RunStatus::Error;
                        failed_steps.push(format!("artifact_snapshot:{snapshot_error}"));
                        run_error = Some(format!("成功产物留证失败：{snapshot_error}"));
                        None
                    }
                }
            } else {
                None
            };

            // 失败运行保留完整沙盒；成功运行只保留登记产物快照。
            let sandbox_rel = if matches!(status, RunStatus::Passed) {
                let _ = std::fs::remove_dir_all(&sandbox);
                None
            } else {
                let keep = self.failures_dir().join(key.slug().replace('#', "__"));
                let _ = std::fs::remove_dir_all(&keep);
                let _ = std::fs::rename(&sandbox, &keep);
                Some(
                    keep.strip_prefix(&self.out_dir)
                        .unwrap_or(&keep)
                        .to_string_lossy()
                        .replace('\\', "/"),
                )
            };

            let cost_usd = if outcome.usage_known {
                estimate_cost_from_env(&outcome.usage)
            } else {
                None
            };
            let run = ProductEvalRun {
                key: key.clone(),
                category: case.category,
                status,
                wall_ms,
                executor_wall_ms,
                validation_wall_ms: outcome.validation_wall_ms,
                delivery_gate_wall_ms: outcome.delivery_gate_wall_ms,
                model_calls: outcome.model_calls,
                tool_calls: outcome.tool_calls,
                prompt_tokens: outcome.usage_known.then_some(outcome.usage.prompt_tokens),
                completion_tokens: outcome
                    .usage_known
                    .then_some(outcome.usage.completion_tokens),
                total_tokens: outcome.usage_known.then_some(outcome.usage.total_tokens),
                cost_usd,
                failed_steps,
                retries: outcome.retries,
                cancellations: if outcome.aborted { 1 } else { 0 },
                artifact_refs,
                tool_log: outcome.tool_log,
                checker_passed,
                checker_total,
                sandbox_rel,
                artifact_snapshot_rel,
                model: model.clone(),
                started_at,
                finished_at: now_rfc3339(),
                error: run_error,
                external_evidence: outcome.external_evidence.clone(),
            };
            self.append_run(&run)?;
            runs.push(run);
            let report = self.build_report(
                &runs,
                &cases,
                opts,
                execution,
                &model,
                &evaluator_binary_sha256,
                opts.provider_endpoint_sha256.as_deref(),
            )?;
            write_report(&self.out_dir, &report)?;
            tracing::info!(
                cell = %runs.last().map(|r| r.key.slug()).unwrap_or_default(),
                status = ?runs.last().map(|r| r.status),
                wall_ms,
                "product-eval 单元格完成"
            );
        }

        self.build_report(
            &runs,
            &cases,
            opts,
            execution,
            &model,
            &evaluator_binary_sha256,
            opts.provider_endpoint_sha256.as_deref(),
        )
    }

    fn validate_external_evidence(
        &self,
        case: &ProductEvalCase,
        key: &MatrixKey,
        execution: &str,
        opts: &RunOptions,
        outcome: &RawExecOutcome,
    ) -> Option<String> {
        let required = super::external_execution_required(case)?;
        let Some(registry) = &self.external_drivers else {
            return Some(required.to_string());
        };
        let Some(evidence) = &outcome.external_evidence else {
            return Some(required.to_string());
        };
        registry
            .validate(
                case,
                key,
                execution,
                opts.batch_label.as_deref(),
                opts.repair_attempts,
                evidence,
            )
            .err()
    }

    #[allow(clippy::too_many_arguments)]
    fn build_report(
        &self,
        runs: &[ProductEvalRun],
        cases: &[ProductEvalCase],
        opts: &RunOptions,
        execution: &str,
        model: &Option<String>,
        evaluator_binary_sha256: &str,
        provider_endpoint_sha256: Option<&str>,
    ) -> Result<ProductEvalReport, ProductEvalError> {
        let completed: std::collections::BTreeSet<MatrixKey> =
            runs.iter().map(|run| run.key.clone()).collect();
        let planned = self.planned_matrix(cases, opts);
        let pending: Vec<MatrixKey> = planned
            .iter()
            .filter(|key| !completed.contains(key))
            .cloned()
            .collect();
        let comparison = compare_report_statistics(runs, &pending);
        Ok(ProductEvalReport {
            schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
            suite_name: self.bundle.suite.name.clone(),
            suite_hash: suite_hash(&self.bundle),
            execution: execution.to_string(),
            model: model.clone(),
            batch_label: opts.batch_label.clone(),
            tags: opts.tags.clone(),
            repair_attempts: opts.repair_attempts,
            run_contract_sha256: Some(self.run_contract_sha256(cases, opts)),
            evaluator_binary_sha256: Some(evaluator_binary_sha256.to_string()),
            provider_endpoint_sha256: provider_endpoint_sha256.map(str::to_string),
            generated_at: now_rfc3339(),
            runs: runs.to_vec(),
            pending,
            metrics: aggregate_metrics(runs),
            per_case: aggregate_per_case(runs),
            comparison: Some(comparison),
        })
    }
}
