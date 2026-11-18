//! CloudRemote 执行后端（统一 Task Runtime 的第五个 Adapter）。
//!
//! 职责边界：
//! - 把统一 Task 的 `start` 变成**一次真实的云端任务**：复用 `/cloud/tasks` 的同一条
//!   提交路径（`cloud_api::start_cloud_task`：持久化入队 + 单写者 runner），不另起队列；
//! - 只回报执行事实：云端终态、退出码、diff 摘要；是否 `succeeded` 仍由独立验证决定。
//!   云端任务的产物是 `cloud_diff`（引用 `cloud-task:{id}`），带回本地仍走既有
//!   `/cloud/tasks/{id}/result`，本后端不自动 apply；
//! - 命令执行要求 `policy_profile = "workspace-execute"`（与 Single Agent 一致）：
//!   `workspace` 档位只允许工作区写入，不允许借云端任务执行命令；
//! - 暂停 = 投递云端取消信号，由唯一 runner 执行远端取消并持久化；
//!   恢复 = 云端任务仍在执行则继续观察，已成功则沿用既有事实，否则新建一次云端
//!   任务——远端没有可恢复的执行游标，这一点在 `resume` 的返回里明说，
//!   不伪装成"从断点继续"；取消 = 同一取消路径，由运行时收口。
//!
//! payload（Runtime 不解释）：
//! ```json
//! { "commands": ["cargo test"], "workspace_dir": "repo", "env_passthrough": ["PATH"],
//!   "timeout_secs": 300, "name": "夜间回归" }
//! ```
//! `commands` 必填；未配置 `OWO_CLOUD_BASE_URL` 时走本地模拟传输（在隔离副本里执行），
//! 配置后走真实远端 HTTP 传输。
//!
//! 依赖 `Arc<AppState>`：提交需要数据根、取消信号表与队列锁，因此在 `build_router`
//! 拿到 `Arc<AppState>` 后注册。后端只持有弱引用，不构成引用环。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use owo_agent_core::cloud_exec::{CloudTaskSpec, TaskRecord, TaskState as CloudTaskState};
use owo_agent_core::task_runtime::{
    codes, BackendRunId, BackendRunReport, BackendSnapshot, ExecutionStrategy, TaskError,
    TaskExecutionBackend, TaskResult, TaskRunContext, TaskRuntime, TaskState,
};
use serde::Deserialize;
use serde_json::json;

use crate::cloud_api;
use crate::AppState;

/// 云端任务执行失败（远端非零退出或执行链失败）。
const CODE_CLOUD_TASK_FAILED: &str = "CLOUD_TASK_FAILED";
/// 云端任务在统一任务之外被取消。
const CODE_CLOUD_TASK_CANCELED: &str = "CLOUD_TASK_CANCELED";
/// 快照缺失/损坏，无法判定执行事实。
const CODE_CLOUD_TASK_ERROR: &str = "CLOUD_TASK_ERROR";
/// 恢复语义：云端任务不支持续跑，恢复会重新提交一次。
const CODE_RESUME_RESTARTS_RUN: &str = "RESUME_RESTARTS_RUN";

/// 允许后台任务执行命令的策略档位（与 Single Agent 后端同一口径）。
const POLICY_WORKSPACE_EXECUTE: &str = "workspace-execute";

/// 监控云端任务的轮询间隔：云端状态由 runner 原子写盘，读取无锁。
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// 统一任务 `payload` 里 CloudRemote 专属的输入。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CloudRemotePayload {
    /// 云端任务名（缺省取用户目标的前 80 个字符）。
    name: Option<String>,
    /// 按序执行的命令（必填；危险模式仍由 cloud_exec 校验拒绝）。
    commands: Vec<String>,
    /// 快照来源目录（缺省统一任务的 `workspace_ref`，再缺省服务端工作区）。
    workspace_dir: Option<String>,
    /// 环境变量白名单（只透传名字，值取自进程环境）。
    env_passthrough: Vec<String>,
    /// 单条命令超时（秒；缺省 300）。
    timeout_secs: Option<u64>,
}

