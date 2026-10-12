use super::request_budget::{
    request_scope_key, task_budget_metadata, LeaseWaitTracker, RequestUsageCollector,
};
use super::sanitize::*;
use super::util::*;
use async_trait::async_trait;
use owo_agent_core::gateway::{ModelProvider, TokenUsage};
use owo_agent_core::goal::Worker;
use owo_agent_core::workswarm::{TeamCoordinator, WorkSwarmError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

/// 单次 worker 执行（span）指标记录。一条 JSONL 行 = 一个 span。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSpanRecord {
    pub span_id: String,
    pub team_id: String,
    pub member_id: String,
    pub role: String,
    /// 内层 worker 类型（agent / echo / sleep / fail…）。
    pub worker_kind: String,
    pub step_id: String,
    /// TaskGraph identity; absent for legacy role steps and non-TaskGraph workers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Host-issued attempt identity; absent in legacy metric records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    /// Host-issued coordinator generation shared by worker spans in one execution wave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_epoch: Option<u64>,
    /// Host-assigned total request ceiling for this attempt, including output repair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_call_budget: Option<u64>,
    pub started_at: String,
    pub ended_at: String,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub ended_at_ms: u64,
    pub wall_ms: u64,
    /// Time spent constructing this task's bounded Team context slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_assembly_ms: Option<u64>,
    #[serde(default)]
    pub provider_wait_ms: u64,
    /// Durable Team request-budget reservation wait, excluding provider latency.
    #[serde(default)]
    pub budget_reservation_wait_ms: u64,
    #[serde(default)]
    pub lease_wait_ms: u64,
    /// succeeded | failed
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 本次执行的模型调用次数（MeasuredProvider 精确计数；非模型 worker 为 0）。
    #[serde(default)]
    pub model_calls: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// shared_snapshot_serial | unknown_concurrent_overlap | usage_unreported | not_applicable.
    #[serde(default)]
    pub usage_attribution: String,
    /// Per-request IDs/model/usage only; never prompt or response content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<owo_agent_protocol::ModelRequestMetricV1>,
    #[serde(default)]
    pub cost_usd: f64,
    /// Whether this span has complete usage attribution and valid configured prices.
    #[serde(default)]
    pub cost_known: bool,
    /// 该步骤的第几次尝试（1 起；>1 = 返工/重试 span）。
    #[serde(default)]
    pub attempt: u32,
    /// 成功后解析到的本次登记输出 Artifact（版本化）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<SpanArtifact>,
}

/// span 关联的输出 Artifact 摘要（不含内容；内容经 CAS ref 另行读取）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanArtifact {
    pub artifact_id: String,
    pub kind: String,
    pub version: u32,
}

// ---------------------------------------------------------------------------
// JSONL 指标日志（TeamRun 数据目录持久化；重启可读）
// ---------------------------------------------------------------------------

/// TeamRun 指标日志：`<run_dir>/<team_id>-metrics.jsonl`（追加写；读侧逐行解析，坏行跳过）。
#[derive(Clone)]
pub struct MetricsJournal {
    path: Arc<PathBuf>,
    /// 追加写串行化（并发 span 落盘互斥）。
    write_lock: Arc<Mutex<()>>,
}

impl MetricsJournal {
    pub fn for_team(run_dir: &Path, team_id: &str) -> Self {
        Self {
            path: Arc::new(run_dir.join(format!("{team_id}-metrics.jsonl"))),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 追加一条 span 记录（目录不存在自动创建；写失败由调用方记日志不中断运行）。
    pub fn append(&self, record: &WorkerSpanRecord) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_string(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&*self.path)?;
        file.write_all(line.as_bytes())
    }

    /// 读取全部 span 记录（逐行解析；坏行/半行跳过——崩溃中断写安全）。
    pub fn read_records(&self) -> Vec<WorkerSpanRecord> {
        let Ok(text) = std::fs::read_to_string(&*self.path) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                serde_json::from_str(trimmed).ok()
            })
            .collect()
    }

    /// 指定步骤已有的 span 数（尝试序数 = 已有数 + 1）。
    pub(crate) fn count_step_spans(&self, step_id: &str) -> u32 {
        self.read_records()
            .iter()
            .filter(|r| r.step_id == step_id)
            .count() as u32
    }
}

