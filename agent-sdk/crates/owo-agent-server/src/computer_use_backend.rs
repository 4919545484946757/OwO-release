//! Computer Use 执行后端（Agent V2 §6 任务 2.2 的第二个 Adapter）。
//!
//! 职责边界：
//! - 把统一 Task 的 `start` 接到既有的**感知闭环** `run_approved_task_on`：窗口身份
//!   复核、OCR 锚点定位、动作门禁、敏感 UI 熔断、动作后验证全部复用，不另起一套
//!   桌面执行栈；
//! - **不绕过任务级审批**：第一次分发只为统一任务创建 `Pending` 的 ComputerTask，
//!   随后 `require_user_input(APPROVAL_REQUIRED)`；用户用既有的
//!   `/computer-use/task/{id}/approve` 批准后，再次 `start` 才会真的操作桌面；
//! - 只向 Task Runtime 提交**执行事实**（步数、动作回执、观察引用），是否成功由
//!   独立验证决定——每步都带 `verify_text` 且动作后 OCR 命中，才提交 `accepted`。
//!
//! 取消：`cancel` 把 ComputerTask 置为 `Cancelled`；闭环在下一个门禁点停下，迟到的
//! 结果由运行时按取消语义丢弃，不会翻成成功。

mod recovery;
use recovery::{JournalSurface, RecoveryJournal, RECOVERY_REQUIRED};

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use owo_agent_core::audit::AuditLog;
use owo_agent_core::computer_task::{ComputerTask, ComputerTaskRegistry};
use owo_agent_core::computer_use::{
    codes as computer_codes, gate_action_name, run_approved_task_on_with_receipts,
    sim_base_url_configured, validate_goals, ActionExecutionReceipt, ActionVerification,
    RealTaskSurface, SimTaskSurface, TaskGoal, TaskReport, TaskSurface,
};
use owo_agent_core::task_runtime::{
    codes, event_type, BackendRunId, BackendRunReport, BackendSnapshot, CheckResult,
    ExecutionStrategy, TaskError, TaskExecutionBackend, TaskResult, TaskRunContext, TaskRuntime,
    VerificationEvidence,
};
use serde::Deserialize;

/// 任务未声明时长时的默认上限（毫秒）。
pub const DEFAULT_MAX_DURATION_MS: u64 = 10 * 60 * 1000;

/// 闭环超时之外额外留给最后一轮验证的余量。
const TIMEOUT_GRACE: Duration = Duration::from_secs(5);

const CODE_HUMAN_TAKEOVER: &str = "HUMAN_TAKEOVER_REQUIRED";
const CODE_APPROVAL_REJECTED: &str = "APPROVAL_REJECTED";
const CODE_ALREADY_COMPLETED: &str = "ALREADY_COMPLETED";
const CODE_CANCELLED: &str = "CANCELLED";
const CODE_BACKEND_ERROR: &str = "BACKEND_ERROR";
const CODE_BACKEND_BUSY: &str = "BACKEND_BUSY";

/// 暂停/恢复时等待旧闭环退出门禁点的上限。
///
/// 闭环只在步与步之间检查任务状态，OCR 或动作本身可能还在路上；恢复必须等它
/// 真的停下，否则两个循环会同时向桌面发输入。
const RESUME_WAIT: Duration = Duration::from_secs(5);

/// 统一任务 `payload` 里 computer-use 专属的输入（Runtime 不解释）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ComputerUsePayload {
    /// 目标应用（进程名/白名单名，如 qq / notepad / msedge）。创建任务时必填。
    #[serde(default)]
    pub target_app: Option<String>,
    /// Optional title of the approved document window, before any unsaved marker.
    #[serde(default)]
    pub target_window_title: Option<String>,
    /// 闭环目标步骤。空目标永远不能进入成功路径。
    #[serde(default)]
    pub goals: Vec<TaskGoal>,
    /// 覆盖任务时长上限（毫秒）。
    #[serde(default)]
    pub max_duration_ms: Option<u64>,
    /// 动作次数预算（显式 0 = 不限；缺省用注册表默认值）。
    #[serde(default)]
    pub action_budget: Option<u32>,
}