/// CloudRemote 执行后端：任务 → 云端任务 → 云端执行事实。
pub struct CloudRemoteBackend {
    state: Weak<AppState>,
    runtime: Weak<TaskRuntime>,
    /// 统一任务 → 云端任务 id（重复 start 时据此沿用仍在执行的云端任务）。
    tasks: Arc<Mutex<HashMap<String, String>>>,
    /// 已在监控的云端任务 id。
    monitors: Arc<Mutex<HashSet<String>>>,
}

impl CloudRemoteBackend {
    pub fn new(state: Weak<AppState>, runtime: Weak<TaskRuntime>) -> Self {
        Self {
            state,
            runtime,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            monitors: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn lookup_task(&self, task_id: &str) -> Option<String> {
        self.tasks
            .lock()
            .ok()
            .and_then(|tasks| tasks.get(task_id).cloned())
    }

    fn remember_task(&self, task_id: &str, cloud_task_id: &str) {
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.insert(task_id.to_string(), cloud_task_id.to_string());
        }
    }
}

#[async_trait::async_trait]
impl TaskExecutionBackend for CloudRemoteBackend {
    fn kind(&self) -> ExecutionStrategy {
        ExecutionStrategy::CloudRemote
    }

    async fn start(&self, context: TaskRunContext) -> TaskResult<BackendRunId> {
        let Some(state) = self.state.upgrade() else {
            return Err(TaskError::new(
                codes::STORE_ERROR,
                "服务状态已释放，无法提交云端任务",
            ));
        };
        let Some(runtime) = self.runtime.upgrade() else {
            return Err(TaskError::new(
                codes::STORE_ERROR,
                "任务运行时已释放，无法监控云端任务",
            ));
        };
        let task_id = context.task_id.0.clone();
        let payload: CloudRemotePayload =
            match serde_json::from_value(context.payload.clone().unwrap_or_else(|| json!({}))) {
                Ok(payload) => payload,
                Err(error) => {
                    return Err(reject(
                        &runtime,
                        &task_id,
                        codes::INVALID_REQUEST,
                        format!("云端任务参数无效：{error}"),
                    ));
                }
            };
        if context.policy_profile.trim() != POLICY_WORKSPACE_EXECUTE {
            return Err(reject(
                &runtime,
                &task_id,
                codes::INVALID_REQUEST,
                format!(
                    "CloudRemote 执行命令，需要 policy_profile={POLICY_WORKSPACE_EXECUTE}（当前 {:?}）",
                    context.policy_profile
                ),
            ));
        }
        let commands: Vec<String> = payload
            .commands
            .iter()
            .map(|command| command.trim().to_string())
            .filter(|command| !command.is_empty())
            .collect();
        if commands.is_empty() {
            return Err(reject(
                &runtime,
                &task_id,
                codes::INVALID_REQUEST,
                "payload.commands 至少需要一条命令",
            ));
        }
        // 重复 start（如暂停后恢复、审批后再次分发）：
        // - 云端任务仍在执行（runner 活着且未收到取消）→ 沿用，不再提交一次；
        // - 云端任务已成功 → 沿用既有的执行事实，不重复执行命令；
        // - 已取消/已失败 → 没有可沿用的运行，重新提交。
        // 取消信号表既判断 runner 是否还在，也判断暂停是否已经投递了取消：
        // 暂停后的恢复必须重新提交，不能接一个正在被取消的云端任务。
        if let Some(cloud_task_id) = self.lookup_task(&task_id) {
            let record = cloud_api::read_persisted_task(&state, &cloud_task_id)
                .ok()
                .flatten();
            let signal = state
                .cloud_cancel_signals
                .lock()
                .await
                .get(&cloud_task_id)
                .cloned();
            let cancel_requested = signal.as_ref().is_some_and(|signal| *signal.borrow());
            let reattach = record.as_ref().is_some_and(|record| {
                matches!(
                    record.state,
                    CloudTaskState::Queued | CloudTaskState::Running
                )
            }) && signal.is_some()
                && !cancel_requested;
            let already_succeeded = record
                .as_ref()
                .is_some_and(|record| record.state == CloudTaskState::Succeeded);
            if reattach || already_succeeded {
                self.spawn_monitor(
                    Arc::clone(&runtime),
                    task_id.clone(),
                    context.attempt,
                    cloud_task_id.clone(),
                );
                return Ok(BackendRunId(cloud_task_id));
            }
        }
        let workspace = match resolve_workspace(
            &state.workspace,
            payload
                .workspace_dir
                .as_deref()
                .or(context.workspace_ref.as_deref()),
        ) {
            Ok(workspace) => workspace,
            Err(message) => {
                return Err(reject(&runtime, &task_id, codes::INVALID_REQUEST, message));
            }
        };
        let name = payload
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| short_goal(&context.user_goal));
        let spec = CloudTaskSpec {
            name,
            workspace_dir: workspace,
            commands,
            env_passthrough: payload.env_passthrough,
            timeout_secs: payload.timeout_secs.unwrap_or(300),
        };
        let started = match cloud_api::start_cloud_task(&state, spec).await {
            Ok(started) => started,
            Err(error) => {
                return Err(reject(
                    &runtime,
                    &task_id,
                    codes::INVALID_REQUEST,
                    format!("云端任务提交失败：{error}"),
                ));
            }
        };
        let cloud_task_id = started.record.task_id.clone();
        self.remember_task(&task_id, &cloud_task_id);
        let _ = runtime.store().append_event(
            &task_id,
            "task.cloud.submitted",
            context.attempt,
            json!({
                "cloud_task_id": cloud_task_id,
                "transport": started.transport,
                "commands": started.record.spec.commands.len(),
                "workspace": started.record.spec.workspace_dir.display().to_string(),
            }),
            Some(&cloud_task_id),
        );
        self.spawn_monitor(
            Arc::clone(&runtime),
            task_id,
            context.attempt,
            cloud_task_id.clone(),
        );
        Ok(BackendRunId(cloud_task_id))
    }

