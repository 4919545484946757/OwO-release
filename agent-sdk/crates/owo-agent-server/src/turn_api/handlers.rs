use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::sse::Sse;
use axum::response::IntoResponse;
use axum::Json;
use futures_util::stream;
use owo_agent_core::permissions::Decision;
use owo_agent_protocol::{PermissionResponse, SseEvent, TurnRequest};
use serde_json::{json, Value};

use super::approval::*;
use super::queue::*;
use super::wire::*;
use owo_agent_server::AppState;

pub(super) fn turn_replay_state(
    events: &[owo_agent_protocol::TurnEventRecord],
    has_more: bool,
    turn_is_running: bool,
) -> owo_agent_protocol::TurnReplayState {
    // A terminal marker in this page is not authoritative while later same-turn
    // events remain beyond the page boundary (for example, legacy save-failure tails).
    if has_more {
        return owo_agent_protocol::TurnReplayState::Active;
    }
    if let Some(state) = events
        .iter()
        .rev()
        .find_map(|record| match &record.payload {
            SseEvent::TurnStats { .. } => Some(owo_agent_protocol::TurnReplayState::Completed),
            event if is_turn_failed_event(event) => {
                Some(owo_agent_protocol::TurnReplayState::Failed)
            }
            _ => None,
        })
    {
        return state;
    }
    if turn_is_running {
        owo_agent_protocol::TurnReplayState::Active
    } else {
        owo_agent_protocol::TurnReplayState::Interrupted
    }
}

