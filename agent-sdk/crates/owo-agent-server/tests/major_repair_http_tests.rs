//! Focused HTTP regressions for major task execution repairs.
//! Also included by route_contract_tests so the full route suite retains coverage.
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use std::sync::Arc;
use tower::ServiceExt;

struct IdleProvider;

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("IdleProvider 不应被调用".to_string())
    }
}

struct ApprovedWriteThenFinalProvider {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for ApprovedWriteThenFinalProvider {
    async fn complete(
        &self,
        messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        use std::sync::atomic::Ordering;

        let step = self.calls.fetch_add(1, Ordering::SeqCst);
        if step == 0 {
            let request = messages
                .iter()
                .rev()
                .find(|message| message.role == "user")
                .and_then(|message| message.content.as_deref())
                .expect("synthetic write fixture requires the original user request");
            Ok(owo_agent_core::ModelOutput::ToolCalls(vec![
                owo_agent_core::gateway::ToolCall {
                    id: "verification-plan-1".into(),
                    name: "verification_plan".into(),
                    arguments: serde_json::json!({"plan":{
                        "plan_id":"approved-text-write",
                        "requirements":[{
                            "requirement_id":"approved-text-content",
                            "covers_requirement_ids":[format!("user-request:{request}")],
                            "validator_id":"workspace-file-contains-v1",
                            "validator_version":"1",
                            "scope":{"kind":"workspace_paths","relative_paths":["approved-output.txt"]},
                            "arguments":{"text":"approved content"},
                            "required":true,
                            "resources":{"cpu_slots":1,"memory_mb":16,"exclusive_workspace":false,"timeout_ms":10000}
                        }]
                    }}),
                },
            ]))
        } else if step == 1 {
            Ok(owo_agent_core::ModelOutput::ToolCalls(vec![
                owo_agent_core::gateway::ToolCall {
                    id: "write-1".to_string(),
                    name: "write_file".to_string(),
                    arguments: serde_json::json!({
                        "path": "approved-output.txt",
                        "content": "approved content\n"
                    }),
                },
            ]))
        } else {
            Ok(owo_agent_core::ModelOutput::Text(
                "write complete".to_string(),
            ))
        }
    }
}

/// Serialize environment-sensitive fixture construction.
static STATE_ENV_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

async fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    let _guard = STATE_ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    // 限流用例可能已把进程级 RPM 调小；恢复默认（600）再构建，避免污染其他用例。
    std::env::remove_var("OWO_API_RPM_GLOBAL");
    build_state_inner().await
}

/// 无锁的 state 构建（调用方负责持有 STATE_ENV_LOCK 并管理 OWO_API_* 环境变量）。
async fn build_state_inner() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    (state, temp)
}

