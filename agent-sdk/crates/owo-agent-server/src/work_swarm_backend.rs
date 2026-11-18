//! WorkSwarm 执行后端（统一 Task Runtime 的第三个 Adapter）。
//!
//! 职责边界：
//! - 把统一 Task 的 `start` 变成**一次真实的团队运行**（`TeamCoordinator::create_team_run`
//!   + 既有的 `run_team_loop`），复用模板/角色/写范围/审批/交付门，不另起编排栈；
//! - 只把团队运行的事实回报给 Task Runtime：步骤事件、Project Space 产物、完成判定记录；
//!   是否 `succeeded` 仍由独立验证决定；
//! - 暂停 = 团队运行的真实取消（WorkSwarm 的 cancel 立即置取消令牌，运行中 Worker 在
//!   回合边界协作中断）；恢复 = `continue` 同一团队运行（已完成步骤不重跑），而不是
//!   新建一次团队把副作用重放一遍；
//! - 取消 = 团队运行取消；统一 Task 的取消语义由运行时收口。
//!
//! 依赖 `Arc<AppState>`：团队运行循环需要服务状态来构建 Worker 注册表，因此在
//! `build_router` 拿到 `Arc<AppState>` 后注册（`AppState::new` 阶段还没有 Arc）。
//! 后端只持有弱引用，不构成引用环。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use owo_agent_core::plan::StepStatus;
use owo_agent_core::task_runtime::{
    codes, BackendRunId, BackendRunReport, BackendSnapshot, CheckResult, ExecutionStrategy,
    TaskError, TaskExecutionBackend, TaskResult, TaskRunContext, TaskRuntime, TaskState,
    VerificationEvidence,
};
use owo_agent_core::workswarm::{
    CreateTeamRequest, RoleSpec, SteerCommand, TeamCoordinator, WorkSwarmError,
};
use owo_agent_protocol::{TeamMode, TeamRun, TeamRunStatus};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;

/// 团队运行失败（含收尾失败）的稳定错误码。
const CODE_TEAM_RUN_FAILED: &str = "TEAM_RUN_FAILED";
/// 团队运行在统一任务之外被取消。
const CODE_TEAM_RUN_CANCELLED: &str = "TEAM_RUN_CANCELLED";
/// 团队运行仍在收尾，暂时无法 continue（可重试）。
const CODE_TEAM_BUSY: &str = "TEAM_BUSY";

/// 监控团队运行的轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// `continue` 的重试次数与间隔：取消后运行循环收尾需要一点时间，
/// 这段窗口内 WorkSwarm 会返回 Conflict（阶段仍在执行）。
const RESUME_ATTEMPTS: usize = 25;
const RESUME_RETRY: Duration = Duration::from_millis(200);

/// 统一任务 `payload` 里 WorkSwarm 专属的输入（Runtime 不解释）。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WorkSwarmPayload {
    /// 覆盖任务目标（缺省用统一任务的 `user_goal`）。
    objective: Option<String>,
    /// 模板 id（`swarmflow` 必填；`team` 可选；`single` 忽略）。
    template_id: Option<String>,
    /// `single` | `team`（缺省） | `swarmflow`。
    mode: Option<String>,
    /// 动态角色（空 = 按模式取模板默认 / 内置默认接力）。
    roles: Vec<RoleSpec>,
    /// 团队预算（原样透传给 TeamRun；`max_cost_usd` / `max_wall_secs` / `max_model_calls`）。
    budget: Value,
    human_policy: Option<String>,
    /// 组队策略：`auto` | `single` | `team`（缺省 auto）。
    strategy: Option<String>,
    /// 团队统一模型（角色 `model` 显式覆盖）。
    model: Option<String>,
    /// 并行开发模式（lead 拆解 → wN 并行 → leader 汇总）。
    parallel: bool,
    max_agent_members: Option<usize>,
    /// 项目工作区绑定（root 必须已存在；缺省只读）。
    workspace: Option<crate::workswarm_api::project_workspace::WorkspaceSpec>,
    /// 允许角色**在声明的 write_paths 内**写入绑定工作区。
    allow_scoped_writes: bool,
}