// ---------------------------------------------------------------------------
// MeasuredRoleWorker（角色 worker 指标包装层）
// ---------------------------------------------------------------------------

/// Marks overlapping worker spans so shared provider snapshot deltas are not
/// falsely attributed to one concurrent worker.
#[derive(Default)]
pub(crate) struct UsageAttributionTracker {
    active: Mutex<Vec<Weak<AtomicBool>>>,
}

impl UsageAttributionTracker {
    pub(crate) fn begin(&self) -> Arc<AtomicBool> {
        let current = Arc::new(AtomicBool::new(false));
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        active.retain(|span| span.strong_count() > 0);
        if !active.is_empty() {
            current.store(true, Ordering::Relaxed);
            for span in active.iter().filter_map(Weak::upgrade) {
                span.store(true, Ordering::Relaxed);
            }
        }
        active.push(Arc::downgrade(&current));
        current
    }

    pub(crate) fn finish(&self, current: &Arc<AtomicBool>) {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        active.retain(|span| {
            span.upgrade()
                .map(|active| !Arc::ptr_eq(&active, current))
                .unwrap_or(false)
        });
    }
}

/// 角色 worker 包装层：记录 span 级状态、model_calls 与无重叠时的 provider 快照用量。
/// 并发重叠期间 token/费用留空并标记原因，避免把共享差值错误分配给某个角色。
/// 指标追加落盘到 [MetricsJournal]，写失败仅记 tracing 警告（不中断运行）。
pub struct MeasuredRoleWorker {
    inner: Arc<dyn Worker>,
    coordinator: Arc<TeamCoordinator>,
    journal: MetricsJournal,
    team_id: String,
    member_id: String,
    role: String,
    worker_kind: String,
    /// agent 角色 = 共享 provider（token 快照差值归因）；内置演示 worker = None。
    provider: Option<Arc<dyn ModelProvider>>,
    /// agent 角色 = per-span 模型调用计数（由 AgentSubagentWorker 递增）。
    model_calls: Option<Arc<AtomicU64>>,
    usage_tracker: Arc<UsageAttributionTracker>,
    request_usage: Option<Arc<RequestUsageCollector>>,
    lease_waits: Option<Arc<LeaseWaitTracker>>,
}

fn take_context_assembly_duration_ms(
    coordinator: &TeamCoordinator,
    team_id: &str,
    role: &str,
    step_id: &str,
    task_id: Option<&str>,
    attempt_id: Option<&str>,
    phase_epoch: Option<u64>,
) -> Option<u64> {
    coordinator.take_context_assembly_timing(
        team_id,
        role,
        step_id,
        task_id,
        attempt_id,
        phase_epoch,
    )
}

