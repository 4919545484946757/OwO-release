//! 云端执行 HTTP API（§12：从 lib.rs 机械外移的 M4a /cloud/* 域）。
//!
//! 路由面（`POST /cloud/tasks`、`GET /cloud/tasks/{id}`、`GET /cloud/tasks/{id}/result`、
//! `POST /cloud/tasks/{id}/cancel`、`GET /cloud/tasks/{id}/events`）与 /openapi.json
//! 登记一致；其中 events 端点为本模块补齐的原 OpenAPI 已登记但缺失的实现
//! （SSE：历史重放 + 实时帧，帧由 sse::hub 承载，submit 经 SseHubSink 发布）。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_core::cloud_exec::{CloudTaskQueue, CloudTaskSpec, TaskRecord};
use owo_agent_server::AppState;

/// 一次云端任务的启动结果（HTTP 入口与统一任务后端共用）。
pub(crate) struct CloudTaskStart {
    pub(crate) record: TaskRecord,
    pub(crate) transport: String,
}

/// 按当前配置构造云端传输；提交入口用它提前验证配置，执行 owner 则持有自己的实例。
fn cloud_transport(
    state: &AppState,
) -> Result<Box<dyn owo_agent_core::cloud_exec::CloudTransport>, String> {
    match std::env::var("OWO_CLOUD_BASE_URL") {
        Ok(url) if !url.trim().is_empty() => Ok(Box::new(
            owo_agent_core::cloud_exec::HttpTransport::new(url)
                .map_err(|error| format!("云端传输初始化失败：{error}"))?,
        )),
        _ => Ok(Box::new(
            owo_agent_core::cloud_exec::MockRemoteTransport::new(
                state.data_root.join("cloud").join("scratch"),
            ),
        )),
    }
}

/// 懒初始化单写者执行队列，并恢复已持久化任务。
async fn cloud_queue(
    state: &AppState,
) -> Result<tokio::sync::MutexGuard<'_, Option<owo_agent_core::cloud_exec::CloudTaskQueue>>, String>
{
    let mut guard = state.cloud_queue.lock().await;
    if guard.is_none() {
        let dir = state.data_root.join("cloud").join("queue");
        std::fs::create_dir_all(&dir).map_err(|error| format!("创建云端队列目录失败：{error}"))?;
        let mut queue =
            owo_agent_core::cloud_exec::CloudTaskQueue::new(dir, cloud_transport(state)?);
        queue
            .recover()
            .map_err(|error| format!("恢复云端任务队列失败：{error}"))?;
        let recovered_queued_ids = queue
            .list()
            .into_iter()
            .filter(|record| record.state == owo_agent_core::cloud_exec::TaskState::Queued)
            .map(|record| record.task_id)
            .collect::<Vec<_>>();
        let mut cancel_signals = state.cloud_cancel_signals.lock().await;
        for task_id in recovered_queued_ids {
            cancel_signals
                .entry(task_id)
                .or_insert_with(|| tokio::sync::watch::channel(false).0);
        }
        *guard = Some(queue);
    }
    Ok(guard)
}

/// 提交云端任务：先持久化并返回 task_id，再由后台 runner 执行。
pub(super) async fn cloud_task_submit(
    State(state): State<Arc<AppState>>,
    Json(spec): Json<owo_agent_core::cloud_exec::CloudTaskSpec>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let started = start_cloud_task(&state, spec)
        .await
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    Ok(Json(json!({
        "ok": true,
        "task": started.record,
        "transport": started.transport,
    })))
}

/// 提交并启动一次云端任务（HTTP 入口与统一任务后端共用）。
///
/// 只负责持久化入队与启动 runner，执行过程不占用调用方；调用方用 task_id
/// 查询持久化快照或订阅 SSE。
#[allow(dead_code)] // 独立编译的 cloud_task_api_tests 目标不调用它
pub(crate) async fn start_cloud_task(
    state: &Arc<AppState>,
    spec: CloudTaskSpec,
) -> Result<CloudTaskStart, String> {
    let transport = cloud_transport(state)?;
    let transport_kind = transport.kind().to_string();
    drop(transport);
    let _submit_guard = state.cloud_submit_lock.lock().await;
    let queue_dir = state.data_root.join("cloud").join("queue");
    let task = CloudTaskQueue::submit_persisted(&queue_dir, spec, &[])?;
    let task_id = task.task_id.clone();
    // Register the SSE stream before the persisted task ID is returned. The client
    // can subscribe immediately even if the background runner has not emitted yet.
    owo_agent_server::sse::hub().prepare_task(task_id.clone());

    let (cancel_tx, _) = tokio::sync::watch::channel(false);
    state
        .cloud_cancel_signals
        .lock()
        .await
        .insert(task_id.clone(), cancel_tx);
    drop(_submit_guard);

    spawn_cloud_runner(Arc::clone(state), task_id);
    Ok(CloudTaskStart {
        record: task,
        transport: transport_kind,
    })
}