/// 桌面面工厂：`None` 时按运行环境自动选择（模拟面 / 真实桌面面）。
pub type SurfaceFactory = dyn Fn() -> Result<Box<dyn TaskSurface>, String> + Send + Sync;

pub struct ComputerUseBackend {
    /// 弱引用：TaskRuntime 持有本后端，强引用会成环。
    runtime: Weak<TaskRuntime>,
    /// 与 HTTP `/computer-use/*` 共用同一个注册表（审批在同一个地方完成）。
    registry: Arc<ComputerTaskRegistry>,
    /// 与 Agent 共用的审计日志（闭环结束后把 scratch 审计并回去）。
    audit: Arc<Mutex<AuditLog>>,
    /// 桌面面工厂；测试注入 Mock，生产按运行环境自动选模拟面/真实面。
    surface_factory: Option<Arc<SurfaceFactory>>,
    /// backend_run_id → computer task id（取消时据此定位）。
    runs: Arc<Mutex<HashMap<String, String>>>,
}

impl ComputerUseBackend {
    pub fn new(
        runtime: Weak<TaskRuntime>,
        registry: Arc<ComputerTaskRegistry>,
        audit: Arc<Mutex<AuditLog>>,
    ) -> Self {
        Self {
            runtime,
            registry,
            audit,
            surface_factory: None,
            runs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 注入桌面面工厂（测试用；生产不设置）。
    pub fn with_surface_factory(mut self, factory: Arc<SurfaceFactory>) -> Self {
        self.surface_factory = Some(factory);
        self
    }

    fn make_surface(&self) -> Result<Box<dyn TaskSurface>, String> {
        if let Some(factory) = &self.surface_factory {
            return factory();
        }
        if sim_base_url_configured() {
            return Ok(Box::new(SimTaskSurface::new()?));
        }
        Ok(Box::new(RealTaskSurface))
    }

    fn active_run_for(&self, computer_task_id: &str) -> Option<String> {
        self.runs.lock().ok().and_then(|runs| {
            runs.iter()
                .find(|(_, id)| id.as_str() == computer_task_id)
                .map(|(run, _)| run.clone())
        })
    }

    /// 等旧闭环退出（它会在下一个门禁点看到暂停而停下）。
    async fn wait_for_run_exit(&self, computer_task_id: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.active_run_for(computer_task_id).is_some() {
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        true
    }

    /// 首次分发：按 payload 创建 Pending 的 ComputerTask（id 与统一任务相同，
    /// 这样审批动作和任务详情指的是同一个标识）。
    fn create_pending(
        &self,
        context: &TaskRunContext,
        payload: &ComputerUsePayload,
    ) -> Result<ComputerTask, TaskError> {
        if let Err(message) = validate_goals(&payload.goals) {
            let code = split_code(&message)
                .unwrap_or(codes::INVALID_REQUEST)
                .to_string();
            return Err(TaskError::new(code, message));
        }
        let target_app = payload
            .target_app
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                TaskError::new(
                    codes::INVALID_REQUEST,
                    "computer_use 任务需要在 payload.target_app 声明目标应用",
                )
            })?;
        if target_app.eq_ignore_ascii_case("notepad") && payload.target_window_title.is_some() {
            for goal in payload
                .goals
                .iter()
                .take(payload.goals.len().saturating_sub(1))
            {
                if goal.action != "key" {
                    continue;
                }
                let mut keys: Vec<_> = goal
                    .value
                    .split('+')
                    .map(|key| match key.trim().to_ascii_lowercase().as_str() {
                        "control" => "ctrl".to_owned(),
                        key => key.to_owned(),
                    })
                    .collect();
                keys.sort_unstable();
                if keys == ["ctrl", "s", "shift"] {
                    return Err(TaskError::new(
                        codes::INVALID_REQUEST,
                        "记事本另存为必须是当前窗口计划的最后一步；请单独确认新对话框后再操作",
                    ));
                }
            }
        }
        let mut allowed_actions: Vec<String> = Vec::new();
        for goal in &payload.goals {
            if goal.action == "set_value" && payload.target_window_title.is_none() {
                return Err(TaskError::new(
                    codes::INVALID_REQUEST,
                    "设置编辑框值需要指定窗口标题",
                ));
            }
            let action = gate_action_name(&goal.action)
                .map_err(|error| TaskError::new(codes::INVALID_ACTION, error))?;
            if !allowed_actions.iter().any(|existing| existing == action) {
                allowed_actions.push(action.to_string());
            }
        }
        let max_duration_ms = payload
            .max_duration_ms
            .filter(|value| *value > 0)
            .or(Some(context.budget.max_duration_ms).filter(|value| *value > 0))
            .unwrap_or(DEFAULT_MAX_DURATION_MS);
        let task = ComputerTask {
            id: context.task_id.as_str().to_string(),
            target_app: target_app.to_string(),
            description: context.user_goal.clone(),
            allowed_actions,
            max_duration_ms,
            state: owo_agent_core::computer_task::TaskState::Pending,
            created_at: chrono::Utc::now().to_rfc3339(),
            fuse_reason: None,
        };
        self.registry
            .create(task.clone())
            .map_err(|error| TaskError::new(codes::INVALID_REQUEST, error))?;
        if let Some(budget) = payload.action_budget {
            let _ = self.registry.set_action_budget(&task.id, budget);
        }
        Ok(task)
    }
}

#[async_trait::async_trait]
impl TaskExecutionBackend for ComputerUseBackend {
    fn kind(&self) -> ExecutionStrategy {
        ExecutionStrategy::ComputerUse
    }

