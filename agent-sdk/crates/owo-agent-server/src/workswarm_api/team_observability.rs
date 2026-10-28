//! 团队可观测面：事件流（SSE/快照）、指标与诊断（从 handlers.rs 拆出）。

use super::*;

/// `GET /teams/{id}/events` 查询参数（`?format=json` 一次性快照，供轮询/契约测试）。
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct TeamEventsQuery {
    #[serde(default)]
    format: Option<String>,
}

/// GET /teams/{id}/events：团队事件流（SSE：审计重放 + 状态轮询，终态后结束）。
///
/// 团队不存在 → 404（先于开流，保持与 `/teams/{id}` 一致的语义）。
pub(super) async fn team_events(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TeamEventsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let team = coordinator
        .get_team_run(&id)
        .await
        .map_err(|e| error_response(&e))?;
    if query.format.as_deref() == Some("json") {
        // 五期（第三路）：快照补 `progress`（seq/counts/current_steps）——轮询降级
        // 不再完全依赖 SSE 帧也能看到当前步骤（快照不可得时为 null，additive 字段）。
        let progress = coordinator
            .progress_snapshot(&id)
            .await
            .ok()
            .and_then(|p| serde_json::to_value(&p).ok());
        return Ok(Json(json!({
            "team_id": team.team_id,
            "status": format!("{:?}", team.status),
            "active": coordinator.is_run_active(&id),
            "interrupted": coordinator.is_interrupted(&id),
            "progress": progress,
            "audit": audit_tail(&coordinator, &id),
        }))
        .into_response());
    }
    // Bound per-subscriber buffering so a slow/disconnected watcher cannot accumulate
    // an unbounded event backlog. New subscribers replay 50 audit entries by default;
    // Last-Event-ID resumes strictly after the supplied team-local audit cursor.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    let after_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let stream_coordinator = Arc::clone(&coordinator);
    let team_id = team.team_id.clone();
    tokio::spawn(async move {
        team_event_stream(stream_coordinator, team_id, tx, after_event_id).await;
    });
    Ok(Sse::new(ReceiverStream::new(rx)).into_response())
}

/// SSE 流任务：按 Last-Event-ID 续传审计（新订阅默认最近 50 条），再 500ms 轮询新增审计 + 状态变化；
/// 审计帧携带团队内单调游标。团队进入终态后结束；客户端断开即退出。
async fn team_event_stream(
    coordinator: Arc<TeamCoordinator>,
    team_id: String,
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    after_event_id: Option<String>,
) {
    if !send_team_event(&tx, json!({ "type": "open", "team_id": team_id }), None).await {
        return;
    }

    // 进度帧（R3）：订阅即发当前快照（客户端立即拿到 current_steps/counts/seq），
    // 此后仅当代次变化才发（seq 单调递增；客户端以 seq 去重/断线续传）。
    let mut last_progress_seq: Option<u64> = None;
    if let Ok(progress) = coordinator.progress_snapshot(&team_id).await {
        last_progress_seq = Some(progress.seq);
        if !send_team_event(
            &tx,
            json!({ "type": "progress", "progress": progress }),
            None,
        )
        .await
        {
            return;
        }
    }

    // 该团队当前的审计条目（session_id = team_id）。
    let team_entries = || -> Vec<owo_agent_core::audit::AuditEntry> {
        let Some(log) = coordinator.audit_log() else {
            return Vec::new();
        };
        let Ok(guard) = log.lock() else {
            return Vec::new();
        };
        guard
            .entries
            .iter()
            .filter(|e| e.session_id == team_id)
            .cloned()
            .collect()
    };

    // 历史重放（最近 50 条）。
    let replay = team_entries();
    let mut seen = replay.len();
    let replay_from = after_event_id
        .as_deref()
        .and_then(|cursor| {
            replay.iter().enumerate().find_map(|(index, entry)| {
                (team_audit_event_id(entry, index) == cursor).then_some(index + 1)
            })
        })
        .unwrap_or_else(|| replay.len().saturating_sub(50));
    for (index, entry) in replay.iter().enumerate().skip(replay_from) {
        if !send_team_event(
            &tx,
            json!({
                "type": "audit",
                "ts": entry.ts,
                "event": entry.event,
                "detail": entry.detail,
            }),
            Some(team_audit_event_id(entry, index)),
        )
        .await
        {
            return;
        }
    }

    let mut last_status: Option<TeamRunStatus> = None;
    loop {
        let Some(team) = coordinator.get_team_run(&team_id).await.ok() else {
            return;
        };
        // 新增审计条目（重放点之后）。
        let entries = team_entries();
        for (index, entry) in entries.iter().enumerate().skip(seen) {
            if !send_team_event(
                &tx,
                json!({
                    "type": "audit",
                    "ts": entry.ts,
                    "event": entry.event,
                    "detail": entry.detail,
                }),
                Some(team_audit_event_id(entry, index)),
            )
            .await
            {
                return;
            }
        }
        seen = entries.len();
        // 状态帧（变化即发；首次必发）。
        if last_status != Some(team.status) {
            last_status = Some(team.status);
            if !send_team_event(
                &tx,
                json!({
                    "type": "state",
                    "status": format!("{:?}", team.status),
                    "active": coordinator.is_run_active(&team_id),
                    "interrupted": coordinator.is_interrupted(&team_id),
                }),
                None,
            )
            .await
            {
                return;
            }
        }
        // 进度帧（seq 变化即发；步骤开始/完成/失败/取消都会推进 coordinator 侧序号）。
        if let Ok(progress) = coordinator.progress_snapshot(&team_id).await {
            if last_progress_seq != Some(progress.seq) {
                last_progress_seq = Some(progress.seq);
                if !send_team_event(
                    &tx,
                    json!({ "type": "progress", "progress": progress }),
                    None,
                )
                .await
                {
                    return;
                }
            }
        }
        if team.status.is_terminal() {
            return;
        }
        tokio::select! {
            _ = tx.closed() => return,
            _ = tokio::time::sleep(Duration::from_millis(250)) => {},
        }
    }
}

