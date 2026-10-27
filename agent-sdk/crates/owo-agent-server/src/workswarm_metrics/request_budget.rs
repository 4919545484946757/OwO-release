//! Durable Team model-request admission, reservation accounting, and request observations.
//!
//! This module owns the request-level boundary before a Team Worker can call its
//! Provider: global budget checks, write-ahead journal durability, bounded batch
//! commit, and per-scope usage/wait observations.

use super::util::rfc3339;
use async_trait::async_trait;
use owo_agent_core::gateway::{ModelCallMetadata, ModelProvider, StreamChunk, TokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// ModelProvider 计数装饰器：complete/complete_stream 各计一次（每次模型调用
/// 恰好经过其一），usage_snapshot 透传内层（token 快照差值归因不受影响）。
///
/// Durable write-ahead slots for a TeamRun-wide model request ceiling.
#[derive(Clone)]
pub(crate) struct RequestReservationJournal {
    path: Arc<PathBuf>,
    write_lock: Arc<Mutex<()>>,
    #[cfg(test)]
    durable_batches: Arc<AtomicU64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RequestReservationRecord {
    schema_version: u32,
    reservation_id: String,
    scope_key: String,
    #[serde(default)]
    scope_limit: Option<u64>,
    reserved_at: String,
}

impl RequestReservationJournal {
    pub(crate) fn for_team(run_dir: &Path, team_id: &str) -> Self {
        Self {
            path: Arc::new(run_dir.join(format!("{team_id}-request-reservations.jsonl"))),
            write_lock: Arc::new(Mutex::new(())),
            #[cfg(test)]
            durable_batches: Arc::new(AtomicU64::new(0)),
        }
    }

    #[allow(clippy::type_complexity)] // 快照三元组就地解构，命名类型只增加噪声
    fn reservation_snapshot(
        &self,
    ) -> Result<(u64, HashMap<String, u64>, HashMap<String, u64>), String> {
        let text = match std::fs::read_to_string(&*self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((0, HashMap::new(), HashMap::new()))
            }
            Err(error) => return Err(format!("模型请求预算账本读取失败：{error}")),
        };
        let mut count = 0u64;
        let mut scope_used = HashMap::<String, u64>::new();
        let mut scope_limits = HashMap::<String, u64>::new();
        for (line_number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let record: RequestReservationRecord = serde_json::from_str(line).map_err(|error| {
                format!(
                    "模型请求预算账本第 {} 行损坏：{error}",
                    line_number.saturating_add(1)
                )
            })?;
            if !matches!(record.schema_version, 1 | 2)
                || record.reservation_id.trim().is_empty()
                || record.scope_key.trim().is_empty()
                || record.reserved_at.trim().is_empty()
                || record.scope_limit.is_some_and(|limit| limit == 0)
            {
                return Err(format!(
                    "模型请求预算账本第 {} 行字段无效",
                    line_number.saturating_add(1)
                ));
            }
            if let Some(limit) = record.scope_limit {
                if scope_limits
                    .insert(record.scope_key.clone(), limit)
                    .is_some_and(|existing| existing != limit)
                {
                    return Err(format!(
                        "模型请求预算账本第 {} 行任务限额与既有记录不一致",
                        line_number.saturating_add(1)
                    ));
                }
            }
            count = count.saturating_add(1);
            let used = scope_used.entry(record.scope_key).or_default();
            *used = used.saturating_add(1);
        }
        Ok((count, scope_used, scope_limits))
    }

    pub(crate) fn reservation_count(&self) -> Result<u64, String> {
        self.reservation_snapshot().map(|(count, _, _)| count)
    }

    pub(super) fn append_reservations(
        &self,
        reservations: &[(String, Option<u64>)],
    ) -> Result<(), String> {
        if reservations.is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("模型请求预算账本目录创建失败：{error}"))?;
        }
        let mut lines = Vec::new();
        for (scope_key, scope_limit) in reservations {
            let record = RequestReservationRecord {
                schema_version: 2,
                reservation_id: uuid::Uuid::new_v4().to_string(),
                scope_key: scope_key.clone(),
                scope_limit: *scope_limit,
                reserved_at: rfc3339(),
            };
            lines.extend(
                serde_json::to_vec(&record)
                    .map_err(|error| format!("模型请求预算记录序列化失败：{error}"))?,
            );
            lines.push(b'\n');
        }
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&*self.path)
            .map_err(|error| format!("模型请求预算账本打开失败：{error}"))?;
        file.write_all(&lines)
            .map_err(|error| format!("模型请求预算记录写入失败：{error}"))?;
        file.sync_data()
            .map_err(|error| format!("模型请求预算记录持久化失败：{error}"))?;
        #[cfg(test)]
        self.durable_batches.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn durable_batch_count(&self) -> u64 {
        self.durable_batches.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub(crate) struct TeamModelRequestBudget {
    state: Arc<Mutex<TeamModelRequestBudgetState>>,
    pending_changed: Arc<Condvar>,
    durable_global_limit: bool,
}

struct PendingRequestReservation {
    scope_key: String,
    scope_limit: Option<u64>,
    response: std::sync::mpsc::SyncSender<Result<(), String>>,
}

struct TeamModelRequestBudgetState {
    limit: Option<u64>,
    used: u64,
    scope_used: HashMap<String, u64>,
    scope_limits: HashMap<String, u64>,
    journal: RequestReservationJournal,
    pending: VecDeque<PendingRequestReservation>,
    flushing: bool,
    journal_error: Option<String>,
}

const REQUEST_RESERVATION_BATCH_MAX: usize = 64;
const REQUEST_RESERVATION_PENDING_MAX: usize = 256;
const REQUEST_RESERVATION_COALESCE_WINDOW: std::time::Duration =
    std::time::Duration::from_millis(1);

fn request_budget_exhausted(used: u64, limit: u64) -> String {
    format!(
        "team_model_call_budget_exhausted:已预留 {} 次模型请求，TeamRun 上限为 {}",
        used, limit
    )
}

fn task_request_budget_exhausted(scope_key: &str, used: u64, limit: u64) -> String {
    format!(
        "task_model_call_budget_exhausted:执行范围 {scope_key} 已预留 {used} 次模型请求，宿主限额为 {limit}"
    )
}

impl TeamModelRequestBudget {
    pub(crate) fn new(limit: u64, journal: RequestReservationJournal) -> Result<Self, String> {
        Self::new_with_global_limit(Some(limit), journal)
    }

    /// Enforce TaskGraph attempt ceilings when no TeamRun-wide durable ceiling was requested.
    /// Capped attempts still use batched durable reservations so recovery cannot reset them.
    pub(crate) fn new_scoped_only(journal: RequestReservationJournal) -> Result<Self, String> {
        Self::new_with_global_limit(None, journal)
    }

    fn new_with_global_limit(
        limit: Option<u64>,
        journal: RequestReservationJournal,
    ) -> Result<Self, String> {
        let (used, scope_used, scope_limits) = journal.reservation_snapshot()?;
        Ok(Self {
            state: Arc::new(Mutex::new(TeamModelRequestBudgetState {
                limit,
                used,
                scope_used,
                scope_limits,
                journal,
                pending: VecDeque::new(),
                flushing: false,
                journal_error: None,
            })),
            pending_changed: Arc::new(Condvar::new()),
            durable_global_limit: limit.is_some(),
        })
    }

    pub(crate) fn requires_durable_reservation(&self, scope_limit: Option<u64>) -> bool {
        self.durable_global_limit || scope_limit.is_some()
    }

    /// Requests arriving together share one append/fsync batch. Each provider call
    /// still receives its own durable journal record before it may reach the provider.
    /// If the process crashes after the batch is synced, unused reservations are
    /// conservatively charged, preserving the existing write-ahead budget contract.
    pub(crate) fn reserve(
        &self,
        scope_key: &str,
        requested_scope_limit: Option<u64>,
    ) -> Result<(), String> {
        let (response_tx, response_rx) = std::sync::mpsc::sync_channel(1);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(error) = &state.journal_error {
            return Err(error.clone());
        }
        if state.limit.is_some_and(|limit| state.used >= limit) {
            return Err(request_budget_exhausted(
                state.used,
                state.limit.unwrap_or(u64::MAX),
            ));
        }
        let scope_limit = match (
            state.scope_limits.get(scope_key).copied(),
            requested_scope_limit,
        ) {
            (Some(existing), Some(requested)) if existing != requested => {
                return Err(format!(
                    "task_model_call_budget_changed:执行范围 {scope_key} 的限额从 {existing} 变为 {requested}"
                ));
            }
            (Some(existing), _) => Some(existing),
            (None, requested) => requested,
        };
        if scope_limit == Some(0) {
            return Err(task_request_budget_exhausted(
                scope_key,
                state.scope_used.get(scope_key).copied().unwrap_or(0),
                0,
            ));
        }
        let scope_used = state.scope_used.get(scope_key).copied().unwrap_or(0);
        if let Some(limit) = scope_limit {
            if scope_used >= limit {
                return Err(task_request_budget_exhausted(scope_key, scope_used, limit));
            }
            state.scope_limits.insert(scope_key.to_string(), limit);
        }
        if state.limit.is_none() && scope_limit.is_none() {
            let used = state.scope_used.entry(scope_key.to_string()).or_default();
            *used = used.saturating_add(1);
            state.used = state.used.saturating_add(1);
            return Ok(());
        }
        while state.pending.len() >= REQUEST_RESERVATION_PENDING_MAX {
            state = self
                .pending_changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
            if let Some(error) = &state.journal_error {
                return Err(error.clone());
            }
            if state.limit.is_some_and(|limit| state.used >= limit) {
                return Err(request_budget_exhausted(
                    state.used,
                    state.limit.unwrap_or(u64::MAX),
                ));
            }
            let scope_used = state.scope_used.get(scope_key).copied().unwrap_or(0);
            if let Some(limit) = scope_limit {
                if scope_used >= limit {
                    return Err(task_request_budget_exhausted(scope_key, scope_used, limit));
                }
            }
        }
        state.pending.push_back(PendingRequestReservation {
            scope_key: scope_key.to_string(),
            scope_limit,
            response: response_tx,
        });
        let should_flush = !state.flushing;
        if should_flush {
            state.flushing = true;
        } else {
            self.pending_changed.notify_one();
        }
        drop(state);

        if should_flush {
            loop {
                let (batch, journal, reservations, decisions) = {
                    let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                    let deadline = Instant::now() + REQUEST_RESERVATION_COALESCE_WINDOW;
                    while state.pending.len() < REQUEST_RESERVATION_BATCH_MAX {
                        let now = Instant::now();
                        if now >= deadline {
                            break;
                        }
                        let remaining = deadline.saturating_duration_since(now);
                        let (next, timeout) = self
                            .pending_changed
                            .wait_timeout(state, remaining)
                            .unwrap_or_else(|error| error.into_inner());
                        state = next;
                        if timeout.timed_out() {
                            break;
                        }
                    }
                    let count = state.pending.len().min(REQUEST_RESERVATION_BATCH_MAX);
                    let batch = state.pending.drain(..count).collect::<Vec<_>>();
                    let global_limit = state.limit.unwrap_or(u64::MAX);
                    let mut global_remaining = global_limit.saturating_sub(state.used);
                    let mut local_scope_used = HashMap::<String, u64>::new();
                    let mut decisions = Vec::with_capacity(batch.len());
                    let mut reservations = Vec::new();
                    for request in &batch {
                        if global_remaining == 0 {
                            decisions
                                .push(Err(request_budget_exhausted(global_limit, global_limit)));
                            continue;
                        }
                        let used = state
                            .scope_used
                            .get(&request.scope_key)
                            .copied()
                            .unwrap_or(0)
                            .saturating_add(
                                local_scope_used
                                    .get(&request.scope_key)
                                    .copied()
                                    .unwrap_or(0),
                            );
                        if let Some(limit) = request.scope_limit {
                            if used >= limit {
                                decisions.push(Err(task_request_budget_exhausted(
                                    &request.scope_key,
                                    used,
                                    limit,
                                )));
                                continue;
                            }
                        }
                        global_remaining = global_remaining.saturating_sub(1);
                        let local = local_scope_used
                            .entry(request.scope_key.clone())
                            .or_default();
                        *local = local.saturating_add(1);
                        reservations.push((request.scope_key.clone(), request.scope_limit));
                        decisions.push(Ok(()));
                    }
                    self.pending_changed.notify_all();
                    (batch, state.journal.clone(), reservations, decisions)
                };

                let append_result = journal.append_reservations(&reservations);
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                match append_result {
                    Ok(()) => {
                        state.used = state.used.saturating_add(reservations.len() as u64);
                        for (scope_key, _) in &reservations {
                            let used = state.scope_used.entry(scope_key.clone()).or_default();
                            *used = used.saturating_add(1);
                        }
                        for (request, decision) in batch.into_iter().zip(decisions) {
                            let _ = request.response.send(decision);
                        }
                    }
                    Err(error) => {
                        state.journal_error = Some(error.clone());
                        for request in batch {
                            let _ = request.response.send(Err(error.clone()));
                        }
                        while let Some(request) = state.pending.pop_front() {
                            let _ = request.response.send(Err(error.clone()));
                        }
                        state.flushing = false;
                        self.pending_changed.notify_all();
                        break;
                    }
                }
                if state.pending.is_empty() {
                    state.flushing = false;
                    self.pending_changed.notify_all();
                    break;
                }
            }
        }

        response_rx
            .recv()
            .map_err(|error| format!("模型请求预算批次响应通道关闭：{error}"))?
    }

    #[cfg(test)]
    pub(crate) fn used(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .used
    }
}

pub(crate) struct RequestUsageCollector {
    by_step: Mutex<HashMap<String, Vec<(ModelCallMetadata, bool)>>>,
    budget_reservation_wait_ms: Mutex<HashMap<String, u64>>,
}

impl Default for RequestUsageCollector {
    fn default() -> Self {
        Self {
            by_step: Mutex::new(HashMap::new()),
            budget_reservation_wait_ms: Mutex::new(HashMap::new()),
        }
    }
}

pub(crate) fn task_budget_metadata(input: &Value) -> (Option<String>, Option<String>, Option<u64>) {
    let task_id = input
        .get("assigned_task_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let workswarm = input.get("_workswarm");
    let attempt_id = workswarm
        .and_then(|metadata| metadata.get("attempt_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let model_call_budget = input
        .get("assigned_model_calls_per_attempt")
        .and_then(Value::as_u64);
    (task_id, attempt_id, model_call_budget)
}

pub(crate) fn request_scope_key(step_id: &str, phase_epoch: Option<u64>) -> String {
    format!("{}#{}", step_id, phase_epoch.unwrap_or(0))
}

/// A TaskGraph cap is per attempt, while metrics and lease observations remain per phase.
/// Length-prefix every free-form identity field so delimiters inside IDs cannot collide.
pub(crate) fn task_attempt_budget_scope_key(
    phase_scope_key: &str,
    task_id: Option<&str>,
    attempt_id: Option<&str>,
) -> String {
    match (task_id, attempt_id) {
        (Some(task_id), Some(attempt_id)) => format!(
            "attempt:{}:{}:{}:{}:{}:{}",
            phase_scope_key.len(),
            phase_scope_key,
            task_id.len(),
            task_id,
            attempt_id.len(),
            attempt_id,
        ),
        _ => phase_scope_key.to_string(),
    }
}

impl RequestUsageCollector {
    pub(crate) fn clear(&self, step_id: &str) {
        self.by_step
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(step_id);
        self.budget_reservation_wait_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(step_id);
    }

    pub(crate) fn record_budget_reservation_wait(&self, step_id: &str, wait_ms: u64) {
        let mut waits = self
            .budget_reservation_wait_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let total = waits.entry(step_id.to_string()).or_default();
        *total = total.saturating_add(wait_ms);
    }

    pub(crate) fn take_budget_reservation_wait(&self, step_id: &str) -> u64 {
        self.budget_reservation_wait_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(step_id)
            .unwrap_or(0)
    }

    pub(crate) fn record(&self, step_id: &str, metadata: ModelCallMetadata, succeeded: bool) {
        self.by_step
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(step_id.to_string())
            .or_default()
            .push((metadata, succeeded));
    }

    pub(crate) fn take(&self, step_id: &str) -> Vec<(ModelCallMetadata, bool)> {
        self.by_step
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(step_id)
            .unwrap_or_default()
    }
}

pub(crate) struct LeaseWaitTracker {
    by_step: Mutex<HashMap<String, u64>>,
}

impl Default for LeaseWaitTracker {
    fn default() -> Self {
        Self {
            by_step: Mutex::new(HashMap::new()),
        }
    }
}

impl LeaseWaitTracker {
    pub(crate) fn record(&self, step_id: &str, wait_ms: u64) {
        self.by_step
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(step_id.to_string(), wait_ms);
    }

    pub(crate) fn take(&self, step_id: &str) -> u64 {
        self.by_step
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(step_id)
            .unwrap_or(0)
    }
}

pub struct MeasuredProvider {
    inner: Arc<dyn ModelProvider>,
    calls: Arc<AtomicU64>,
    request_usage: Option<Arc<RequestUsageCollector>>,
    step_id: Option<String>,
    budget_scope_key: Option<String>,
    team_request_budget: Option<Arc<TeamModelRequestBudget>>,
    request_scope_limit: Option<u64>,
}

impl MeasuredProvider {
    #[cfg(test)]
    pub fn new(inner: Arc<dyn ModelProvider>, calls: Arc<AtomicU64>) -> Self {
        Self {
            inner,
            calls,
            request_usage: None,
            step_id: None,
            budget_scope_key: None,
            team_request_budget: None,
            request_scope_limit: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_request_usage(
        inner: Arc<dyn ModelProvider>,
        calls: Arc<AtomicU64>,
        request_usage: Arc<RequestUsageCollector>,
        step_id: String,
    ) -> Self {
        Self {
            inner,
            calls,
            request_usage: Some(request_usage),
            step_id: Some(step_id),
            budget_scope_key: None,
            team_request_budget: None,
            request_scope_limit: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_request_budget(
        inner: Arc<dyn ModelProvider>,
        calls: Arc<AtomicU64>,
        request_usage: Arc<RequestUsageCollector>,
        scope_key: String,
        team_request_budget: Option<Arc<TeamModelRequestBudget>>,
    ) -> Self {
        Self::new_with_scoped_request_budget(
            inner,
            calls,
            request_usage,
            scope_key,
            team_request_budget,
            None,
        )
    }

    pub(crate) fn new_with_scoped_request_budget(
        inner: Arc<dyn ModelProvider>,
        calls: Arc<AtomicU64>,
        request_usage: Arc<RequestUsageCollector>,
        scope_key: String,
        team_request_budget: Option<Arc<TeamModelRequestBudget>>,
        request_scope_limit: Option<u64>,
    ) -> Self {
        Self {
            inner,
            calls,
            request_usage: Some(request_usage),
            step_id: Some(scope_key.clone()),
            budget_scope_key: Some(scope_key),
            team_request_budget,
            request_scope_limit,
        }
    }

    pub(crate) fn new_with_task_attempt_request_budget(
        inner: Arc<dyn ModelProvider>,
        calls: Arc<AtomicU64>,
        request_usage: Arc<RequestUsageCollector>,
        metric_scope_key: String,
        budget_scope_key: String,
        team_request_budget: Option<Arc<TeamModelRequestBudget>>,
        request_scope_limit: Option<u64>,
    ) -> Self {
        let mut provider = Self::new_with_scoped_request_budget(
            inner,
            calls,
            request_usage,
            metric_scope_key,
            team_request_budget,
            request_scope_limit,
        );
        provider.budget_scope_key = Some(budget_scope_key);
        provider
    }

    async fn reserve_request(&self) -> Result<(), String> {
        if let Some(budget) = &self.team_request_budget {
            let budget = Arc::clone(budget);
            let scope_key = self
                .budget_scope_key
                .clone()
                .or_else(|| self.step_id.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let scope_limit = self.request_scope_limit;
            let started = Instant::now();
            if budget.requires_durable_reservation(scope_limit) {
                let reservation =
                    tokio::task::spawn_blocking(move || budget.reserve(&scope_key, scope_limit))
                        .await;
                if let (Some(collector), Some(step_id)) = (&self.request_usage, &self.step_id) {
                    collector.record_budget_reservation_wait(
                        step_id,
                        started.elapsed().as_millis() as u64,
                    );
                }
                reservation.map_err(|error| format!("模型请求预算预留线程失败：{error}"))??;
            } else {
                budget.reserve(&scope_key, scope_limit)?;
            }
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn record_unknown_request(&self, latency_ms: u64, succeeded: bool) {
        if let (Some(collector), Some(step_id)) = (&self.request_usage, &self.step_id) {
            collector.record(
                step_id,
                ModelCallMetadata {
                    latency_ms: Some(latency_ms),
                    ..ModelCallMetadata::default()
                },
                succeeded,
            );
        }
    }
}

#[async_trait]
impl ModelProvider for MeasuredProvider {
    fn validate_image_input(&self, model: Option<&str>, has_images: bool) -> Result<(), String> {
        // A declaration check does not reserve a request or emit execution observations.
        self.inner.validate_image_input(model, has_images)
    }

    async fn complete(
        &self,
        messages: &[owo_agent_core::ChatMessage],
        tools: &[owo_agent_core::tools::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        self.reserve_request().await?;
        let started = Instant::now();
        let result = self.inner.complete(messages, tools).await;
        self.record_unknown_request(started.elapsed().as_millis() as u64, result.is_ok());
        result
    }

    async fn complete_stream(
        &self,
        messages: &[owo_agent_core::ChatMessage],
        tools: &[owo_agent_core::tools::ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        self.reserve_request().await?;
        let started = Instant::now();
        let result = self.inner.complete_stream(messages, tools, on_delta).await;
        self.record_unknown_request(started.elapsed().as_millis() as u64, result.is_ok());
        result
    }

    async fn complete_with_model(
        &self,
        model: Option<&str>,
        messages: &[owo_agent_core::ChatMessage],
        tools: &[owo_agent_core::tools::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        self.reserve_request().await?;
        let started = Instant::now();
        let result = self.inner.complete_with_model(model, messages, tools).await;
        self.record_unknown_request(started.elapsed().as_millis() as u64, result.is_ok());
        result
    }

    async fn complete_stream_with_model(
        &self,
        model: Option<&str>,
        messages: &[owo_agent_core::ChatMessage],
        tools: &[owo_agent_core::tools::ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        self.reserve_request().await?;
        let started = Instant::now();
        let result = self
            .inner
            .complete_stream_with_model(model, messages, tools, on_delta)
            .await;
        self.record_unknown_request(started.elapsed().as_millis() as u64, result.is_ok());
        result
    }

    async fn complete_stream_with_reasoning(
        &self,
        messages: &[owo_agent_core::ChatMessage],
        tools: &[owo_agent_core::tools::ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        self.reserve_request().await?;
        let started = Instant::now();
        let result = self
            .inner
            .complete_stream_with_reasoning(messages, tools, on_chunk)
            .await;
        self.record_unknown_request(started.elapsed().as_millis() as u64, result.is_ok());
        result
    }

    async fn complete_stream_with_reasoning_and_model(
        &self,
        model: Option<&str>,
        messages: &[owo_agent_core::ChatMessage],
        tools: &[owo_agent_core::tools::ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        self.complete_stream_with_reasoning_and_model_observed(model, messages, tools, on_chunk)
            .await
            .map(|observed| observed.output)
    }

    async fn complete_stream_with_reasoning_and_model_observed(
        &self,
        model: Option<&str>,
        messages: &[owo_agent_core::ChatMessage],
        tools: &[owo_agent_core::tools::ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<owo_agent_core::gateway::ObservedModelOutput, String> {
        self.reserve_request().await?;
        let started = Instant::now();
        match self
            .inner
            .complete_stream_with_reasoning_and_model_observed(model, messages, tools, on_chunk)
            .await
        {
            Ok(mut observed) => {
                observed.metadata.latency_ms = Some(started.elapsed().as_millis() as u64);
                if let (Some(collector), Some(step_id)) = (&self.request_usage, &self.step_id) {
                    collector.record(step_id, observed.metadata.clone(), true);
                }
                Ok(observed)
            }
            Err(error) => {
                self.record_unknown_request(started.elapsed().as_millis() as u64, false);
                Err(error)
            }
        }
    }

    fn usage_snapshot(&self) -> TokenUsage {
        self.inner.usage_snapshot()
    }
}
