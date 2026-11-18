//! 统一 Task Runtime：任务创建、状态迁移、执行后端分发、独立验证与启动恢复。
//!
//! 本模块是**外层协调层**：不替代 Agent Turn、Goal、WorkSwarm、Computer Use 或
//! CloudExec 的内部实现，只给它们提供统一的身份、状态、事件、审批和交付模型。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::model::*;
use super::store::TaskStore;

/// 创建任务请求。
#[derive(Debug, Clone)]
pub struct CreateTaskRequest {
    pub title: Option<String>,
    pub user_goal: String,
    pub execution_strategy: ExecutionStrategy,
    pub budget: TaskBudget,
    pub policy_profile: String,
    pub workspace_ref: Option<String>,
    pub session_refs: Vec<String>,
    /// 后端专属输入（不透明 JSON），原样交给执行后端。
    pub payload: Option<Value>,
}

impl CreateTaskRequest {
    pub fn new(user_goal: impl Into<String>) -> Self {
        Self {
            title: None,
            user_goal: user_goal.into(),
            execution_strategy: ExecutionStrategy::SingleAgent,
            budget: TaskBudget::default(),
            policy_profile: "workspace".to_string(),
            workspace_ref: None,
            session_refs: Vec::new(),
            payload: None,
        }
    }
}

/// 后端运行标识（由各 Adapter 定义，Runtime 只做透传）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendRunId(pub String);

/// 交给执行后端的不可变上下文。
#[derive(Debug, Clone)]
pub struct TaskRunContext {
    pub task_id: TaskId,
    pub attempt: u32,
    pub user_goal: String,
    pub workspace_ref: Option<String>,
    pub session_refs: Vec<String>,
    pub budget: TaskBudget,
    /// 后端专属输入（不透明 JSON，Runtime 不解释）。
    pub payload: Option<Value>,
    pub approved_workflow_revision: Option<Value>,
    /// 创建任务时的策略档位（如 `workspace`、`workspace-execute`）。后端据此决定
    /// 无交互审批通道下允许哪些副作用，不得自行放宽 Policy 的判定。
    pub policy_profile: String,
}

/// 后端一次执行结束后提交的事实（不含"成功"结论）。
///
/// 后端只能报告"跑了什么、有什么证据"，是否 `succeeded` 由独立验证决定。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendRunReport {
    pub backend_run_id: String,
    /// 本次执行使用的会话（便于前端跳转/复现）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub steps: usize,
    #[serde(default)]
    pub tool_calls: usize,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    /// 宿主完成判定的线上字符串（Accepted / Candidate / Unverified / Blocked / Aborted）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_record: Option<Value>,
    /// 回合最终文本（截断后入库，避免事件表膨胀）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_text: Option<String>,
}

/// 事件与检查点里保留的最终文本上限。
const REPORT_TEXT_LIMIT: usize = 2_000;

/// 后端状态快照（用于 inspect / 恢复判断）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendSnapshot {
    pub state: String,
    pub detail: Value,
}

/// 执行后端契约（Agent V2 §6 任务 2.2）。
///
/// 不要求所有后端都实现所有操作；不能实现时返回 `UNSUPPORTED`，不能伪装成功。
/// 后端不得自行把统一 Task 标记为 Succeeded——只提交执行事实和产物。
#[async_trait::async_trait]
pub trait TaskExecutionBackend: Send + Sync {
    fn kind(&self) -> ExecutionStrategy;

    async fn start(&self, context: TaskRunContext) -> TaskResult<BackendRunId>;

    async fn pause(&self, _run_id: &BackendRunId) -> TaskResult<()> {
        Err(unsupported_error(self.kind(), "pause"))
    }

    async fn resume(&self, _run_id: &BackendRunId) -> TaskResult<()> {
        Err(unsupported_error(self.kind(), "resume"))
    }

    async fn cancel(&self, _run_id: &BackendRunId) -> TaskResult<()> {
        Err(unsupported_error(self.kind(), "cancel"))
    }

    async fn inspect(&self, _run_id: &BackendRunId) -> TaskResult<BackendSnapshot> {
        Err(unsupported_error(self.kind(), "inspect"))
    }
}

fn unsupported_error(strategy: ExecutionStrategy, operation: &str) -> TaskError {
    TaskError::new(
        codes::UNSUPPORTED,
        format!(
            "执行后端 {} 不支持 {operation}（不得伪装成功）",
            strategy.as_str()
        ),
    )
}

// ----------------------------------------------------------------------
// Verification Pipeline（§7 任务 3.5）
// ----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckResult {
    pub passed: bool,
    pub detail: String,
}

impl CheckResult {
    pub fn pass(detail: impl Into<String>) -> Self {
        Self {
            passed: true,
            detail: detail.into(),
        }
    }

    pub fn fail(detail: impl Into<String>) -> Self {
        Self {
            passed: false,
            detail: detail.into(),
        }
    }
}

/// 五类验证证据。缺省表示"没有该证据"，不能算通过。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationEvidence {
    /// 结构验证：文件存在、格式正确、字段齐全。
    pub structural: Option<CheckResult>,
    /// 执行验证：测试、命令、编译、运行结果。
    pub execution: Option<CheckResult>,
    /// 目标验证：是否满足用户目标和 Acceptance Criteria（必须有）。
    pub goal: Option<CheckResult>,
    /// 副作用验证：桌面操作后真实状态是否改变。
    pub side_effect: Option<CheckResult>,
    /// 独立复核：独立检查器或第二阶段模型复核。
    pub review: Option<CheckResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedCheck {
    pub name: String,
    pub present: bool,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub task_id: String,
    pub verified: bool,
    pub checks: Vec<NamedCheck>,
    pub summary: String,
}