fn team_audit_event_id(entry: &owo_agent_core::audit::AuditEntry, index: usize) -> String {
    format!("{}#{}", entry.ts, index.saturating_add(1))
}

async fn send_team_event(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    frame: Value,
    event_id: Option<String>,
) -> bool {
    let mut event = Event::default().data(frame.to_string());
    if let Some(id) = event_id {
        event = event.id(id);
    }
    tx.send(Ok(event)).await.is_ok()
}

/// GET /teams/{id}/metrics：TeamRun 指标汇总（五期 · 第三路）。
///
/// 数据源 = TeamRun 数据目录的 `metrics.jsonl`（`MeasuredRoleWorker` 追加落盘），
/// 每次请求从文件聚合——无进程内账本，重启后仍可读取。响应：
/// `summary`（span 数/成败/返工/总墙钟/调用·token·费用/最慢 Worker/Artifact 版本数）+
/// `roles`（按角色聚合）+ `workers`（span 明细）+ `budget`（预算状态与耗尽原因）。
pub(super) async fn team_metrics(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    // 团队不存在 → 404（与 /teams/{id} 语义一致）。
    let team = coordinator
        .get_team_run(&id)
        .await
        .map_err(|e| error_response(&e))?;
    let journal = workswarm_metrics::MetricsJournal::for_team(coordinator.run_dir(), &id);
    let records = journal.read_records();
    let mut payload = workswarm_metrics::aggregate_metrics(&id, &records, &team.budget);
    workswarm_metrics::attach_request_budget_status(
        &mut payload,
        &team.budget,
        workswarm_metrics::RequestReservationJournal::for_team(coordinator.run_dir(), &id)
            .reservation_count(),
    );
    let lifecycle_journal =
        workswarm_metrics::TeamLifecycleMetricsJournal::for_team(coordinator.run_dir(), &id);
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "lifecycle".to_string(),
            workswarm_metrics::aggregate_lifecycle_metrics(&lifecycle_journal.read_records()),
        );
        object.insert(
            "lifecycle_metrics_file".to_string(),
            json!(lifecycle_journal.path().display().to_string()),
        );
    }
    // 宿主监督结论与指标同一响应返回，避免调用方为了判断停滞再发一次进度请求。
    if let Ok(progress) = coordinator.progress_snapshot(&id).await {
        if let Ok(supervisor) = serde_json::to_value(progress.supervisor) {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("supervisor".to_string(), supervisor);
            }
        }
    }
    // 数据源路径（可观测：UI/运维可直接定位 TeamRun 数据目录里的指标文件）。
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "metrics_file".to_string(),
            json!(journal.path().display().to_string()),
        );
    }
    Ok(Json(payload))
}

