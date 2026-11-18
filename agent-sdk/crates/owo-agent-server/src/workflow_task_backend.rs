//! Workflow 执行后端（统一 Task Runtime 的第四个 Adapter）。
//!
//! 职责边界：
//! - 把统一 Task 的 `start` 变成**一次真实的 .owflow 运行**：复用 `/workflow/{name}/run`
//!   同一条装配路径（`workflow_api::start_run`），不另起一套工作流执行栈；
//! - 只回报运行事实：步骤记录、终态、回滚点；是否 `succeeded` 仍由独立验证决定。
//!   工作流引擎的 `Succeeded` 是自报，因此本后端**不自动验证**，任务停在 `verifying`；
//! - 暂停 = 运行协作式中止；恢复在新 attempt 中复用持久步骤结果与上下文，
//!   重建控制流但不重放已完成副作用，未知效果待人工核对；
//! - 取消 = 运行中止；统一 Task 的取消语义由运行时收口。
//!
//! payload（Runtime 不解释）：
//! ```json
//! { "name": "release-check", "backend": "real", "ctx": {}, "approval_timeout_ms": 120000 }
//! ```
//! `name` 必填，指向 workspace 下（深度 ≤ 3）的 `.owflow` 文件 stem；`backend`
//! 缺省 `real`（真实执行，桌面动作仍被门禁拒绝），`mock` 是沙箱，不产生真实效果。
//!
//! 依赖 `Arc<AppState>`：运行装配需要工作区与数据根，因此在 `build_router` 拿到
//! `Arc<AppState>` 后注册。后端只持有弱引用，不构成引用环。

mod recovery;
use owo_agent_core::workflow::WORKFLOW_RECOVERY_REQUIRED;
use recovery::TaskWorkflowJournal;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use axum::Json;
use owo_agent_core::task_runtime::{
    codes, BackendRunId, BackendRunReport, BackendSnapshot, ExecutionStrategy, TaskError,
    TaskExecutionBackend, TaskResult, TaskRunContext, TaskRuntime, TaskState,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::workflow_api::{self, RunEntry, RunRequest};
use crate::AppState;

/// 工作流运行失败（引擎进入 Failed，可能已回滚）。
const CODE_WORKFLOW_RUN_FAILED: &str = "WORKFLOW_RUN_FAILED";
/// 工作流运行在统一任务之外被中止。
const CODE_WORKFLOW_ABORTED: &str = "WORKFLOW_ABORTED";
/// 运行记录缺失或运行器报错（无法判定执行事实）。
const CODE_WORKFLOW_ERROR: &str = "WORKFLOW_ERROR";
/// 监控运行的轮询间隔：工作流步骤通常是文件/断言级动作，终态来得很快。
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// 统一任务 `payload` 里 Workflow 专属的输入。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WorkflowTaskPayload {
    /// 工作区内的 `.owflow` 文件名（不含扩展名）。
    name: String,
    /// `real`（缺省，真实执行）| `mock`（沙箱）。
    backend: Option<String>,
    /// 运行上下文（进入运行快照，供前端展示；引擎 v1 的步骤输出不读它）。
    ctx: Option<Value>,
    /// 人审超时（毫秒；缺省 120s，与 HTTP 入口一致）。
    approval_timeout_ms: Option<u64>,
}

/// Workflow 执行后端：任务 → .owflow 运行 → 运行事实。
pub struct WorkflowTaskBackend {
    state: Weak<AppState>,
    runtime: Weak<TaskRuntime>,
    /// 任务 → 运行 id（恢复/重复 start 时据此判断能否沿用同一次运行）。
    runs: Arc<Mutex<HashMap<String, String>>>,
    /// 已在监控的运行 id（防止重复 start 起两个监控器）。
    monitors: Arc<Mutex<HashSet<String>>>,
}

