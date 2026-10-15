//! E1.4 验收：turn 异步适配桥接测试。
//!
//! 用 axum 搭建符合 agent-server 契约的 mock（`/auth/token`、会话、SSE turn、
//! abort/diff/revert），验证：
//! - submit → 立即 thinking 回包（< 1s）→ poll 拿到最终结果与候选；
//! - cancel → abort 被调用、会话清理；
//! - turn_failed → error 映射；
//! - 401 → 重读 token 文件重试；
//! - token 只来自 `<data_root>/auth/token`，不调用公开 `/auth/token`；回合带 `read_only: true`。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{extract::State, Json, Router};
use owo_intent_gateway::bridge::ImeBridge;
use owo_intent_gateway::protocol::*;
use owo_intent_gateway::state::ImeState;
use owo_intent_gateway::OwoHttpClient;
use tokio_stream::wrappers::ReceiverStream;

const SESSION: &str = "0123456789abcdef0123456789abcdef";

#[derive(Clone, Copy, PartialEq)]
enum TurnMode {
    Complete,
    Failed,
    Approval,
    Slow,
}

struct MockServer {
    mode: TurnMode,
    fail_first_turn_401: bool,
    has_diff: bool,
    abort_called: AtomicBool,
    turn_attempts: AtomicUsize,
    auth_token_called: AtomicBool,
    last_turn_body: std::sync::Mutex<Option<serde_json::Value>>,
    last_bearer: std::sync::Mutex<Option<String>>,
}

impl MockServer {
    fn new(mode: TurnMode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            fail_first_turn_401: false,
            has_diff: false,
            abort_called: AtomicBool::new(false),
            turn_attempts: AtomicUsize::new(0),
            auth_token_called: AtomicBool::new(false),
            last_turn_body: std::sync::Mutex::new(None),
            last_bearer: std::sync::Mutex::new(None),
        })
    }

    async fn serve(self: &Arc<Self>) -> String {
        let app = Router::new()
            .route("/auth/token", get(auth_token))
            .route("/health", get(health))
            .route("/session", post(create_session))
            .route("/session/{id}/turn", post(turn))
            .route("/session/{id}/abort", post(abort))
            .route("/session/{id}/diff", get(diff))
            .route("/session/{id}/revert", post(revert))
            .with_state(Arc::clone(self));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }
}

async fn auth_token(State(mock): State<Arc<MockServer>>) -> Json<serde_json::Value> {
    mock.auth_token_called.store(true, Ordering::SeqCst);
    Json(serde_json::json!({ "token": "public-endpoint-token" }))
}

/// 每个测试独立的 data_root，写入 `auth/token`。
fn temp_data_root(token: &str) -> std::path::PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "owo-gateway-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(root.join("auth")).unwrap();
    std::fs::write(
        root.join("auth").join("token"),
        format!(
            "{token}
"
        ),
    )
    .unwrap();
    root
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "healthy": true, "version": "mock-0.1", "auto_approve": false }))
}

async fn create_session() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "id": "agent-sess-1",
        "workspace": ".",
        "model": "mock",
        "created_at": "2026-09-29T00:00:00Z",
    }))
}

async fn abort(State(mock): State<Arc<MockServer>>) -> axum::http::StatusCode {
    mock.abort_called.store(true, Ordering::SeqCst);
    axum::http::StatusCode::OK
}

async fn diff(State(mock): State<Arc<MockServer>>) -> Json<serde_json::Value> {
    if mock.has_diff {
        Json(serde_json::json!([{ "path": "hello.txt", "before": null, "after": "hi" }]))
    } else {
        Json(serde_json::json!([]))
    }
}

async fn revert() -> axum::http::StatusCode {
    axum::http::StatusCode::OK
}

