//! Gateway expression routing regressions against a real local HTTP listener.
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use owo_intent_gateway::{bridge::ImeBridge, protocol::*, state::ImeState, OwoHttpClient};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
const SESSION: &str = "0123456789abcdef0123456789abcdef";
#[derive(Default)]
struct Probe {
    calls: AtomicUsize,
    generic: AtomicUsize,
    prompt: Mutex<String>,
    token_file: Mutex<Option<std::path::PathBuf>>,
    mode: u8,
    first_started: tokio::sync::Notify,
    release_first: tokio::sync::Notify,
    first_finished: tokio::sync::Notify,
}
async fn expression(
    State(probe): State<Arc<Probe>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let rotated = probe.mode == 4 && probe.calls.load(Ordering::SeqCst) > 0;
    assert_eq!(
        headers["authorization"],
        if rotated {
            "Bearer rotated-fixture"
        } else {
            "Bearer file-token"
        }
    );
    assert_eq!(body.as_object().unwrap().len(), 1);
    *probe.prompt.lock().unwrap() = body["prompt"].as_str().unwrap().into();
    let attempt = probe.calls.fetch_add(1, Ordering::SeqCst);
    if probe.mode == 4 && attempt == 0 {
        std::fs::write(
            probe.token_file.lock().unwrap().as_ref().unwrap(),
            "rotated-fixture\n",
        )
        .unwrap();
        return Err(StatusCode::UNAUTHORIZED);
    }
    if probe.mode == 5 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if probe.mode == 6 {
        if attempt == 0 {
            // Model an accepted upstream job that outlives its HTTP connection.
            // Dropping the handler's JoinHandle does not cancel this backend job.
            return tokio::spawn(async move {
                probe.first_started.notify_one();
                probe.release_first.notified().await;
                probe.first_finished.notify_one();
                Ok(Json(serde_json::json!({"text":"旧任务迟到结果"})))
            })
            .await
            .unwrap();
        }
        return Ok(Json(serde_json::json!({"text":"新任务结果"})));
    }
    match probe.mode {
        1 => return Err(StatusCode::BAD_GATEWAY),
        2 => tokio::time::sleep(Duration::from_millis(400)).await,
        3 => return Ok(Json(serde_json::json!({"text":"中".repeat(4097)}))),
        _ => {}
    }
    Ok(Json(serde_json::json!({"text":"改写后的文字"})))
}
async fn forbidden(State(probe): State<Arc<Probe>>) -> StatusCode {
    probe.generic.fetch_add(1, Ordering::SeqCst);
    StatusCode::INTERNAL_SERVER_ERROR
}
fn req(action: Action, id: &str) -> AgentIpcRequest {
    AgentIpcRequest {
        schema_version: AGENT_PROTOCOL_VERSION,
        action,
        session_id: SESSION.into(),
        request_id: id.into(),
        idempotency_key: id.into(),
        capabilities: SUPPORTED_CAPABILITIES.iter().map(|s| (*s).into()).collect(),
        protocol_min: 2,
        protocol_max: AGENT_PROTOCOL_VERSION,
        user_input: "改写你好".into(),
        input: InputView {
            selected_text: "改写你好".into(),
            ..Default::default()
        },
        application: ApplicationView {
            context_id: "ctx-test".into(),
            ..Default::default()
        },
        parent_request_id: String::new(),
        required_features: vec![],
        session_context: String::new(),
        context_entries: vec![],
        selection: None,
        command_id: String::new(),
        page: 0,
        task_revision: 0,
        slot_updates: vec![],
    }
}
async fn call(bridge: &ImeBridge, request: &AgentIpcRequest) -> AgentIpcResponse {
    AgentIpcResponse::decode(
        &bridge
            .handle_frame(request.encode().unwrap())
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn setup(
    mode: u8,
) -> (
    Arc<Probe>,
    Arc<ImeState>,
    Arc<ImeBridge>,
    std::path::PathBuf,
) {
    let probe = Arc::new(Probe {
        mode,
        ..Default::default()
    });
    let app = Router::new()
        .route("/expression", post(expression))
        .route("/session", post(forbidden))
        .route("/session/{id}/turn", post(forbidden))
        .route("/auth/token", axum::routing::get(forbidden))
        .with_state(probe.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    static ID: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "owo-expression-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(root.join("auth")).unwrap();
    std::fs::write(root.join("auth/token"), "file-token\n").unwrap();
    *probe.token_file.lock().unwrap() = Some(root.join("auth/token"));
    let client = Arc::new(OwoHttpClient::new(&base, root.join("auth/token")).unwrap());
    let state = Arc::new(ImeState::new(Duration::from_secs(60)));
    let bridge = ImeBridge::new(state.clone(), client, "unused", &base);
    (probe, state, bridge, root)
}
async fn start(bridge: &ImeBridge, effects: bool) {
    let mut submit = req(Action::Submit, "submit");
    if !effects {
        submit
            .capabilities
            .retain(|s| s != CAPABILITY_OUTPUT_EFFECTS);
    }
    let draft = call(bridge, &submit).await;
    assert_eq!(draft.task.intent, "text.rewrite");
    let mut commit = submit;
    commit.action = Action::Select;
    commit.request_id = "commit".into();
    commit.idempotency_key = "commit".into();
    commit.command_id = "task-commit".into();
    commit.task_revision = draft.task.revision;
    assert_eq!(call(bridge, &commit).await.status, Status::Thinking);
}
async fn finish(bridge: &ImeBridge, effects: bool) -> AgentIpcResponse {
    for _ in 0..100 {
        let mut poll = req(Action::Poll, "poll");
        if !effects {
            poll.capabilities.retain(|c| c != CAPABILITY_OUTPUT_EFFECTS);
        }
        let response = call(bridge, &poll).await;
        if response.status != Status::Thinking {
            return response;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("expression timed out");
}
#[tokio::test]
async fn expression_uses_only_dedicated_route_and_negotiates_effects() {
    for effects in [true, false] {
        let (probe, _, bridge, _) = setup(0).await;
        start(&bridge, effects).await;
        let response = finish(&bridge, effects).await;
        assert_eq!(response.status, Status::AgentMode);
        assert_eq!(response.commands.len(), 1);
        assert_eq!(response.commands[0].id, "insert-reply");
        assert_eq!(response.commands[0].effect.is_some(), effects);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
        assert!(probe
            .prompt
            .lock()
            .unwrap()
            .contains("<<<TEXT\n你好\nTEXT>>>"));
    }
}
#[tokio::test]
async fn non_expression_hands_off_without_model_or_insert() {
    let (probe, _, bridge, _) = setup(0).await;
    let mut request = req(Action::Submit, "submit");
    request.input.selected_text = "帮我找文件".into();
    let response = call(&bridge, &request).await;
    assert_eq!(response.commands.len(), 1);
    assert_eq!(response.commands[0].id, "open-agent");
    assert!(!response.commands[0].commit_task);
    assert!(response.commands[0].effect.is_none());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn expression_failure_and_oversize_never_create_insert_candidate_or_retry() {
    for mode in [1, 3] {
        let (probe, _, bridge, _) = setup(mode).await;
        start(&bridge, true).await;
        let result = finish(&bridge, true).await;
        assert_eq!(result.status, Status::Error);
        assert!(result.commands.is_empty());
        assert!(!result.retryable);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn cancelled_late_expression_cannot_reappear() {
    let (probe, state, bridge, _) = setup(2).await;
    start(&bridge, true).await;
    while probe.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        call(&bridge, &req(Action::Cancel, "cancel")).await.status,
        Status::Cancelled
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!state.has_session(SESSION));
    assert!(call(&bridge, &req(Action::Poll, "poll"))
        .await
        .commands
        .is_empty());
}
#[tokio::test]
async fn missing_credentials_do_not_fall_back_to_public_token_route() {
    let (probe, _, bridge, root) = setup(0).await;
    std::fs::remove_file(root.join("auth/token")).unwrap();
    start(&bridge, true).await;
    assert_eq!(finish(&bridge, true).await.status, Status::Error);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unauthorized_expression_rereads_rotated_file_once_and_never_bootstraps_public_token() {
    for mode in [4, 5] {
        let (probe, _, bridge, _) = setup(mode).await;
        start(&bridge, true).await;
        let result = finish(&bridge, true).await;
        assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
        assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
        if mode == 4 {
            assert_eq!(result.status, Status::AgentMode);
        } else {
            assert_eq!(result.status, Status::Error);
            assert!(result.commands.is_empty());
        }
    }
}

#[tokio::test]
async fn invalid_frame_is_rejected_before_model_or_public_auth_access() {
    let (probe, state, bridge, _) = setup(0).await;
    assert!(bridge.handle_frame(b"{bad".to_vec()).await.is_none());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
    assert_eq!(state.session_count(), 0);
}

// A real TCP peer accepts the expression, then disconnects during its JSON body.
// This exercises a transport failure after acceptance, rather than an HTTP error.
#[tokio::test]
async fn interrupted_expression_body_is_not_retried_or_replayed_by_polling() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_, _, _, root) = setup(0).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel();
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0 && request.len() < 16384);
            request.extend_from_slice(&buffer[..n]);
        }
        assert!(request.starts_with(b"POST /expression HTTP/1.1\r\n"));
        observed.fetch_add(1, Ordering::SeqCst);
        accepted_tx.send(()).unwrap();
        disconnect_rx.await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"text\":").await.unwrap();
        socket.shutdown().await.unwrap();
        drop(socket);
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                next = listener.accept() => {
                    observed.fetch_add(1, Ordering::SeqCst);
                    drop(next.unwrap());
                }
            }
        }
    });
    let client = Arc::new(OwoHttpClient::new(&base, root.join("auth/token")).unwrap());
    let bridge = ImeBridge::new(
        Arc::new(ImeState::new(Duration::from_secs(60))),
        client,
        "unused",
        &base,
    );
    start(&bridge, true).await;
    tokio::time::timeout(Duration::from_secs(2), accepted_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        call(&bridge, &req(Action::Poll, "pending")).await.status,
        Status::Thinking
    );
    disconnect_tx.send(()).unwrap();
    let result = finish(&bridge, true).await;
    assert_eq!(result.status, Status::Error);
    assert!(result.commands.is_empty());
    assert!(!result.retryable);
    for i in 0..5 {
        let response = call(&bridge, &req(Action::Poll, &format!("after-drop-{i}"))).await;
        // Completed errors are delivered once; later polls return idle AgentMode.
        assert_eq!(response.status, Status::AgentMode);
        assert!(response.commands.is_empty());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    stop_tx.send(()).unwrap();
    peer.await.unwrap();
}