pub(crate) async fn turn_events(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<TurnEventsQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    crate::session_api::load_session(&state, &session_id)?;
    // Read one lookahead record so a bounded page can distinguish "more replay data"
    // from a genuinely interrupted turn after the producer has already exited.
    let page_limit = query.limit.unwrap_or(256).clamp(1, 999);
    let mut events = state
        .store
        .turn_events_after(
            &session_id,
            Some(&query.turn_id),
            query.after_seq,
            page_limit + 1,
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let has_more = events.len() > page_limit;
    if has_more {
        events.truncate(page_limit);
    }
    let active_turn_id = state
        .turn_controls
        .lock()
        .map_err(poison)?
        .active_id(&session_id);
    let replay_state = turn_replay_state(
        &events,
        has_more,
        active_turn_id.as_deref() == Some(query.turn_id.as_str()),
    );
    let active = replay_state == owo_agent_protocol::TurnReplayState::Active;
    let next_after_seq = events
        .last()
        .map(|record| record.seq)
        .unwrap_or(query.after_seq);
    Ok(Json(json!({
        "events": events,
        "active": active,
        "state": replay_state,
        "next_after_seq": next_after_seq,
    })))
}

struct TurnLifecycleGuard {
    state: Arc<AppState>,
    session_id: String,
    turn_id: String,
    queue: Arc<TurnEventQueue>,
}

impl Drop for TurnLifecycleGuard {
    fn drop(&mut self) {
        let mut controls = self
            .state
            .turn_controls
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        controls.finish(&self.session_id, &self.turn_id);
        drop(controls);
        crate::activity_api::end_activity(&self.state, &self.session_id);
        self.queue.close();
    }
}

pub(crate) async fn turn(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<TurnRequest>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    // §13 批次六：遥测功能计数（默认关时零开销早退；仅数字，无内容）。
    crate::observability_api::record_telemetry_counter("turn", 1);
    let session = crate::session_api::load_session(&state, &id)?;
    let turn_provider = request
        .model_connection
        .as_ref()
        .map(owo_agent_core::gateway::custom_model_provider)
        .transpose()
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    let mut effective_prompt = request.prompt.clone();
    if request
        .attachments
        .len()
        .saturating_add(request.content_parts.len())
        > 32
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "input/too_many_content_parts".into(),
        ));
    }
    let mut content_parts = request.content_parts.clone();
    for id_ref in request
        .attachments
        .iter()
        .filter(|id| crate::artifact_review_api::input_assets::is_asset_id(id))
    {
        content_parts
            .push(crate::artifact_review_api::input_assets::reference(&state, &id, id_ref).await?);
    }
    let resolved_inputs = crate::artifact_review_api::input_assets::resolve(
        &state,
        &id,
        &session.workspace,
        &content_parts,
    )
    .await?;
    effective_prompt.push_str(&resolved_inputs.prompt_suffix);
    // A1-2 多模态（取优合并自远端 engine）：图片附件 → base64 data URL 进 images
    // （真正进视觉上下文）；文本类附件维持路径注入。
    let mut attachment_images: Vec<owo_agent_core::MessageImage> = resolved_inputs.images;
    use base64::Engine as _;
    if !request.attachments.is_empty() {
        let root = session.workspace.canonicalize().map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("input/workspace_unavailable: {e}"),
            )
        })?;
        let legacy = request
            .attachments
            .iter()
            .any(|id| !crate::artifact_review_api::input_assets::is_asset_id(id));
        let dir = if legacy {
            let dir = crate::session_api::attachment_dir(&root, &id)
                .canonicalize()
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("input/legacy_directory_unavailable: {e}"),
                    )
                })?;
            if !dir.starts_with(&root) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "input/legacy_directory_escaped".into(),
                ));
            }
            dir
        } else {
            crate::session_api::attachment_dir(&root, &id)
        };
        let mut image_bytes = attachment_images
            .iter()
            .map(|image| {
                let encoded = image.url.rsplit(',').next().unwrap_or_default();
                encoded.len() / 4 * 3 - encoded.bytes().rev().take_while(|b| *b == b'=').count()
            })
            .sum::<usize>();
        let mut lines = Vec::new();
        for attachment in &request.attachments {
            if crate::artifact_review_api::input_assets::is_asset_id(attachment) {
                continue;
            }
            let safe = Path::new(attachment)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(attachment);
            let path = dir.join(safe);
            if !path.is_file() {
                return Err((StatusCode::BAD_REQUEST, format!("附件不存在：{safe}")));
            }
            let path = path.canonicalize().map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("input/legacy_path_unavailable: {e}"),
                )
            })?;
            if !path.starts_with(&dir) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "input/legacy_attachment_escaped".into(),
                ));
            }
            let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            let is_image = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "webp" | "gif"
                    )
                })
                .unwrap_or(false);
            if is_image {
                const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
                if size > MAX_IMAGE_BYTES {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("图片附件过大（{size} 字节 > 5MB）：{safe}"),
                    ));
                }
                if attachment_images.len() >= 8
                    || image_bytes.saturating_add(size as usize) > 20 * 1024 * 1024
                {
                    return Err((StatusCode::BAD_REQUEST, "input/image_input_budget".into()));
                }
                use std::io::Read as _;
                let mut bytes = Vec::new();
                std::fs::File::open(&path)
                    .and_then(|file| file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes))
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("附件读取失败：{e}")))?;
                image_bytes = image_bytes.saturating_add(bytes.len());
                if bytes.len() as u64 > MAX_IMAGE_BYTES || image_bytes > 20 * 1024 * 1024 {
                    return Err((StatusCode::BAD_REQUEST, "input/image_input_budget".into()));
                }
                let media_type = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.to_ascii_lowercase())
                    .map(|lower| match lower.as_str() {
                        "jpg" => "jpeg".to_string(),
                        other => other.to_string(),
                    })
                    .unwrap_or_else(|| "png".to_string());
                attachment_images.push(owo_agent_core::MessageImage {
                    url: format!(
                        "data:image/{media_type};base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ),
                });
                lines.push(format!("- {}（图片，{} 字节，已附到消息）", safe, size));
                continue;
            }
            lines.push(format!(
                "- {}（{} 字节，路径 {}）",
                safe,
                size,
                path.display()
            ));
        }
        effective_prompt.push_str("\n\n附件：\n");
        effective_prompt.push_str(&lines.join("\n"));
    }

    let image_bytes: usize = attachment_images
        .iter()
        .map(|image| {
            let encoded = image.url.rsplit(',').next().unwrap_or_default();
            encoded.len() / 4 * 3 - encoded.bytes().rev().take_while(|b| *b == b'=').count()
        })
        .sum();
    if attachment_images.len() > 8 || image_bytes > 20 * 1024 * 1024 {
        return Err((StatusCode::BAD_REQUEST, "input/image_input_budget".into()));
    }
    let turn_lock = {
        let mut locks = state.turn_locks.lock().map_err(poison)?;
        locks
            .entry(id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let turn_guard = turn_lock
        .try_lock_owned()
        .map_err(|_| (StatusCode::CONFLICT, "该会话已有回合正在运行".to_string()))?;
    // Load the authoritative snapshot only after acquiring the per-session turn lock.
    let session = crate::session_api::load_session(&state, &id)?;
    let mut session = session;
    // Reject before event registration, checkpoints, or session mutations. Native sends check again.
    let has_images = !attachment_images.is_empty()
        || session
            .messages
            .iter()
            .any(super::input_preflight::message_has_image);
    let admission_provider = turn_provider
        .clone()
        .unwrap_or_else(|| state.agent.provider());
    let admission_model = request
        .model_connection
        .as_ref()
        .map(|c| c.model.as_str())
        .or(session.model_override.as_deref());
    admission_provider
        .validate_image_input(admission_model, has_images)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    crate::artifact_review_api::input_assets::rehydrate_history(&state, &mut session).await?;
    // R8：全局并发 turn 上限 + 关闭中拒绝新回合。
    let concurrency_permit = state.shutdown_gate.try_acquire_turn().map_err(|busy| {
        // R10：错误码表接入（domain/reason/retryable 统一前缀，见 error_codes.rs）。
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("[gateway/unavailable/retryable] {busy}"),
        )
    })?;
    // R8/R9：用量预算硬熔断（Agent 4 交付 usage；超限停轮，错误码贯穿，请求用户加额后恢复）。
    if crate::usage::global().check_budget() {
        let reason = crate::usage::global()
            .hard_stop_reason()
            .unwrap_or_else(|| "用量预算超限".to_string());
        let (status, body) = crate::usage::budget_exceeded_response(&reason);
        let detail = body
            .0
            .get("detail")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&reason)
            .to_string();
        return Err((
            status,
            format!("[{}] {detail}", crate::usage::BUDGET_ERROR_CODE),
        ));
    }

    let event_queue = Arc::new(TurnEventQueue::new());
    let turn_id = request
        .turn_id
        .as_deref()
        .map(|id| {
            owo_agent_protocol::canonical_turn_id(id)
                .ok_or_else(|| (StatusCode::BAD_REQUEST, "turn/invalid_id".to_string()))
        })
        .transpose()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if !state
        .store
        .turn_events_after(&id, Some(&turn_id), 0, 1)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .is_empty()
    {
        return Err((
            StatusCode::CONFLICT,
            format!("turn/already_accepted: {turn_id}; replay instead of resubmitting"),
        ));
    }
    let receiver_alive = Arc::new(());
    let receiver_weak = Arc::downgrade(&receiver_alive);
    let abort_flag = state
        .turn_controls
        .lock()
        .map_err(poison)?
        .register(&id, &turn_id)
        .map_err(|code| (StatusCode::CONFLICT, code.to_string()))?;
    let lifecycle = TurnLifecycleGuard {
        state: Arc::clone(&state),
        session_id: id.clone(),
        turn_id: turn_id.clone(),
        queue: Arc::clone(&event_queue),
    };
    let approver = ChannelApprover {
        pending: Arc::clone(&state.pending_approvals),
        pending_sessions: Arc::clone(&state.pending_approval_sessions),
        session_id: id.clone(),
        abort: Arc::clone(&abort_flag),
    };

    let agent = match turn_provider {
        Some(provider) => Arc::new(
            state
                .agent
                .with_turn_provider(provider)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?,
        ),
        None => Arc::clone(&state.agent),
    };
    let store = Arc::clone(&state.store);
    let sessions = Arc::clone(&state.sessions);
    let traces_dir = state.traces_dir.clone();
    let state_for_audit = Arc::clone(&state);
    // A8-2（取优合并自远端 engine）：活跃回合快照供 /activity 轮询。
    let state_for_activity = Arc::clone(&state);
    let producer_store = Arc::clone(&store);
    let producer_session_id = session.id.clone();
    let producer_turn_id = turn_id.clone();
    let trace_started = std::time::Instant::now();
    let trace_started_at = chrono::Utc::now().to_rfc3339();
    let trace_prompt = effective_prompt.clone();
    let stream_queue = Arc::clone(&event_queue);
    tokio::spawn(async move {
        let _turn_guard = turn_guard;
        let _lifecycle = lifecycle;
        let _concurrency_permit = concurrency_permit;
        let mut current = session;
        let original_model = current.model.clone();
        let original_override = current.model_override.clone();
        if let Some(connection) = &request.model_connection {
            current.set_model_override(Some(connection.model.clone()));
        }
        let stream_abort = Arc::clone(&abort_flag);
        let mut tool_starts: std::collections::HashMap<String, std::time::Instant> =
            std::collections::HashMap::new();
        let producer_queue = Arc::clone(&event_queue);
        let producer_receiver = receiver_weak.clone();
        let producer_store = Arc::clone(&producer_store);
        let producer_session_id = producer_session_id.clone();
        let producer_turn_id = producer_turn_id.clone();
        crate::activity_api::begin_activity(&state_for_activity, &current.id);
        let mut on_event = |event: &owo_agent_core::TurnEvent| {
            // A8-2：活跃回合快照随事件推进（thinking/tool/waiting_approval…）。
            crate::activity_api::update_activity(&state_for_activity, &producer_session_id, event);
            // §13 批次九：工具耗时埋点——ToolStart/ToolResult 以 id 配对，差值进
            // /metrics/runtime 的 tool_durations_ms 样本（超上限丢最旧，见 observability_api）。
            match event {
                owo_agent_core::TurnEvent::ToolStart { id, .. } => {
                    tool_starts.insert(id.clone(), std::time::Instant::now());
                }
                owo_agent_core::TurnEvent::ToolResult { id, .. } => {
                    if let Some(started) = tool_starts.remove(id) {
                        crate::observability_api::record_tool_duration_ms(
                            started.elapsed().as_millis() as u64,
                        );
                    }
                }
                _ => {}
            }
            if let Some(sse) = to_sse(event) {
                if persist_and_queue_event(
                    producer_store.as_ref(),
                    &producer_session_id,
                    &producer_turn_id,
                    &producer_queue,
                    &producer_receiver,
                    sse,
                )
                .is_err()
                {
                    // 客户端断开或有界队列溢出后，尽快停止无主/过载回合。
                    stream_abort.store(true, Ordering::Relaxed);
                }
            }
        };
        // ask_user（取优合并自远端 engine）：提问经 SSE 下发，答案经
        // POST /session/{id}/answer/{question_id} 回填；超时 300s 自动收口。
        let questioner = TurnQuestioner {
            state: Arc::clone(&state_for_activity),
            session_id: current.id.clone(),
            turn_id: producer_turn_id.clone(),
            store: Arc::clone(&producer_store),
            queue: Arc::clone(&producer_queue),
            receiver: producer_receiver.clone(),
        };
        let mut success_stats = None;
        let options = owo_agent_core::TurnExecutionOptions {
            session_checkpoint: Some(owo_agent_core::agent::TurnSessionCheckpoint::new(
                Arc::clone(&producer_store),
            )),
            read_only: request.read_only.unwrap_or(false),
            turn_id: Some(producer_turn_id.clone()),
            // 模式是会话属性：纯对话模式只下发联网检索工具，且执行层拒绝其它工具。
            mode: current.mode,
            input: Some(owo_agent_protocol::StoredUserInput {
                prompt: request.prompt.clone(),
                content_parts: content_parts.clone(),
                legacy_attachments: request
                    .attachments
                    .iter()
                    .filter(|id| !crate::artifact_review_api::input_assets::is_asset_id(id))
                    .cloned()
                    .collect(),
            }),
        };
        // 回合级 watchdog：模型流/收尾链路一旦静默（实测出现过 20~30 分钟零事件），
        // 这里按总时长兜底中止，避免"永远执行中"。默认 30 分钟，可用
        // OWO_TURN_MAX_DURATION_SECS 覆盖（30..=86400）。
        let turn_budget = turn_watchdog_budget();
        let turn_result = tokio::time::timeout(
            turn_budget,
            agent.run_turn_with_images_options(
                &mut current,
                &effective_prompt,
                &attachment_images,
                &approver,
                Some(&questioner),
                &abort_flag,
                &mut on_event,
                &options,
            ),
        )
        .await;
        let turn_result = match turn_result {
            Ok(result) => result,
            Err(_) => {
                abort_flag.store(true, Ordering::Relaxed);
                Err(owo_agent_core::AgentError::Session(format!(
                    "turn/watchdog_timeout: 回合超过 {}s 未结束（可能卡在模型流或收尾链路），已强制中止",
                    turn_budget.as_secs()
                )))
            }
        };
        match turn_result {
            Ok(outcome) => {
                let trace = owo_agent_core::TraceRecord::from_outcome(&current, &outcome);
                let _ = owo_agent_core::save_trace(&traces_dir, &trace);
                // §3.2：trace 落盘后发布 traces 域失效（每个回合至多一次）。
                crate::event_stream::hub()
                    .publish_invalidate(crate::event_stream::InvalidateDomain::Traces);
                let mut cost_usd = 0.0f64;
                if outcome.usage.total_tokens > 0 {
                    let input_price = std::env::var("OWO_MODEL_INPUT_PRICE_PER_MTOK")
                        .ok()
                        .and_then(|value| value.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let output_price = std::env::var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK")
                        .ok()
                        .and_then(|value| value.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let cost = outcome.usage.cost_estimate_usd(input_price, output_price);
                    cost_usd = cost;
                    if let Ok(mut audit) = state_for_audit.agent.audit_log().lock() {
                        audit.record(
                            "model",
                            "usage",
                            Some(current.id.clone()),
                            Some(true),
                            format!(
                                "prompt={} completion={} total={} cost_usd≈{:.6}",
                                outcome.usage.prompt_tokens,
                                outcome.usage.completion_tokens,
                                outcome.usage.total_tokens,
                                cost
                            ),
                        );
                    }
                    // R8：用量与成本归集（Agent 4 交付 usage_router 的会话维度记录）。
                    crate::usage::global().record_tokens(
                        crate::usage::UsageDimension::Session,
                        &current.id,
                        Some(&current.id),
                        outcome.usage.prompt_tokens,
                        outcome.usage.completion_tokens,
                    );
                }
                // Delay successful terminal stats until session persistence succeeds.
                success_stats = Some(SseEvent::TurnStats {
                    steps: outcome.steps,
                    duration_ms: outcome.duration_ms,
                    prompt_tokens: outcome.usage.prompt_tokens,
                    completion_tokens: outcome.usage.completion_tokens,
                    total_tokens: outcome.usage.total_tokens,
                    cost_usd,
                    completion_status: outcome.completion_status,
                    completion_record: current.completion_record.clone(),
                    model_calls: outcome
                        .model_calls
                        .iter()
                        .map(|call| owo_agent_protocol::ModelRequestMetricV1 {
                            request_id: call.metadata.request_id.clone(),
                            model: call.metadata.model.clone(),
                            usage: call.metadata.usage.map(|usage| {
                                owo_agent_protocol::ModelTokenUsageV1 {
                                    prompt_tokens: usage.prompt_tokens,
                                    completion_tokens: usage.completion_tokens,
                                    total_tokens: usage.total_tokens,
                                }
                            }),
                            latency_ms: call.metadata.latency_ms,
                            succeeded: call.succeeded,
                        })
                        .collect(),
                });
            }
            Err(error) => {
                let error_text = error.to_string();
                crate::logging::error(
                    "agent",
                    None,
                    "回合执行失败",
                    &[("session_id", serde_json::json!(current.id))],
                );
                let trace = if matches!(&error, owo_agent_core::AgentError::Aborted) {
                    owo_agent_core::TraceRecord::from_aborted(
                        &current,
                        &trace_prompt,
                        &trace_started_at,
                        trace_started.elapsed().as_millis() as u64,
                        &error_text,
                    )
                } else {
                    owo_agent_core::TraceRecord::from_error(
                        &current,
                        &trace_prompt,
                        &trace_started_at,
                        trace_started.elapsed().as_millis() as u64,
                        &error_text,
                    )
                };
                let completion_status = trace
                    .completion_record
                    .as_ref()
                    .map(|record| record.status)
                    .unwrap_or(owo_agent_protocol::CompletionStatusV1::Unverified);
                current.completion_record = trace.completion_record.clone();
                let completion_record = current.completion_record.clone();
                let _ = owo_agent_core::save_trace(&traces_dir, &trace);
                crate::event_stream::hub()
                    .publish_invalidate(crate::event_stream::InvalidateDomain::Traces);
                let _ = persist_and_queue_event(
                    producer_store.as_ref(),
                    &producer_session_id,
                    &producer_turn_id,
                    &producer_queue,
                    &producer_receiver,
                    // 取优合并（远端 engine）：显式 TurnFailed 终态，前端不再停留在
                    // 「执行中」；`is_turn_failed_event` 同时识别旧的 Progress 前缀。
                    SseEvent::TurnFailed {
                        message: error_text.to_string(),
                        completion_status,
                        completion_record,
                    },
                );
            }
        }
        current.model = original_model;
        current.model_override = original_override;
        // A8-2：回合结束（成功/失败）一律从活跃快照移除。
        // The lifecycle guard removes activity only after persistence, audit and queue closure.
        if let Ok(mut sessions) = sessions.lock() {
            sessions.insert(current.id.clone(), current.clone());
        }
        if let Err(error) = store.save(&current) {
            let _ = persist_and_queue_event(
                producer_store.as_ref(),
                &producer_session_id,
                &producer_turn_id,
                &producer_queue,
                &producer_receiver,
                SseEvent::TurnFailed {
                    message: format!("session save failed: {error}"),
                    completion_status: owo_agent_protocol::CompletionStatusV1::Unverified,
                    completion_record: None,
                },
            );
        } else if let Some(stats) = success_stats {
            let _ = persist_and_queue_event(
                producer_store.as_ref(),
                &producer_session_id,
                &producer_turn_id,
                &producer_queue,
                &producer_receiver,
                stats,
            );
        }
        crate::audit_api::flush_audit(&state_for_audit);
        producer_queue.close();
    });

    let event_stream = stream::unfold(
        (stream_queue, receiver_alive),
        |(queue, receiver_alive)| async move {
            loop {
                match queue.pop() {
                    TurnEventPop::Event(event) => {
                        return Some((
                            to_event(event.seq, event.event),
                            (Arc::clone(&queue), receiver_alive),
                        ));
                    }
                    TurnEventPop::Empty => queue.notify.notified().await,
                    TurnEventPop::Closed => return None,
                }
            }
        },
    );
    let mut response = Sse::new(event_stream).into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&turn_id) {
        response.headers_mut().insert("x-owo-turn-id", value);
    }
    Ok(response)
}