    async fn start(&self, context: TaskRunContext) -> TaskResult<BackendRunId> {
        let run_id = BackendRunId(uuid::Uuid::new_v4().to_string());
        let Some(runtime) = self.runtime.upgrade() else {
            return Err(TaskError::new(
                codes::STORE_ERROR,
                "任务运行时已释放，无法分发 computer-use 执行",
            ));
        };
        let task_id = context.task_id.as_str().to_string();

        let payload: ComputerUsePayload = match context.payload.as_ref() {
            Some(value) => match serde_json::from_value(value.clone()) {
                Ok(payload) => payload,
                Err(error) => {
                    let error = TaskError::new(
                        codes::INVALID_REQUEST,
                        format!("computer_use payload 无法解析：{error}"),
                    );
                    let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
                    return Err(error);
                }
            },
            None => {
                let error = TaskError::new(
                    codes::INVALID_REQUEST,
                    "computer_use 任务缺少 payload（target_app / goals）",
                );
                let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
                return Err(error);
            }
        };

        // 同一个统一任务同时只允许一个闭环在跑（避免两个循环同时向桌面发输入）。
        // 例外：暂停后的恢复要等旧闭环在门禁点停下，而不是直接判定"已在运行"。
        if self.active_run_for(&task_id).is_some() {
            let paused = self
                .registry
                .get(&task_id)
                .map(|task| task.state == owo_agent_core::computer_task::TaskState::Paused)
                .unwrap_or(false);
            if !(paused && self.wait_for_run_exit(&task_id, RESUME_WAIT).await) {
                let error = TaskError::new(
                    codes::INVALID_REQUEST,
                    format!("computer-use 任务 {task_id} 已有执行在运行"),
                );
                let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
                return Err(error);
            }
        }

        let computer_task = match self.registry.get(&task_id) {
            Some(task) => task,
            None => {
                let task = match self.create_pending(&context, &payload) {
                    Ok(task) => task,
                    Err(error) => {
                        let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
                        return Err(error);
                    }
                };
                // 审批是 computer-use 的硬门禁：不在后端里"自动批准"。
                let message = format!(
                    "computer-use 任务 {} 等待人工批准（目标应用 {}，{} 步）；批准后再次 start 执行",
                    task.id,
                    task.target_app,
                    payload.goals.len()
                );
                let _ = runtime.require_user_input(&task_id, codes::APPROVAL_REQUIRED, &message);
                return Err(TaskError::new(codes::APPROVAL_REQUIRED, message));
            }
        };

        // 已存在的 ComputerTask 也可能是空目标（老接口创建），仍然不允许执行。
        if let Err(message) = validate_goals(&payload.goals) {
            let code = split_code(&message)
                .unwrap_or(codes::EMPTY_GOALS)
                .to_string();
            let error = TaskError::new(code, message);
            let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
            return Err(error);
        }

        match computer_task.state {
            owo_agent_core::computer_task::TaskState::Approved
            | owo_agent_core::computer_task::TaskState::Running => {}
            owo_agent_core::computer_task::TaskState::Pending => {
                let message = format!(
                    "computer-use 任务 {} 仍等待人工批准（目标应用 {}）；批准后再次 start 执行",
                    computer_task.id, computer_task.target_app
                );
                let _ = runtime.require_user_input(&task_id, codes::APPROVAL_REQUIRED, &message);
                return Err(TaskError::new(codes::APPROVAL_REQUIRED, message));
            }
            owo_agent_core::computer_task::TaskState::Paused => {
                // 用户在统一任务里点"继续"会重新分发到这里：先确认旧闭环已经停下，
                // 再把 ComputerTask 恢复成可执行。暂停不是敏感熔断，不需要人工再批一次。
                if !self.wait_for_run_exit(&computer_task.id, RESUME_WAIT).await {
                    let message = format!(
                        "computer-use 任务 {} 的旧执行仍未停下，稍后重试或先取消",
                        computer_task.id
                    );
                    let _ = runtime.require_user_input(&task_id, CODE_BACKEND_BUSY, &message);
                    return Err(TaskError::new(CODE_BACKEND_BUSY, message));
                }
                if let Err(error) = self.registry.resume(&computer_task.id) {
                    let message = format!(
                        "computer-use 任务 {} 无法恢复（{error}）；需人工接管后再 start",
                        computer_task.id
                    );
                    let _ = runtime.require_user_input(&task_id, CODE_HUMAN_TAKEOVER, &message);
                    return Err(TaskError::new(CODE_HUMAN_TAKEOVER, message));
                }
            }
            owo_agent_core::computer_task::TaskState::Fused => {
                let reason = computer_task.fuse_reason.clone().unwrap_or_default();
                let message = format!(
                    "computer-use 任务 {} 处于 Fused（{}）；需人工接管后先 resume 再 start",
                    computer_task.id, reason
                );
                let _ = runtime.require_user_input(&task_id, CODE_HUMAN_TAKEOVER, &message);
                return Err(TaskError::new(CODE_HUMAN_TAKEOVER, message));
            }
            owo_agent_core::computer_task::TaskState::Rejected => {
                let error = TaskError::new(
                    CODE_APPROVAL_REJECTED,
                    format!("computer-use 任务 {} 已被拒绝，不执行", computer_task.id),
                );
                let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
                return Err(error);
            }
            owo_agent_core::computer_task::TaskState::Cancelled => {
                let error = TaskError::new(
                    CODE_CANCELLED,
                    format!("computer-use 任务 {} 已取消，不执行", computer_task.id),
                );
                let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
                return Err(error);
            }
            owo_agent_core::computer_task::TaskState::Completed => {
                // 已完成的桌面任务不重放副作用：重跑要新建任务并重新审批。
                let error = TaskError::new(
                    CODE_ALREADY_COMPLETED,
                    format!(
                        "computer-use 任务 {} 已完成，拒绝重复执行（避免重复副作用）",
                        computer_task.id
                    ),
                );
                let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
                return Err(error);
            }
        }

        let surface = match self.make_surface() {
            Ok(surface) => surface,
            Err(error) => {
                let error = TaskError::new(CODE_BACKEND_ERROR, error);
                let _ = runtime.fail_backend_run(&task_id, &error.code, &error.message);
                return Err(error);
            }
        };

        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(run_id.0.clone(), computer_task.id.clone());
        }
        let registry = Arc::clone(&self.registry);
        let audit = Arc::clone(&self.audit);
        let runs = Arc::clone(&self.runs);
        let backend_run_id = run_id.0.clone();
        let goals = payload.goals;
        tokio::spawn(async move {
            run_computer_task(
                runtime,
                registry,
                audit,
                context,
                backend_run_id.clone(),
                computer_task,
                goals,
                surface,
                payload.target_window_title,
            )
            .await;
            if let Ok(mut runs) = runs.lock() {
                runs.remove(&backend_run_id);
            }
        });
        Ok(run_id)
    }