    async fn pause(&self, run_id: &BackendRunId) -> TaskResult<()> {
        let Some(state) = self.state.upgrade() else {
            return Err(TaskError::new(
                codes::STORE_ERROR,
                "服务状态已释放，无法取消云端任务",
            ));
        };
        // 暂停 = 云端取消：远端没有"冻结"语义，取消后只能重新提交。
        cloud_api::request_cloud_cancel(&state, &run_id.0)
            .await
            .map_err(|(_, error)| TaskError::new(CODE_CLOUD_TASK_ERROR, error))?;
        Ok(())
    }

    async fn resume(&self, run_id: &BackendRunId) -> TaskResult<()> {
        // 只有两种恢复不需要重跑：
        // - 云端任务仍在执行且没收到取消（暂停并未让远端停下）→ 继续观察；
        // - 云端任务已经成功 → 沿用既有执行事实。
        // 其余情况（暂停已投递取消、失败、远端已终止）没有可恢复的游标，
        // `start` 会重新提交一次，这里必须明说命令会再执行。
        if let Some(state) = self.state.upgrade() {
            let record = cloud_api::read_persisted_task(&state, &run_id.0)
                .ok()
                .flatten();
            if record
                .as_ref()
                .is_some_and(|record| record.state == CloudTaskState::Succeeded)
            {
                return Ok(());
            }
            let active = record.as_ref().is_some_and(|record| {
                matches!(
                    record.state,
                    CloudTaskState::Queued | CloudTaskState::Running
                )
            });
            let signal = state
                .cloud_cancel_signals
                .lock()
                .await
                .get(&run_id.0)
                .cloned();
            let cancel_requested = signal.as_ref().is_some_and(|signal| *signal.borrow());
            if active && signal.is_some() && !cancel_requested {
                return Ok(());
            }
        }
        Err(TaskError::new(
            CODE_RESUME_RESTARTS_RUN,
            format!(
                "云端任务不支持断点续跑：运行 {} 已取消或已终止，恢复将重新提交一次（命令会再执行）",
                run_id.0
            ),
        ))
    }

    async fn cancel(&self, run_id: &BackendRunId) -> TaskResult<()> {
        let Some(state) = self.state.upgrade() else {
            return Ok(());
        };
        // 已到终态或不存在时取消无副作用；任务取消语义由运行时收口。
        let _ = cloud_api::request_cloud_cancel(&state, &run_id.0).await;
        Ok(())
    }

