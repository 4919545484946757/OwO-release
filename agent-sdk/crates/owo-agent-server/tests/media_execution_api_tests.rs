//! Actual Daemon turn -> approval -> builtin media HTTP -> Artifact/CAS -> history.
//! Isolated process and loopback service only; not a cloud, browser or team-lease test.
use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use base64::Engine as _;
use http_body_util::BodyExt;
use owo_agent_core::{
    gateway::{ChatMessage, ModelOutput, ModelProvider, ToolCall},
    permissions::{PermissionProfile, Policy},
    sqlite_store::SqliteSessionStore,
    tools::{ToolRegistry, ToolSpec},
    Agent, CasStore,
};
use owo_agent_server::{build_router, AppState};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

#[derive(Default)]
struct ScriptedModel(Mutex<VecDeque<ModelOutput>>);
impl ScriptedModel {
    fn prepare(&self, tool: &str, output: &str) {
        let args = if tool == "generate_image" {
            json!({"prompt":"loopback media contract", "output":output})
        } else {
            json!({"text":"loopback speech contract", "format":"wav", "output":output})
        };
        *self.0.lock().unwrap() = VecDeque::from([
            ModelOutput::ToolCalls(vec![ToolCall {
                id: "media-call".into(),
                name: tool.into(),
                arguments: args,
            }]),
            ModelOutput::Text("fixture complete".into()),
        ]);
    }
}
#[async_trait::async_trait]
impl ModelProvider for ScriptedModel {
    async fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ModelOutput, String> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or("fixture model exhausted".into())
    }
}
fn child(name: &str) -> bool {
    if std::env::var("OWO_MEDIA_API_TEST_CHILD").ok().as_deref() == Some(name) {
        return false;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", name, "--test-threads=1", "--nocapture"])
        .env("OWO_MEDIA_API_TEST_CHILD", name)
        .env("OWO_CLOUD_ENABLED", "false")
        .env("OWO_MODEL_INPUT_CAPABILITIES", "[]");
    for name in [
        "OPENAI_API_KEY",
        "OWO_IMAGE_API_KEY",
        "OWO_IMAGE_API_KEY_ENV",
        "OWO_TTS_API_KEY",
        "OWO_TTS_API_KEY_ENV",
        "OWO_API_RPM_GLOBAL",
        "OWO_USAGE_COST_BUDGET_USD",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ] {
        cmd.env_remove(name);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
fn make_state(root: &Path, model: Arc<ScriptedModel>) -> Arc<AppState> {
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let policy = Policy::new(&workspace);
    policy.set_profile(PermissionProfile::AutoReview);
    let agent = Agent::new(model, ToolRegistry::new(), policy, Default::default());
    Arc::new(AppState::new(
        agent,
        SqliteSessionStore::open(&root.join("sessions.db")).unwrap(),
        root.join("traces"),
        root.to_owned(),
        workspace,
    ))
}
fn req(state: &AppState, method: &str, path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(
            "authorization",
            format!("Bearer {}", state.auth_token.token()),
        )
        .header("content-type", "application/json")
        .body(if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap()
}
async fn json_response(response: Response) -> Value {
    assert!(response.status().is_success(), "{:?}", response.status());
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}
#[derive(Clone)]
struct MediaRequest {
    path: String,
    body: Value,
    authorized: bool,
}
#[derive(Clone, Default)]
struct ServiceState {
    response: Arc<Mutex<(u16, Vec<u8>, bool)>>,
    requests: Arc<Mutex<Vec<MediaRequest>>>,
}
struct Service {
    state: ServiceState,
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn media_service() -> Service {
    use axum::extract::{OriginalUri, State};
    async fn serve(
        State(state): State<ServiceState>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        state.requests.lock().unwrap().push(MediaRequest {
            path: uri.path().into(),
            body,
            authorized: headers.contains_key("authorization"),
        });
        let (status, bytes, stall) = state.response.lock().unwrap().clone();
        let body = if stall {
            use futures_util::StreamExt;
            Body::from_stream(
                futures_util::stream::once(async { Ok::<Vec<u8>, std::io::Error>(vec![0; 4]) })
                    .chain(futures_util::stream::pending()),
            )
        } else {
            Body::from(bytes)
        };
        (
            StatusCode::from_u16(status).unwrap(),
            [("content-type", "application/octet-stream")],
            body,
        )
            .into_response()
    }
    let state = ServiceState::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/image/images/generations", post(serve))
        .route("/speech/audio/speech", post(serve))
        .with_state(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Service { state, url, task }
}
async fn profiles(app: &Router, state: &AppState, url: &str) {
    let saved=json_response(app.clone().oneshot(req(state,"PUT","/settings/media-connections",json!({
        "schema_version":1,"revision":0,"profiles":[
            {"id":"image","name":"isolated image","base_url":format!("{url}/image"),"model":"fixture-image","credential_env":null,"services":["image_generation"]},
            {"id":"speech","name":"isolated speech","base_url":format!("{url}/speech"),"model":"fixture-speech","credential_env":null,"services":["text_to_speech"]}
        ],"assignments":{"image_generation":"image","text_to_speech":"speech"}
    }))).await.unwrap()).await;
    assert_eq!(saved["catalog"]["revision"], 1);
    assert!(saved["services"]["image_generation"]["verified"].is_null());
}
async fn create_session(app: &Router, state: &AppState) -> String {
    json_response(
        app.clone()
            .oneshot(req(
                state,
                "POST",
                "/session",
                json!({"workspace":state.workspace,"model":"scripted-chat"}),
            ))
            .await
            .unwrap(),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .into()
}
async fn turn(
    app: &Router,
    state: &Arc<AppState>,
    session: &str,
    allow: bool,
    cancel: Option<ServiceState>,
) -> Vec<Value> {
    let response = app
        .clone()
        .oneshot(req(
            state,
            "POST",
            &format!("/session/{session}/turn"),
            json!({"prompt":"media contract"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let turn_id = response.headers()["x-owo-turn-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let mut body = response.into_body();
    let mut pending = String::new();
    let mut events = Vec::new();
    let mut approvals = 0;
    let mut cancellation = None;
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(frame) = body.frame().await {
            if let Ok(bytes) = frame.unwrap().into_data() {
                pending.push_str(std::str::from_utf8(&bytes).unwrap());
                while let Some(end) = pending.find("\n\n") {
                    let event = pending[..end].to_owned();
                    pending.drain(..end + 2);
                    for line in event.lines().filter_map(|line| line.strip_prefix("data: ")) {
                        let value: Value = serde_json::from_str(line).unwrap();
                        if value["type"] == "permission_request" {
                            approvals += 1;
                            let id = value["request_id"].as_str().unwrap();
                            let approval = app
                                .clone()
                                .oneshot(req(
                                    state,
                                    "POST",
                                    &format!("/session/{session}/permission/{id}"),
                                    json!({"allow":allow,"scope":"once"}),
                                ))
                                .await
                                .unwrap();
                            assert_eq!(approval.status(), 200);
                            if let Some(service) = cancel.clone() {
                                let app = app.clone();
                                let state = state.clone();
                                let session = session.to_owned();
                                let turn_id = turn_id.clone();
                                cancellation = Some(tokio::spawn(async move {
                                    tokio::time::timeout(Duration::from_secs(5), async {
                                        loop {
                                            if !service.requests.lock().unwrap().is_empty() {
                                                break;
                                            }
                                            tokio::time::sleep(Duration::from_millis(5)).await;
                                        }
                                    })
                                    .await
                                    .expect("media HTTP must start before cancellation");
                                    let response = app
                                        .oneshot(req(
                                            &state,
                                            "POST",
                                            &format!("/session/{session}/abort"),
                                            json!({"turn_id":turn_id}),
                                        ))
                                        .await
                                        .unwrap();
                                    assert!(response.status().is_success());
                                }));
                            }
                        }
                        events.push(value);
                    }
                }
            }
        }
    })
    .await
    .expect("media turn must terminate, including cancellation");
    if let Some(task) = cancellation {
        task.await.unwrap();
    }
    assert_eq!(
        approvals, 1,
        "media execution must use an explicit once-only approval"
    );
    events
}
// A real one-pixel PNG with a correctly checksummed ancillary chunk at the requested size.
fn png(size: usize) -> Vec<u8> {
    let mut image=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==").unwrap();
    let at = image.len() - 12;
    let mut chunk = b"tEXtComment\0".to_vec();
    chunk.resize(4 + size - image.len() - 12, b'x');
    let mut crc = 0xffff_ffffu32;
    for byte in &chunk {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 1 { 0xedb8_8320 } else { 0 };
        }
    }
    let mut addition = ((chunk.len() - 4) as u32).to_be_bytes().to_vec();
    addition.extend(chunk);
    addition.extend((!crc).to_be_bytes());
    image.splice(at..at, addition);
    assert_eq!(image.len(), size);
    image
}
fn wav(size: usize) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend(((size - 8) as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(8000u32.to_le_bytes());
    out.extend(8000u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(8u16.to_le_bytes());
    out.extend(b"data");
    out.extend(((size - 44) as u32).to_le_bytes());
    out.resize(size, 128);
    out
}
#[tokio::test]
async fn approved_builtin_media_http_publishes_all_sizes_and_survives_host_reopen() {
    if child("approved_builtin_media_http_publishes_all_sizes_and_survives_host_reopen") {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let model = Arc::new(ScriptedModel::default());
    let state = make_state(root.path(), model.clone());
    let app = build_router(state.clone());
    let service = media_service().await;
    profiles(&app, &state, &service.url).await;
    let mut artifacts = Vec::new();
    for tool in ["generate_image", "text_to_speech"] {
        let session = create_session(&app, &state).await;
        for size in [2048, 102400, 1048576] {
            let image = tool == "generate_image";
            let bytes = if image { png(size) } else { wav(size) };
            let output = format!(
                "outputs/{tool}-{size}.{}",
                if image { "png" } else { "wav" }
            );
            *service.state.response.lock().unwrap() = (
                200,
                if image {
                    serde_json::to_vec(&json!({"data":[{"b64_json":base64::engine::general_purpose::STANDARD.encode(&bytes)}]})).unwrap()
                } else {
                    bytes.clone()
                },
                false,
            );
            model.prepare(tool, &output);
            let events = turn(&app, &state, &session, true, None).await;
            assert!(
                !events.iter().any(|event| event["type"] == "error"),
                "{events:?}"
            );
            let result = events
                .iter()
                .find(|event| event["type"] == "tool_result")
                .unwrap();
            assert_eq!(result["ok"], true, "{result}");
            assert_eq!(result["outputs"].as_array().unwrap().len(), 1);
            let artifact = result["outputs"][0].clone();
            assert_eq!(artifact["bytes"], size);
            assert_eq!(artifact["sha256"], CasStore::hash_of(&bytes));
            assert_eq!(
                artifact["mime_type"],
                if image { "image/png" } else { "audio/wav" }
            );
            assert_eq!(std::fs::read(state.workspace.join(&output)).unwrap(), bytes);
            let wire = serde_json::to_string(&events).unwrap();
            assert!(wire.len() < 32 * 1024, "large media must stay out of SSE");
            assert!(
                !wire.contains("data:image/")
                    && !wire.contains("data:audio/")
                    && !wire.contains("data_url")
            );
            let history = json_response(
                app.clone()
                    .oneshot(req(
                        &state,
                        "GET",
                        &format!("/session/{session}"),
                        Value::Null,
                    ))
                    .await
                    .unwrap(),
            )
            .await;
            assert!(
                history["messages"]
                    .to_string()
                    .contains(artifact["artifact_id"].as_str().unwrap()),
                "persisted history must retain structured output"
            );
            let audit = state.agent.audit_log();
            assert!(audit
                .lock()
                .unwrap()
                .entries
                .iter()
                .any(|entry| entry.session_id == session
                    && entry.tool.as_deref() == Some(tool)
                    && entry.approved == Some(true)));
            artifacts.push((artifact, bytes));
        }
    }
    let requests = service.state.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 6);
    for MediaRequest {
        path,
        body,
        authorized: has_auth,
    } in requests
    {
        assert!(
            !has_auth,
            "named credential-free loopback profiles must not borrow chat credentials"
        );
        let image = path.starts_with("/image/");
        assert_eq!(
            body["model"],
            if image {
                "fixture-image"
            } else {
                "fixture-speech"
            }
        );
        assert_eq!(
            path,
            if image {
                "/image/images/generations"
            } else {
                "/speech/audio/speech"
            }
        );
    }
    let old_token = state.auth_token.token().to_owned();
    let reopened = make_state(root.path(), model);
    let app = build_router(reopened.clone());
    for (artifact, bytes) in artifacts {
        let path = format!(
            "/artifacts/{}/content?raw=true",
            artifact["artifact_id"].as_str().unwrap()
        );
        let response = app
            .clone()
            .oneshot(req(&reopened, "GET", &path, Value::Null))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"],
            artifact["mime_type"].as_str().unwrap()
        );
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap()
                .as_ref(),
            bytes
        );
        let mut range = req(&reopened, "GET", &path, Value::Null);
        range
            .headers_mut()
            .insert("range", "bytes=10-29".parse().unwrap());
        let response = app.clone().oneshot(range).await.unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap()
                .as_ref(),
            &bytes[10..30]
        );
        for auth in [None, Some(old_token.as_str())] {
            let mut request = Request::builder().uri(&path);
            if let Some(auth) = auth {
                request = request.header("authorization", format!("Bearer {auth}"));
            }
            assert_eq!(
                app.clone()
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status(),
                401
            );
        }
    }
}
#[tokio::test]
async fn denied_media_never_contacts_service_or_publishes_output() {
    if child("denied_media_never_contacts_service_or_publishes_output") {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let model = Arc::new(ScriptedModel::default());
    let state = make_state(root.path(), model.clone());
    let app = build_router(state.clone());
    let service = media_service().await;
    profiles(&app, &state, &service.url).await;
    for tool in ["generate_image", "text_to_speech"] {
        model.prepare(tool, "outputs/denied");
        let session = create_session(&app, &state).await;
        let events = turn(&app, &state, &session, false, None).await;
        assert!(events
            .iter()
            .any(|event| event["type"] == "tool_result" && event["ok"] == false));
        assert!(events.iter().all(|event| event
            .get("outputs")
            .is_none_or(|outputs| outputs.as_array().is_some_and(Vec::is_empty))));
    }
    assert!(service.state.requests.lock().unwrap().is_empty());
    assert!(!state.workspace.join("outputs").exists());
}
#[tokio::test]
async fn failed_invalid_and_cancelled_media_do_not_report_published_success() {
    if child("failed_invalid_and_cancelled_media_do_not_report_published_success") {
        return;
    }
    for (status, bytes, stall) in [
        (503, b"unavailable".to_vec(), false),
        (200, b"not audio".to_vec(), false),
        (200, vec![], true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(ScriptedModel::default());
        let state = make_state(root.path(), model.clone());
        let app = build_router(state.clone());
        let service = media_service().await;
        profiles(&app, &state, &service.url).await;
        *service.state.response.lock().unwrap() = (status, bytes, stall);
        model.prepare("text_to_speech", "outputs/failed.wav");
        let session = create_session(&app, &state).await;
        let events = turn(
            &app,
            &state,
            &session,
            true,
            stall.then(|| service.state.clone()),
        )
        .await;
        assert!(
            !events
                .iter()
                .any(|event| event["type"] == "tool_result" && event["ok"] == true),
            "{events:?}"
        );
        assert!(events.iter().all(|event| event
            .get("outputs")
            .is_none_or(|outputs| outputs.as_array().is_some_and(Vec::is_empty))));
        assert_eq!(service.state.requests.lock().unwrap().len(), 1);
        assert!(
            !state.workspace.join("outputs/failed.wav").exists(),
            "invalid or interrupted media must not commit an output file"
        );
    }
}

#[tokio::test]
async fn invalid_builtin_media_is_rejected_before_file_commit_and_can_retry() {
    if child("invalid_builtin_media_is_rejected_before_file_commit_and_can_retry") {
        return;
    }
    for tool in ["generate_image", "text_to_speech"] {
        let root = tempfile::tempdir().unwrap();
        let model = Arc::new(ScriptedModel::default());
        let state = make_state(root.path(), model.clone());
        let app = build_router(state.clone());
        let service = media_service().await;
        profiles(&app, &state, &service.url).await;
        let image = tool == "generate_image";
        let output = if image {
            "outputs/retry.png"
        } else {
            "outputs/retry.wav"
        };
        let invalid = b"successful HTTP response, invalid media";
        *service.state.response.lock().unwrap() = (
            200,
            if image {
                serde_json::to_vec(&json!({"data":[{"b64_json":base64::engine::general_purpose::STANDARD.encode(invalid)}]})).unwrap()
            } else {
                invalid.to_vec()
            },
            false,
        );
        let session = create_session(&app, &state).await;
        model.prepare(tool, output);
        let failed = turn(&app, &state, &session, true, None).await;
        assert!(failed
            .iter()
            .any(|event| event["type"] == "tool_result" && event["ok"] == false));
        assert!(
            !state.workspace.join(output).exists(),
            "failed media must not poison the requested output path: {tool}"
        );
        let bytes = if image { png(2048) } else { wav(2048) };
        *service.state.response.lock().unwrap() = (
            200,
            if image {
                serde_json::to_vec(&json!({"data":[{"b64_json":base64::engine::general_purpose::STANDARD.encode(&bytes)}]})).unwrap()
            } else {
                bytes.clone()
            },
            false,
        );
        model.prepare(tool, output);
        let retry = turn(&app, &state, &session, true, None).await;
        assert!(retry
            .iter()
            .any(|event| event["type"] == "tool_result" && event["ok"] == true));
        assert_eq!(std::fs::read(state.workspace.join(output)).unwrap(), bytes);
        assert_eq!(service.state.requests.lock().unwrap().len(), 2);
    }
}
