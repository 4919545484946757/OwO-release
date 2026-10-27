//! Isolated Settings HTTP persistence, validation and native provider admission.
use owo_agent_core::{
    gateway::{ChatMessage, ModelOutput, ModelProvider},
    permissions::Policy,
    sqlite_store::SqliteSessionStore,
    tools::{ToolRegistry, ToolSpec},
    Agent,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
struct Idle;
#[async_trait::async_trait]
impl ModelProvider for Idle {
    async fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ModelOutput, String> {
        Err("fixture: no model request expected".into())
    }
}
#[tokio::test]
async fn settings_image_declaration_roundtrip_and_invalid_update_is_atomic() {
    if std::env::var_os("OWO_INPUT_SETTINGS_TEST_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "settings_image_declaration_roundtrip_and_invalid_update_is_atomic",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("OWO_INPUT_SETTINGS_TEST_CHILD", "1")
            .env("OWO_MODEL_INPUT_CAPABILITIES", "[]")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let agent = Agent::new(
        Arc::new(Idle),
        ToolRegistry::new(),
        Policy::new(dir.path()),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&dir.path().join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        dir.path().join("traces"),
        dir.path().to_owned(),
        dir.path().to_owned(),
    ));
    let token = state.auth_token.token().to_string();
    let router = owo_agent_server::build_router(state);
    let request = |method: &str, body: Value| {
        axum::http::Request::builder()
            .method(method)
            .uri("/settings")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    };
    let binding = json!({"model":"fixture","base_url":"http://127.0.0.1:4999/v1","api_format":"openai","image_input":false});
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            json!({"model_input_capabilities":[binding.clone()]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = router
        .clone()
        .oneshot(request("GET", Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["model_input_capabilities"][0]["image_input"], false);
    assert_eq!(
        value["runtime"]["model_input_capabilities"][0]["image_input"],
        false
    );
    assert!(value["runtime"]["input_capability_config_error"].is_null());
    assert_eq!(value["model_input_capabilities"][0].get("verified"), None);
    let before = std::fs::read(dir.path().join("settings.json")).unwrap();
    let mut bad = binding.clone();
    bad["base_url"] = json!("https://user:secret@example.com/v1");
    let response = router
        .clone()
        .oneshot(request("POST", json!({"model_input_capabilities":[bad]})))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).unwrap(),
        before
    );
    assert_eq!(
        owo_agent_core::gateway::input_capabilities::input_capability_bindings_from_env()
            .unwrap()
            .len(),
        1
    );
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            json!({"model_input_capabilities":[binding.clone(),binding]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let response = router
        .oneshot(request("POST", json!({"model_input_capabilities":[]})))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        owo_agent_core::gateway::input_capabilities::input_capability_bindings_from_env()
            .unwrap()
            .is_empty()
    );
}

fn fixture_app(
    provider: Arc<dyn ModelProvider>,
) -> (
    tempfile::TempDir,
    Arc<owo_agent_server::AppState>,
    axum::Router,
) {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        Agent::new(
            provider,
            ToolRegistry::new(),
            Policy::new(dir.path()),
            Default::default(),
        ),
        SqliteSessionStore::open(&dir.path().join("index.db")).unwrap(),
        dir.path().join("traces"),
        dir.path().to_owned(),
        dir.path().to_owned(),
    ));
    let app = owo_agent_server::build_router(state.clone());
    (dir, state, app)
}
fn http_request(
    state: &owo_agent_server::AppState,
    uri: &str,
    value: Value,
) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri(uri)
        .header(
            "authorization",
            format!("Bearer {}", state.auth_token.token()),
        )
        .header("content-type", "application/json")
        .body(axum::body::Body::from(value.to_string()))
        .unwrap()
}
async fn body_text(response: axum::response::Response) -> String {
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}
#[tokio::test]
async fn input_preflight_is_read_only_and_turn_rejects_history_images_before_checkpoint() {
    use owo_agent_core::MessageImage;
    let (dir, state, app) = fixture_app(Arc::new(Idle));
    let mut session = owo_agent_core::session::Session::new(dir.path(), "display-only", None);
    let mut message = ChatMessage::user("old input".into());
    // Persisted immutable refs can exist without materialized images after restart.
    message.input = Some(owo_agent_protocol::StoredUserInput {
        prompt: "old input".into(),
        content_parts: vec![owo_agent_protocol::ContentPart::Image {
            asset: owo_agent_protocol::AssetRef {
                artifact_id: "input-00000000-0000-4000-8000-000000000001".into(),
                sha256: "a".repeat(64),
            },
        }],
        legacy_attachments: Vec::new(),
    });
    session.messages.push(message);
    session
        .messages
        .push(ChatMessage::assistant_text("old answer".into()));
    state.store.save(&session).unwrap();
    let before = serde_json::to_value(&session).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let mut connection = json!({"model":"selected","base_url":base,"api_format":"openai"});
    for (declaration, code) in [
        (None, "input/image_capability_unknown"),
        (Some(false), "input/image_not_supported"),
        (Some(true), ""),
    ] {
        if let Some(image_input) = declaration {
            connection["input_capabilities"] = json!({"model":"selected","base_url":base,"api_format":"openai","image_input":image_input});
        }
        let value =
            json!({"session_id":session.id,"has_images":false,"model_connection":connection});
        let response = app
            .clone()
            .oneshot(http_request(&state, "/input/preflight", value))
            .await
            .unwrap();
        if declaration == Some(true) {
            assert_eq!(response.status(), 200);
            assert_eq!(body_text(response).await, r#"{"ok":true}"#);
        } else {
            assert_eq!(response.status(), 400);
            assert!(body_text(response).await.starts_with(code));
        }
        assert_eq!(
            serde_json::to_value(state.store.load(&session.id).unwrap()).unwrap(),
            before
        );
        assert!(state
            .store
            .turn_events_after(&session.id, None, 0, 32)
            .unwrap()
            .is_empty());
    }
    // Excluding the image history permits text with an explicitly unsupported connection.
    connection["input_capabilities"]["image_input"] = false.into();
    let response=app.clone().oneshot(http_request(&state,"/input/preflight",
        json!({"session_id":session.id,"history_keep":0,"has_images":false,"model_connection":connection}))).await.unwrap();
    assert_eq!(response.status(), 200);
    // Newly replayed images are checked even when all old history will be removed.
    let response=app.clone().oneshot(http_request(&state,"/input/preflight",
        json!({"session_id":session.id,"history_keep":0,"has_images":true,"model_connection":connection}))).await.unwrap();
    assert_eq!(response.status(), 400);
    assert!(body_text(response)
        .await
        .starts_with("input/image_not_supported"));
    // Main turn applies the same guard synchronously before rehydration and turn registration.
    for stored_images in [false, true] {
        if stored_images {
            session.messages[0].input = None;
            session.messages[0].images = vec![MessageImage::from_url("data:image/png;base64,QUJD")];
            state.store.save(&session).unwrap();
        }
        let expected = serde_json::to_value(state.store.load(&session.id).unwrap()).unwrap();
        let response = app
            .clone()
            .oneshot(http_request(
                &state,
                &format!("/session/{}/turn", session.id),
                json!({"prompt":"new textual input","model_connection":connection}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        assert!(body_text(response)
            .await
            .starts_with("input/image_not_supported"));
        assert_eq!(
            serde_json::to_value(state.store.load(&session.id).unwrap()).unwrap(),
            expected
        );
        assert!(state
            .store
            .turn_events_after(&session.id, None, 0, 32)
            .unwrap()
            .is_empty());
    }
    // Invalid/ambiguous context never changes the session.
    for value in [
        json!({"session_id":session.id,"history_keep":99}),
        json!({"history_keep":0}),
        json!({"session_id":session.id,"model":"other"}),
        json!({"has_images":true,"verified":true}),
    ] {
        let response = app
            .clone()
            .oneshot(http_request(&state, "/input/preflight", value))
            .await
            .unwrap();
        assert!(response.status().is_client_error());
    }
    let response = app
        .clone()
        .oneshot(http_request(
            &state,
            "/input/preflight",
            json!({"session_id":uuid::Uuid::new_v4().to_string()}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/input/preflight")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert!(
        matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock),
        "admission checks must never send provider requests"
    );
}
#[tokio::test]
async fn input_preflight_default_provider_uses_anthropic_or_openai_actual_config() {
    if std::env::var_os("OWO_PREFLIGHT_DEFAULT_TEST_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "input_preflight_default_provider_uses_anthropic_or_openai_actual_config",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("OWO_PREFLIGHT_DEFAULT_TEST_CHILD", "1")
            .env("OWO_MODEL_INPUT_CAPABILITIES", "[]")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (_dir, state, app) =
        fixture_app(Arc::new(owo_agent_core::gateway::DeferredProvider::new()));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    for format in ["openai", "anthropic"] {
        std::env::set_var("OWO_PROVIDER", format);
        std::env::set_var("OPENAI_BASE_URL", &base);
        std::env::set_var("OPENAI_MODEL", "openai-actual");
        std::env::set_var("OPENAI_API_KEY", "fixture-only");
        std::env::set_var("ANTHROPIC_BASE_URL", &base);
        std::env::set_var("ANTHROPIC_MODEL", "anthropic-actual");
        std::env::set_var("ANTHROPIC_API_KEY", "fixture-only");
        let model = format!("{format}-actual");
        std::env::set_var(
            "OWO_MODEL_INPUT_CAPABILITIES",
            json!([{"model":model,"base_url":base,"api_format":format,"image_input":true}])
                .to_string(),
        );
        let response = app
            .clone()
            .oneshot(http_request(
                &state,
                "/input/preflight",
                json!({"has_images":true}),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            200,
            "{format}: {}",
            body_text(response).await
        );
        let response = app
            .clone()
            .oneshot(http_request(
                &state,
                "/input/preflight",
                json!({"has_images":true,"model":"another-worker"}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        assert!(body_text(response)
            .await
            .starts_with("input/image_capability_unknown"));
    }
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
}