    async fn inspect(&self, run_id: &BackendRunId) -> TaskResult<BackendSnapshot> {
        let Some(state) = self.state.upgrade() else {
            return Err(TaskError::new(
                codes::STORE_ERROR,
                "服务状态已释放，无法读取云端任务",
            ));
        };
        let record = cloud_api::read_persisted_task(&state, &run_id.0)
            .map_err(|error| TaskError::new(CODE_CLOUD_TASK_ERROR, error))?
            .ok_or_else(|| {
                TaskError::new(
                    codes::TASK_NOT_FOUND,
                    format!("云端任务不存在：{}", run_id.0),
                )
            })?;
        Ok(BackendSnapshot {
            state: cloud_state_str(record.state).to_string(),
            detail: json!({
                "cloud_task_id": record.task_id,
                "remote_id": record.remote_id,
                "retry_count": record.retry_count,
                "last_error": record.last_error,
                "duration_ms": record.duration_ms,
                "commands": record.spec.commands.len(),
            }),
        })
    }
}

impl CloudRemoteBackend {
    /// 监控一次云端任务：云端终态回报给统一 Task Runtime。
    fn spawn_monitor(
        &self,
        runtime: Arc<TaskRuntime>,
        task_id: String,
        attempt: u32,
        cloud_task_id: String,
    ) {
        // 监控器按「云端任务 + attempt」去重：恢复会为同一云端任务挂新监控器，
        // 旧监控器在下一次轮询发现 attempt 变化后自行退出。
        let monitor_key = format!("{cloud_task_id}#{attempt}");
        {
            let Ok(mut monitors) = self.monitors.lock() else {
                return;
            };
            if !monitors.insert(monitor_key.clone()) {
                return;
            }
        }
        let monitors = Arc::clone(&self.monitors);
        let state = Weak::clone(&self.state);
        tokio::spawn(async move {
            let started = Instant::now();
            let mut started_reported = false;
            let mut retry_reported = 0u32;
            while let Ok(task) = runtime.get_task(&task_id) {
                if matches!(
                    task.state,
                    TaskState::Succeeded | TaskState::Failed | TaskState::Cancelled
                ) {
                    break;
                }
                // 恢复会重新提交（attempt +1）：旧云端任务的监控器不能收口新尝试。
                if task.current_attempt != attempt {
                    break;
                }
                let Some(state) = state.upgrade() else {
                    break;
                };
                let record = match cloud_api::read_persisted_task(&state, &cloud_task_id) {
                    Ok(Some(record)) => record,
                    Ok(None) => {
                        let _ = runtime.fail_backend_run(
                            &task_id,
                            CODE_CLOUD_TASK_ERROR,
                            &format!("云端任务 {cloud_task_id} 的持久化快照不存在"),
                        );
                        break;
                    }
                    Err(error) => {
                        let _ = runtime.fail_backend_run(
                            &task_id,
                            CODE_CLOUD_TASK_ERROR,
                            &format!("读取云端任务 {cloud_task_id} 失败：{error}"),
                        );
                        break;
                    }
                };
                if !started_reported && !matches!(record.state, CloudTaskState::Queued) {
                    started_reported = true;
                    let _ = runtime.store().append_event(
                        &task_id,
                        "task.cloud.started",
                        attempt,
                        json!({"cloud_task_id": cloud_task_id}),
                        Some(&cloud_task_id),
                    );
                }
                if record.retry_count > retry_reported {
                    retry_reported = record.retry_count;
                    let _ = runtime.store().append_event(
                        &task_id,
                        "task.cloud.retried",
                        attempt,
                        json!({
                            "cloud_task_id": cloud_task_id,
                            "retry_count": record.retry_count,
                        }),
                        Some(&cloud_task_id),
                    );
                }
                match record.state {
                    CloudTaskState::Queued | CloudTaskState::Running => {
                        tokio::time::sleep(POLL_INTERVAL).await;
                    }
                    CloudTaskState::Succeeded => {
                        finish_success(&runtime, &task_id, attempt, &record, started);
                        break;
                    }
                    CloudTaskState::Failed => {
                        let message = record
                            .last_error
                            .clone()
                            .or_else(|| {
                                record
                                    .result
                                    .as_ref()
                                    .map(|result| first_line(&result.stderr))
                                    .filter(|line| !line.is_empty())
                            })
                            .unwrap_or_else(|| "云端任务执行失败（未记录原因）".to_string());
                        let _ = runtime.store().append_event(
                            &task_id,
                            "task.cloud.failed",
                            attempt,
                            json!({
                                "cloud_task_id": cloud_task_id,
                                "retry_count": record.retry_count,
                            }),
                            Some(&cloud_task_id),
                        );
                        let _ =
                            runtime.fail_backend_run(&task_id, CODE_CLOUD_TASK_FAILED, &message);
                        break;
                    }
                    CloudTaskState::Canceled => {
                        // 暂停触发的取消由运行时转成 task.backend.paused。
                        let _ = runtime.fail_backend_run(
                            &task_id,
                            CODE_CLOUD_TASK_CANCELED,
                            &format!("云端任务 {cloud_task_id} 已取消"),
                        );
                        break;
                    }
                }
            }
            if let Ok(mut monitors) = monitors.lock() {
                monitors.remove(&monitor_key);
            }
        });
    }
}