/// 构造请求（R7：自动附带本 state 的 bearer token）。
fn request(
    state: &Arc<owo_agent_server::AppState>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> axum::http::Request<axum::body::Body> {
    use axum::http::{header, Method, Request};
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

#[tokio::test]
async fn http_turn_readonly_is_request_scoped_and_never_relaxes_parent_policy() {
    for (profile, read_only, should_write) in [
        (owo_agent_core::PermissionProfile::FullAccess, true, false),
        (owo_agent_core::PermissionProfile::FullAccess, false, true),
        (owo_agent_core::PermissionProfile::ReadOnly, false, false),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let policy = Policy::new(&workspace);
        policy.set_profile(profile);
        let agent = Agent::new(
            Arc::new(ApprovedWriteThenFinalProvider {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            ToolRegistry::new(),
            policy,
            Default::default(),
        );
        let store = SqliteSessionStore::open(&temp.path().join("index.db")).unwrap();
        let state = Arc::new(owo_agent_server::AppState::new(
            agent,
            store,
            temp.path().join("traces"),
            temp.path().to_path_buf(),
            workspace.clone(),
        ));
        let session = state
            .store
            .create(&workspace, "fixture-model", None)
            .unwrap();
        let app = build_router(Arc::clone(&state));
        let body =
            serde_json::json!({ "prompt": "write fixture", "read_only": read_only }).to_string();
        let response = app
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{}/turn", session.id),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            axum::body::to_bytes(response.into_body(), 1024 * 1024),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("turn_stats"));
        assert!(String::from_utf8_lossy(&bytes).contains("\"tool\":\"verification_plan\""));
        assert!(!String::from_utf8_lossy(&bytes).contains("tool/verification_plan_required"));
        assert_eq!(workspace.join("approved-output.txt").exists(), should_write);
        assert_eq!(state.agent.permission_profile(), profile);
    }
    let (state, _temp) = test_state().await;
    let response = build_router(Arc::clone(&state))
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let schema: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        schema["components"]["schemas"]["TurnRequest"]["properties"]["read_only"]["type"],
        "boolean"
    );
}

#[tokio::test]
async fn http_scoped_cancellation_preserves_prestart_and_does_not_abort_next_turn() {
    use std::sync::atomic::Ordering;
    let (state, _temp) = test_state().await;
    let session = state
        .store
        .create(&state.workspace, "fixture-model", None)
        .unwrap();
    let app = build_router(Arc::clone(&state));
    let old_id = "01234567-89ab-4def-8123-456789abcdef";
    let next_id = "01234567-89ab-4def-8123-456789abcdee";
    let payload = serde_json::json!({"turn_id":old_id}).to_string();
    let uri = format!("/session/{}/abort", session.id);
    let queued = app
        .clone()
        .oneshot(request(&state, "POST", &uri, Some(&payload)))
        .await
        .unwrap();
    assert_eq!(queued.status().as_u16(), 200);
    let body = axum::body::to_bytes(queued.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["state"], "cancellation_queued");
    let old = state
        .turn_controls
        .lock()
        .unwrap()
        .register(&session.id, old_id)
        .unwrap();
    assert!(old.load(Ordering::Acquire));
    state
        .turn_controls
        .lock()
        .unwrap()
        .finish(&session.id, old_id);
    let next = state
        .turn_controls
        .lock()
        .unwrap()
        .register(&session.id, next_id)
        .unwrap();
    let late = app
        .clone()
        .oneshot(request(&state, "POST", &uri, Some(&payload)))
        .await
        .unwrap();
    assert_eq!(late.status().as_u16(), 200);
    assert!(!next.load(Ordering::Acquire));
    let malformed = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &uri,
            Some(r#"{"turn_id":"invalid"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(malformed.status().as_u16(), 400);
    assert!(!next.load(Ordering::Acquire));
    let next_payload = serde_json::json!({"turn_id":next_id}).to_string();
    let targeted = app
        .clone()
        .oneshot(request(&state, "POST", &uri, Some(&next_payload)))
        .await
        .unwrap();
    assert_eq!(targeted.status().as_u16(), 200);
    assert!(next.load(Ordering::Acquire));
    state
        .turn_controls
        .lock()
        .unwrap()
        .finish(&session.id, next_id);
    let legacy = app
        .oneshot(request(&state, "POST", &uri, None))
        .await
        .unwrap();
    assert_eq!(legacy.status().as_u16(), 200);
    let body = axum::body::to_bytes(legacy.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["state"], "no_active_turn");
}

#[tokio::test]
async fn expired_approval_channel_does_not_create_a_reusable_grant() {
    let (state, _temp) = test_state().await;
    let session = state
        .store
        .create(&state.workspace, "fixture-model", None)
        .unwrap();
    let permission = Policy::new(&state.workspace)
        .evaluate("read_file", &serde_json::json!({"path":"fixture.txt"}));
    let workspace_id = state
        .workspace
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(state
        .grants
        .grant_from_scope(
            &permission,
            &workspace_id,
            owo_agent_core::grant_store::GrantScope::Workspace
        )
        .is_some());
    let before = state.grants.list().len();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    drop(receiver);
    state
        .pending_approvals
        .lock()
        .unwrap()
        .insert(permission.request_id.clone(), (sender, permission.clone()));
    state
        .pending_approval_sessions
        .lock()
        .unwrap()
        .insert(permission.request_id.clone(), session.id.clone());
    let response = build_router(Arc::clone(&state))
        .oneshot(request(
            &state,
            "POST",
            &format!(
                "/session/{}/permission/{}",
                session.id, permission.request_id
            ),
            Some(r#"{"allow":true,"scope":"workspace"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 410);
    assert_eq!(state.grants.list().len(), before);
    assert!(state.pending_approvals.lock().unwrap().is_empty());
    assert!(state.pending_approval_sessions.lock().unwrap().is_empty());
}

async fn custom_fixture(
    format: &str,
) -> (String, tokio::task::JoinHandle<(String, serde_json::Value)>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let format = format.to_string();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        let (header_end, length) = loop {
            let mut buffer = [0u8; 8192];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            raw.extend_from_slice(&buffer[..count]);
            assert!(raw.len() < 1024 * 1024);
            if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&raw[..end]);
                let len = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break (end + 4, len);
            }
        };
        while raw.len() < header_end + length {
            let mut buffer = [0u8; 8192];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            raw.extend_from_slice(&buffer[..count]);
        }
        let headers = String::from_utf8(raw[..header_end].to_vec()).unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&raw[header_end..header_end + length]).unwrap();
        let stream = if format == "anthropic" {
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"CUSTOM_CONNECTION_OK\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        } else {
            "data: {\"choices\":[{\"delta\":{\"content\":\"CUSTOM_CONNECTION_OK\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n"
        };
        let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", stream.len(), stream);
        socket.write_all(reply.as_bytes()).await.unwrap();
        (headers, body)
    });
    (base, task)
}

#[tokio::test]
async fn http_custom_connection_routes_endpoint_key_format_full_url_and_preserves_defaults() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let secret = "custom-fixture-secret-never-persist";
    for (format, full, suffix, expected_path) in [
        ("openai", false, "/v1", "/v1/chat/completions"),
        ("openai", true, "/custom/completion", "/custom/completion"),
        ("anthropic", false, "/v1", "/v1/messages"),
        ("anthropic", true, "/native/complete", "/native/complete"),
    ] {
        let (base, fixture) = custom_fixture(format).await;
        let session = state
            .store
            .create(&state.workspace, "original-model", None)
            .unwrap();
        let payload = serde_json::json!({"prompt":"hello", "read_only":true,
            "model_connection":{"model":"custom-model", "base_url":format!("{base}{suffix}"),
            "api_format":format, "use_full_url":full, "api_key":secret, "temperature":0, "timeout_secs":3}
        }).to_string();
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{}/turn", session.id),
                Some(&payload),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            axum::body::to_bytes(response.into_body(), 1024 * 1024),
        )
        .await
        .unwrap()
        .unwrap();
        let events = String::from_utf8_lossy(&bytes);
        assert!(events.contains("CUSTOM_CONNECTION_OK"), "{events}");
        assert!(events.contains("turn_stats"), "{events}");
        assert!(!events.contains(secret));
        let (headers, body) = tokio::time::timeout(std::time::Duration::from_secs(2), fixture)
            .await
            .unwrap()
            .unwrap();
        assert!(
            headers.starts_with(&format!("POST {expected_path} ")),
            "{headers}"
        );
        if format == "anthropic" {
            assert!(headers
                .to_lowercase()
                .contains(&format!("x-api-key: {secret}")));
            assert!(headers.to_lowercase().contains("anthropic-version:"));
        } else {
            assert!(headers
                .to_lowercase()
                .contains(&format!("authorization: bearer {secret}")));
        }
        assert_eq!(body["model"], "custom-model");
        assert_eq!(body["temperature"].as_f64(), Some(0.0));
        let saved = state.store.load(&session.id).unwrap();
        assert_eq!(saved.model, "original-model");
        assert!(saved.model_override.is_none());
        // The next ordinary turn still reaches the original host provider.
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{}/turn", session.id),
                Some(r#"{"prompt":"default"}"#),
            ))
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("IdleProvider"));
    }
    fn assert_no_secret(dir: &std::path::Path, secret: &[u8]) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                assert_no_secret(&path, secret);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                assert!(
                    !bytes.windows(secret.len()).any(|part| part == secret),
                    "secret persisted in {:?}",
                    path
                );
            }
        }
    }
    assert_no_secret(temp.path(), secret.as_bytes());
}