pub(crate) async fn respond_permission(
    State(state): State<Arc<AppState>>,
    AxumPath((session_id, request_id)): AxumPath<(String, String)>,
    Json(response): Json<PermissionResponse>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let belongs_to_session = state
        .pending_approval_sessions
        .lock()
        .map_err(poison)?
        .get(&request_id)
        .map(|pending_session| pending_session == &session_id)
        .unwrap_or(false);
    if !belongs_to_session {
        return Err((
            StatusCode::NOT_FOUND,
            format!("审批请求不存在：{request_id}"),
        ));
    }
    let (sender, request) = state
        .pending_approvals
        .lock()
        .map_err(poison)?
        .remove(&request_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("审批请求不存在：{request_id}"),
            )
        })?;
    state
        .pending_approval_sessions
        .lock()
        .map_err(poison)?
        .remove(&request_id);
    let session_workspace = crate::session_api::load_session(&state, &session_id)?.workspace;
    let approval_workspace_id = session_workspace
        .canonicalize()
        .unwrap_or(session_workspace)
        .to_string_lossy()
        .into_owned();
    let mut staged_grant = None;
    let decision = if response.allow {
        // §5.4 只有只读动作可转换成可复用 Grant；写入/执行仅批准当前请求。
        // 破坏性/注入请求不允许生成 grant（scope 一律忽略，仅放行本次）。
        if let Some(scope) = response.scope.as_deref() {
            let level_ok = !request.is_destructive();
            if level_ok {
                if let Some(scope_enum) = owo_agent_core::grant_store::GrantScope::parse(scope) {
                    if let Some(grant) =
                        state
                            .grants
                            .grant_from_scope(&request, &approval_workspace_id, scope_enum)
                    {
                        staged_grant = Some(grant);
                    }
                }
            }
        }
        Decision::Allow
    } else {
        Decision::Deny
    };
    sender
        .send(decision)
        .map_err(|_| (StatusCode::GONE, "审批通道已关闭".to_string()))?;
    // A closed/expired responder must not create a reusable grant.
    let grant_created = staged_grant.is_some();
    if let Some(grant) = staged_grant {
        state.grants.insert(grant);
    }
    Ok(Json(json!({
        "ok": true,
        "allowed": response.allow,
        "granted": grant_created,
    })))
}