/// 纯函数验证：所有提供的检查都必须通过；`goal` 必须存在且通过。
pub fn evaluate_evidence(task_id: &str, evidence: &VerificationEvidence) -> VerificationReport {
    let mut checks = Vec::new();
    for (name, check) in [
        ("structural", &evidence.structural),
        ("execution", &evidence.execution),
        ("goal", &evidence.goal),
        ("side_effect", &evidence.side_effect),
        ("review", &evidence.review),
    ] {
        match check {
            Some(result) => checks.push(NamedCheck {
                name: name.to_string(),
                present: true,
                passed: result.passed,
                detail: result.detail.clone(),
            }),
            None => checks.push(NamedCheck {
                name: name.to_string(),
                present: false,
                passed: false,
                detail: "缺少该维度证据".to_string(),
            }),
        }
    }
    let goal_present = evidence.goal.is_some();
    let goal_passed = evidence
        .goal
        .as_ref()
        .map(|check| check.passed)
        .unwrap_or(false);
    let provided_all_pass = checks
        .iter()
        .filter(|check| check.present)
        .all(|check| check.passed);
    let verified = goal_present && goal_passed && provided_all_pass;
    let summary = if verified {
        "验证通过（目标与已提供的证据全部通过）".to_string()
    } else if !goal_present {
        "验证未通过：缺少目标验证证据".to_string()
    } else if !goal_passed {
        "验证未通过：目标验证失败".to_string()
    } else {
        "验证未通过：至少一项证据检查失败".to_string()
    };
    VerificationReport {
        task_id: task_id.to_string(),
        verified,
        checks,
        summary,
    }
}

/// 恢复扫描报告。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub scanned: usize,
    pub interrupted: Vec<String>,
    pub needs_user_input: Vec<String>,
    pub requeued: Vec<String>,
    pub unchanged: Vec<String>,
}

// ----------------------------------------------------------------------
// Runtime
// ----------------------------------------------------------------------

pub struct TaskRuntime {
    store: Arc<TaskStore>,
    backends: RwLock<HashMap<ExecutionStrategy, Arc<dyn TaskExecutionBackend>>>,
    /// task_id → 正在运行的后端运行标识（取消时据此通知后端）。
    runs: RwLock<HashMap<String, BackendRunId>>,
}