#[tokio::test]
async fn http_custom_connection_rejects_invalid_before_accepting_turn_and_matches_schema() {
    let (state, _temp) = test_state().await;
    let session = state
        .store
        .create(&state.workspace, "default", None)
        .unwrap();
    for connection in [
        serde_json::json!({"model":"x","base_url":"file:///tmp"}),
        serde_json::json!({"model":"x","base_url":"https://user:secret@example.com/v1"}),
        serde_json::json!({"model":"x","base_url":"https://unrelated-custom.invalid/v1"}),
        serde_json::json!({"model":"x","base_url":"http://localhost:7777","timeout_secs":0}),
        serde_json::json!({"model":"x","base_url":"http://localhost:7777","api_format":"unsupported"}),
    ] {
        let response = build_router(Arc::clone(&state))
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{}/turn", session.id),
                Some(
                    &serde_json::json!({"prompt":"hello","model_connection":connection})
                        .to_string(),
                ),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 400);
    }
    let response = build_router(Arc::clone(&state))
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let schema: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_str(include_str!("../../../clients/ts/openapi.json")).unwrap();
    for name in ["CustomModelConnection", "TurnRequest"] {
        assert_eq!(
            schema["components"]["schemas"][name],
            snapshot["components"]["schemas"][name]
        );
    }
    let connection: owo_agent_protocol::CustomModelConnection = serde_json::from_value(
        serde_json::json!({"model":"x","base_url":"http://localhost","api_key":"test-secret"}),
    )
    .unwrap();
    assert!(!format!("{connection:?}").contains("test-secret"));
}