pub struct WorkSwarmBackend {
    /// 服务状态（团队运行循环需要它构建 Worker 注册表）。
    state: Weak<AppState>,
    /// 统一任务运行时（提交执行事实 / 查询任务状态）。
    runtime: Weak<TaskRuntime>,
    /// 统一任务 id → 团队运行 id（恢复时继续同一次运行，不重放已完成的步骤）。
    teams: Arc<Mutex<HashMap<String, String>>>,
    /// 正在监控的团队运行（防止重复 start/resume 起两个监控器）。
    monitors: Arc<Mutex<HashSet<String>>>,
}

impl WorkSwarmBackend {
    pub fn new(state: Weak<AppState>, runtime: Weak<TaskRuntime>) -> Self {
        Self {
            state,
            runtime,
            teams: Arc::new(Mutex::new(HashMap::new())),
            monitors: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn coordinator(&self) -> TaskResult<Arc<TeamCoordinator>> {
        let state = self.state.upgrade().ok_or_else(|| {
            TaskError::new(
                codes::STORE_ERROR,
                "服务状态已释放，无法操作 WorkSwarm 团队运行",
            )
        })?;
        state.workswarm.coordinator().map_err(|error| {
            TaskError::new(
                codes::STORE_ERROR,
                format!("WorkSwarm 协调器不可用：{error}"),
            )
        })
    }

    fn team_for_task(&self, task_id: &str) -> TaskResult<Option<String>> {
        let teams = self
            .teams
            .lock()
            .map_err(|_| TaskError::new(codes::STORE_ERROR, "团队运行映射已损坏"))?;
        Ok(teams.get(task_id).cloned())
    }

    /// 已存在的团队运行：能继续就继续，已成功就补收尾，已终止就明确报错。
    async fn resume_existing(
        &self,
        state: &Arc<AppState>,
        runtime: &Arc<TaskRuntime>,
        coordinator: &Arc<TeamCoordinator>,
        task_id: &str,
        team_id: &str,
    ) -> TaskResult<BackendRunId> {
        let team = match coordinator.get_team_run(team_id).await {
            Ok(team) => team,
            // 运行记录不在了（数据被清理）：丢弃映射，按新任务重新建队。
            Err(WorkSwarmError::NotFound(_)) => {
                if let Ok(mut teams) = self.teams.lock() {
                    teams.remove(task_id);
                }
                return Err(TaskError::new(
                    codes::TASK_NOT_FOUND,
                    format!("团队运行 {team_id} 已不存在，请重新创建任务"),
                ));
            }
            Err(error) => {
                return Err(TaskError::new(
                    codes::STORE_ERROR,
                    format!("读取团队运行失败：{error}"),
                ));
            }
        };
        match team.status {
            TeamRunStatus::Created | TeamRunStatus::Running | TeamRunStatus::AwaitingHuman => {
                // 仍在运行：幂等返回同一个运行 id，不再起第二个循环。
                self.spawn_monitor(
                    Arc::clone(runtime),
                    Arc::clone(coordinator),
                    task_id.to_string(),
                    team_id.to_string(),
                );
                Ok(BackendRunId(team_id.to_string()))
            }
            TeamRunStatus::Succeeded => {
                // 团队已跑完但统一任务还没收尾（例如暂停期间完成的）：交给监控器补收尾。
                self.spawn_monitor(
                    Arc::clone(runtime),
                    Arc::clone(coordinator),
                    task_id.to_string(),
                    team_id.to_string(),
                );
                Ok(BackendRunId(team_id.to_string()))
            }
            TeamRunStatus::Cancelled | TeamRunStatus::Failed => {
                // 暂停触发的取消 / 中断恢复：continue 同一运行（已完成步骤不重跑）。
                self.continue_team(coordinator, team_id).await?;
                tokio::spawn(crate::workswarm_api::run_team_loop(
                    Arc::clone(state),
                    Arc::clone(coordinator),
                    team_id.to_string(),
                ));
                self.spawn_monitor(
                    Arc::clone(runtime),
                    Arc::clone(coordinator),
                    task_id.to_string(),
                    team_id.to_string(),
                );
                Ok(BackendRunId(team_id.to_string()))
            }
        }
    }

    /// `continue` 需要运行循环不在飞行中；暂停后循环收尾需要短暂等待。
    async fn continue_team(
        &self,
        coordinator: &Arc<TeamCoordinator>,
        team_id: &str,
    ) -> TaskResult<()> {
        let mut last_conflict = None;
        for _ in 0..RESUME_ATTEMPTS {
            // 取消后旧运行循环还需要一点时间退出；它未退时 `continue` 就算成功，
            // 新循环也会被 `try_claim_loop` 拒之门外——先等它真正退出。
            if coordinator.is_loop_alive(team_id) {
                last_conflict = Some("运行循环仍在收尾".to_string());
                tokio::time::sleep(RESUME_RETRY).await;
                continue;
            }
            match coordinator
                .apply_steer(team_id, &SteerCommand::Continue)
                .await
            {
                Ok(_) => return Ok(()),
                Err(WorkSwarmError::Conflict(message)) => {
                    last_conflict = Some(message);
                    tokio::time::sleep(RESUME_RETRY).await;
                }
                Err(error) => {
                    return Err(TaskError::new(
                        codes::INVALID_REQUEST,
                        format!("恢复团队运行失败：{error}"),
                    ));
                }
            }
        }
        Err(TaskError::new(
            CODE_TEAM_BUSY,
            format!(
                "团队运行仍在收尾，暂时无法恢复：{}",
                last_conflict.unwrap_or_default()
            ),
        ))
    }

    /// 新建一次团队运行并启动运行循环。
    async fn create_team(
        &self,
        state: &Arc<AppState>,
        runtime: &Arc<TaskRuntime>,
        coordinator: &Arc<TeamCoordinator>,
        context: &TaskRunContext,
    ) -> TaskResult<BackendRunId> {
        let task_id = context.task_id.as_str().to_string();
        let payload: WorkSwarmPayload = match context.payload.as_ref() {
            None | Some(Value::Null) => WorkSwarmPayload::default(),
            Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
                TaskError::new(
                    codes::INVALID_REQUEST,
                    format!("work_swarm payload 无效：{error}"),
                )
            })?,
        };
        let mode = match payload.mode.as_deref() {
            None | Some("") => TeamMode::Team,
            Some("single") => TeamMode::Single,
            Some("team") => TeamMode::Team,
            Some("swarmflow") => TeamMode::Swarmflow,
            Some(other) => {
                return Err(TaskError::new(
                    codes::INVALID_REQUEST,
                    format!("未知 work_swarm mode：{other}（支持 single/team/swarmflow）"),
                ));
            }
        };
        let strategy = match payload.strategy.as_deref() {
            None | Some("") => None,
            Some(raw) => Some(
                owo_agent_core::team_strategy::TeamSelectionMode::parse(raw).map_err(
                    |message| {
                        TaskError::new(codes::INVALID_REQUEST, format!("组队策略无效：{message}"))
                    },
                )?,
            ),
        };
        // 工作区绑定先校验（路径非法时不建队、不执行任何步骤）。
        // 统一任务显式指定了 workspace_ref 时，缺省绑定到该目录（可写），
        // 与 Single Agent 后端的 workspace_ref 语义保持一致。
        let workspace_spec = payload.workspace.clone().or_else(|| {
            context
                .workspace_ref
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(
                    |root| crate::workswarm_api::project_workspace::WorkspaceSpec {
                        root: root.to_string(),
                        read_only: false,
                        write_allowed_paths: Vec::new(),
                        tree_depth: None,
                    },
                )
        });
        let workspace_binding = match workspace_spec.as_ref() {
            Some(spec) => Some(
                crate::workswarm_api::project_workspace::validate_workspace_spec(spec).map_err(
                    |message| {
                        TaskError::new(
                            codes::INVALID_REQUEST,
                            format!("work_swarm workspace 无效：{message}"),
                        )
                    },
                )?,
            ),
            None => None,
        };
        let mut budget = payload.budget.clone();
        match budget.as_object_mut() {
            Some(object) => {
                object.insert(
                    "allow_scoped_writes".to_string(),
                    json!(payload.allow_scoped_writes),
                );
            }
            None => {
                budget = json!({ "allow_scoped_writes": payload.allow_scoped_writes });
            }
        }
        let mut request = CreateTeamRequest::new(
            payload
                .objective
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(&context.user_goal),
            mode,
        );
        request.template_id = payload.template_id;
        request.roles = payload.roles;
        request.budget = budget;
        request.human_policy = payload.human_policy;
        request.strategy = strategy;
        request.model = payload.model.filter(|value| !value.trim().is_empty());
        request.parallel = payload.parallel;
        request.max_agent_members = payload.max_agent_members;
        // 工作流工具面预检：注册表里没有的工具直接拒绝建队，不等到 Worker 执行才失败。
        let team = coordinator
            .create_team_run_with_preflight(&request, |roles, parallel, template_id| {
                state
                    .agent
                    .preflight_workflow_roles(roles, parallel, template_id)
                    .map_err(WorkSwarmError::Validation)
                    .and_then(|issues| {
                        if issues.is_empty() {
                            Ok(())
                        } else {
                            Err(WorkSwarmError::Validation(format!(
                                "workflow preflight: {}",
                                serde_json::to_string(&issues).unwrap_or_default()
                            )))
                        }
                    })
            })
            .await
            .map_err(|error| {
                TaskError::new(codes::INVALID_REQUEST, format!("创建团队运行失败：{error}"))
            })?;
        // 绑定在运行循环启动前落盘（首迭代构建 Worker 注册表即生效）。
        if let Some(mut binding) = workspace_binding {
            binding.team_id = team.team_id.clone();
            binding.project_id = team.project_space_id.clone().unwrap_or_default();
            crate::workswarm_api::project_workspace::save_binding(coordinator.run_dir(), &binding)
                .map_err(|message| {
                    TaskError::new(codes::STORE_ERROR, format!("保存工作区绑定失败：{message}"))
                })?;
        }
        let team_id = team.team_id.clone();
        if let Ok(mut teams) = self.teams.lock() {
            teams.insert(task_id.clone(), team_id.clone());
        }
        let _ = runtime.store().append_event(
            &task_id,
            "task.team.created",
            context.attempt,
            json!({
                "team_id": team_id,
                "mode": format!("{:?}", team.mode),
                "template_id": team.template_id,
                "project_space_id": team.project_space_id,
            }),
            Some(&team_id),
        );
        tokio::spawn(crate::workswarm_api::run_team_loop(
            Arc::clone(state),
            Arc::clone(coordinator),
            team_id.clone(),
        ));
        self.spawn_monitor(
            Arc::clone(runtime),
            Arc::clone(coordinator),
            task_id,
            team_id.clone(),
        );
        Ok(BackendRunId(team_id))
    }