fn spawn_cloud_runner(worker_state: Arc<AppState>, task_id: String) {
    // 每个请求只负责持久化入队与启动 runner；执行过程不占用 HTTP handler，
    // 客户端可立即用 task_id 订阅 SSE。队列锁仍保证单写者和串行 transport。
    tokio::spawn(async move {
        let mut queue_guard = match cloud_queue(&worker_state).await {
            Ok(guard) => guard,
            Err(error) => {
                tracing::error!(%error, "cloud task queue initialization failed");
                owo_agent_server::sse::hub().publish(
                    &task_id,
                    json!({ "task_id": task_id, "event": "runner_error", "kind": "failed", "error": error }).to_string(),
                );
                owo_agent_server::sse::hub().mark_completed(&task_id);
                return;
            }
        };
        let Some(queue) = queue_guard.as_mut() else {
            return;
        };
        // A submit can persist a new task while another runner owns the queue
        // lock. Refresh only after acquiring the single-writer execution lock.
        if let Err(error) = queue.recover() {
            tracing::error!(%error, "cloud task queue refresh failed");
            owo_agent_server::sse::hub().publish(
                &task_id,
                json!({ "task_id": task_id, "event": "runner_error", "kind": "failed", "error": error }).to_string(),
            );
            owo_agent_server::sse::hub().mark_completed(&task_id);
            return;
        }
        // Signal-map entries are dispatch tokens. Snapshot them with the queue
        // so concurrent submit runners cannot immediately retry an item that
        // the previous runner already returned to Queued.
        let dispatch_tokens = worker_state
            .cloud_cancel_signals
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let pending_tasks = queue
            .list()
            .into_iter()
            .filter(|record| {
                record.state == owo_agent_core::cloud_exec::TaskState::Queued
                    && dispatch_tokens.contains(&record.task_id)
            })
            .map(|record| record.task_id)
            .collect::<Vec<_>>();
        for selected_task in pending_tasks {
            let sink = owo_agent_server::sse::sink(selected_task.clone());
            let cancel_rx = worker_state
                .cloud_cancel_signals
                .lock()
                .await
                .get(&selected_task)
                .map(|signal| signal.subscribe())
                .unwrap_or_else(|| tokio::sync::watch::channel(false).1);
            let run_result = queue
                .run_task_to_terminal_with_cancel(&selected_task, &sink, cancel_rx)
                .await;
            let terminal = queue.record(&selected_task).is_some_and(|record| {
                matches!(
                    &record.state,
                    owo_agent_core::cloud_exec::TaskState::Succeeded
                        | owo_agent_core::cloud_exec::TaskState::Failed
                        | owo_agent_core::cloud_exec::TaskState::Canceled
                )
            });
            let run_failed = run_result.is_err();
            worker_state
                .cloud_cancel_signals
                .lock()
                .await
                .remove(&selected_task);
            if let Err(error) = run_result {
                tracing::error!(task_id = %selected_task, %error, "cloud task runner failed");
                owo_agent_server::sse::hub().publish(
                    &selected_task,
                    json!({
                        "task_id": selected_task,
                        "event": "runner_error",
                        "kind": "failed",
                        "error": error
                    })
                    .to_string(),
                );
            }
            if terminal || run_failed {
                owo_agent_server::sse::hub().mark_completed(&selected_task);
            }
        }
    });
}

