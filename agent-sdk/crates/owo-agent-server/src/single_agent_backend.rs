//! Single Agent 真实执行后端（Agent V2 §6 任务 2.2 的第一个 Adapter）。
//!
//! 职责边界：
//! - 把统一 Task 的 `start` 变成**真实的一次 Agent 回合**（`Agent::run_turn_*`），
//!   复用既有会话、策略、工具宿主、审计与完成判定，不另起一套执行栈；
//! - 只向 Task Runtime 提交**执行事实**（步数、用量、宿主完成判定、产物），
//!   不把任务直接标成 `succeeded`——是否成功由独立验证决定；
//! - 后台执行没有交互审批通道，因此使用 [`WorkspaceApprover`]：读放行、写仅限
//!   工作区内、命令执行只在 `policy_profile = "workspace-execute"` 时显式放行。
//!   这**不放宽** Policy 的判定，只处理策略层已经判为"需要询问"的请求。
//!
//! 取消：`cancel` 只置中止位；回合返回 `Aborted` 后由运行时按取消语义收口，
//! 迟到的结果会被 `complete_backend_run` / `fail_backend_run` 丢弃。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use owo_agent_core::permissions::WorkspaceApprover;
use owo_agent_core::task_runtime::{
    codes, BackendRunId, BackendRunReport, ExecutionStrategy, TaskError, TaskExecutionBackend,
    TaskResult, TaskRunContext, TaskRuntime,
};
use owo_agent_core::{Agent, Session, SessionMode, SessionStore, TurnEvent, TurnExecutionOptions};

/// 显式允许后台任务执行命令的策略档位。默认（`workspace`）只允许工作区内写入。
pub const POLICY_WORKSPACE_EXECUTE: &str = "workspace-execute";

/// 单次运行最多登记的工具事件数（防止异常长回合把事件表撑爆；不影响实际执行）。
const MAX_LOGGED_TOOL_EVENTS: usize = 200;