    /// 监控团队运行：把步骤迁移、产物与终态回报给统一 Task Runtime。
    fn spawn_monitor(
        &self,
        runtime: Arc<TaskRuntime>,
        coordinator: Arc<TeamCoordinator>,
        task_id: String,
        team_id: String,
    ) {
        {
            let Ok(mut monitors) = self.monitors.lock() else {
                return;
            };
            if !monitors.insert(team_id.clone()) {
                return;
            }
        }
        let monitors = Arc::clone(&self.monitors);
        tokio::spawn(async move {
            let started = Instant::now();
            let mut observed: HashMap<String, String> = HashMap::new();
            let mut started_steps: HashSet<String> = HashSet::new();
            let mut awaiting_human_notified = false;
            while let Ok(task) = runtime.get_task(&task_id) {
                if matches!(
                    task.state,
                    TaskState::Succeeded | TaskState::Failed | TaskState::Cancelled
                ) {
                    break;
                }
                let attempt = task.current_attempt;
                let team = match coordinator.get_team_run(&team_id).await {
                    Ok(team) => team,
                    Err(error) => {
                        let _ = runtime.store().append_event(
                            &task_id,
                            "task.backend.monitor_error",
                            attempt,
                            json!({"team_id": team_id, "error": error.to_string()}),
                            Some(&team_id),
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                record_step_events(
                    &runtime,
                    &coordinator,
                    &task_id,
                    &team_id,
                    attempt,
                    &mut observed,
                    &mut started_steps,
                );
                if matches!(team.status, TeamRunStatus::AwaitingHuman) && !awaiting_human_notified {
                    awaiting_human_notified = true;
                    let _ = runtime.store().append_event(
                        &task_id,
                        "task.backend.awaiting_human",
                        attempt,
                        json!({"team_id": team_id}),
                        Some(&team_id),
                    );
                }
                match team.status {
                    TeamRunStatus::Succeeded => {
                        finish_success(&runtime, &coordinator, &task_id, &team_id, &team, started)
                            .await;
                        break;
                    }
                    TeamRunStatus::Failed => {
                        let message = failed_steps_summary(&coordinator, &team_id);
                        let _ = runtime.fail_backend_run(&task_id, CODE_TEAM_RUN_FAILED, &message);
                        break;
                    }
                    TeamRunStatus::Cancelled => {
                        if task.state == TaskState::Paused {
                            // 暂停触发的取消：保持 paused，等 resume 继续同一次运行。
                            let _ = runtime.store().append_event(
                                &task_id,
                                "task.backend.paused",
                                attempt,
                                json!({"team_id": team_id}),
                                Some(&team_id),
                            );
                        } else {
                            let _ = runtime.fail_backend_run(
                                &task_id,
                                CODE_TEAM_RUN_CANCELLED,
                                "团队运行已取消",
                            );
                        }
                        break;
                    }
                    _ => {}
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            if let Ok(mut monitors) = monitors.lock() {
                monitors.remove(&team_id);
            }
        });
    }
}

#[async_trait::async_trait]
impl TaskExecutionBackend for WorkSwarmBackend {
    fn kind(&self) -> ExecutionStrategy {
        ExecutionStrategy::WorkSwarm
    }

    async fn start(&self, context: TaskRunContext) -> TaskResult<BackendRunId> {
        let state = self.state.upgrade().ok_or_else(|| {
            TaskError::new(
                codes::STORE_ERROR,
                "服务状态已释放，无法分发 WorkSwarm 执行",
            )
        })?;
        let runtime = self.runtime.upgrade().ok_or_else(|| {
            TaskError::new(
                codes::STORE_ERROR,
                "任务运行时已释放，无法分发 WorkSwarm 执行",
            )
        })?;
        let coordinator = self.coordinator()?;
        let task_id = context.task_id.as_str().to_string();
        if let Some(team_id) = self.team_for_task(&task_id)? {
            return self
                .resume_existing(&state, &runtime, &coordinator, &task_id, &team_id)
                .await;
        }
        self.create_team(&state, &runtime, &coordinator, &context)
            .await
    }

    /// 暂停 = 真实取消团队运行（取消令牌立即生效，运行中 Worker 在回合边界中断）。
    /// 恢复由 `start` 对同一次运行 `continue` 完成：已完成步骤不重跑，副作用不重放。
    async fn pause(&self, run_id: &BackendRunId) -> TaskResult<()> {
        let coordinator = self.coordinator()?;
        coordinator
            .apply_steer(&run_id.0, &SteerCommand::Cancel)
            .await
            .map_err(|error| {
                TaskError::new(codes::STORE_ERROR, format!("暂停团队运行失败：{error}"))
            })?;
        Ok(())
    }

    /// 后端没有可原地恢复的暂停状态：恢复由运行时重新分发（`start` → `continue`）。
    async fn resume(&self, _run_id: &BackendRunId) -> TaskResult<()> {
        Ok(())
    }

    async fn cancel(&self, run_id: &BackendRunId) -> TaskResult<()> {
        let coordinator = self.coordinator()?;
        match coordinator
            .apply_steer(&run_id.0, &SteerCommand::Cancel)
            .await
        {
            Ok(_) => Ok(()),
            // 运行记录不在了 → 没有可取消的执行，按已完成处理。
            Err(WorkSwarmError::NotFound(_)) => Ok(()),
            Err(error) => Err(TaskError::new(
                codes::STORE_ERROR,
                format!("取消团队运行失败：{error}"),
            )),
        }
    }

    async fn inspect(&self, run_id: &BackendRunId) -> TaskResult<BackendSnapshot> {
        let coordinator = self.coordinator()?;
        let team = coordinator
            .get_team_run(&run_id.0)
            .await
            .map_err(|error| TaskError::new(codes::TASK_NOT_FOUND, error.to_string()))?;
        let steps = coordinator
            .load_run_state(&run_id.0)
            .map(|state| state.records.len())
            .unwrap_or(0);
        Ok(BackendSnapshot {
            state: format!("{:?}", team.status).to_lowercase(),
            detail: json!({
                "team_id": team.team_id,
                "mode": format!("{:?}", team.mode),
                "template_id": team.template_id,
                "project_space_id": team.project_space_id,
                "steps": steps,
            }),
        })
    }
}

/// 步骤迁移 → 统一任务事件（同一状态只报一次）。
fn record_step_events(
    runtime: &TaskRuntime,
    coordinator: &TeamCoordinator,
    task_id: &str,
    team_id: &str,
    attempt: u32,
    observed: &mut HashMap<String, String>,
    started_steps: &mut HashSet<String>,
) {
    let Ok(run_state) = coordinator.load_run_state(team_id) else {
        return;
    };
    for record in run_state.records.values() {
        let status = format!("{:?}", record.status).to_lowercase();
        let previous = observed.get(&record.step_id).cloned();
        if previous.as_deref() == Some(status.as_str()) {
            continue;
        }
        let completed = matches!(
            record.status,
            StepStatus::Succeeded | StepStatus::Failed | StepStatus::Aborted
        );
        // 轮询可能错过极短步骤的 Running 窗口（Ready → Succeeded 一次跳完）；
        // 成功步骤确实执行过，补一条 started，而不是展示成"从未开始"。
        let should_start = matches!(record.status, StepStatus::Running)
            || (completed && matches!(record.status, StepStatus::Succeeded));
        observed.insert(record.step_id.clone(), status.clone());
        let payload = json!({"team_id": team_id, "step_id": record.step_id, "status": status});
        if should_start && started_steps.insert(record.step_id.clone()) {
            let _ = runtime.store().append_event(
                task_id,
                "task.step.started",
                attempt,
                payload.clone(),
                Some(team_id),
            );
        }
        if completed {
            let _ = runtime.store().append_event(
                task_id,
                "task.step.completed",
                attempt,
                payload,
                Some(team_id),
            );
        }
    }
}

fn failed_steps_summary(coordinator: &TeamCoordinator, team_id: &str) -> String {
    let Ok(run_state) = coordinator.load_run_state(team_id) else {
        return "团队运行失败".to_string();
    };
    let failed: Vec<&str> = run_state
        .records
        .values()
        .filter(|record| matches!(record.status, StepStatus::Failed | StepStatus::Aborted))
        .map(|record| record.step_id.as_str())
        .collect();
    if failed.is_empty() {
        "团队运行失败（无失败步骤记录，可能是收尾/交付门失败）".to_string()
    } else {
        format!("团队运行失败：{}", failed.join(", "))
    }
}

/// 团队运行成功：登记 Project Space 产物、提交执行事实。
///
/// 只有宿主的完成判定为 `accepted`（证据收据已绑定）时才把执行事实作为目标证据
/// 送入独立验证；其余情况保持 `verifying`，交给人或后续验证阶段。
async fn finish_success(
    runtime: &TaskRuntime,
    coordinator: &TeamCoordinator,
    task_id: &str,
    team_id: &str,
    team: &TeamRun,
    started: Instant,
) {
    let run_state = coordinator.load_run_state(team_id).ok();
    let completion_record = run_state
        .as_ref()
        .and_then(|state| state.completion_record.clone());
    let completion_status = completion_record
        .as_ref()
        .and_then(|record| serde_json::to_value(record.status).ok())
        .and_then(|value| value.as_str().map(str::to_string));
    if let Some(space_id) = team.project_space_id.as_deref() {
        if let Ok(space) = coordinator.get_project_space(space_id).await {
            if let Ok(items) = coordinator.list_artifacts(&space).await {
                for artifact in items {
                    let _ = runtime.record_artifact(
                        task_id,
                        &artifact.kind,
                        &artifact.content_ref,
                        None,
                        None,
                    );
                }
            }
        }
    }
    let steps = run_state
        .as_ref()
        .map(|state| state.records.len())
        .unwrap_or(0);
    let report = BackendRunReport {
        backend_run_id: team_id.to_string(),
        session_id: None,
        steps,
        tool_calls: 0,
        duration_ms: started.elapsed().as_millis() as u64,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        completion_status: completion_status.clone(),
        completion_record: completion_record
            .as_ref()
            .and_then(|record| serde_json::to_value(record).ok()),
        final_text: None,
    };
    if let Err(error) = runtime.complete_backend_run(task_id, report) {
        tracing::warn!("任务 {task_id} 提交团队执行事实失败：{error}");
        return;
    }
    if completion_status.as_deref() == Some("accepted") {
        let evidence = VerificationEvidence {
            execution: Some(CheckResult::pass(format!(
                "WorkSwarm 团队运行完成：{steps} 个步骤"
            ))),
            goal: Some(CheckResult::pass("宿主完成判定 accepted（证据收据已绑定）")),
            ..Default::default()
        };
        if let Err(error) = runtime.verify_task(task_id, evidence) {
            tracing::warn!("任务 {task_id} 自动验证失败：{error}");
        }
    }
}