#[tokio::test]
async fn cancelled_request_finishing_after_reused_session_cannot_replace_new_result() {
    let (probe, _, bridge, _) = setup(6).await;
    start(&bridge, true).await;
    tokio::time::timeout(Duration::from_secs(2), probe.first_started.notified())
        .await
        .unwrap();
    assert_eq!(
        call(&bridge, &req(Action::Cancel, "cancel-old"))
            .await
            .status,
        Status::Cancelled
    );
    // Reuse the same session and request keys: cancel must clear old cache entries.
    start(&bridge, true).await;
    let fresh = finish(&bridge, true).await;
    assert_eq!(fresh.status, Status::AgentMode);
    assert_eq!(fresh.message, "新任务结果");
    assert_eq!(fresh.commands.len(), 1);
    let expected = fresh.commands[0].effect.clone();
    probe.release_first.notify_one();
    tokio::time::timeout(Duration::from_secs(2), probe.first_finished.notified())
        .await
        .unwrap();
    for i in 0..5 {
        let response = call(&bridge, &req(Action::Poll, &format!("new-after-old-{i}"))).await;
        assert_eq!(response.status, Status::AgentMode);
        assert_eq!(response.commands.len(), 1);
        assert_eq!(response.commands[0].effect, expected);
        assert!(!response.message.contains("旧任务"));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn edited_input_revokes_old_http_result_without_automatically_starting_new_model() {
    let (probe, _, bridge, _) = setup(6).await;
    start(&bridge, true).await;
    tokio::time::timeout(Duration::from_secs(2), probe.first_started.notified())
        .await
        .unwrap();
    let mut changed = req(Action::Submit, "edited-input");
    changed.user_input = "翻译你好".into();
    changed.input.selected_text = "翻译你好".into();
    let draft = call(&bridge, &changed).await;
    assert_eq!(draft.task.intent, "text.translate");
    assert_eq!(draft.commands[0].id, "task-commit");
    probe.release_first.notify_one();
    tokio::time::timeout(Duration::from_secs(2), probe.first_finished.notified())
        .await
        .unwrap();
    for i in 0..5 {
        let response = call(&bridge, &req(Action::Poll, &format!("edited-poll-{i}"))).await;
        assert_eq!(response.commands.len(), 1);
        assert_eq!(response.commands[0].id, "task-commit");
        assert!(response.commands[0].effect.is_none());
        assert!(!response.message.contains("旧任务"));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn conflicting_execution_key_never_replays_effect_or_starts_second_http_request() {
    let (probe, _, bridge, _) = setup(6).await;
    start(&bridge, true).await;
    tokio::time::timeout(Duration::from_secs(2), probe.first_started.notified())
        .await
        .unwrap();
    let mut commit = req(Action::Select, "commit");
    commit.command_id = "task-commit".into();
    commit.task_revision = 1;
    let mut conflict = commit.clone();
    conflict.request_id = "changed-payload".into();
    conflict.input.selected_text = "different synthetic text".into();
    let rejected = call(&bridge, &conflict).await;
    assert_eq!(rejected.error_code, "idempotency_conflict");
    assert!(rejected.commands.is_empty());
    commit.request_id = "pending-exact-retry".into();
    assert_eq!(call(&bridge, &commit).await.status, Status::Thinking);
    probe.release_first.notify_one();
    let original = finish(&bridge, true).await;
    assert_eq!(original.commands.len(), 1);
    assert!(original.commands[0].effect.is_some());
    let rejected = call(&bridge, &conflict).await;
    assert_eq!(rejected.error_code, "idempotency_conflict");
    assert!(rejected.commands.is_empty());
    commit.request_id = "completed-exact-retry".into();
    let replay = call(&bridge, &commit).await;
    assert_eq!(replay.commands, original.commands);
    assert_eq!(replay.request_id, "completed-exact-retry");
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn retried_draft_keeps_confirmation_valid_and_calls_expression_once() {
    let (probe, _, bridge, _) = setup(0).await;
    let submit = req(Action::Submit, "draft-retry-submit");
    let draft = call(&bridge, &submit).await;
    let mut retry = submit.clone();
    retry.request_id = "draft-redelivery".into();
    let repeated = call(&bridge, &retry).await;
    assert_eq!(repeated.task, draft.task);
    assert_eq!(repeated.state_revision, draft.state_revision);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    let mut commit = submit;
    commit.action = Action::Select;
    commit.request_id = "draft-confirm-request".into();
    commit.idempotency_key = "draft-confirm-key".into();
    commit.command_id = "task-commit".into();
    commit.task_revision = draft.task.revision;
    assert_eq!(call(&bridge, &commit).await.status, Status::Thinking);
    let result = finish(&bridge, true).await;
    assert_eq!(result.commands.len(), 1);
    assert!(result.commands[0].effect.is_some());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rejected_patch_cannot_change_subsequently_confirmed_expression_prompt() {
    let (probe, _, bridge, _) = setup(0).await;
    let submit = req(Action::Submit, "atomic-submit");
    let draft = call(&bridge, &submit).await;
    let text = draft
        .task
        .slots
        .iter()
        .find(|s| s.id == "text")
        .unwrap()
        .value
        .clone();
    let mut patch = submit.clone();
    patch.action = Action::Select;
    patch.request_id = "atomic-patch".into();
    patch.idempotency_key = "atomic-patch".into();
    patch.task_revision = draft.task.revision;
    patch.slot_updates = vec![
        SlotUpdate {
            slot_id: "text".into(),
            value: text.clone(),
            consumed_ranges: vec![],
            lock: false,
        },
        SlotUpdate {
            slot_id: "text".into(),
            value: "unconfirmed synthetic change".into(),
            consumed_ranges: vec![],
            lock: true,
        },
        SlotUpdate {
            slot_id: "text".into(),
            value: "conflicting change".into(),
            consumed_ranges: vec![],
            lock: true,
        },
    ];
    let rejected = call(&bridge, &patch).await;
    assert_eq!(rejected.error_code, "slot_locked");
    assert_eq!(rejected.task, draft.task);
    let replay = call(&bridge, &submit).await;
    assert_eq!(replay.task, draft.task);
    assert_eq!(replay.commands, draft.commands);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    let mut commit = submit;
    commit.action = Action::Select;
    commit.request_id = "atomic-confirm".into();
    commit.idempotency_key = "atomic-confirm".into();
    commit.command_id = "task-commit".into();
    commit.task_revision = draft.task.revision;
    assert_eq!(call(&bridge, &commit).await.status, Status::Thinking);
    let result = finish(&bridge, true).await;
    assert_eq!(result.commands.len(), 1);
    let prompt = probe.prompt.lock().unwrap().clone();
    assert!(prompt.contains(&format!("<<<TEXT\n{text}\nTEXT>>>")));
    assert!(!prompt.contains("unconfirmed synthetic change"));
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn accepted_patch_retry_confirms_once_with_revised_expression_text() {
    let (probe, _, bridge, _) = setup(0).await;
    let submit = req(Action::Submit, "edit-submit");
    let draft = call(&bridge, &submit).await;
    let original = draft
        .task
        .slots
        .iter()
        .find(|s| s.id == "text")
        .unwrap()
        .value
        .clone();
    let mut patch = submit.clone();
    patch.action = Action::Select;
    patch.request_id = "edit-request".into();
    patch.idempotency_key = "edit-key".into();
    patch.task_revision = draft.task.revision;
    patch.slot_updates = vec![
        SlotUpdate {
            slot_id: "text".into(),
            value: original,
            consumed_ranges: vec![],
            lock: false,
        },
        SlotUpdate {
            slot_id: "text".into(),
            value: "revised synthetic source".into(),
            consumed_ranges: vec![],
            lock: true,
        },
    ];
    let edited = call(&bridge, &patch).await;
    assert!(edited.error_code.is_empty());
    let mut retry = patch;
    retry.request_id = "edit-redelivery".into();
    let repeated = call(&bridge, &retry).await;
    assert!(repeated.error_code.is_empty());
    assert_eq!(repeated.task, edited.task);
    assert_eq!(repeated.state_revision, edited.state_revision);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    let mut commit = submit;
    commit.action = Action::Select;
    commit.request_id = "edit-confirm".into();
    commit.idempotency_key = "edit-confirm".into();
    commit.command_id = "task-commit".into();
    commit.task_revision = edited.task.revision;
    assert_eq!(call(&bridge, &commit).await.status, Status::Thinking);
    assert_eq!(finish(&bridge, true).await.commands.len(), 1);
    assert!(probe
        .prompt
        .lock()
        .unwrap()
        .contains("<<<TEXT\nrevised synthetic source\nTEXT>>>"));
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn edited_pending_expression_ignores_late_http_result_until_reconfirmed() {
    let (probe, _, bridge, _) = setup(6).await;
    start(&bridge, true).await;
    tokio::time::timeout(Duration::from_secs(2), probe.first_started.notified())
        .await
        .unwrap();
    let mut patch = req(Action::Select, "edit-inflight");
    patch.task_revision = 1;
    patch.slot_updates = vec![
        SlotUpdate {
            slot_id: "text".into(),
            value: "你好".into(),
            consumed_ranges: vec![],
            lock: false,
        },
        SlotUpdate {
            slot_id: "text".into(),
            value: "edited while pending".into(),
            consumed_ranges: vec![],
            lock: true,
        },
    ];
    let edited = call(&bridge, &patch).await;
    assert!(edited.error_code.is_empty());
    assert_eq!(edited.task.revision, 2);
    probe.release_first.notify_one();
    tokio::time::timeout(Duration::from_secs(2), probe.first_finished.notified())
        .await
        .unwrap();
    for i in 0..5 {
        let response = call(&bridge, &req(Action::Poll, &format!("edited-inflight-{i}"))).await;
        assert_eq!(response.task, edited.task);
        assert_eq!(response.commands.len(), 1);
        assert_eq!(response.commands[0].id, "task-commit");
        assert!(response.commands[0].effect.is_none());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    let mut commit = req(Action::Select, "commit");
    commit.command_id = "task-commit".into();
    commit.task_revision = 1;
    let stale = call(&bridge, &commit).await;
    assert_eq!(stale.error_code, "task_revision_mismatch");
    assert!(stale.commands.is_empty());
    commit.request_id = "edited-confirm".into();
    commit.idempotency_key = "edited-confirm".into();
    commit.task_revision = edited.task.revision;
    assert_eq!(call(&bridge, &commit).await.status, Status::Thinking);
    let fresh = finish(&bridge, true).await;
    assert_eq!(fresh.commands.len(), 1);
    assert!(probe
        .prompt
        .lock()
        .unwrap()
        .contains("<<<TEXT\nedited while pending\nTEXT>>>"));
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fresh_draft_after_consumed_result_cannot_replay_old_http_candidate() {
    let (probe, _, bridge, _) = setup(0).await;
    start(&bridge, true).await;
    let original = finish(&bridge, true).await;
    assert!(original.commands[0].effect.is_some());
    let fresh_submit = req(Action::Submit, "same-text-new-draft");
    let draft = call(&bridge, &fresh_submit).await;
    assert_eq!(draft.task.revision, 2);
    assert_eq!(draft.commands[0].id, "task-commit");
    let mut old_commit = req(Action::Select, "commit");
    old_commit.command_id = "task-commit".into();
    old_commit.task_revision = 1;
    old_commit.request_id = "old-confirm-retry".into();
    let stale = call(&bridge, &old_commit).await;
    assert_eq!(stale.error_code, "task_revision_mismatch");
    assert!(stale.commands.is_empty());
    for i in 0..5 {
        let response = call(
            &bridge,
            &req(Action::Poll, &format!("new-draft-observe-{i}")),
        )
        .await;
        assert_eq!(response.task, draft.task);
        assert_eq!(response.commands, draft.commands);
        assert!(response.commands[0].effect.is_none());
    }
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    let mut commit = fresh_submit;
    commit.action = Action::Select;
    commit.request_id = "fresh-confirm".into();
    commit.idempotency_key = "fresh-confirm".into();
    commit.command_id = "task-commit".into();
    commit.task_revision = draft.task.revision;
    assert_eq!(call(&bridge, &commit).await.status, Status::Thinking);
    assert!(finish(&bridge, true).await.commands[0].effect.is_some());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    assert_eq!(probe.generic.load(Ordering::SeqCst), 0);
}