/// GET /teams/{id}/diagnostic：脱敏诊断导出（五期 · 第三路）。
///
/// 汇集 TeamRun / 任务视图 / 产物（含 CAS 内容预览）/ 评审记录 / 交接 / 指标 /
/// 审计尾迹，统一经脱敏（凭据类键值与令牌 → `[REDACTED]`、超长文本截断），
/// 供「下载诊断信息」。评审记录复用 WorkSwarm `space.db` 独立连接（打开失败
/// 不阻塞其余诊断面，reviews 缺席即其信号）。
pub(super) async fn team_diagnostic(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    let team = coordinator
        .get_team_run(&id)
        .await
        .map_err(|e| error_response(&e))?;
    let run_state = coordinator
        .load_run_state(&id)
        .map_err(|e| error_response(&e))?;

    // 任务视图（错误信息等自由文本脱敏）。
    let tasks: Vec<Value> = task_view(&run_state)
        .iter()
        .map(workswarm_metrics::sanitize_value)
        .collect();

    // 产物 + 评审记录（评审存储打开失败 → 跳过，不阻塞导出）。
    let review_store =
        SqliteProjectSpaceStore::open(&state.data_root.join("workswarm").join("space.db")).ok();
    let mut artifacts_json: Vec<Value> = Vec::new();
    let mut reviews_json: Vec<Value> = Vec::new();
    if let Some(space_id) = team.project_space_id.as_deref() {
        if let Ok(space) = coordinator.get_project_space(space_id).await {
            if let Ok(artifacts) = coordinator.list_artifacts(&space).await {
                for a in artifacts {
                    let preview = coordinator
                        .cas()
                        .get_text(a.content_ref.strip_prefix("cas://sha256:").unwrap_or(""))
                        .map(|c| {
                            let head: String = c.chars().take(200).collect();
                            workswarm_metrics::sanitize_text(&head)
                        })
                        .unwrap_or_default();
                    artifacts_json.push(json!({
                        "artifact_id": a.artifact_id,
                        "kind": a.kind,
                        "version": a.version,
                        "producer": a.producer,
                        "content_ref": a.content_ref,
                        "review_state": format!("{:?}", a.review_state),
                        "supersedes_artifact_id": a.supersedes_artifact_id,
                        "created_at": a.created_at,
                        "preview": preview,
                    }));
                    if let Some(store) = review_store.as_ref() {
                        if let Ok(records) = store.list_artifact_reviews(&a.artifact_id).await {
                            for r in records {
                                let value = serde_json::to_value(&r).unwrap_or_else(|_| json!({}));
                                reviews_json.push(workswarm_metrics::sanitize_value(&value));
                            }
                        }
                    }
                }
            }
        }
    }

    // 交接记录（completed_summary 等自由文本脱敏）。
    let handoffs_json: Vec<Value> = coordinator
        .list_handoffs(&id)
        .await
        .unwrap_or_default()
        .iter()
        .map(|h| workswarm_metrics::sanitize_value(&serde_json::to_value(h).unwrap_or(json!({}))))
        .collect();

    // 宿主监督快照（无模型调用；与 /events 的 progress.supervisor 同口径）。
    let supervisor = coordinator
        .progress_snapshot(&id)
        .await
        .ok()
        .and_then(|progress| serde_json::to_value(progress.supervisor).ok())
        .map(|value| workswarm_metrics::sanitize_value(&value));

    // 审计尾迹（最近 200 条，detail 脱敏）。
    let audit_tail_json: Vec<Value> = match coordinator.audit_log() {
        Some(log) => match log.lock() {
            Ok(entries) => entries
                .entries
                .iter()
                .filter(|e| e.session_id == id)
                .rev()
                .take(200)
                .map(|e| {
                    json!({
                        "ts": e.ts,
                        "event": e.event,
                        "tool": e.tool,
                        "detail": workswarm_metrics::sanitize_text(&e.detail),
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };

    // 指标（与 /teams/{id}/metrics 同一聚合口径）。
    let journal = workswarm_metrics::MetricsJournal::for_team(coordinator.run_dir(), &id);
    let mut metrics =
        workswarm_metrics::aggregate_metrics(&id, &journal.read_records(), &team.budget);
    workswarm_metrics::attach_request_budget_status(
        &mut metrics,
        &team.budget,
        workswarm_metrics::RequestReservationJournal::for_team(coordinator.run_dir(), &id)
            .reservation_count(),
    );
    let lifecycle_journal =
        workswarm_metrics::TeamLifecycleMetricsJournal::for_team(coordinator.run_dir(), &id);
    if let Some(object) = metrics.as_object_mut() {
        object.insert(
            "lifecycle".to_string(),
            workswarm_metrics::aggregate_lifecycle_metrics(&lifecycle_journal.read_records()),
        );
        object.insert(
            "lifecycle_metrics_file".to_string(),
            json!(lifecycle_journal.path().display().to_string()),
        );
    }
    let metrics_file = journal.path().display().to_string();

    // TeamRun 本体（budget 等自由 JSON 脱敏）+ 运行标志。
    let mut team_value = serde_json::to_value(&team).unwrap_or_else(|_| json!({}));
    team_value = workswarm_metrics::sanitize_value(&team_value);
    if let Some(obj) = team_value.as_object_mut() {
        obj.insert("active".to_string(), json!(coordinator.is_run_active(&id)));
        obj.insert(
            "interrupted".to_string(),
            json!(coordinator.is_interrupted(&id)),
        );
    }

    Ok(Json(json!({
        "team_id": team.team_id,
        "generated_at": workswarm_metrics::rfc3339(),
        "team": team_value,
        "tasks": tasks,
        "artifacts": artifacts_json,
        "reviews": reviews_json,
        "handoffs": handoffs_json,
        "supervisor": supervisor,
        "metrics": metrics,
        "metrics_file": metrics_file,
        "audit_tail": audit_tail_json,
        "redaction": {
            "applied": true,
            "note": "凭据类键值与令牌已替换为 [REDACTED]；超长文本截断（*_tokens 为用量计数，不属凭据）"
        },
    })))
}