/// 读取持久化快照（提交方与统一任务后端都不等 transport 写锁）。
#[allow(dead_code)] // 独立编译的 cloud_task_api_tests 目标不调用它
pub(crate) fn read_persisted_task(
    state: &AppState,
    task_id: &str,
) -> Result<Option<owo_agent_core::cloud_exec::TaskRecord>, String> {
    if task_id.len() > 80
        || !task_id.starts_with("cloud-")
        || !task_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Ok(None);
    }
    let path = state
        .data_root
        .join("cloud")
        .join("queue")
        .join(format!("{task_id}.json"));
    if !path.is_file() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let record: owo_agent_core::cloud_exec::TaskRecord = serde_json::from_str(&content)
        .map_err(|error| format!("云端任务持久化快照损坏（{task_id}）：{error}"))?;
    if record.task_id != task_id {
        return Err(format!("云端任务快照身份不匹配：{task_id}"));
    }
    Ok(Some(record))
}

/// 查询云端任务：从持久化快照读取，不等待后台 transport 持有的写锁。
pub(super) async fn cloud_task_status(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let record = read_persisted_task(&state, &id)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
        .ok_or((StatusCode::NOT_FOUND, format!("云端任务 {id} 不存在")))?;
    let usage = json!({
        "duration_ms": record.duration_ms,
        "diff_count": record.result.as_ref().map(|result| result.diff.len()).unwrap_or(0),
        "retry_count": record.retry_count
    });
    Ok(Json(json!({
        "state": format!("{:?}", record.state),
        "retry_count": record.retry_count,
        "last_error": record.last_error,
        "created_at": record.created_at,
        "duration_ms": record.duration_ms,
        "usage": usage,
    })))
}

/// 获取云端任务结果：读取与状态查询相同的持久化快照。
pub(super) async fn cloud_task_result(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let record = read_persisted_task(&state, &id)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
        .ok_or((StatusCode::NOT_FOUND, format!("云端任务 {id} 不存在")))?;
    let result = record
        .result
        .ok_or((StatusCode::CONFLICT, format!("云端任务 {id} 尚无结果")))?;
    let diff_summary = owo_agent_core::cloud_exec::describe_diff(&result.diff);
    Ok(Json(
        json!({ "ok": true, "result": result, "diff_summary": diff_summary }),
    ))
}

/// 取消云端任务：活动任务只投递取消信号，由唯一 runner 执行远端取消与持久化。
pub(super) async fn cloud_task_cancel(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_str = request_cloud_cancel(&state, &id).await?;
    Ok(Json(
        json!({ "ok": true, "task_id": id, "state": state_str }),
    ))
}

/// 请求取消一次云端任务（HTTP 入口与统一任务后端共用）。
///
/// 活动运行（信号表里有条目）只投递取消信号，由唯一 runner 收口；否则直接
/// 在队列上取消。返回取消后的线上状态字符串。
#[allow(dead_code)] // 独立编译的 cloud_task_api_tests 目标不调用它
pub(crate) async fn request_cloud_cancel(
    state: &Arc<AppState>,
    id: &str,
) -> Result<&'static str, (StatusCode, String)> {
    if let Some(signal) = state.cloud_cancel_signals.lock().await.get(id).cloned() {
        signal.send_replace(true);
        return Ok("cancel_requested");
    }

    let mut guard = cloud_queue(state)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let queue = guard.as_mut().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "云端队列未初始化".to_string(),
    ))?;
    let was_canceled = queue
        .record(id)
        .is_some_and(|record| record.state == owo_agent_core::cloud_exec::TaskState::Canceled);
    queue
        .cancel(id)
        .await
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    state.cloud_cancel_signals.lock().await.remove(id);
    if !was_canceled {
        owo_agent_server::sse::hub().publish(
            id,
            json!({ "task_id": id, "event": "canceled", "kind": "canceled" }).to_string(),
        );
        owo_agent_server::sse::hub().mark_completed(id);
    }
    Ok("canceled")
}
// 注：`GET /cloud/tasks/{id}/events` 的处理器由 sse.rs::router 单一提供
//（lib.rs 仅经 merge(sse::router) 注册一处）。本模块曾补齐一份带 404 校验
// 的实现，与 sse::router 重复注册导致 build_router 启动 panic
//（Overlapping method route，运行时冒烟发现）；按 sse.rs 的 Lane D 设计
// 收敛为单一注册点，严格版实现移除（帧管线 sse::hub/sink 仍由本模块消费）。