impl MeasuredRoleWorker {
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    pub fn new(
        inner: Arc<dyn Worker>,
        coordinator: Arc<TeamCoordinator>,
        journal: MetricsJournal,
        team_id: String,
        member_id: String,
        role: String,
        worker_kind: String,
        provider: Option<Arc<dyn ModelProvider>>,
        model_calls: Option<Arc<AtomicU64>>,
    ) -> Self {
        Self::new_with_usage_tracker(
            inner,
            coordinator,
            journal,
            team_id,
            member_id,
            role,
            worker_kind,
            provider,
            model_calls,
            Arc::new(UsageAttributionTracker::default()),
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_usage_tracker(
        inner: Arc<dyn Worker>,
        coordinator: Arc<TeamCoordinator>,
        journal: MetricsJournal,
        team_id: String,
        member_id: String,
        role: String,
        worker_kind: String,
        provider: Option<Arc<dyn ModelProvider>>,
        model_calls: Option<Arc<AtomicU64>>,
        usage_tracker: Arc<UsageAttributionTracker>,
        request_usage: Option<Arc<RequestUsageCollector>>,
        lease_waits: Option<Arc<LeaseWaitTracker>>,
    ) -> Self {
        Self {
            inner,
            coordinator,
            journal,
            team_id,
            member_id,
            role,
            worker_kind,
            provider,
            model_calls,
            usage_tracker,
            request_usage,
            lease_waits,
        }
    }

    /// 成功后解析本次登记的输出 Artifact（该角色当前最高版本）。
    async fn resolve_output_artifact(&self) -> Option<SpanArtifact> {
        let team = self.coordinator.get_team_run(&self.team_id).await.ok()?;
        let space_id = team.project_space_id.as_deref()?;
        let space = self.coordinator.get_project_space(space_id).await.ok()?;
        let artifacts = self.coordinator.list_artifacts(&space).await.ok()?;
        artifacts
            .iter()
            .filter(|a| a.producer == self.member_id)
            .max_by_key(|a| a.version)
            .map(|a| SpanArtifact {
                artifact_id: a.artifact_id.clone(),
                kind: a.kind.clone(),
                version: a.version,
            })
    }
}

#[async_trait]
impl Worker for MeasuredRoleWorker {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn run(&self, input: &Value) -> Result<String, String> {
        let workswarm = input.get("_workswarm");
        let raw_step_id = workswarm
            .and_then(|w| w.get("step_id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let step_id = raw_step_id.to_string();
        let (task_id, attempt_id, model_call_budget) = task_budget_metadata(input);
        let phase_epoch = workswarm
            .and_then(|w| w.get("phase_epoch"))
            .and_then(Value::as_u64);
        let scope_key = request_scope_key(&step_id, phase_epoch);
        let attempt = self.journal.count_step_spans(&step_id).saturating_add(1);
        self.coordinator
            .mark_phase_step_running(&self.team_id, &step_id);
        self.coordinator.record_runtime_event(
            &self.team_id,
            "team.worker.started",
            format!(
                "role={} step_id={} epoch={}",
                self.role,
                step_id,
                scope_key.rsplit('#').next().unwrap_or("0")
            ),
        );
        let started_at_ms = now_ms();
        let started_at = rfc3339();
        let usage_before = self.provider.as_ref().map(|p| p.usage_snapshot());
        let calls_before = self
            .model_calls
            .as_ref()
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0);
        if let Some(collector) = &self.request_usage {
            collector.clear(&scope_key);
        }
        let overlap = self.usage_tracker.begin();
        let started = Instant::now();

        let result = self.inner.run(input).await;
        let context_assembly_ms = take_context_assembly_duration_ms(
            &self.coordinator,
            &self.team_id,
            &self.role,
            &step_id,
            task_id.as_deref(),
            attempt_id.as_deref(),
            phase_epoch,
        );

        let wall_ms = started.elapsed().as_millis() as u64;
        let ended_at_ms = now_ms();
        let ended_at = rfc3339();
        // 先采集快照，再关闭追踪窗口；若下一 Worker 在快照期间启动，双方均标为重叠。
        let usage_after = self
            .provider
            .as_ref()
            .map(|provider| provider.usage_snapshot());
        self.usage_tracker.finish(&overlap);
        let overlapped = overlap.load(Ordering::Relaxed);
        let atomic_call_delta = self
            .model_calls
            .as_ref()
            .map(|c| c.load(Ordering::Relaxed).saturating_sub(calls_before))
            .unwrap_or(0);
        let request_observations = self
            .request_usage
            .as_ref()
            .map(|collector| collector.take(&scope_key))
            .unwrap_or_default();
        let budget_reservation_wait_ms = self
            .request_usage
            .as_ref()
            .map(|collector| collector.take_budget_reservation_wait(&scope_key))
            .unwrap_or(0);
        let model_calls = if self.request_usage.is_some() {
            request_observations.len() as u64
        } else {
            atomic_call_delta
        };
        let request_usage = if model_calls > 0
            && request_observations.len() as u64 == model_calls
            && request_observations
                .iter()
                .all(|(request, _)| request.usage.is_some())
        {
            let mut total = TokenUsage::default();
            for (request, _) in &request_observations {
                if let Some(usage) = &request.usage {
                    total.add(usage);
                }
            }
            Some(total)
        } else {
            None
        };
        let requests = request_observations
            .iter()
            .map(
                |(request, succeeded)| owo_agent_protocol::ModelRequestMetricV1 {
                    request_id: request.request_id.clone(),
                    model: request.model.clone(),
                    usage: request
                        .usage
                        .map(|usage| owo_agent_protocol::ModelTokenUsageV1 {
                            prompt_tokens: usage.prompt_tokens,
                            completion_tokens: usage.completion_tokens,
                            total_tokens: usage.total_tokens,
                        }),
                    latency_ms: request.latency_ms,
                    succeeded: *succeeded,
                },
            )
            .collect::<Vec<_>>();
        let usage_delta = request_usage.or_else(|| match (&usage_before, &usage_after) {
            (Some(before), Some(after)) if !overlapped => {
                let delta = after.saturating_sub(before);
                // 未上报 usage 不伪装为零用量。
                (delta.total_tokens > 0 || delta.prompt_tokens > 0 || delta.completion_tokens > 0)
                    .then_some(delta)
            }
            _ => None,
        });
        let request_usage_complete = model_calls > 0
            && requests.len() as u64 == model_calls
            && request_observations
                .iter()
                .all(|(request, _)| request.usage.is_some());
        let usage_attribution = if self.provider.is_none() {
            "not_applicable"
        } else if request_usage_complete {
            if requests.iter().all(|request| request.request_id.is_some()) {
                "request_id_scoped"
            } else {
                "request_scoped_without_id"
            }
        } else if overlapped {
            "unknown_concurrent_overlap"
        } else if usage_delta.is_some() {
            "shared_snapshot_serial"
        } else {
            "usage_unreported"
        };
        let cost_known = usage_delta.is_some() && configured_token_prices().is_some();
        let cost_usd = if cost_known {
            usage_delta
                .as_ref()
                .map(|usage| estimate_cost_usd(usage.prompt_tokens, usage.completion_tokens))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let (outcome, error) = match &result {
            Ok(_) => ("succeeded", None),
            Err(e) => ("failed", Some(sanitize_text(&truncate_chars(e, 500)))),
        };
        let artifact = if result.is_ok() {
            self.resolve_output_artifact().await
        } else {
            None
        };
        let provider_wait_ms = requests
            .iter()
            .filter_map(|request| request.latency_ms)
            .sum();
        let lease_wait_ms = self
            .lease_waits
            .as_ref()
            .map(|waits| waits.take(&scope_key))
            .unwrap_or(0);
        let record = WorkerSpanRecord {
            span_id: format!("span-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            team_id: self.team_id.clone(),
            member_id: self.member_id.clone(),
            role: self.role.clone(),
            worker_kind: self.worker_kind.clone(),
            step_id,
            task_id,
            attempt_id,
            phase_epoch: workswarm
                .and_then(|value| value.get("phase_epoch"))
                .and_then(Value::as_u64),
            model_call_budget,
            started_at,
            ended_at,
            started_at_ms,
            ended_at_ms,
            wall_ms,
            context_assembly_ms,
            provider_wait_ms,
            budget_reservation_wait_ms,
            lease_wait_ms,
            outcome: outcome.to_string(),
            error,
            model_calls,
            prompt_tokens: usage_delta.as_ref().map(|d| d.prompt_tokens),
            completion_tokens: usage_delta.as_ref().map(|d| d.completion_tokens),
            total_tokens: usage_delta.as_ref().map(|d| d.total_tokens),
            usage_attribution: usage_attribution.to_string(),
            requests,
            cost_usd,
            cost_known,
            attempt,
            artifact,
        };
        for request in &record.requests {
            self.coordinator.record_runtime_event(
                &self.team_id,
                "team.model.request_completed",
                format!(
                    "role={} step_id={} request_id={} model={} latency_ms={} usage_tokens={}",
                    self.role,
                    record.step_id,
                    request.request_id.as_deref().unwrap_or("unknown"),
                    request.model.as_deref().unwrap_or("unknown"),
                    request.latency_ms.unwrap_or(0),
                    request.usage.map(|usage| usage.total_tokens).unwrap_or(0),
                ),
            );
        }
        if record.lease_wait_ms > 0 {
            self.coordinator.record_runtime_event(
                &self.team_id,
                "team.lease.wait_completed",
                format!(
                    "role={} step_id={} wait_ms={}",
                    self.role, record.step_id, record.lease_wait_ms
                ),
            );
        }
        self.coordinator.record_runtime_event(
            &self.team_id,
            "team.worker.finished",
            format!(
                "role={} step_id={} outcome={} wall_ms={}",
                self.role, record.step_id, record.outcome, record.wall_ms
            ),
        );
        if let Err(e) = self.journal.append(&record) {
            tracing::warn!(
                team_id = %self.team_id,
                role = %self.role,
                error = %e,
                "workswarm 指标落盘失败（不影响运行）"
            );
        }
        result
    }
}

// ---------------------------------------------------------------------------
// 聚合与预算
// ---------------------------------------------------------------------------

/// 团队指标聚合：summary（总览）/ roles（按角色）/ workers（span 明细）/ budget（预算状态）。
pub fn aggregate_metrics(team_id: &str, records: &[WorkerSpanRecord], budget: &Value) -> Value {
    let now = now_ms();
    let mut spans = records.to_vec();
    spans.sort_by_key(|r| (r.started_at_ms, r.span_id.clone()));

    let span_count = spans.len() as u64;
    let succeeded_spans = spans.iter().filter(|r| r.outcome == "succeeded").count() as u64;
    let failed_spans = spans.iter().filter(|r| r.outcome == "failed").count() as u64;
    let rework_count = spans.iter().filter(|r| r.attempt > 1).count() as u64;
    let task_budgeted_spans = spans
        .iter()
        .filter(|record| record.model_call_budget.is_some())
        .count() as u64;
    let task_model_call_budget_overruns = spans
        .iter()
        .filter(|record| {
            record
                .model_call_budget
                .is_some_and(|budget| record.model_calls > budget)
        })
        .count() as u64;
    let worker_wall_ms_sum: u64 = spans.iter().map(|r| r.wall_ms).sum();
    let provider_wait_ms_sum: u64 = spans.iter().map(|r| r.provider_wait_ms).sum();
    let budget_reservation_wait_ms_sum: u64 =
        spans.iter().map(|r| r.budget_reservation_wait_ms).sum();
    let lease_wait_ms_sum: u64 = spans.iter().map(|r| r.lease_wait_ms).sum();
    let wall_window_ms = match spans.first() {
        Some(first) => now.saturating_sub(first.started_at_ms),
        None => 0,
    };
    let model_calls: u64 = spans.iter().map(|r| r.model_calls).sum();
    let prompt_tokens = sum_opt(spans.iter().filter_map(|r| r.prompt_tokens));
    let completion_tokens = sum_opt(spans.iter().filter_map(|r| r.completion_tokens));
    let total_tokens = sum_opt(spans.iter().filter_map(|r| r.total_tokens));
    let cost_usd = round6(spans.iter().map(|r| r.cost_usd).sum());
    let cost_known = spans
        .iter()
        .all(|record| record.model_calls == 0 || record.cost_known);
    let slowest = spans
        .iter()
        .max_by_key(|r| (r.wall_ms, r.span_id.clone()))
        .map(|r| {
            json!({
                "span_id": r.span_id,
                "role": r.role,
                "step_id": r.step_id,
                "wall_ms": r.wall_ms,
            })
        })
        .unwrap_or(Value::Null);
    let artifact_versions: BTreeSet<&str> = spans
        .iter()
        .filter_map(|r| r.artifact.as_ref().map(|a| a.artifact_id.as_str()))
        .collect();

    // 按角色聚合（BTreeMap = 角色名稳定排序）。
    #[derive(Default)]
    struct RoleAgg {
        spans: u64,
        succeeded: u64,
        failed: u64,
        rework: u64,
        wall_ms_sum: u64,
        provider_wait_ms_sum: u64,
        budget_reservation_wait_ms_sum: u64,
        lease_wait_ms_sum: u64,
        model_calls: u64,
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
        total_tokens: Option<u64>,
        cost_micro: u64,
        artifacts: BTreeSet<String>,
    }
    let mut roles: BTreeMap<String, RoleAgg> = BTreeMap::new();
    for r in &spans {
        let agg = roles.entry(r.role.clone()).or_default();
        agg.spans += 1;
        if r.outcome == "succeeded" {
            agg.succeeded += 1;
        } else {
            agg.failed += 1;
        }
        if r.attempt > 1 {
            agg.rework += 1;
        }
        agg.wall_ms_sum += r.wall_ms;
        agg.provider_wait_ms_sum += r.provider_wait_ms;
        agg.budget_reservation_wait_ms_sum += r.budget_reservation_wait_ms;
        agg.lease_wait_ms_sum += r.lease_wait_ms;
        agg.model_calls += r.model_calls;
        agg.prompt_tokens = add_opt(agg.prompt_tokens, r.prompt_tokens);
        agg.completion_tokens = add_opt(agg.completion_tokens, r.completion_tokens);
        agg.total_tokens = add_opt(agg.total_tokens, r.total_tokens);
        agg.cost_micro += (r.cost_usd * 1_000_000.0).round() as u64;
        if let Some(a) = &r.artifact {
            agg.artifacts.insert(a.artifact_id.clone());
        }
    }
    let roles_json: Vec<Value> = roles
        .into_iter()
        .map(|(role, agg)| {
            json!({
                "role": role,
                "spans": agg.spans,
                "succeeded": agg.succeeded,
                "failed": agg.failed,
                "rework": agg.rework,
                "wall_ms_sum": agg.wall_ms_sum,
                "provider_wait_ms_sum": agg.provider_wait_ms_sum,
                "budget_reservation_wait_ms_sum": agg.budget_reservation_wait_ms_sum,
                "lease_wait_ms_sum": agg.lease_wait_ms_sum,
                "model_calls": agg.model_calls,
                "prompt_tokens": agg.prompt_tokens,
                "completion_tokens": agg.completion_tokens,
                "total_tokens": agg.total_tokens,
                "cost_usd": round6(agg.cost_micro as f64 / 1_000_000.0),
                "artifact_versions": agg.artifacts.len(),
            })
        })
        .collect();

    let workers_json: Vec<Value> = spans
        .iter()
        .map(|r| serde_json::to_value(r).unwrap_or_else(|_| json!({})))
        .collect();

    json!({
        "team_id": team_id,
        "generated_at": rfc3339(),
        "attribution_note": "model_calls 逐 span 精确；响应提供逐请求 usage 时按 request_id 与 span 精确归因；否则仅在无并发重叠时使用共享 provider 快照，并发重叠且无逐请求 usage 的 token/cost 留空",
        "summary": {
            "span_count": span_count,
            "succeeded_spans": succeeded_spans,
            "failed_spans": failed_spans,
            "rework_count": rework_count,
            "task_budgeted_spans": task_budgeted_spans,
            "task_model_call_budget_overruns": task_model_call_budget_overruns,
            "wall_window_ms": wall_window_ms,
            "worker_wall_ms_sum": worker_wall_ms_sum,
            "provider_wait_ms_sum": provider_wait_ms_sum,
            "budget_reservation_wait_ms_sum": budget_reservation_wait_ms_sum,
            "lease_wait_ms_sum": lease_wait_ms_sum,
            "model_calls": model_calls,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": total_tokens,
            "cost_usd": cost_usd,
            "cost_known": cost_known,
            "slowest_worker": slowest,
            "artifact_versions": artifact_versions.len(),
        },
        "roles": roles_json,
        "workers": workers_json,
        "context_assembly": super::lifecycle::aggregate_context_assembly_metrics(records),
        "execution_epochs": super::execution_epochs::aggregate_execution_epochs(records),
        "budget": budget_state(budget, records, now),
    })
}

/// Attach durable TeamRun request-cap status to the existing metrics response.
pub(crate) fn attach_request_budget_status(
    payload: &mut Value,
    budget: &Value,
    reservations: Result<u64, String>,
) {
    let Some(configured_limit) = budget.get("max_model_calls") else {
        return;
    };
    let Some(output_budget) = payload.get_mut("budget").and_then(Value::as_object_mut) else {
        return;
    };
    output_budget.insert("max_model_calls".to_string(), configured_limit.clone());
    let Some(limit) = configured_limit.as_u64() else {
        output_budget.insert("request_budget_known".to_string(), json!(false));
        output_budget.insert(
            "request_budget_error".to_string(),
            json!("max_model_calls must be a nonnegative integer"),
        );
        return;
    };
    match reservations {
        Ok(used) => {
            output_budget.insert("reserved_model_calls".to_string(), json!(used));
            output_budget.insert(
                "remaining_model_calls".to_string(),
                json!(limit.saturating_sub(used)),
            );
            output_budget.insert("request_budget_known".to_string(), json!(true));
            output_budget.insert("request_limit_reached".to_string(), json!(used >= limit));
        }
        Err(error) => {
            output_budget.insert("request_budget_known".to_string(), json!(false));
            output_budget.insert(
                "request_budget_error".to_string(),
                json!(sanitize_text(&error)),
            );
        }
    }
}

/// 预算状态（对比 TeamRun.budget 的 additive 扩展字段与当前累计指标）。
pub fn budget_state(budget: &Value, records: &[WorkerSpanRecord], now_ms: u64) -> Value {
    let spent = round6(records.iter().map(|r| r.cost_usd).sum());
    let window = records
        .iter()
        .map(|r| r.started_at_ms)
        .min()
        .map(|first| now_ms.saturating_sub(first))
        .unwrap_or(0);
    let reason = budget_exhaustion_reason(budget, records, now_ms);
    let cost_known = records
        .iter()
        .all(|record| record.model_calls == 0 || record.cost_known);
    json!({
        "max_cost_usd": budget.get("max_cost_usd").and_then(Value::as_f64).map(Value::from).or_else(|| budget.get("max_cost_usd").cloned()).unwrap_or(Value::Null),
        "spent_usd": spent,
        "spent_known": cost_known,
        "max_wall_secs": budget.get("max_wall_secs").and_then(Value::as_u64).map(Value::from).or_else(|| budget.get("max_wall_secs").cloned()).unwrap_or(Value::Null),
        "wall_window_ms": window,
        "exceeded": reason.is_some(),
        "reason": reason,
    })
}

fn latest_task_budget_spans(records: &[WorkerSpanRecord]) -> BTreeMap<String, &WorkerSpanRecord> {
    let mut latest = BTreeMap::new();
    for record in records
        .iter()
        .filter(|record| record.model_call_budget.is_some())
    {
        let key = record
            .task_id
            .as_deref()
            .unwrap_or(&record.step_id)
            .to_string();
        let replace = latest.get(&key).is_none_or(|current: &&WorkerSpanRecord| {
            (record.ended_at_ms, record.span_id.as_str())
                > (current.ended_at_ms, current.span_id.as_str())
        });
        if replace {
            latest.insert(key, record);
        }
    }
    latest
}

/// 预算耗尽判定（运行循环预算门 + metrics budget.reason 复查共用）。
///
/// `max_cost_usd`（f64）：累计估算费用 `spent > limit` → 耗尽；
/// `max_wall_secs`（u64，>0 生效）：团队指标窗口（首 span 起 → 现在）超限 → 耗尽；
/// 未配置 → None（不启用该门；与既有 GoalBudget 步数/重试熔断正交互补）。
pub fn budget_exhaustion_reason(
    budget: &Value,
    records: &[WorkerSpanRecord],
    now_ms: u64,
) -> Option<String> {
    if let Some(value) = budget
        .get("max_model_calls")
        .filter(|value| !value.is_null())
    {
        if value.as_u64().is_none() {
            return Some(
                "预算配置无效：max_model_calls 必须是非负整数，已停止调度下一阶段".to_string(),
            );
        }
    }
    if let Some(value) = budget.get("max_cost_usd").filter(|value| !value.is_null()) {
        match value.as_f64() {
            Some(limit) if limit.is_finite() && limit >= 0.0 => {}
            _ => {
                return Some(
                    "预算配置无效：max_cost_usd 必须是有限的非负数字，已停止调度下一阶段"
                        .to_string(),
                );
            }
        }
    }
    if let Some(value) = budget.get("max_wall_secs").filter(|value| !value.is_null()) {
        if value.as_u64().is_none() {
            return Some(
                "预算配置无效：max_wall_secs 必须是非负整数，已停止调度下一阶段".to_string(),
            );
        }
    }
    if let Some(record) = latest_task_budget_spans(records)
        .into_values()
        .find(|record| {
            record
                .model_call_budget
                .is_some_and(|limit| record.model_calls > limit)
        })
    {
        return Some(format!(
            "任务模型请求超出宿主上限：task_id={} observed={} limit={}；已停止调度下一阶段",
            record.task_id.as_deref().unwrap_or("unknown"),
            record.model_calls,
            record.model_call_budget.unwrap_or_default()
        ));
    }
    if let Some(limit) = budget.get("max_cost_usd").and_then(Value::as_f64) {
        let usage_unknown = records.iter().any(|record| {
            record.model_calls > 0
                && (!record.cost_known
                    || !matches!(
                        record.usage_attribution.as_str(),
                        "shared_snapshot_serial"
                            | "request_id_scoped"
                            | "request_scoped_without_id"
                    ))
        });
        if usage_unknown {
            return Some(
                "费用预算无法核验：存在未归因的模型用量（包括并发重叠）；为避免超支，已停止调度下一阶段".to_string(),
            );
        }
        let spent: f64 = records.iter().map(|r| r.cost_usd).sum();
        if spent > limit {
            return Some(format!(
                "费用预算耗尽：累计估算 {spent:.6} USD > 上限 {limit:.6} USD（已停止调度下一阶段；可调整预算后 continue 恢复）"
            ));
        }
    }
    if let Some(secs) = budget
        .get("max_wall_secs")
        .and_then(Value::as_u64)
        .filter(|s| *s > 0)
    {
        let first = records.iter().map(|r| r.started_at_ms).min();
        if let Some(first) = first {
            let window_ms = now_ms.saturating_sub(first);
            if window_ms > secs.saturating_mul(1000) {
                return Some(format!(
                    "墙钟预算耗尽：团队已运行 {:.1}s > 上限 {secs}s（已停止调度下一阶段；可调整预算后 continue 恢复）",
                    window_ms as f64 / 1000.0
                ));
            }
        }
    }
    None
}

/// 运行循环预算门：读 TeamRun 预算 + 指标日志，返回耗尽原因（None = 继续调度）。
pub async fn team_budget_exhaustion(
    coordinator: &Arc<TeamCoordinator>,
    team_id: &str,
) -> Result<Option<String>, WorkSwarmError> {
    let team = coordinator.get_team_run(team_id).await?;
    let journal = MetricsJournal::for_team(coordinator.run_dir(), team_id);
    let records = journal.read_records();
    let now = now_ms();
    if let Some(reason) = budget_exhaustion_reason(&team.budget, &records, now) {
        return Ok(Some(reason));
    }
    // Completed spans do not exist during the first in-flight worker. A persisted
    // team timestamp keeps the same wall deadline across phases and restarts.
    if let Some(secs) = team
        .budget
        .get("max_wall_secs")
        .and_then(Value::as_u64)
        .filter(|s| *s > 0)
    {
        let started = chrono::DateTime::parse_from_rfc3339(&team.created_at)
            .map_err(|error| {
                WorkSwarmError::Run(format!("team wall budget timestamp invalid: {error}"))
            })?
            .timestamp_millis();
        let elapsed = now.saturating_sub(started.max(0) as u64);
        if elapsed > secs.saturating_mul(1000) {
            return Ok(Some(format!(
                "wall budget exhausted: {elapsed}ms exceeds {secs}s; scheduling stopped"
            )));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod measured_provider_tests {
    use super::super::request_budget::MeasuredProvider;
    use async_trait::async_trait;
    use owo_agent_core::gateway::{ModelProvider, StreamChunk};
    use owo_agent_core::tools::ToolSpec;
    use owo_agent_core::{ChatMessage, ModelOutput};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    struct RecordingProvider {
        model: Mutex<Option<String>>,
    }

    #[async_trait]
    impl ModelProvider for RecordingProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            Ok(ModelOutput::Text("ok".to_string()))
        }

        async fn complete_stream_with_reasoning_and_model(
            &self,
            model: Option<&str>,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
            on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
        ) -> Result<ModelOutput, String> {
            *self.model.lock().unwrap() = model.map(str::to_string);
            on_chunk(StreamChunk::Content("ok".to_string()));
            Ok(ModelOutput::Text("ok".to_string()))
        }
    }

    #[tokio::test]
    async fn model_and_reasoning_override_pass_through_exactly_once() {
        let calls = Arc::new(AtomicU64::new(0));
        let inner = Arc::new(RecordingProvider {
            model: Mutex::new(None),
        });
        let measured = MeasuredProvider::new(inner.clone(), calls.clone());
        let mut chunks = Vec::new();
        measured
            .complete_stream_with_reasoning_and_model(
                Some("worker-model"),
                &[ChatMessage::user("task".to_string())],
                &[],
                &mut |chunk| chunks.push(chunk),
            )
            .await
            .unwrap();

        assert_eq!(inner.model.lock().unwrap().as_deref(), Some("worker-model"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(chunks.len(), 1);
    }
}