pub struct SingleAgentBackend {
    /// 弱引用：TaskRuntime 持有本后端，强引用会成环。
    runtime: Weak<TaskRuntime>,
    agent: Arc<Agent>,
    store: Arc<dyn SessionStore>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    /// 服务启动时确定的工作区根（任务未指定 workspace_ref 时使用）。
    workspace_root: PathBuf,
    /// backend_run_id → 中止位。
    runs: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl SingleAgentBackend {
    pub fn new(
        runtime: Weak<TaskRuntime>,
        agent: Arc<Agent>,
        store: Arc<dyn SessionStore>,
        sessions: Arc<Mutex<HashMap<String, Session>>>,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            runtime,
            agent,
            store,
            sessions,
            workspace_root,
            runs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl TaskExecutionBackend for SingleAgentBackend {
    fn kind(&self) -> ExecutionStrategy {
        ExecutionStrategy::SingleAgent
    }

    async fn start(&self, context: TaskRunContext) -> TaskResult<BackendRunId> {
        let run_id = BackendRunId(uuid::Uuid::new_v4().to_string());
        let Some(runtime) = self.runtime.upgrade() else {
            return Err(TaskError::new(
                codes::STORE_ERROR,
                "任务运行时已释放，无法分发单 Agent 执行",
            ));
        };
        let abort = Arc::new(AtomicBool::new(false));
        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(run_id.0.clone(), Arc::clone(&abort));
        }
        let store = Arc::clone(&self.store);
        let agent = Arc::clone(&self.agent);
        let sessions = Arc::clone(&self.sessions);
        let runs = Arc::clone(&self.runs);
        let workspace_root = self.workspace_root.clone();
        let backend_run_id = run_id.0.clone();
        tokio::spawn(async move {
            run_single_agent(
                runtime,
                agent,
                store,
                sessions,
                workspace_root,
                context,
                backend_run_id.clone(),
                abort,
            )
            .await;
            if let Ok(mut runs) = runs.lock() {
                runs.remove(&backend_run_id);
            }
        });
        Ok(run_id)
    }

    async fn cancel(&self, run_id: &BackendRunId) -> TaskResult<()> {
        self.signal(&run_id.0, "取消")
    }

    /// 暂停：置中止位，回合在下一个检查点退出。
    ///
    /// 只停不续——回合没有可原地恢复的执行状态，恢复由运行时重新 `start`
    /// 一个新回合完成；这样不会出现"暂停后后台还在跑"的假象。
    async fn pause(&self, run_id: &BackendRunId) -> TaskResult<()> {
        self.signal(&run_id.0, "暂停")
    }

    async fn resume(&self, _run_id: &BackendRunId) -> TaskResult<()> {
        Ok(())
    }
}

impl SingleAgentBackend {
    fn signal(&self, backend_run_id: &str, action: &str) -> TaskResult<()> {
        let runs = self.runs.lock().map_err(|_| {
            TaskError::new(
                codes::STORE_ERROR,
                format!("执行运行表已损坏，无法{action}"),
            )
        })?;
        match runs.get(backend_run_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }
            None => Err(TaskError::new(
                codes::TASK_NOT_FOUND,
                format!("后端运行 {backend_run_id} 已结束或不存在"),
            )),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_single_agent(
    runtime: Arc<TaskRuntime>,
    agent: Arc<Agent>,
    store: Arc<dyn SessionStore>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    workspace_root: PathBuf,
    context: TaskRunContext,
    backend_run_id: String,
    abort: Arc<AtomicBool>,
) {
    let task_id = context.task_id.as_str().to_string();
    let workspace = match resolve_workspace(&workspace_root, context.workspace_ref.as_deref()) {
        Ok(workspace) => workspace,
        Err(message) => {
            let _ = runtime.fail_backend_run(&task_id, codes::INVALID_REQUEST, &message);
            return;
        }
    };
    let model = default_model();
    let mut session = match store.create(&workspace, &model, None) {
        Ok(session) => session,
        Err(error) => {
            let _ = runtime.fail_backend_run(
                &task_id,
                codes::STORE_ERROR,
                &format!("创建执行会话失败：{error}"),
            );
            return;
        }
    };
    if let Ok(mut all) = sessions.lock() {
        all.insert(session.id.clone(), session.clone());
    }
    crate::event_stream::hub().publish_invalidate(crate::event_stream::InvalidateDomain::Sessions);
    let _ = runtime.store().append_event(
        &task_id,
        "task.session.created",
        context.attempt,
        serde_json::json!({
            "session_id": session.id,
            "workspace": workspace.display().to_string(),
            "policy_profile": context.policy_profile,
        }),
        Some(&backend_run_id),
    );

    let allow_execute = context.policy_profile.trim() == POLICY_WORKSPACE_EXECUTE;
    let approver = WorkspaceApprover {
        workspace: workspace.clone(),
        allow_writes: true,
        allow_execute,
    };
    // 任务事件写入走 Task Store；会话库（SessionStore）不承担任务事件。
    let task_store = runtime.store();
    // 预算里的 0 表示该维度不额外限制（见 TaskBudget 文档）。
    let max_tool_calls = match context.budget.max_tool_calls {
        0 => usize::MAX,
        value => value as usize,
    };
    let mut tool_calls = 0usize;
    let mut over_budget = false;
    // 回调只活到本回合结束：放在内层作用域，回合结束后再读 tool_calls 的最终值。
    let turn = {
        let abort_for_events = Arc::clone(&abort);
        let mut on_event = |event: &TurnEvent| match event {
            TurnEvent::ToolStart { tool, .. } => {
                tool_calls = tool_calls.saturating_add(1);
                if tool_calls > max_tool_calls {
                    // 预算只做硬熔断，不假装成功：回合停止后由运行时记为失败。
                    over_budget = true;
                    abort_for_events.store(true, Ordering::SeqCst);
                    let _ = task_store.append_event(
                        &task_id,
                        "task.budget.exceeded",
                        context.attempt,
                        serde_json::json!({"max_tool_calls": max_tool_calls}),
                        Some(&backend_run_id),
                    );
                    return;
                }
                if tool_calls <= MAX_LOGGED_TOOL_EVENTS {
                    let _ = task_store.append_event(
                        &task_id,
                        owo_agent_core::task_runtime::event_type::TASK_TOOL_STARTED,
                        context.attempt,
                        serde_json::json!({"tool": tool}),
                        Some(&backend_run_id),
                    );
                }
            }
            TurnEvent::ToolResult { tool, ok, .. } if tool_calls <= MAX_LOGGED_TOOL_EVENTS => {
                let _ = task_store.append_event(
                    &task_id,
                    owo_agent_core::task_runtime::event_type::TASK_TOOL_COMPLETED,
                    context.attempt,
                    serde_json::json!({"tool": tool, "ok": ok}),
                    Some(&backend_run_id),
                );
            }
            _ => {}
        };
        let options = TurnExecutionOptions {
            read_only: false,
            turn_id: Some(backend_run_id.clone()),
            mode: SessionMode::Agent,
            input: None,
            session_checkpoint: Some(owo_agent_core::agent::TurnSessionCheckpoint::new(
                Arc::clone(&store),
            )),
        };
        // 同时服从服务端单回合看门狗与任务自带的时限预算（0 = 不额外限制）。
        let mut turn_budget = crate::turn_api::turn_watchdog_budget();
        if context.budget.max_duration_ms > 0 {
            let requested = std::time::Duration::from_millis(context.budget.max_duration_ms);
            turn_budget = turn_budget.min(requested);
        }
        tokio::time::timeout(
            turn_budget,
            agent.run_turn_with_images_options(
                &mut session,
                &context.user_goal,
                &[],
                &approver,
                None,
                &abort,
                &mut on_event,
                &options,
            ),
        )
        .await
    };
    if let Ok(mut all) = sessions.lock() {
        all.insert(session.id.clone(), session.clone());
    }

    let result = match turn {
        Ok(result) => result,
        Err(_) => {
            abort.store(true, Ordering::SeqCst);
            Err(owo_agent_core::AgentError::Session(
                "task/timeout: 任务超过服务端单回合时限仍未结束，已强制中止".to_string(),
            ))
        }
    };
    match result {
        Ok(outcome) => {
            record_artifacts(&runtime, &task_id, &session, &backend_run_id);
            let completion_status = serde_json::to_value(outcome.completion_status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string));
            let report = BackendRunReport {
                backend_run_id: backend_run_id.clone(),
                session_id: Some(session.id.clone()),
                steps: outcome.steps,
                tool_calls,
                duration_ms: outcome.duration_ms,
                prompt_tokens: outcome.usage.prompt_tokens,
                completion_tokens: outcome.usage.completion_tokens,
                total_tokens: outcome.usage.total_tokens,
                completion_status: completion_status.clone(),
                completion_record: session
                    .completion_record
                    .as_ref()
                    .and_then(|record| serde_json::to_value(record).ok()),
                final_text: outcome.final_text.clone(),
            };
            if over_budget {
                let _ = runtime.fail_backend_run(
                    &task_id,
                    codes::BUDGET_EXCEEDED,
                    &format!("工具调用超过预算上限 {max_tool_calls}，执行已中止"),
                );
                return;
            }
            if abort.load(Ordering::SeqCst) {
                // 取消（或超时）途中产生的半截结果不能算完成；任务若已被取消，
                // 运行时按取消语义收口，不会把迟到结果翻成成功。
                let _ = runtime.fail_backend_run(
                    &task_id,
                    "ABORTED",
                    "任务执行被中止（取消或超时），结果不算完成",
                );
                return;
            }
            if let Err(error) = runtime.complete_backend_run(&task_id, report) {
                tracing::warn!("任务 {task_id} 提交执行事实失败：{error}");
            }
            // 只有宿主完成判定为 accepted（绑定证据收据）时，才把它当作目标验证证据
            // 送入独立验证；其余情况保持 verifying，交给人或后续验证阶段判定。
            if completion_status.as_deref() == Some("accepted") {
                let evidence = owo_agent_core::task_runtime::VerificationEvidence {
                    execution: Some(owo_agent_core::task_runtime::CheckResult::pass(format!(
                        "单 Agent 回合完成：{} 步，{} 次工具调用",
                        outcome.steps, tool_calls
                    ))),
                    goal: Some(owo_agent_core::task_runtime::CheckResult::pass(
                        "宿主完成判定 accepted（证据收据已绑定候选版本）",
                    )),
                    ..Default::default()
                };
                if let Err(error) = runtime.verify_task(&task_id, evidence) {
                    tracing::warn!("任务 {task_id} 自动验证失败：{error}");
                }
            }
        }
        Err(error) => {
            let code = if matches!(error, owo_agent_core::AgentError::Aborted) {
                "ABORTED"
            } else {
                "BACKEND_ERROR"
            };
            if let Err(record_error) = runtime.fail_backend_run(&task_id, code, &error.to_string())
            {
                tracing::warn!("任务 {task_id} 记录失败状态时出错：{record_error}");
            }
        }
    }
}

/// 把本回合的写入收据登记为任务产物（路径 + 快照 sha256）。
fn record_artifacts(runtime: &TaskRuntime, task_id: &str, session: &Session, turn_id: &str) {
    for receipt in session
        .execution_receipts
        .iter()
        .filter(|receipt| receipt.turn_id == turn_id)
    {
        for file in &receipt.changed_files {
            let sha256 = receipt.after_hashes.get(file).cloned().flatten();
            let size_bytes = receipt
                .snapshot_keys
                .get(file)
                .and_then(|absolute| std::fs::metadata(absolute).ok())
                .map(|meta| meta.len());
            let _ = runtime.record_artifact(task_id, "file", file, sha256, size_bytes);
        }
    }
}

/// 任务工作区：显式指定且存在的目录优先，否则用服务启动时的工作区根。
fn resolve_workspace(root: &Path, requested: Option<&str>) -> Result<PathBuf, String> {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(root.to_path_buf());
    };
    let path = PathBuf::from(requested);
    if !path.is_dir() {
        return Err(format!("任务工作区不存在或不是目录：{requested}"));
    }
    Ok(path)
}

fn default_model() -> String {
    std::env::var("OPENAI_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| owo_agent_core::gateway::DEFAULT_MODEL_ID.to_string())
}