/// ask_user 的 SSE 提问通道（取优合并自远端 engine）：
/// 展示问题（UserQuestion）→ 等待应答（oneshot，300s 超时）→ UserAnswered 收口。
/// 任一环节失败都返回 None，工具层转成明确结果，回合不会静默挂死。
struct TurnQuestioner {
    state: Arc<AppState>,
    session_id: String,
    turn_id: String,
    store: Arc<dyn owo_agent_core::SessionStore>,
    queue: Arc<TurnEventQueue>,
    receiver: std::sync::Weak<()>,
}

impl TurnQuestioner {
    fn emit(&self, event: SseEvent) {
        let _ = persist_and_queue_event(
            self.store.as_ref(),
            &self.session_id,
            &self.turn_id,
            &self.queue,
            &self.receiver,
            event,
        );
    }
}

#[async_trait::async_trait]
impl owo_agent_core::question::Questioner for TurnQuestioner {
    async fn ask(
        &self,
        question: &owo_agent_core::question::UserQuestion,
    ) -> Option<owo_agent_core::question::QuestionAnswer> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.state.pending_questions.lock().ok()?;
            pending.insert(question.question_id.clone(), tx);
        }
        {
            let mut sessions = self.state.pending_question_sessions.lock().ok()?;
            sessions.insert(question.question_id.clone(), self.session_id.clone());
        }
        self.emit(SseEvent::UserQuestion {
            question_id: question.question_id.clone(),
            question: question.question.clone(),
            options: question.options.clone(),
        });
        match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
            Ok(Ok(answer)) => {
                self.emit(SseEvent::UserAnswered {
                    question_id: answer.question_id.clone(),
                    answer: answer.answer.clone(),
                    source: "user".to_string(),
                });
                Some(answer)
            }
            _ => {
                // 超时/通道销毁/回合中止：清注册并告知前端（提问必须最终有结果）。
                if let Ok(mut pending) = self.state.pending_questions.lock() {
                    pending.remove(&question.question_id);
                }
                if let Ok(mut sessions) = self.state.pending_question_sessions.lock() {
                    sessions.remove(&question.question_id);
                }
                self.emit(SseEvent::UserAnswered {
                    question_id: question.question_id.clone(),
                    answer: String::new(),
                    source: "timeout".to_string(),
                });
                None
            }
        }
    }
}

/// 回合总时长上限（回合级 watchdog）。
fn turn_watchdog_budget() -> std::time::Duration {
    let secs = std::env::var("OWO_TURN_MAX_DURATION_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (30..=86_400).contains(value))
        .unwrap_or(1_800);
    std::time::Duration::from_secs(secs)
}