/// 云端任务成功：登记 diff 产物、提交执行事实。
///
/// 不自动验证：退出码是远端自报，diff 需要人/验证阶段带回本地检查。
fn finish_success(
    runtime: &TaskRuntime,
    task_id: &str,
    attempt: u32,
    record: &TaskRecord,
    started: Instant,
) {
    let cloud_task_id = record.task_id.as_str();
    let result = record.result.as_ref();
    let diff_count = result.map(|result| result.diff.len()).unwrap_or(0);
    let exit_code = result.and_then(|result| result.exit_code);
    let _ = runtime.store().append_event(
        task_id,
        "task.cloud.completed",
        attempt,
        json!({
            "cloud_task_id": cloud_task_id,
            "exit_code": exit_code,
            "diff_count": diff_count,
            "retry_count": record.retry_count,
        }),
        Some(cloud_task_id),
    );
    let _ = runtime.record_artifact(
        task_id,
        "cloud_diff",
        &format!("cloud-task:{cloud_task_id}"),
        None,
        None,
    );
    let report = BackendRunReport {
        backend_run_id: cloud_task_id.to_string(),
        session_id: None,
        steps: record.spec.commands.len(),
        tool_calls: 0,
        duration_ms: if record.duration_ms > 0 {
            record.duration_ms
        } else {
            started.elapsed().as_millis() as u64
        },
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        completion_status: Some("candidate".to_string()),
        completion_record: Some(json!({
            "exit_code": exit_code,
            "diff_count": diff_count,
            "diff_truncated": result.map(|result| result.diff_truncated).unwrap_or(false),
            "retry_count": record.retry_count,
            "artifact": format!("cloud-task:{cloud_task_id}"),
        })),
        final_text: Some(format!(
            "云端任务 {cloud_task_id} 完成：退出码 {}，diff {} 项",
            exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "未知".to_string()),
            diff_count
        )),
    };
    if let Err(error) = runtime.complete_backend_run(task_id, report) {
        tracing::warn!("任务 {task_id} 提交云端执行事实失败：{error}");
    }
}

fn cloud_state_str(state: CloudTaskState) -> &'static str {
    match state {
        CloudTaskState::Queued => "queued",
        CloudTaskState::Running => "running",
        CloudTaskState::Succeeded => "succeeded",
        CloudTaskState::Failed => "failed",
        CloudTaskState::Canceled => "canceled",
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
    let _ = runtime.fail_backend_run(task_id, code, &message);
    TaskError::new(code, message)
}

/// 解析快照来源目录：相对路径按服务端工作区解析，必须存在且是目录。
fn resolve_workspace(root: &Path, requested: Option<&str>) -> Result<PathBuf, String> {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(root.to_path_buf());
    };
    let path = PathBuf::from(requested);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    if !path.is_dir() {
        return Err(format!("工作区不存在或不是目录：{}", path.display()));
    }
    Ok(path)
}

fn short_goal(goal: &str) -> String {
    let trimmed = goal.trim();
    if trimmed.is_empty() {
        "cloud task".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}