impl WorkflowTaskBackend {
    pub fn new(state: Weak<AppState>, runtime: Weak<TaskRuntime>) -> Self {
        Self {
            state,
            runtime,
            runs: Arc::new(Mutex::new(HashMap::new())),
            monitors: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn lookup_run(&self, task_id: &str) -> Option<String> {
        self.runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(task_id).cloned())
    }

    fn remember_run(&self, task_id: &str, run_id: &str) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(task_id.to_string(), run_id.to_string());
        }
    }
}

#[async_trait::async_trait]
impl TaskExecutionBackend for WorkflowTaskBackend {
    fn kind(&self) -> ExecutionStrategy {
        ExecutionStrategy::Workflow
    }

    async fn start(&self, context: TaskRunContext) -> TaskResult<BackendRunId> {
        let Some(state) = self.state.upgrade() else {
            return Err(TaskError::new(
                codes::STORE_ERROR,
                "服务状态已释放，无法启动工作流运行",
            ));
        };
        let Some(runtime) = self.runtime.upgrade() else {
            return Err(TaskError::new(
                codes::STORE_ERROR,
                "任务运行时已释放，无法监控工作流运行",
            ));
        };
        let payload: WorkflowTaskPayload =
            match serde_json::from_value(context.payload.clone().unwrap_or_else(|| json!({}))) {
                Ok(payload) => payload,
                Err(error) => {
                    return Err(reject(
                        &runtime,
                        &context.task_id.0,
                        codes::INVALID_REQUEST,
                        format!("工作流参数无效：{error}"),
                    ));
                }
            };
        let name = payload.name.trim();
        if name.is_empty() {
            return Err(reject(
                &runtime,
                &context.task_id.0,
                codes::INVALID_REQUEST,
                "payload.name 必须指向工作区内的 .owflow 工作流",
            ));
        }
        let backend = match payload.backend.as_deref() {
            None | Some("real") => "real",
            Some("mock") => "mock",
            Some(other) => {
                return Err(reject(
                    &runtime,
                    &context.task_id.0,
                    codes::INVALID_REQUEST,
                    format!("未知工作流后端：{other}（支持 real|mock）"),
                ));
            }
        };
        // 重复 start（如审批后再次分发）：同一运行仍在跑且未被要求中止就沿用。
        // 已被中止（暂停）的运行不能沿用——引擎会在下一个边界停下，恢复要新建运行。
        if let Some(run_id) = self.lookup_run(&context.task_id.0) {
            if let Some(entry) = workflow_api::get_run(&run_id) {
                let finished = entry
                    .outcome
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .is_some();
                let stopping = entry.cancel_flag.load(Ordering::SeqCst)
                    || entry.abort_requested.load(Ordering::SeqCst);
                if !finished && stopping {
                    return Err(reject(
                        &runtime,
                        &context.task_id.0,
                        WORKFLOW_RECOVERY_REQUIRED,
                        "prior workflow has not stopped; wait before resuming",
                    ));
                }
                if !finished && !stopping {
                    self.spawn_monitor(
                        Arc::clone(&runtime),
                        context.task_id.0.clone(),
                        context.attempt,
                        run_id.clone(),
                    );
                    return Ok(BackendRunId(run_id));
                }
            }
        }
        let ctx = match payload.ctx {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(map)) => map,
            Some(_) => {
                return Err(reject(
                    &runtime,
                    &context.task_id.0,
                    codes::INVALID_REQUEST,
                    "payload.ctx 必须是 JSON 对象",
                ));
            }
        };
        let request = RunRequest {
            ctx,
            backend: Some(backend.to_string()),
            approval_timeout_ms: payload.approval_timeout_ms,
        };
        let journal = Arc::new(
            TaskWorkflowJournal::new(Arc::clone(&runtime), &context, &state.workspace).map_err(
                |error| {
                    reject(
                        &runtime,
                        &context.task_id.0,
                        WORKFLOW_RECOVERY_REQUIRED,
                        format!("Cannot identify the workflow workspace: {error}"),
                    )
                },
            )?,
        );
        let recovery = workflow_api::RunRecovery {
            store: journal,
            approved_revision: context.approved_workflow_revision.clone(),
            work_root: state
                .data_root
                .join("workflow-task-runs")
                .join(context.task_id.as_str()),
        };
        let started = match workflow_api::start_run_with_recovery(
            &state,
            name,
            request,
            Some(recovery),
        )
        .await
        {
            Ok(started) => started,
            Err((status, Json(body))) => {
                let message = body["error"]
                    .as_str()
                    .unwrap_or("工作流运行启动失败")
                    .to_string();
                return Err(reject(
                    &runtime,
                    &context.task_id.0,
                    if message.contains(WORKFLOW_RECOVERY_REQUIRED) {
                        WORKFLOW_RECOVERY_REQUIRED
                    } else {
                        codes::INVALID_REQUEST
                    },
                    format!("{message}（HTTP {status}）"),
                ));
            }
        };
        let run_id = started.run_id.clone();
        self.remember_run(&context.task_id.0, &run_id);
        let _ = runtime.store().append_event(
            &context.task_id.0,
            "task.workflow.created",
            context.attempt,
            json!({
                "run_id": run_id,
                "name": started.entry.name,
                "backend": started.entry.backend,
                "journal_version": 2,
            }),
            Some(&run_id),
        );
        self.spawn_monitor(
            Arc::clone(&runtime),
            context.task_id.0.clone(),
            context.attempt,
            run_id.clone(),
        );
        Ok(BackendRunId(run_id))
    }

    async fn pause(&self, run_id: &BackendRunId) -> TaskResult<()> {
        // 工作流没有"冻结"：暂停 = 协作式中止。运行循环在步骤边界停下，
        // 已完成的步骤保留在 outcome 里，不撤掉副作用。
        if !workflow_api::request_abort(&run_id.0) {
            return Err(TaskError::new(
                codes::TASK_NOT_FOUND,
                format!("工作流运行不存在：{}", run_id.0),
            ));
        }
        Ok(())
    }

    async fn resume(&self, run_id: &BackendRunId) -> TaskResult<()> {
        // Quiesce the old executor before a new attempt can read its last facts.
        if let Some(entry) = workflow_api::get_run(&run_id.0) {
            for _ in 0..100 {
                if entry
                    .outcome
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .is_some()
                {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            return Err(TaskError::new(
                WORKFLOW_RECOVERY_REQUIRED,
                "旧运行仍在停止，请等待后再恢复；不会启动第二个执行者",
            ));
        }
        // On a fresh process the durable journal, not a registry handle, is authoritative.
        Ok(())
    }

    async fn cancel(&self, run_id: &BackendRunId) -> TaskResult<()> {
        // 运行已到终态时中止请求无副作用；任务取消语义由运行时收口。
        workflow_api::request_abort(&run_id.0);
        Ok(())
    }

    async fn inspect(&self, run_id: &BackendRunId) -> TaskResult<BackendSnapshot> {
        let entry = workflow_api::get_run(&run_id.0).ok_or_else(|| {
            TaskError::new(
                codes::TASK_NOT_FOUND,
                format!("工作流运行不存在：{}", run_id.0),
            )
        })?;
        let outcome = entry
            .outcome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let pending = workflow_api::workflow_backend::pending_approvals(&run_id.0);
        let state = if !pending.is_empty() {
            "waiting_approval".to_string()
        } else {
            outcome
                .as_ref()
                .and_then(|value| value["state"].as_str().map(str::to_string))
                .unwrap_or_else(|| "running".to_string())
        };
        let steps = outcome
            .as_ref()
            .and_then(|value| value["steps"].as_array().map(|all| all.len()))
            .unwrap_or(0);
        Ok(BackendSnapshot {
            state,
            detail: json!({
                "run_id": entry.run_id,
                "name": entry.name,
                "backend": entry.backend,
                "steps": steps,
                "rollback_to": outcome
                    .as_ref()
                    .map(|value| value["rollback_to"].clone()),
            }),
        })
    }
}

impl WorkflowTaskBackend {
    /// 监控一次运行：步骤记录与终态回报给统一 Task Runtime。
    fn spawn_monitor(
        &self,
        runtime: Arc<TaskRuntime>,
        task_id: String,
        attempt: u32,
        run_id: String,
    ) {
        {
            let Ok(mut monitors) = self.monitors.lock() else {
                return;
            };
            if !monitors.insert(run_id.clone()) {
                return;
            }
        }
        let monitors = Arc::clone(&self.monitors);
        tokio::spawn(async move {
            let started = Instant::now();
            let mut awaiting_human_notified = false;
            while let Ok(task) = runtime.get_task(&task_id) {
                if matches!(
                    task.state,
                    TaskState::Succeeded | TaskState::Failed | TaskState::Cancelled
                ) {
                    break;
                }
                // 恢复会新建一次运行（attempt +1）：旧运行的监控器不能再用旧结果
                // 收口新尝试。这里只比 attempt，不比运行映射——新监控器可能在
                // dispatch 登记新 run_id 之前就看到旧映射。
                if task.current_attempt != attempt {
                    break;
                }
                let Some(entry) = workflow_api::get_run(&run_id) else {
                    let _ = runtime.fail_backend_run(
                        &task_id,
                        CODE_WORKFLOW_ERROR,
                        &format!("工作流运行 {run_id} 不在运行注册表里（进程重启？）"),
                    );
                    break;
                };
                if !awaiting_human_notified {
                    let pending = workflow_api::workflow_backend::pending_approvals(&run_id);
                    if !pending.is_empty() {
                        awaiting_human_notified = true;
                        let _ = runtime.store().append_event(
                            &task_id,
                            "task.backend.awaiting_human",
                            attempt,
                            json!({
                                "run_id": run_id,
                                "pending": serde_json::to_value(&pending)
                                    .unwrap_or_else(|_| json!([])),
                            }),
                            Some(&run_id),
                        );
                    }
                }
                let outcome = entry
                    .outcome
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                let Some(outcome) = outcome else {
                    tokio::time::sleep(POLL_INTERVAL).await;
                    continue;
                };
                finish_run(&runtime, &task_id, attempt, &entry, &outcome, started);
                break;
            }
            if let Ok(mut monitors) = monitors.lock() {
                monitors.remove(&run_id);
            }
        });
    }
}

/// 运行终态 → 统一任务：步骤事实 + 完成/失败/中止。
///
/// 工作流引擎只在校验点暴露结果，因此步骤事件按引擎记录的完成顺序回报
/// （`task.step.completed`），不伪造开始时间。
fn finish_run(
    runtime: &TaskRuntime,
    task_id: &str,
    attempt: u32,
    entry: &RunEntry,
    outcome: &Value,
    started: Instant,
) {
    let run_id = entry.run_id.as_str();
    let steps: Vec<Value> = outcome["steps"].as_array().cloned().unwrap_or_default();
    for step in &steps {
        let _ = runtime.store().append_event(
            task_id,
            "task.step.completed",
            attempt,
            json!({
                "run_id": run_id,
                "step_id": step["id"],
                "kind": step["kind"],
                "ok": step["ok"],
                "detail": step["detail"],
                "reused": step["reused"].as_bool().unwrap_or(false),
            }),
            Some(run_id),
        );
    }
    if outcome["metrics"].is_object() {
        let _ = runtime.store().append_event(
            task_id,
            "task.workflow.metrics",
            attempt,
            json!({"run_id":run_id,"metrics":outcome["metrics"]}),
            Some(run_id),
        );
    }
    let rollback_to = outcome["rollback_to"].clone();
    match outcome["state"].as_str().unwrap_or("unknown") {
        "succeeded" => {
            let Some(tool_calls) = outcome["metrics"]["tool_calls"]
                .as_u64()
                .and_then(|count| usize::try_from(count).ok())
            else {
                let _ = runtime.require_user_input(
                    task_id,
                    WORKFLOW_RECOVERY_REQUIRED,
                    "工作流缺少可信的操作统计，不能用0代替",
                );
                return;
            };
            let _ = runtime.store().append_event(
                task_id,
                "task.workflow.completed",
                attempt,
                json!({
                    "run_id": run_id,
                    "name": entry.name,
                    "backend": entry.backend,
                    "steps": steps.len(),
                }),
                Some(run_id),
            );
            // mock 运行只证明接线，不证明真实效果；real 运行的 Succeeded 也是引擎自报。
            // 两者都不自动验证，任务停在 verifying，等待独立验证。
            let status = if entry.backend == "real" {
                "candidate"
            } else {
                "unverified"
            };
            let _ = runtime.record_artifact(
                task_id,
                "workflow_outcome",
                &format!("workflow-run:{run_id}"),
                None,
                None,
            );
            let report = BackendRunReport {
                backend_run_id: run_id.to_string(),
                session_id: None,
                steps: steps.len(),
                tool_calls,
                duration_ms: started.elapsed().as_millis() as u64,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                completion_status: Some(status.to_string()),
                completion_record: Some(outcome.clone()),
                final_text: Some(format!(
                    "工作流 {}（{}）完成，共 {} 步",
                    entry.name,
                    entry.backend,
                    steps.len()
                )),
            };
            if let Err(error) = runtime.complete_backend_run(task_id, report) {
                tracing::warn!("任务 {task_id} 提交工作流执行事实失败：{error}");
            }
        }
        "failed" => {
            let detail = failure_detail(entry);
            let _ = runtime.store().append_event(
                task_id,
                "task.workflow.failed",
                attempt,
                json!({
                    "run_id": run_id,
                    "name": entry.name,
                    "rollback_to": rollback_to,
                }),
                Some(run_id),
            );
            let _ = runtime.fail_backend_run(task_id, CODE_WORKFLOW_RUN_FAILED, &detail);
        }
        "aborted" => {
            // 暂停触发的运行中止由运行时转成 task.backend.paused；取消/外部中止转成失败。
            let _ = runtime.fail_backend_run(
                task_id,
                CODE_WORKFLOW_ABORTED,
                &format!("工作流运行 {run_id} 已中止"),
            );
        }
        other => {
            let detail = outcome["error"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("工作流运行状态异常：{other}"));
            if detail.contains(WORKFLOW_RECOVERY_REQUIRED) {
                let _ = runtime.require_user_input(task_id, WORKFLOW_RECOVERY_REQUIRED, &detail);
            } else {
                let _ = runtime.fail_backend_run(task_id, CODE_WORKFLOW_ERROR, &detail);
            }
        }
    }
}

/// 输入不成立：先收口任务（不能停在 running），再把错误交给分发层。
fn reject(
    runtime: &TaskRuntime,
    task_id: &str,
    code: &str,
    message: impl Into<String>,
) -> TaskError {
    let message = message.into();
    if code == WORKFLOW_RECOVERY_REQUIRED {
        let _ = runtime.require_user_input(task_id, code, &message);
    } else {
        let _ = runtime.fail_backend_run(task_id, code, &message);
    }
    TaskError::new(code, message)
}

/// 失败原因：优先取引擎审计里的 `workflow.failed` 明细（含回滚点），否则给通用说明。
fn failure_detail(entry: &RunEntry) -> String {
    entry
        .audit_tail
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|record| record["event"].as_str() == Some("workflow.failed"))
        .and_then(|record| record["detail"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "工作流运行失败（未记录失败步骤明细）".to_string())
}