async fn turn(
    State(mock): State<Arc<MockServer>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    *mock.last_turn_body.lock().unwrap() = Some(body);
    *mock.last_bearer.lock().unwrap() = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let attempt = mock.turn_attempts.fetch_add(1, Ordering::SeqCst);
    if mock.fail_first_turn_401 && attempt == 0 {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(16);
    let mode = mock.mode;
    let mock_handle = Arc::clone(&mock);
    tokio::spawn(async move {
        let send = |name: &'static str, data: &'static str| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(Ok(Event::default().event(name).data(data))).await;
            }
        };
        match mode {
            TurnMode::Complete => {
                send(
                    "progress",
                    r#"{"type":"progress","message":"模型调用","v":1}"#,
                )
                .await;
                send(
                    "tool_use",
                    r#"{"type":"tool_use","id":"t1","tool":"list_dir","args":{},"v":1}"#,
                )
                .await;
                send(
                    "tool_result",
                    r#"{"type":"tool_result","id":"t1","tool":"list_dir","ok":true,"v":1}"#,
                )
                .await;
                send(
                    "final",
                    r#"{"type":"final","text":"mock 回复：已找到 3 个文件","v":1}"#,
                )
                .await;
            }
            TurnMode::Failed => {
                send(
                    "turn_failed",
                    r#"{"type":"turn_failed","message":"模型调用失败：500","v":1}"#,
                )
                .await;
            }
            TurnMode::Approval => {
                send(
                    "permission_request",
                    r#"{"type":"permission_request","request_id":"ap-1","tool":"write_file","args":{},"reason":"写入需要确认","v":1}"#,
                )
                .await;
                // 模拟审批等待：挂住直到 abort 或 15s（真实服务端是 300s 超时）。
                for _ in 0..150 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if mock_handle.abort_called.load(Ordering::SeqCst) {
                        break;
                    }
                }
            }
            TurnMode::Slow => {
                for index in 0..300 {
                    let frame = format!(r#"{{"type":"progress","message":"步骤 {index}","v":1}}"#);
                    if tx
                        .send(Ok(Event::default().event("progress").data(frame)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if mock_handle.abort_called.load(Ordering::SeqCst) {
                        let _ = tx
                            .send(Ok(Event::default()
                                .event("turn_failed")
                                .data(r#"{"type":"turn_failed","message":"已取消","v":1}"#)))
                            .await;
                        break;
                    }
                }
            }
        }
    });
    Sse::new(ReceiverStream::new(rx)).into_response()
}

// ────────────────────────── 测试辅助 ──────────────────────────

fn request(action: Action, request_id: &str) -> AgentIpcRequest {
    AgentIpcRequest {
        schema_version: AGENT_PROTOCOL_VERSION,
        action,
        session_id: SESSION.to_string(),
        request_id: request_id.to_string(),
        parent_request_id: String::new(),
        idempotency_key: request_id.to_string(),
        capabilities: SUPPORTED_CAPABILITIES
            .iter()
            .map(|capability| (*capability).to_string())
            .collect(),
        protocol_min: 2,
        protocol_max: 3,
        required_features: vec![
            "protocol.negotiation".to_string(),
            "input.structured".to_string(),
            "commands.risk".to_string(),
            "tasks.slots".to_string(),
        ],
        user_input: "帮我找文件".to_string(),
        input: InputView::default(),
        application: ApplicationView::default(),
        session_context: String::new(),
        context_entries: Vec::new(),
        command_id: String::new(),
        page: 0,
        task_revision: 0,
        slot_updates: Vec::new(),
    }
}

async fn call(bridge: &ImeBridge, request: &AgentIpcRequest) -> AgentIpcResponse {
    let bytes = bridge
        .handle_frame(request.encode().expect("请求编码"))
        .await
        .expect("必须返回响应");
    AgentIpcResponse::decode(&bytes).expect("响应必须合法")
}

fn spawn_bridge(base_url: &str) -> (Arc<ImeState>, Arc<ImeBridge>) {
    let state = Arc::new(ImeState::new(Duration::from_secs(60)));
    let data_root = temp_data_root("file-token");
    let client = Arc::new(OwoHttpClient::new(
        base_url,
        owo_intent_gateway::http::token_path(&data_root),
    ));
    let bridge = ImeBridge::new(state.clone(), client, ".", base_url);
    (state, bridge)
}

/// 轮询直到非 thinking 状态（或超时）。
async fn poll_until_final(bridge: &ImeBridge, timeout: Duration) -> AgentIpcResponse {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut counter = 0;
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "轮询超时：任务未在 {timeout:?} 内完成"
        );
        counter += 1;
        let poll = request(Action::Poll, &format!("poll-{counter}"));
        let response = call(bridge, &poll).await;
        if response.status != Status::Thinking {
            return response;
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

// ────────────────────────── 测试 ──────────────────────────

#[tokio::test]
async fn submit_then_poll_returns_final_with_candidates() {
    let mock = MockServer::new(TurnMode::Complete);
    let base_url = mock.serve().await;
    let (state, bridge) = spawn_bridge(&base_url);

    // submit → 立即收到 thinking（不得等待 turn 结束）。
    let started = tokio::time::Instant::now();
    let submit = request(Action::Submit, "req-1");
    let immediate = call(&bridge, &submit).await;
    assert_eq!(immediate.status, Status::Thinking);
    assert!(immediate.can_cancel);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "thinking 回包必须 < 1s（10s 管道超时预算）"
    );

    // poll 直到最终结果。
    let final_response = poll_until_final(&bridge, Duration::from_secs(10)).await;
    assert_eq!(final_response.status, Status::AgentMode);
    assert!(
        final_response.message.contains("已找到 3 个文件"),
        "message 应为模型最终回复：{}",
        final_response.message
    );
    assert_eq!(final_response.commands.len(), 1);
    assert_eq!(final_response.commands[0].id, "insert-reply");
    assert_eq!(final_response.commands[0].category, "text.insert");
    let effect = final_response.commands[0]
        .effect
        .as_ref()
        .expect("已协商 output.effects 时插入回复携带 commit_text");
    assert_eq!(effect.kind, EffectKind::CommitText);
    assert!(effect.text.contains("已找到 3 个文件"));
    assert!(effect.selection_sha256.is_empty());

    // 凭据与回合约束。
    assert!(
        !mock.auth_token_called.load(Ordering::SeqCst),
        "网关不得调用公开 /auth/token"
    );
    assert_eq!(
        mock.last_bearer.lock().unwrap().as_deref(),
        Some("Bearer file-token"),
        "Bearer 必须来自 token 文件（已 trim）"
    );
    let body = mock.last_turn_body.lock().unwrap().clone().unwrap();
    assert_eq!(
        body["read_only"],
        serde_json::json!(true),
        "输入法回合必须只读"
    );

    // select insert-reply → completed，文本回传。
    let mut select = request(Action::Select, "req-2");
    select.command_id = "insert-reply".to_string();
    let selected = call(&bridge, &select).await;
    assert_eq!(selected.status, Status::Completed);
    assert!(selected.message.contains("已找到 3 个文件"));

    assert!(state.has_session(SESSION));
}

#[tokio::test]
async fn cancel_calls_abort_and_cleans_session() {
    let mock = MockServer::new(TurnMode::Slow);
    let base_url = mock.serve().await;
    let (state, bridge) = spawn_bridge(&base_url);

    let submit = request(Action::Submit, "req-1");
    assert_eq!(call(&bridge, &submit).await.status, Status::Thinking);
    // 给 turn 任务一点时间建立连接。
    tokio::time::sleep(Duration::from_millis(300)).await;

    let cancel = request(Action::Cancel, "req-2");
    let response = call(&bridge, &cancel).await;
    assert_eq!(response.status, Status::Cancelled);

    // abort 必须被调用（bridge 收到取消信号后）。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !mock.abort_called.load(Ordering::SeqCst) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "cancel 后 5s 内必须调用 abort"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!state.has_session(SESSION), "cancel 后会话必须清理");
}

#[tokio::test]
async fn turn_failed_maps_to_error() {
    let mock = MockServer::new(TurnMode::Failed);
    let base_url = mock.serve().await;
    let (_state, bridge) = spawn_bridge(&base_url);

    let submit = request(Action::Submit, "req-1");
    assert_eq!(call(&bridge, &submit).await.status, Status::Thinking);

    let final_response = poll_until_final(&bridge, Duration::from_secs(10)).await;
    assert_eq!(final_response.status, Status::Error);
    assert_eq!(final_response.error_code, "agent_failed");
    assert!(final_response.retryable);
    assert!(final_response.message.contains("500"));
}

#[tokio::test]
async fn approval_waiting_surfaces_confirm_candidate() {
    let mock = MockServer::new(TurnMode::Approval);
    let base_url = mock.serve().await;
    let (_state, bridge) = spawn_bridge(&base_url);

    let submit = request(Action::Submit, "req-1");
    assert_eq!(call(&bridge, &submit).await.status, Status::Thinking);

    // 等待 permission_request 帧抵达。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "10s 内应出现等待确认状态"
        );
        let poll = request(Action::Poll, "poll-x");
        let response = call(&bridge, &poll).await;
        if response.status == Status::WaitingForConfirmation {
            assert!(response.can_cancel);
            assert_eq!(response.commands.len(), 1);
            assert!(response.commands[0].id.starts_with("confirm-ap-1"));
            assert!(response.commands[0].label.contains("write_file"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    // 收尾：取消避免测试挂尾。
    let cancel = request(Action::Cancel, "req-9");
    let _ = call(&bridge, &cancel).await;
}

#[tokio::test]
async fn unauthorized_turn_refreshes_token_and_retries() {
    let mut mock = MockServer::new(TurnMode::Complete);
    Arc::get_mut(&mut mock).unwrap().fail_first_turn_401 = true;
    let base_url = mock.serve().await;
    let (_state, bridge) = spawn_bridge(&base_url);

    let submit = request(Action::Submit, "req-1");
    assert_eq!(call(&bridge, &submit).await.status, Status::Thinking);

    let final_response = poll_until_final(&bridge, Duration::from_secs(10)).await;
    assert_eq!(
        final_response.status,
        Status::AgentMode,
        "401 后必须刷新 token 并重试成功：{final_response:?}"
    );
    assert_eq!(
        mock.turn_attempts.load(Ordering::SeqCst),
        2,
        "turn 应被请求两次（401 + 重试）"
    );
}

#[tokio::test]
async fn diff_produces_review_candidates() {
    let mut mock = MockServer::new(TurnMode::Complete);
    Arc::get_mut(&mut mock).unwrap().has_diff = true;
    let base_url = mock.serve().await;
    let (_state, bridge) = spawn_bridge(&base_url);

    let submit = request(Action::Submit, "req-1");
    assert_eq!(call(&bridge, &submit).await.status, Status::Thinking);

    let final_response = poll_until_final(&bridge, Duration::from_secs(10)).await;
    assert_eq!(final_response.status, Status::AgentMode);
    assert!(
        final_response.message.contains("1 个文件"),
        "message 应带改动摘要：{}",
        final_response.message
    );
    let ids: Vec<&str> = final_response
        .commands
        .iter()
        .map(|command| command.id.as_str())
        .collect();
    assert!(ids.contains(&"insert-reply"), "候选：{ids:?}");
    assert!(ids.contains(&"view-diff"), "候选：{ids:?}");
    assert!(ids.contains(&"revert-all"), "候选：{ids:?}");
    // 风险字段不变量：revert-all 必须合法（low，无需 high_risk）。
    let revert = final_response
        .commands
        .iter()
        .find(|command| command.id == "revert-all")
        .unwrap();
    assert_eq!(revert.risk_level, RiskLevel::Low);
    assert!(!revert.high_risk);
}

#[tokio::test]
async fn invalid_frame_returns_none() {
    let mock = MockServer::new(TurnMode::Complete);
    let base_url = mock.serve().await;
    let (_state, bridge) = spawn_bridge(&base_url);

    assert!(
        bridge.handle_frame(b"not json".to_vec()).await.is_none(),
        "非法请求必须断连（不回伪造响应）"
    );
    assert!(
        bridge
            .handle_frame(br#"{"schema_version":3,"action":"submit"}"#.to_vec())
            .await
            .is_none(),
        "缺字段请求必须断连"
    );
}

#[test]
fn daemon_descriptor_and_token_file_resolution() {
    use owo_intent_gateway::http::{daemon_base_url, gateway_workspace, token_path};
    let root = temp_data_root("  spaced-token \r\n");
    assert!(daemon_base_url(&root).is_err(), "缺 daemon.json 必须报错");
    std::fs::create_dir_all(root.join("runtime")).unwrap();
    std::fs::write(
        root.join("runtime").join("daemon.json"),
        r#"{"pid":1,"port":48123,"api_version":"1","started_at":"2026-10-16T00:00:00Z"}"#,
    )
    .unwrap();
    assert_eq!(daemon_base_url(&root).unwrap(), "http://127.0.0.1:48123");
    assert_eq!(
        gateway_workspace(&root),
        root.join("gateway").join("workspace")
    );
    assert_eq!(token_path(&root), root.join("auth").join("token"));
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn missing_token_file_maps_to_error_without_public_endpoint() {
    let mock = MockServer::new(TurnMode::Complete);
    let base_url = mock.serve().await;
    let state = Arc::new(ImeState::new(Duration::from_secs(60)));
    let missing = std::env::temp_dir()
        .join("owo-gateway-test-no-such-dir")
        .join("token");
    let client = Arc::new(OwoHttpClient::new(base_url.clone(), missing));
    let bridge = ImeBridge::new(state, client, ".", base_url);
    assert_eq!(
        call(&bridge, &request(Action::Submit, "req-1"))
            .await
            .status,
        Status::Thinking
    );
    let final_response = poll_until_final(&bridge, Duration::from_secs(10)).await;
    assert_eq!(final_response.status, Status::Error);
    assert!(!mock.auth_token_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn expression_task_draft_commit_runs_read_only_turn_with_fenced_prompt() {
    let mock = MockServer::new(TurnMode::Complete);
    let base_url = mock.serve().await;
    let (_state, bridge) = spawn_bridge(&base_url);

    let mut submit = request(Action::Submit, "req-1");
    submit.user_input = "翻译成英文今天天气很好".to_string();
    submit.input.raw_pinyin = "fanyichengyingwenjintiantianqihenhao".to_string();
    submit.input.selected_text = submit.user_input.clone();
    submit.input.natural_language = submit.user_input.clone();
    submit.input.input_mode = "agent".to_string();
    let draft = call(&bridge, &submit).await;
    assert_eq!(
        draft.status,
        Status::AgentMode,
        "规则层意图直接返回草稿：{draft:?}"
    );
    assert_eq!(draft.task.intent, "text.translate");
    assert_eq!(draft.task.revision, 1);
    assert_eq!(draft.commands.len(), 1);
    assert!(draft.commands[0].commit_task);
    assert_eq!(
        mock.turn_attempts.load(Ordering::SeqCst),
        0,
        "草稿阶段不得发起回合"
    );

    // 旧版本提交被拒绝。
    let mut stale = request(Action::Select, "req-2");
    stale.command_id = "task-commit".to_string();
    stale.task_revision = 0;
    let rejected = call(&bridge, &stale).await;
    assert_eq!(rejected.error_code, "task_revision_mismatch");

    let mut commit = request(Action::Select, "req-3");
    commit.command_id = "task-commit".to_string();
    commit.task_revision = 1;
    let started = call(&bridge, &commit).await;
    assert_eq!(started.status, Status::Thinking);

    let final_response = poll_until_final(&bridge, Duration::from_secs(10)).await;
    assert_eq!(final_response.status, Status::AgentMode);
    assert_eq!(final_response.commands[0].id, "insert-reply");
    let body = mock.last_turn_body.lock().unwrap().clone().unwrap();
    assert_eq!(body["read_only"], serde_json::json!(true));
    let prompt = body["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with("把下面围栏中的文字翻译成英文。"),
        "{prompt}"
    );
    assert!(
        prompt.contains("<<<TEXT\n今天天气很好\nTEXT>>>"),
        "{prompt}"
    );
}

#[tokio::test]
async fn non_intent_submit_still_starts_turn() {
    let mock = MockServer::new(TurnMode::Complete);
    let base_url = mock.serve().await;
    let (_state, bridge) = spawn_bridge(&base_url);
    let mut submit = request(Action::Submit, "req-1");
    submit.input.selected_text = "帮我找文件".to_string();
    submit.input.raw_pinyin = "bangwozhaowenjian".to_string();
    assert_eq!(call(&bridge, &submit).await.status, Status::Thinking);
    let final_response = poll_until_final(&bridge, Duration::from_secs(10)).await;
    assert_eq!(final_response.status, Status::AgentMode);
    assert_eq!(mock.turn_attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn insert_reply_without_effects_capability_keeps_v3_shape() {
    let mock = MockServer::new(TurnMode::Complete);
    let base_url = mock.serve().await;
    let (_state, bridge) = spawn_bridge(&base_url);

    let mut submit = request(Action::Submit, "req-1");
    submit
        .capabilities
        .retain(|capability| capability != CAPABILITY_OUTPUT_EFFECTS);
    let immediate = call(&bridge, &submit).await;
    assert_eq!(immediate.status, Status::Thinking);
    let final_response = poll_until_final(&bridge, Duration::from_secs(10)).await;
    assert_eq!(final_response.commands[0].id, "insert-reply");
    assert!(final_response.commands[0].effect.is_none());
    assert!(!final_response
        .capabilities
        .iter()
        .any(|capability| capability == CAPABILITY_OUTPUT_EFFECTS));
    let encoded = String::from_utf8(final_response.encode().unwrap()).unwrap();
    assert!(!encoded.contains("\"effect\""));
}