    async fn pause(&self, run_id: &BackendRunId) -> TaskResult<()> {
        let computer_task_id = self
            .runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(&run_id.0).cloned())
            .ok_or_else(|| {
                TaskError::new(
                    codes::TASK_NOT_FOUND,
                    format!("computer-use 运行 {} 已结束或不存在", run_id.0),
                )
            })?;
        self.registry
            .pause(&computer_task_id, "用户暂停 computer-use 执行")
            .map(|_| ())
            .map_err(|error| TaskError::new(codes::INVALID_TRANSITION, error))
    }

    async fn resume(&self, run_id: &BackendRunId) -> TaskResult<()> {
        let Some(computer_task_id) = self
            .runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(&run_id.0).cloned())
        else {
            // 运行表里已经没有它：旧闭环早已结束，恢复交给下一次 start 处理。
            return Ok(());
        };
        if !self.wait_for_run_exit(&computer_task_id, RESUME_WAIT).await {
            return Err(TaskError::new(
                CODE_BACKEND_BUSY,
                format!("computer-use 任务 {computer_task_id} 的旧执行仍未停下"),
            ));
        }
        let state = self
            .registry
            .get(&computer_task_id)
            .map(|task| task.state)
            .ok_or_else(|| {
                TaskError::new(
                    codes::TASK_NOT_FOUND,
                    format!("computer-use 任务 {computer_task_id} 不存在"),
                )
            })?;
        if state == owo_agent_core::computer_task::TaskState::Paused {
            self.registry
                .resume(&computer_task_id)
                .map(|_| ())
                .map_err(|error| TaskError::new(codes::INVALID_TRANSITION, error))?;
        }
        Ok(())
    }

    async fn cancel(&self, run_id: &BackendRunId) -> TaskResult<()> {
        let computer_task_id = self
            .runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(&run_id.0).cloned())
            .ok_or_else(|| {
                TaskError::new(
                    codes::TASK_NOT_FOUND,
                    format!("computer-use 运行 {} 已结束或不存在", run_id.0),
                )
            })?;
        self.registry
            .cancel(&computer_task_id)
            .map(|_| ())
            .map_err(|error| TaskError::new(codes::INVALID_TRANSITION, error))
    }

    async fn inspect(&self, run_id: &BackendRunId) -> TaskResult<BackendSnapshot> {
        let computer_task_id = self
            .runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(&run_id.0).cloned())
            .ok_or_else(|| {
                TaskError::new(
                    codes::TASK_NOT_FOUND,
                    format!("computer-use 运行 {} 已结束或不存在", run_id.0),
                )
            })?;
        let task = self.registry.get(&computer_task_id).ok_or_else(|| {
            TaskError::new(
                codes::TASK_NOT_FOUND,
                format!("computer-use 任务 {computer_task_id} 不存在"),
            )
        })?;
        Ok(BackendSnapshot {
            state: format!("{:?}", task.state).to_lowercase(),
            detail: serde_json::to_value(task).unwrap_or(serde_json::Value::Null),
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_computer_task(
    runtime: Arc<TaskRuntime>,
    registry: Arc<ComputerTaskRegistry>,
    audit: Arc<Mutex<AuditLog>>,
    context: TaskRunContext,
    backend_run_id: String,
    computer_task: ComputerTask,
    goals: Vec<TaskGoal>,
    surface: Box<dyn TaskSurface>,
    target_window_title: Option<String>,
) {
    let task_id = context.task_id.as_str().to_string();
    let task_store = runtime.store();
    let _ = task_store.append_event(
        &task_id,
        event_type::TASK_COMPUTER_STARTED,
        context.attempt,
        serde_json::json!({
            "journal_version": 1,
            "plan_ref": RecoveryJournal::plan_ref(&context),
            "computer_task_id": computer_task.id,
            "target_app": computer_task.target_app,
            "goals": goals.len(),
            "allowed_actions": computer_task.allowed_actions,
        }),
        Some(&backend_run_id),
    );

    let started = Instant::now();
    let max_duration = Duration::from_millis(computer_task.max_duration_ms.max(1_000));
    // 闭环本身按步做门禁与超时判定；这里再加一层硬上限，防止单步阻塞把任务挂死。
    let mut scratch = AuditLog::default();
    let session_label = format!("agent-task:{task_id}");
    let outcome = match RecoveryJournal::load(
        Arc::clone(&runtime),
        Arc::clone(&registry),
        &context,
        &backend_run_id,
        &goals,
    ) {
        Ok((journal, remaining)) => {
            let mut surface = JournalSurface::new(surface, journal.clone(), target_window_title);
            let mut record_receipt =
                |stage, receipt: &ActionExecutionReceipt| journal.record(stage, receipt);
            let result = if remaining.is_empty() {
                Ok(registry
                    .complete(&computer_task.id)
                    .map(|state| TaskReport {
                        task_id: computer_task.id.clone(),
                        steps: 0,
                        state,
                        detail: "全部步骤已有确认回执，没有新输入".into(),
                        receipts: Vec::new(),
                    }))
            } else {
                tokio::time::timeout(
                    max_duration + TIMEOUT_GRACE,
                    run_approved_task_on_with_receipts(
                        &registry,
                        &mut scratch,
                        &session_label,
                        &computer_task.id,
                        &remaining,
                        &mut surface,
                        &mut record_receipt,
                    ),
                )
                .await
            };
            result.map(|inner| {
                inner.and_then(|mut report| {
                    report.receipts = journal.receipts()?;
                    Ok(report)
                })
            })
        }
        Err(error) => {
            let _ = registry.pause_if_executable(&computer_task.id, &error);
            Ok(Err(error))
        }
    };
    if let Ok(mut log) = audit.lock() {
        log.entries.extend(scratch.entries);
    }
    crate::event_stream::hub().publish_invalidate(crate::event_stream::InvalidateDomain::Computer);

    let duration_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Err(_elapsed) => {
            let _ = registry.pause(
                &computer_task.id,
                &format!("执行超过最长时长 {}ms", computer_task.max_duration_ms),
            );
            let _ = runtime.fail_backend_run(
                &task_id,
                codes::BUDGET_EXCEEDED,
                &format!(
                    "computer-use 执行超过最长时长 {}ms，已强制中止",
                    computer_task.max_duration_ms
                ),
            );
        }
        Ok(Err(message)) => {
            let code = split_code(&message).map(str::to_string).unwrap_or_else(|| {
                match registry.get(&computer_task.id).map(|task| task.state) {
                    Some(owo_agent_core::computer_task::TaskState::Fused) => {
                        CODE_HUMAN_TAKEOVER.to_string()
                    }
                    _ => CODE_BACKEND_ERROR.to_string(),
                }
            });
            let _ = task_store.append_event(
                &task_id,
                event_type::TASK_BACKEND_FAILED,
                context.attempt,
                serde_json::json!({"code": code, "message": message}),
                Some(&backend_run_id),
            );
            if matches!(
                code.as_str(),
                computer_codes::STALE_WINDOW_IDENTITY
                    | computer_codes::FOREGROUND_CHANGED
                    | computer_codes::TARGET_NOT_FOUND
                    | computer_codes::ANCHOR_AMBIGUOUS
                    | RECOVERY_REQUIRED
            ) {
                let _ = runtime.require_user_input(&task_id, &code, &message);
            } else {
                let _ = runtime.fail_backend_run(&task_id, &code, &message);
            }
        }
        Ok(Ok(report)) => {
            let verified = report
                .receipts
                .iter()
                .filter(|receipt| receipt.verification == ActionVerification::Verified)
                .count();
            let failed = report
                .receipts
                .iter()
                .filter(|receipt| {
                    matches!(
                        receipt.verification,
                        ActionVerification::Failed { .. } | ActionVerification::NotVerified
                    )
                })
                .count();
            let all_verified = !report.receipts.is_empty() && verified == report.receipts.len();
            // 每步都有 verify_text 且命中才算 accepted；有目标未声明验证文本时只能算
            // candidate，交给人或后续验证阶段判定，不自己宣布成功。
            let completion_status = if all_verified {
                "accepted"
            } else {
                "candidate"
            };
            let _ = task_store.append_event(
                &task_id,
                event_type::TASK_COMPUTER_COMPLETED,
                context.attempt,
                serde_json::json!({
                    "computer_task_id": computer_task.id,
                    "steps": report.receipts.len(),
                    "executed_steps": report.steps,
                    "receipts": report.receipts.len(),
                    "verified": verified,
                    "unverified_or_failed": failed,
                    "duration_ms": duration_ms,
                }),
                Some(&backend_run_id),
            );
            let run_report = BackendRunReport {
                backend_run_id: backend_run_id.clone(),
                session_id: None,
                steps: report.receipts.len(),
                // 历史确认步骤只复用回执；工具次数仅计本次实际输入。
                tool_calls: report.steps,
                duration_ms,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                completion_status: Some(completion_status.to_string()),
                completion_record: serde_json::to_value(&report.receipts).ok(),
                final_text: Some(format!(
                    "computer-use 任务 {} 完成 {} 步，本次新输入 {} 次（{verified} 个验证通过）",
                    computer_task.id,
                    report.receipts.len(),
                    report.steps
                )),
            };
            if let Err(error) = runtime.complete_backend_run(&task_id, run_report) {
                tracing::warn!("任务 {task_id} 提交 computer-use 执行事实失败：{error}");
            }
            if all_verified {
                let evidence = VerificationEvidence {
                    execution: Some(CheckResult::pass(format!(
                        "computer-use 闭环执行 {} 步，动作门禁全部通过",
                        report.steps
                    ))),
                    side_effect: Some(CheckResult::pass(format!(
                        "{} 个动作回执均有动作后观察引用",
                        report.receipts.len()
                    ))),
                    goal: Some(CheckResult::pass(format!(
                        "持久回执确认每步动作后 OCR 命中 verify_text（{verified}/{}）",
                        report.receipts.len()
                    ))),
                    ..Default::default()
                };
                if let Err(error) = runtime.verify_task(&task_id, evidence) {
                    tracing::warn!("任务 {task_id} 自动验证失败：{error}");
                }
            }
        }
    }
}

/// 闭环错误是 `CODE: message` 形式时取出稳定错误码；否则返回 None。
fn split_code(message: &str) -> Option<&str> {
    let (head, _) = message.split_once(": ")?;
    let head = head.trim();
    if !head.is_empty()
        && head.len() <= 40
        && head
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit())
    {
        Some(head)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_code_recognises_stable_codes_only() {
        assert_eq!(
            split_code("ANCHOR_AMBIGUOUS: 找到 2 个候选"),
            Some("ANCHOR_AMBIGUOUS")
        );
        assert_eq!(split_code("第 1 步动作执行失败：超时"), None);
        assert_eq!(split_code("plain message"), None);
    }
}