#[tokio::test]
async fn http_custom_connections_remain_isolated_between_simultaneous_sessions() {
    let (state, _temp) = test_state().await;
    let (base_a, fixture_a) = custom_fixture("openai").await;
    let (base_b, fixture_b) = custom_fixture("openai").await;
    let session_a = state
        .store
        .create(&state.workspace, "default", None)
        .unwrap();
    let session_b = state
        .store
        .create(&state.workspace, "default", None)
        .unwrap();
    let app = build_router(Arc::clone(&state));
    let run = |session: String, base: String, model: &str, key: &str| {
        let request = request(
            &state,
            "POST",
            &format!("/session/{session}/turn"),
            Some(
                &serde_json::json!({"prompt":"hi","model_connection":{
                    "model":model,"base_url":base,"api_key":key,"timeout_secs":3
                }})
                .to_string(),
            ),
        );
        let app = app.clone();
        async move {
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status().as_u16(), 200);
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains("turn_stats"));
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        tokio::join!(
            run(session_a.id, base_a, "model-a", "fake-key-a"),
            run(session_b.id, base_b, "model-b", "fake-key-b")
        );
    })
    .await
    .unwrap();
    let (headers_a, body_a) = fixture_a.await.unwrap();
    let (headers_b, body_b) = fixture_b.await.unwrap();
    assert_eq!(body_a["model"], "model-a");
    assert_eq!(body_b["model"], "model-b");
    assert!(headers_a.contains("fake-key-a") && !headers_a.contains("fake-key-b"));
    assert!(headers_b.contains("fake-key-b") && !headers_b.contains("fake-key-a"));
}

#[tokio::test]
async fn http_custom_connection_timeout_is_turn_local_and_cleans_up() {
    let (state, _temp) = test_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let holding = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        drop(socket);
    });
    let session = state
        .store
        .create(&state.workspace, "default", None)
        .unwrap();
    let app = build_router(Arc::clone(&state));
    let payload = serde_json::json!({"prompt":"hi","model_connection":{
        "model":"slow","base_url":base,"timeout_secs":1
    }})
    .to_string();
    let response = app
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/turn", session.id),
            Some(&payload),
        ))
        .await
        .unwrap();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        axum::body::to_bytes(response.into_body(), 1024 * 1024),
    )
    .await
    .unwrap()
    .unwrap();
    let events = String::from_utf8_lossy(&bytes);
    assert!(
        events.contains("provider/response_header_timeout"),
        "{events}"
    );
    assert!(events.contains("turn_failed"));
    holding.abort();
}