impl TaskRuntime {
    pub fn new(store: TaskStore) -> Self {
        Self {
            store: Arc::new(store),
            backends: RwLock::new(HashMap::new()),
            runs: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_store(store: Arc<TaskStore>) -> Self {
        Self {
            store,
            backends: RwLock::new(HashMap::new()),
            runs: RwLock::new(HashMap::new()),
        }
    }

    pub fn store(&self) -> Arc<TaskStore> {
        Arc::clone(&self.store)
    }

    /// 注册执行后端。重复注册同一策略时后者覆盖前者。
    pub fn register_backend(&self, backend: Arc<dyn TaskExecutionBackend>) {
        if let Ok(mut backends) = self.backends.write() {
            backends.insert(backend.kind(), backend);
        }
    }

    pub fn backend_registered(&self, strategy: ExecutionStrategy) -> bool {
        self.backends
            .read()
            .map(|backends| backends.contains_key(&strategy))
            .unwrap_or(false)
    }

    fn prepare_task(request: CreateTaskRequest) -> TaskResult<TaskRecord> {
        let goal = request.user_goal.trim();
        if goal.is_empty() {
            return Err(TaskError::new(
                codes::EMPTY_GOALS,
                "用户目标为空，拒绝创建任务",
            ));
        }
        let title = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| first_line(goal));
        let mut record = TaskRecord::new(
            title,
            goal,
            request.execution_strategy,
            request.budget,
            request.policy_profile,
        );
        record.workspace_ref = request.workspace_ref;
        record.session_refs = request.session_refs;
        record.payload = request.payload;
        record.state = TaskState::Queued;
        Ok(record)
    }

    /// Durable admission, including the task and both initial events, in one transaction.
    pub fn create_task_idempotent(
        &self,
        request: CreateTaskRequest,
        key: &str,
        fingerprint: &Value,
    ) -> TaskResult<(TaskRecord, bool)> {
        if key.trim().is_empty() {
            return Err(TaskError::new(
                "INVALID_REQUEST",
                "Admission key must not be empty",
            ));
        }
        let record = Self::prepare_task(request)?;
        self.store.create_task_idempotent(&record, key, fingerprint)
    }

    pub fn create_task(&self, request: CreateTaskRequest) -> TaskResult<TaskRecord> {
        let record = Self::prepare_task(request)?;
        self.store.create_task(&record)?;
        // 创建事件已经在 create_task 内写入；这里补一条排队事件，让时间线可直接读。
        self.store.append_event(
            record.task_id.as_str(),
            event_type::TASK_QUEUED,
            0,
            json!({"execution_strategy": record.execution_strategy.as_str()}),
            None,
        )?;
        self.get_task(record.task_id.as_str())
    }

    pub fn get_task(&self, task_id: &str) -> TaskResult<TaskRecord> {
        self.store
            .get_task(task_id)?
            .ok_or_else(|| TaskError::not_found(task_id))
    }

    pub fn list_tasks(
        &self,
        state: Option<TaskState>,
        limit: usize,
        offset: usize,
    ) -> TaskResult<Vec<TaskRecord>> {
        self.store.list_tasks(state, limit, offset)
    }

    pub fn events(
        &self,
        task_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> TaskResult<Vec<TaskEvent>> {
        self.get_task(task_id)?;
        self.store.list_events(task_id, after_sequence, limit)
    }

    pub fn artifacts(&self, task_id: &str) -> TaskResult<Vec<TaskArtifact>> {
        self.get_task(task_id)?;
        self.store.list_artifacts(task_id)
    }

    pub fn latest_checkpoint(&self, task_id: &str) -> TaskResult<Option<TaskCheckpoint>> {
        self.store.latest_checkpoint(task_id)
    }

    /// 开始任务：Queued/Draft/Planning/Paused/Interrupted/NeedsUserInput → Running。
    /// 返回任务记录与"是否注册了后端"。未注册后端时调用方必须显示为未分发，
    /// 不能把状态变化当成执行已经开始。
    pub fn start_task(&self, task_id: &str) -> TaskResult<(TaskRecord, bool)> {
        let record =
            self.store
                .transition(task_id, TaskState::Running, Some("任务开始执行"), None)?;
        self.store.bump_attempt(task_id)?;
        let registered = self.backend_registered(record.execution_strategy);
        // `transition` 进入 Running 时已经写过 `task.execution.started`；
        // 这里不再重复追加同一类型事件（时间线里会出现两条一模一样的记录）。
        // 执行策略与是否分发由 `task.backend.started` / `task.backend.unsupported` 表达。
        Ok((self.get_task(task_id)?, registered))
    }

    /// 分发到已注册的后端。没有后端时写 `task.backend.unsupported` 并返回 `UNSUPPORTED`。
    pub async fn dispatch_start(&self, task_id: &str) -> TaskResult<BackendRunId> {
        self.dispatch_start_with_revision(task_id, None).await
    }

    async fn dispatch_start_with_revision(
        &self,
        task_id: &str,
        approved_workflow_revision: Option<Value>,
    ) -> TaskResult<BackendRunId> {
        let task = self.get_task(task_id)?;
        let backend = self
            .backends
            .read()
            .ok()
            .and_then(|backends| backends.get(&task.execution_strategy).cloned());
        let Some(backend) = backend else {
            self.store.append_event(
                task_id,
                "task.backend.unsupported",
                task.current_attempt,
                json!({"execution_strategy": task.execution_strategy.as_str()}),
                None,
            )?;
            return Err(TaskError::new(
                codes::UNSUPPORTED,
                format!(
                    "执行后端 {} 尚未注册；任务已进入 running，但没有开始执行",
                    task.execution_strategy.as_str()
                ),
            ));
        };
        let context = TaskRunContext {
            task_id: task.task_id.clone(),
            attempt: task.current_attempt,
            user_goal: task.user_goal.clone(),
            workspace_ref: task.workspace_ref.clone(),
            session_refs: task.session_refs.clone(),
            budget: task.budget.clone(),
            payload: task.payload.clone(),
            approved_workflow_revision,
            policy_profile: task.policy_profile.clone(),
        };
        let run_id = backend.start(context).await?;
        if let Ok(mut runs) = self.runs.write() {
            runs.insert(task_id.to_string(), run_id.clone());
        }
        self.store.append_event(
            task_id,
            "task.backend.started",
            task.current_attempt,
            json!({
                "execution_strategy": task.execution_strategy.as_str(),
                "backend_run_id": run_id.0,
            }),
            None,
        )?;
        Ok(run_id)
    }

    /// 当前正在运行的后端运行标识（没有则为 `None`）。
    pub fn backend_run_id(&self, task_id: &str) -> Option<BackendRunId> {
        self.runs
            .read()
            .ok()
            .and_then(|runs| runs.get(task_id).cloned())
    }

    /// 提交后端执行完成的事实：登记检查点，并把任务从 `Running` 推进到
    /// `Verifying`——**不会**直接进入 `Succeeded`。
    ///
    /// 任务已被取消/终态时只记录事件，不改状态；后端迟到的结果不能复活任务。
    pub fn complete_backend_run(
        &self,
        task_id: &str,
        report: BackendRunReport,
    ) -> TaskResult<TaskRecord> {
        let task = self.get_task(task_id)?;
        self.forget_run(task_id);
        self.store.append_event(
            task_id,
            event_type::TASK_BACKEND_COMPLETED,
            task.current_attempt,
            report_payload(&report),
            Some(&report.backend_run_id),
        )?;
        match task.state {
            TaskState::Running => {
                let snapshot_ref = report
                    .session_id
                    .clone()
                    .map(|session_id| format!("session:{session_id}"))
                    .unwrap_or_else(|| format!("task:{task_id}"));
                let mut metadata = report_payload(&report);
                if let Some(object) = metadata.as_object_mut() {
                    object.insert("attempt".to_string(), json!(task.current_attempt));
                }
                let checkpoint = TaskCheckpoint {
                    checkpoint_id: uuid::Uuid::new_v4().to_string(),
                    task_id: task.task_id.clone(),
                    attempt: task.current_attempt,
                    created_at: Utc::now().to_rfc3339(),
                    phase: "backend_completed".to_string(),
                    state_snapshot_ref: snapshot_ref,
                    workspace_revision: None,
                    artifact_refs: Vec::new(),
                    metadata,
                };
                self.store.create_checkpoint(&checkpoint)?;
                self.store.transition(
                    task_id,
                    TaskState::Verifying,
                    Some("执行完成，提交执行事实，等待独立验证"),
                    None,
                )
            }
            TaskState::CancelRequested => self.store.transition(
                task_id,
                TaskState::Cancelled,
                Some("任务取消：后端已停止"),
                Some("任务取消：后端已停止"),
            ),
            _ => Ok(task),
        }
    }

    /// 提交后端执行失败：记录事件；`Running` → `Failed`，取消路径保持取消语义。
    pub fn fail_backend_run(
        &self,
        task_id: &str,
        code: &str,
        message: &str,
    ) -> TaskResult<TaskRecord> {
        let task = self.get_task(task_id)?;
        self.forget_run(task_id);
        // 暂停态的"后端停止"不是失败：用户主动暂停后后端收手是预期行为。
        let event = if task.state == TaskState::Paused {
            "task.backend.paused"
        } else {
            event_type::TASK_BACKEND_FAILED
        };
        self.store.append_event(
            task_id,
            event,
            task.current_attempt,
            json!({"code": code, "message": truncate_text(message, REPORT_TEXT_LIMIT)}),
            None,
        )?;
        match task.state {
            TaskState::Running => {
                self.store
                    .transition(task_id, TaskState::Failed, Some(message), Some(message))
            }
            TaskState::CancelRequested => self.store.transition(
                task_id,
                TaskState::Cancelled,
                Some("任务取消：后端已停止"),
                Some("任务取消：后端已停止"),
            ),
            _ => Ok(task),
        }
    }

    /// 后端需要的输入/授权还没到位（例如 computer-use 任务必须先经人工批准）：
    /// `Running` → `NeedsUserInput`，用户处理后可以再次 `start` 重试。
    ///
    /// 这不是失败：不写 `failed`、不清空尝试预算，事件里保留稳定 `code`。
    pub fn require_user_input(
        &self,
        task_id: &str,
        code: &str,
        message: &str,
    ) -> TaskResult<TaskRecord> {
        let task = self.get_task(task_id)?;
        self.forget_run(task_id);
        self.store.append_event(
            task_id,
            event_type::TASK_USER_INPUT_REQUIRED,
            task.current_attempt,
            json!({"code": code, "message": truncate_text(message, REPORT_TEXT_LIMIT)}),
            None,
        )?;
        if task.state == TaskState::Running {
            return self.store.transition(
                task_id,
                TaskState::NeedsUserInput,
                Some(message),
                Some(message),
            );
        }
        Ok(task)
    }

    fn forget_run(&self, task_id: &str) {
        if let Ok(mut runs) = self.runs.write() {
            runs.remove(task_id);
        }
    }

    /// 取消任务并通知后端停止。后端不支持取消时只取消任务记录，
    /// 迟到结果由 [`Self::complete_backend_run`] 丢弃——不假装已经停下。
    pub async fn cancel_task_with_backend(
        &self,
        task_id: &str,
        reason: &str,
    ) -> TaskResult<TaskRecord> {
        let task = self.get_task(task_id)?;
        if let Some(run_id) = self.backend_run_id(task_id) {
            if let Some(backend) = self.backend_for(task.execution_strategy) {
                match backend.cancel(&run_id).await {
                    Ok(()) => {
                        let _ = self.store.append_event(
                            task_id,
                            "task.backend.cancel_requested",
                            task.current_attempt,
                            json!({"backend_run_id": run_id.0}),
                            None,
                        );
                    }
                    Err(error) => {
                        let _ = self.store.append_event(
                            task_id,
                            "task.backend.cancel_unsupported",
                            task.current_attempt,
                            json!({"backend_run_id": run_id.0, "code": error.code}),
                            None,
                        );
                    }
                }
            }
        }
        self.cancel_task(task_id, reason)
    }

    fn backend_for(&self, strategy: ExecutionStrategy) -> Option<Arc<dyn TaskExecutionBackend>> {
        self.backends
            .read()
            .ok()
            .and_then(|backends| backends.get(&strategy).cloned())
    }

    /// 暂停任务并通知执行后端。
    ///
    /// 顺序是**先落状态、再通知**：后端收到暂停后可能立刻返回一个"被中止"的结果，
    /// 此时任务已经是 `paused`，`fail_backend_run` 只会追加事件，不会把暂停翻成失败。
    pub async fn pause_task_with_backend(
        &self,
        task_id: &str,
        reason: &str,
    ) -> TaskResult<TaskRecord> {
        let record = self.pause_task(task_id, reason)?;
        let task = self.get_task(task_id)?;
        let Some(run_id) = self.backend_run_id(task_id) else {
            return Ok(record);
        };
        let Some(backend) = self.backend_for(task.execution_strategy) else {
            return Ok(record);
        };
        match backend.pause(&run_id).await {
            Ok(()) => {
                let _ = self.store.append_event(
                    task_id,
                    "task.backend.pause_requested",
                    task.current_attempt,
                    json!({"backend_run_id": run_id.0}),
                    None,
                );
            }
            Err(error) => {
                // 不能假装已经停下：如实记录后端不支持，任务仍停在 paused，
                // 由调用方决定是否取消。
                let _ = self.store.append_event(
                    task_id,
                    "task.backend.pause_unsupported",
                    task.current_attempt,
                    json!({"backend_run_id": run_id.0, "code": error.code}),
                    None,
                );
            }
        }
        Ok(record)
    }

    pub fn pause_task(&self, task_id: &str, reason: &str) -> TaskResult<TaskRecord> {
        self.store
            .transition(task_id, TaskState::Paused, Some(reason), None)
    }

    /// 恢复任务：先让后端回到可执行状态，再落状态并**重新分发**。
    ///
    /// 只把状态改回 `running` 会让任务显示"运行中"却没有任何执行者，
    /// 因此恢复成功后一定会向执行后端重新提交一次运行。
    pub async fn resume_task_with_backend(&self, task_id: &str) -> TaskResult<TaskRecord> {
        self.resume_task_with_revision(task_id, None).await
    }

    pub async fn resume_task_with_revision(
        &self,
        task_id: &str,
        approved_workflow_revision: Option<Value>,
    ) -> TaskResult<TaskRecord> {
        let task = self.get_task(task_id)?;
        if approved_workflow_revision.is_some()
            && (task.execution_strategy != ExecutionStrategy::Workflow
                || !matches!(task.state, TaskState::Paused | TaskState::NeedsUserInput))
        {
            return Err(TaskError::new(
                codes::INVALID_REQUEST,
                "an approved workflow revision requires a paused workflow task",
            ));
        }
        // 后端恢复要在状态仍是 paused 时做：它可能等旧执行停手，
        // 期间旧执行迟到的失败结果不能把任务翻成 failed。
        if let Some(run_id) = self.backend_run_id(task_id) {
            if let Some(backend) = self.backend_for(task.execution_strategy) {
                match backend.resume(&run_id).await {
                    Ok(()) => {
                        let _ = self.store.append_event(
                            task_id,
                            "task.backend.resumed",
                            task.current_attempt,
                            json!({"backend_run_id": run_id.0}),
                            None,
                        );
                    }
                    Err(error) => {
                        let _ = self.store.append_event(
                            task_id,
                            "task.backend.resume_unsupported",
                            task.current_attempt,
                            json!({"backend_run_id": run_id.0, "code": error.code}),
                            None,
                        );
                    }
                }
            }
        }
        if task.execution_strategy == ExecutionStrategy::Workflow {
            if task.state == TaskState::Running {
                return Ok(task);
            }
            self.resume_task_from(task_id, Some((task.state, task.current_attempt)))?;
        } else {
            self.resume_task(task_id)?;
        }
        // 重新分发是一次新的执行尝试，事件与产物都记在新 attempt 上。
        self.store.bump_attempt(task_id)?;
        if let Err(error) = self
            .dispatch_start_with_revision(task_id, approved_workflow_revision)
            .await
        {
            // 没有可用的执行后端时不能停在"运行中"：退回暂停态并保留原因。
            let current = self.get_task(task_id)?;
            if current.state == TaskState::Running {
                let _ = self.store.transition(
                    task_id,
                    TaskState::Paused,
                    Some(&error.message),
                    Some(&error.message),
                );
            }
            return Err(error);
        }
        self.get_task(task_id)
    }

    pub fn resume_task(&self, task_id: &str) -> TaskResult<TaskRecord> {
        self.resume_task_from(task_id, None)
    }

    fn resume_task_from(
        &self,
        task_id: &str,
        expected: Option<(TaskState, u32)>,
    ) -> TaskResult<TaskRecord> {
        let record = self.store.transition_if_current(
            task_id,
            TaskState::Running,
            Some("用户恢复任务"),
            None,
            expected,
        )?;
        self.store.append_event(
            task_id,
            event_type::TASK_RESUMED,
            record.current_attempt,
            json!({"state": record.state.as_str()}),
            None,
        )?;
        Ok(record)
    }

    pub fn cancel_task(&self, task_id: &str, reason: &str) -> TaskResult<TaskRecord> {
        let task = self.get_task(task_id)?;
        if task.state.is_terminal() {
            return Err(TaskError::new(
                codes::INVALID_TRANSITION,
                format!("任务 {task_id} 已处于终态 {}", task.state),
            ));
        }
        let running_like = matches!(
            task.state,
            TaskState::Running
                | TaskState::AwaitingApproval
                | TaskState::Verifying
                | TaskState::Repairing
        );
        if running_like {
            self.store
                .transition(task_id, TaskState::CancelRequested, Some(reason), None)?;
        }
        let record =
            self.store
                .transition(task_id, TaskState::Cancelled, Some(reason), Some(reason))?;
        // 取消已经通知过后端（见 cancel_task_with_backend）；这里清掉运行登记，
        // 后端迟到的 complete/fail 只会追加事件，不会复活任务。
        self.forget_run(task_id);
        Ok(record)
    }

    pub fn request_approval(
        &self,
        task_id: &str,
        action: &str,
        scope: Value,
        expires_at: Option<String>,
    ) -> TaskResult<TaskApproval> {
        let task = self.get_task(task_id)?;
        let approval = TaskApproval {
            approval_id: uuid::Uuid::new_v4().to_string(),
            task_id: task.task_id.clone(),
            action: action.to_string(),
            scope,
            requested_at: Utc::now().to_rfc3339(),
            expires_at,
            decided_at: None,
            decision: None,
            decided_by: None,
        };
        self.store.request_approval(&approval)?;
        Ok(approval)
    }

    pub fn decide_approval(
        &self,
        task_id: &str,
        approval_id: &str,
        decision: ApprovalDecision,
        decided_by: Option<&str>,
    ) -> TaskResult<TaskApproval> {
        let approval = self
            .store
            .get_approval(approval_id)?
            .ok_or_else(|| TaskError::new(codes::TASK_NOT_FOUND, "审批不存在"))?;
        if approval.task_id.as_str() != task_id {
            return Err(TaskError::new(codes::INVALID_REQUEST, "审批不属于该任务"));
        }
        self.store
            .decide_approval(approval_id, decision, decided_by)
    }

    pub fn approvals(&self, task_id: &str, pending_only: bool) -> TaskResult<Vec<TaskApproval>> {
        self.store.list_approvals(task_id, pending_only)
    }

    /// 独立验证。只有验证通过才允许进入 Succeeded。
    pub fn verify_task(
        &self,
        task_id: &str,
        evidence: VerificationEvidence,
    ) -> TaskResult<VerificationReport> {
        let task = self.get_task(task_id)?;
        if task.state.is_terminal() {
            return Err(TaskError::new(
                codes::INVALID_TRANSITION,
                format!("任务 {task_id} 已处于终态 {}", task.state),
            ));
        }
        self.store
            .transition(task_id, TaskState::Verifying, Some("开始独立验证"), None)?;
        let report = evaluate_evidence(task_id, &evidence);
        self.store.append_event(
            task_id,
            event_type::TASK_VERIFICATION_COMPLETED,
            task.current_attempt,
            serde_json::to_value(&report).unwrap_or(Value::Null),
            None,
        )?;
        if report.verified {
            let summary = format!("验证通过：{}", report.summary);
            self.store
                .transition(task_id, TaskState::Succeeded, None, Some(&summary))?;
        } else if task.current_attempt <= task.budget.max_repairs {
            self.store
                .transition(task_id, TaskState::Repairing, Some(&report.summary), None)?;
        } else {
            self.store.transition(
                task_id,
                TaskState::NeedsUserInput,
                Some(&report.summary),
                None,
            )?;
        }
        Ok(report)
    }

    /// 记录检查点（同时更新任务上的 last_checkpoint_id）。
    pub fn record_checkpoint(
        &self,
        task_id: &str,
        phase: &str,
        state_snapshot_ref: &str,
        workspace_revision: Option<String>,
        artifact_refs: Vec<String>,
        metadata: Value,
    ) -> TaskResult<TaskCheckpoint> {
        let task = self.get_task(task_id)?;
        let checkpoint = TaskCheckpoint {
            checkpoint_id: uuid::Uuid::new_v4().to_string(),
            task_id: task.task_id.clone(),
            attempt: task.current_attempt,
            created_at: Utc::now().to_rfc3339(),
            phase: phase.to_string(),
            state_snapshot_ref: state_snapshot_ref.to_string(),
            workspace_revision,
            artifact_refs,
            metadata,
        };
        self.store.create_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn record_artifact(
        &self,
        task_id: &str,
        kind: &str,
        uri_or_path: &str,
        sha256: Option<String>,
        size_bytes: Option<u64>,
    ) -> TaskResult<TaskArtifact> {
        let task = self.get_task(task_id)?;
        let artifact = TaskArtifact {
            artifact_id: uuid::Uuid::new_v4().to_string(),
            task_id: task.task_id.clone(),
            attempt: task.current_attempt,
            kind: kind.to_string(),
            uri_or_path: uri_or_path.to_string(),
            sha256,
            size_bytes,
            created_at: Utc::now().to_rfc3339(),
            verification_status: ArtifactVerification::Pending,
        };
        self.store.add_artifact(&artifact)?;
        Ok(artifact)
    }

    /// 进程重启后的恢复扫描（§7 任务 3.3）。
    ///
    /// 没有后端运行态可供核对的统一做法：
    /// - 执行中/等待审批/验证/修复 → `Interrupted` → `NeedsUserInput`（不盲目重放副作用）；
    /// - 排队/草稿 → 重新排队；
    /// - 澄清/计划/暂停 → `NeedsUserInput`；
    /// - 取消请求 → `Cancelled`。
    pub fn recover_on_start(&self) -> TaskResult<RecoveryReport> {
        let tasks = self.store.non_terminal_tasks()?;
        let mut report = RecoveryReport {
            scanned: tasks.len(),
            ..RecoveryReport::default()
        };
        for task in tasks {
            let id = task.task_id.as_str();
            match task.state {
                TaskState::Running
                | TaskState::AwaitingApproval
                | TaskState::Verifying
                | TaskState::Repairing => {
                    self.store.transition(
                        id,
                        TaskState::Interrupted,
                        Some("进程重启，任务执行中断"),
                        None,
                    )?;
                    self.store.transition(
                        id,
                        TaskState::NeedsUserInput,
                        Some("存在未知的外部副作用，需人工确认后再恢复"),
                        None,
                    )?;
                    report.interrupted.push(id.to_string());
                    report.needs_user_input.push(id.to_string());
                }
                TaskState::Draft | TaskState::Queued => {
                    if task.state == TaskState::Draft {
                        self.store.transition(
                            id,
                            TaskState::Queued,
                            Some("启动恢复：重新排队"),
                            None,
                        )?;
                    }
                    report.requeued.push(id.to_string());
                }
                TaskState::Clarifying | TaskState::Planning | TaskState::Paused => {
                    self.store.transition(
                        id,
                        TaskState::NeedsUserInput,
                        Some("进程重启，任务需要用户确认后继续"),
                        None,
                    )?;
                    report.needs_user_input.push(id.to_string());
                }
                TaskState::CancelRequested => {
                    self.store.transition(
                        id,
                        TaskState::Cancelled,
                        Some("启动恢复：完成取消请求"),
                        None,
                    )?;
                    report.interrupted.push(id.to_string());
                }
                TaskState::Interrupted | TaskState::NeedsUserInput => {
                    report.unchanged.push(id.to_string());
                }
                TaskState::Succeeded | TaskState::Failed | TaskState::Cancelled => {}
            }
        }
        Ok(report)
    }
}

/// 事件/检查点里保留的报告载荷（最终文本截断）。
fn report_payload(report: &BackendRunReport) -> Value {
    let mut value = serde_json::to_value(report).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        if let Some(text) = report.final_text.as_deref() {
            object.insert(
                "final_text".to_string(),
                json!(truncate_text(text, REPORT_TEXT_LIMIT)),
            );
        }
    }
    value
}

fn truncate_text(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(limit).collect();
    truncated.push('…');
    truncated
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or(text).trim();
    if line.chars().count() <= 60 {
        return line.to_string();
    }
    let mut title: String = line.chars().take(60).collect();
    title.push('…');
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> TaskRuntime {
        TaskRuntime::new(TaskStore::open_in_memory().unwrap())
    }

    fn create(runtime: &TaskRuntime, goal: &str) -> TaskRecord {
        runtime
            .create_task(CreateTaskRequest::new(goal))
            .expect("创建任务")
    }

    #[test]
    fn create_task_queues_and_emits_events() {
        let runtime = runtime();
        let task = create(&runtime, "修复登录按钮");
        assert_eq!(task.state, TaskState::Queued);
        assert_eq!(task.title, "修复登录按钮");
        let events = runtime.events(task.task_id.as_str(), 0, 10).unwrap();
        assert_eq!(events[0].event_type, event_type::TASK_CREATED);
        assert_eq!(events[1].event_type, event_type::TASK_QUEUED);
    }

    #[test]
    fn empty_goal_is_rejected() {
        let runtime = runtime();
        let error = runtime
            .create_task(CreateTaskRequest::new("   "))
            .unwrap_err();
        assert_eq!(error.code, codes::EMPTY_GOALS);
    }

    #[test]
    fn verification_is_required_for_success() {
        let runtime = runtime();
        let task = create(&runtime, "验证闭环");
        runtime.start_task(task.task_id.as_str()).unwrap();
        let report = runtime
            .verify_task(
                task.task_id.as_str(),
                VerificationEvidence {
                    goal: Some(CheckResult::pass("按钮已出现")),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(report.verified);
        assert_eq!(
            runtime.get_task(task.task_id.as_str()).unwrap().state,
            TaskState::Succeeded
        );
    }

    #[test]
    fn failed_verification_enters_bounded_repair() {
        let runtime = runtime();
        let task = create(&runtime, "修复失败");
        runtime.start_task(task.task_id.as_str()).unwrap();
        let report = runtime
            .verify_task(
                task.task_id.as_str(),
                VerificationEvidence {
                    goal: Some(CheckResult::fail("按钮仍然不可点")),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!report.verified);
        assert_eq!(
            runtime.get_task(task.task_id.as_str()).unwrap().state,
            TaskState::Repairing
        );
    }

    #[test]
    fn cancel_goes_through_cancel_requested() {
        let runtime = runtime();
        let task = create(&runtime, "取消任务");
        runtime.start_task(task.task_id.as_str()).unwrap();
        let cancelled = runtime
            .cancel_task(task.task_id.as_str(), "用户取消")
            .unwrap();
        assert_eq!(cancelled.state, TaskState::Cancelled);
        let events = runtime.events(task.task_id.as_str(), 0, 100).unwrap();
        let types: Vec<&str> = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect();
        assert!(types.contains(&event_type::TASK_CANCEL_REQUESTED));
        assert!(types.contains(&event_type::TASK_CANCELLED));
    }

    #[test]
    fn recovery_never_replays_uncertain_side_effects() {
        let runtime = runtime();
        let task = create(&runtime, "长任务");
        runtime.start_task(task.task_id.as_str()).unwrap();
        let report = runtime.recover_on_start().unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.needs_user_input.len(), 1);
        assert_eq!(
            runtime.get_task(task.task_id.as_str()).unwrap().state,
            TaskState::NeedsUserInput
        );
    }

    #[test]
    fn unknown_backend_reports_unsupported_instead_of_success() {
        let runtime = runtime();
        let task = create(&runtime, "未注册后端");
        let (record, registered) = runtime.start_task(task.task_id.as_str()).unwrap();
        assert!(!registered);
        assert_eq!(record.state, TaskState::Running);
        let events = runtime.events(task.task_id.as_str(), 0, 100).unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == event_type::TASK_EXECUTION_STARTED));
    }

    #[test]
    fn require_user_input_parks_the_task_without_failing_it() {
        let runtime = runtime();
        let task = create(&runtime, "需要批准的桌面任务");
        runtime.start_task(task.task_id.as_str()).unwrap();
        let updated = runtime
            .require_user_input(
                task.task_id.as_str(),
                codes::APPROVAL_REQUIRED,
                "computer-use 任务等待人工批准",
            )
            .unwrap();
        assert_eq!(updated.state, TaskState::NeedsUserInput);
        let events = runtime.events(task.task_id.as_str(), 0, 100).unwrap();
        let event = events
            .iter()
            .find(|event| event.event_type == event_type::TASK_USER_INPUT_REQUIRED)
            .expect("应记录待处理事件");
        assert_eq!(event.payload["code"], codes::APPROVAL_REQUIRED);
        assert!(!events
            .iter()
            .any(|event| event.event_type == event_type::TASK_FAILED));
        // 用户处理后可以再次 start（needs_user_input → running）。
        let restarted = runtime.start_task(task.task_id.as_str()).unwrap().0;
        assert_eq!(restarted.state, TaskState::Running);
    }

    #[test]
    fn backend_completion_enters_verifying_not_succeeded() {
        let runtime = runtime();
        let task = create(&runtime, "真实执行");
        runtime.start_task(task.task_id.as_str()).unwrap();
        let updated = runtime
            .complete_backend_run(
                task.task_id.as_str(),
                BackendRunReport {
                    backend_run_id: "run-1".to_string(),
                    session_id: Some("session-1".to_string()),
                    steps: 3,
                    tool_calls: 2,
                    completion_status: Some("candidate".to_string()),
                    final_text: Some("改好了".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        // 后端只能把任务推进到 verifying：成功必须由独立验证决定。
        assert_eq!(updated.state, TaskState::Verifying);
        let events = runtime.events(task.task_id.as_str(), 0, 100).unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == event_type::TASK_BACKEND_COMPLETED));
        let checkpoint = runtime
            .latest_checkpoint(task.task_id.as_str())
            .unwrap()
            .expect("应登记执行完成检查点");
        assert_eq!(checkpoint.phase, "backend_completed");
        assert_eq!(checkpoint.state_snapshot_ref, "session:session-1");
    }

    #[test]
    fn backend_failure_marks_task_failed() {
        let runtime = runtime();
        let task = create(&runtime, "执行失败");
        runtime.start_task(task.task_id.as_str()).unwrap();
        let updated = runtime
            .fail_backend_run(task.task_id.as_str(), "BACKEND_ERROR", "模型不可用")
            .unwrap();
        assert_eq!(updated.state, TaskState::Failed);
        assert_eq!(updated.terminal_summary.as_deref(), Some("模型不可用"));
        let events = runtime.events(task.task_id.as_str(), 0, 100).unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == event_type::TASK_BACKEND_FAILED));
    }

    #[test]
    fn late_backend_result_cannot_resurrect_cancelled_task() {
        let runtime = runtime();
        let task = create(&runtime, "取消后迟到");
        runtime.start_task(task.task_id.as_str()).unwrap();
        runtime
            .cancel_task(task.task_id.as_str(), "用户取消")
            .unwrap();
        let updated = runtime
            .complete_backend_run(
                task.task_id.as_str(),
                BackendRunReport {
                    backend_run_id: "run-late".to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.state, TaskState::Cancelled);
    }

    struct RecordingBackend {
        cancelled: std::sync::atomic::AtomicBool,
        paused: std::sync::atomic::AtomicBool,
        resumed: std::sync::atomic::AtomicBool,
        starts: std::sync::atomic::AtomicUsize,
    }

    impl RecordingBackend {
        fn new() -> Self {
            Self {
                cancelled: std::sync::atomic::AtomicBool::new(false),
                paused: std::sync::atomic::AtomicBool::new(false),
                resumed: std::sync::atomic::AtomicBool::new(false),
                starts: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl TaskExecutionBackend for RecordingBackend {
        fn kind(&self) -> ExecutionStrategy {
            ExecutionStrategy::SingleAgent
        }

        async fn start(&self, _context: TaskRunContext) -> TaskResult<BackendRunId> {
            self.starts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(BackendRunId("recording-run".to_string()))
        }

        async fn pause(&self, _run_id: &BackendRunId) -> TaskResult<()> {
            self.paused.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        async fn resume(&self, _run_id: &BackendRunId) -> TaskResult<()> {
            self.resumed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        async fn cancel(&self, _run_id: &BackendRunId) -> TaskResult<()> {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancel_notifies_registered_backend() {
        let runtime = runtime();
        let backend = Arc::new(RecordingBackend::new());
        runtime.register_backend(backend.clone());
        let task = create(&runtime, "取消要通知后端");
        runtime.start_task(task.task_id.as_str()).unwrap();
        let run_id = runtime.dispatch_start(task.task_id.as_str()).await.unwrap();
        assert_eq!(run_id, BackendRunId("recording-run".to_string()));
        assert_eq!(runtime.backend_run_id(task.task_id.as_str()), Some(run_id));
        let cancelled = runtime
            .cancel_task_with_backend(task.task_id.as_str(), "用户取消")
            .await
            .unwrap();
        assert_eq!(cancelled.state, TaskState::Cancelled);
        assert!(backend.cancelled.load(std::sync::atomic::Ordering::SeqCst));
        assert!(runtime.backend_run_id(task.task_id.as_str()).is_none());
    }

    #[tokio::test]
    async fn pause_and_resume_notify_backend_and_redispatch() {
        let runtime = runtime();
        let backend = Arc::new(RecordingBackend::new());
        runtime.register_backend(backend.clone());
        let task = create(&runtime, "暂停要通知后端");
        runtime.start_task(task.task_id.as_str()).unwrap();
        runtime.dispatch_start(task.task_id.as_str()).await.unwrap();
        assert_eq!(backend.starts.load(std::sync::atomic::Ordering::SeqCst), 1);

        let paused = runtime
            .pause_task_with_backend(task.task_id.as_str(), "用户暂停")
            .await
            .unwrap();
        assert_eq!(paused.state, TaskState::Paused);
        assert!(
            backend.paused.load(std::sync::atomic::Ordering::SeqCst),
            "暂停必须通知执行后端"
        );
        assert!(runtime
            .events(task.task_id.as_str(), 0, 100)
            .unwrap()
            .iter()
            .any(|event| event.event_type == "task.backend.pause_requested"));

        let resumed = runtime
            .resume_task_with_backend(task.task_id.as_str())
            .await
            .unwrap();
        assert_eq!(resumed.state, TaskState::Running);
        assert!(
            backend.resumed.load(std::sync::atomic::Ordering::SeqCst),
            "恢复必须通知执行后端"
        );
        assert_eq!(
            backend.starts.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "恢复要重新分发，而不是只把状态改回 running"
        );
        assert!(runtime.backend_run_id(task.task_id.as_str()).is_some());
        assert_eq!(
            runtime.get_task(task.task_id.as_str()).unwrap().state,
            TaskState::Running
        );
    }

    #[tokio::test]
    async fn resume_without_backend_returns_to_paused() {
        let runtime = runtime();
        let task = create(&runtime, "没有后端可恢复");
        runtime.start_task(task.task_id.as_str()).unwrap();
        runtime
            .pause_task(task.task_id.as_str(), "用户暂停")
            .unwrap();
        let error = runtime
            .resume_task_with_backend(task.task_id.as_str())
            .await
            .unwrap_err();
        assert_eq!(error.code, codes::UNSUPPORTED);
        // 不能停在"运行中却没有执行者"。
        assert_eq!(
            runtime.get_task(task.task_id.as_str()).unwrap().state,
            TaskState::Paused
        );
    }

    #[test]
    fn approval_flow_requires_explicit_decision() {
        let runtime = runtime();
        let task = create(&runtime, "需要审批");
        // 先进入 Planning，审批请求从计划阶段发起。
        runtime
            .store
            .transition(task.task_id.as_str(), TaskState::Planning, None, None)
            .unwrap();
        let approval = runtime
            .request_approval(
                task.task_id.as_str(),
                "desktop_click",
                json!({"app": "notepad"}),
                None,
            )
            .unwrap();
        assert_eq!(
            runtime.get_task(task.task_id.as_str()).unwrap().state,
            TaskState::AwaitingApproval
        );
        let decided = runtime
            .decide_approval(
                task.task_id.as_str(),
                &approval.approval_id,
                ApprovalDecision::Approved,
                Some("user"),
            )
            .unwrap();
        assert_eq!(decided.decision, Some(ApprovalDecision::Approved));
        assert_eq!(
            runtime.get_task(task.task_id.as_str()).unwrap().state,
            TaskState::Running
        );
    }
}
