//! Signed IME intent admission. Never starts a backend or grants a tool.
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use hmac::{Hmac, Mac};
use owo_agent_core::task_runtime::CreateTaskRequest;
use owo_agent_protocol::{
    ExpressionOriginCapsule, ImeTaskReceipt, ImeTaskRequest, ImeTaskStatus, IME_TASK_ORIGIN_KEY_ID,
    IME_TASK_ORIGIN_PURPOSE,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

type Failure = (StatusCode, Json<Value>);
fn fail(status: StatusCode, code: &'static str) -> Failure {
    (status, Json(json!({"ok":false,"code":code})))
}
fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn decode_hex(value: &str) -> Option<Zeroizing<Vec<u8>>> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    (0..32)
        .map(|i| u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok())
        .collect::<Option<Vec<_>>>()
        .map(Zeroizing::new)
}
pub(crate) struct ImeTaskGate {
    enabled: bool,
    key: Option<Zeroizing<[u8; 32]>>,
    last_wall_ms: Mutex<u64>,
}
impl ImeTaskGate {
    pub(crate) fn from_env() -> Self {
        let enabled = std::env::var("OWO_IME_TASKS_ENABLED")
            .ok()
            .is_some_and(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
        let secret = std::env::var("OWO_EXPRESSION_ORIGIN_KEY")
            .ok()
            .map(Zeroizing::new);
        let key = secret
            .as_ref()
            .and_then(|v| decode_hex(v))
            .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
            .filter(|v| v.iter().any(|b| *b != 0))
            .map(Zeroizing::new);
        Self {
            enabled,
            key,
            last_wall_ms: Mutex::new(0),
        }
    }
    fn verify(&self, capsule: &ExpressionOriginCapsule, prompt: &str) -> Result<(), Failure> {
        if !self.enabled {
            return Err(fail(StatusCode::FORBIDDEN, "ime/access_disabled"));
        }
        let key = self
            .key
            .as_ref()
            .ok_or_else(|| fail(StatusCode::FORBIDDEN, "ime/origin_not_registered"))?;
        if capsule.signature.alg != "HMAC-SHA256"
            || capsule.signature.key_id != IME_TASK_ORIGIN_KEY_ID
        {
            return Err(fail(StatusCode::FORBIDDEN, "ime/origin_signature"));
        }
        let bytes = capsule
            .payload
            .canonical_task_bytes()
            .map_err(|_| fail(StatusCode::FORBIDDEN, "ime/origin_scope"))?;
        let tag = decode_hex(&capsule.signature.value)
            .ok_or_else(|| fail(StatusCode::FORBIDDEN, "ime/origin_signature"))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_ref())
            .map_err(|_| fail(StatusCode::FORBIDDEN, "ime/origin_signature"))?;
        mac.update(IME_TASK_ORIGIN_PURPOSE);
        mac.update(&bytes);
        mac.verify_slice(&tag)
            .map_err(|_| fail(StatusCode::FORBIDDEN, "ime/origin_signature"))?;
        if digest(prompt.as_bytes()) != capsule.payload.prompt_sha256 {
            return Err(fail(StatusCode::FORBIDDEN, "ime/origin_prompt_mismatch"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|t| u64::try_from(t.as_millis()).ok())
            .ok_or_else(|| fail(StatusCode::FORBIDDEN, "ime/origin_clock"))?;
        let mut last = self
            .last_wall_ms
            .lock()
            .map_err(|_| fail(StatusCode::FORBIDDEN, "ime/origin_unavailable"))?;
        if now < *last {
            return Err(fail(StatusCode::FORBIDDEN, "ime/origin_clock"));
        }
        *last = now;
        let fresh = &capsule.payload.freshness;
        let end = fresh
            .issued_at_ms
            .checked_add(fresh.ttl_ms)
            .ok_or_else(|| fail(StatusCode::FORBIDDEN, "ime/origin_expired"))?;
        if fresh.issued_at_ms > now.saturating_add(5000) || now >= end {
            return Err(fail(StatusCode::FORBIDDEN, "ime/origin_expired"));
        }
        Ok(())
    }
}
fn task_error(error: owo_agent_core::task_runtime::TaskError) -> Failure {
    let status = match error.code.as_str() {
        "TASK_NOT_FOUND" => StatusCode::NOT_FOUND,
        "IDEMPOTENCY_CONFLICT" => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({"ok":false,"code":error.code})))
}
async fn submit(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Json(body): Json<ImeTaskRequest>,
) -> Result<(StatusCode, Json<ImeTaskReceipt>), Failure> {
    if !crate::expression_api::expression_source_allowed(&headers) {
        return Err(fail(StatusCode::FORBIDDEN, "ime/invalid_source"));
    }
    if body.prompt.trim().is_empty() || body.prompt.chars().count() > 8192 {
        return Err(fail(StatusCode::BAD_REQUEST, "ime/invalid_input"));
    }
    state
        .ime_task_gate
        .verify(&body.origin_capsule, &body.prompt)?;
    let p = &body.origin_capsule.payload;
    // Stable intent identity survives fresh nonces and issuer restarts.
    let identity = serde_json::to_vec(&(&p.issuer, &p.binding))
        .map_err(|_| fail(StatusCode::BAD_REQUEST, "ime/invalid_input"))?;
    let fingerprint = serde_json::to_vec(&(&p.prompt_sha256, &p.origin_request_sha256))
        .map_err(|_| fail(StatusCode::BAD_REQUEST, "ime/invalid_input"))?;
    let key = format!("ime-intent:v1:{}", digest(identity));
    let mut request = CreateTaskRequest::new(body.prompt);
    // Host-selected workspace only; caller cannot add policy, backend or workspace overrides.
    request.workspace_ref = Some(state.workspace.to_string_lossy().into_owned());
    let (task, duplicate) = state
        .tasks
        .create_task_idempotent(request, &key, &json!({"intent_sha256":digest(fingerprint)}))
        .map_err(task_error)?;
    let id = task.task_id.to_string();
    Ok((
        if duplicate {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(ImeTaskReceipt {
            task_id: id.clone(),
            state: task.state.as_str().into(),
            origin_verified: true,
            duplicate,
            open_route: format!("/agent-tasks/{id}"),
            requires_main_ui: true,
        }),
    ))
}
async fn status(
    State(state): State<Arc<crate::AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ImeTaskStatus>, Failure> {
    if !state.ime_task_gate.enabled {
        return Err(fail(StatusCode::FORBIDDEN, "ime/access_disabled"));
    }
    let task = state.tasks.get_task(&id).map_err(task_error)?;
    let pending = state.tasks.approvals(&id, true).map_err(task_error)?.len();
    let after = state
        .tasks
        .store()
        .last_event_sequence(&id)
        .map_err(task_error)?;
    Ok(Json(ImeTaskStatus {
        task_id: id.clone(),
        state: task.state.as_str().into(),
        pending_approvals: pending,
        last_sequence: after,
        open_route: format!("/agent-tasks/{id}"),
        requires_main_ui: true,
    }))
}
pub(crate) fn router(state: Arc<crate::AppState>) -> Router {
    if !crate::agent_bridge_api::bridge_enabled() {
        return Router::new().with_state(state);
    }
    Router::new()
        .route("/agent-bridge/ime/intents", post(submit))
        .route("/agent-bridge/ime/tasks/{id}", get(status))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use owo_agent_core::{
        permissions::Policy, sqlite_store::SqliteSessionStore, tools::ToolRegistry, Agent,
    };
    use owo_agent_protocol::{IME_TASK_ORIGIN_AUDIENCE, IME_TASK_ORIGIN_SCOPE};
    use tower::ServiceExt;

    struct Idle;
    #[async_trait::async_trait]
    impl owo_agent_core::gateway::ModelProvider for Idle {
        async fn complete(
            &self,
            _: &[owo_agent_core::ChatMessage],
            _: &[owo_agent_core::ToolSpec],
        ) -> Result<owo_agent_core::ModelOutput, String> {
            panic!("Admission must not invoke a model");
        }
    }
    fn fixture() -> (Arc<crate::AppState>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let agent = Agent::new(
            Arc::new(Idle),
            ToolRegistry::new(),
            Policy::new(&workspace),
            Default::default(),
        );
        let store = SqliteSessionStore::open(&temp.path().join("sessions.db")).unwrap();
        let mut raw = crate::AppState::new(
            agent,
            store,
            temp.path().join("traces"),
            temp.path().to_path_buf(),
            workspace,
        );
        raw.ime_task_gate = ImeTaskGate {
            enabled: true,
            key: Some(Zeroizing::new([7; 32])),
            last_wall_ms: Mutex::new(0),
        };
        (Arc::new(raw), temp)
    }
    fn body(prompt: &str) -> ImeTaskRequest {
        let golden: Value = serde_json::from_str(include_str!(
            "../../../../contracts/expression-origin/synthetic-golden.json"
        ))
        .unwrap();
        let mut capsule: ExpressionOriginCapsule =
            serde_json::from_value(golden["capsule"].clone()).unwrap();
        capsule.payload.audience = IME_TASK_ORIGIN_AUDIENCE.into();
        capsule.payload.scope.mode = IME_TASK_ORIGIN_SCOPE.into();
        capsule.payload.prompt_sha256 = digest(prompt);
        capsule.payload.freshness.issued_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        capsule.signature.key_id = IME_TASK_ORIGIN_KEY_ID.into();
        let mut request = ImeTaskRequest {
            prompt: prompt.into(),
            origin_capsule: capsule,
        };
        sign(&mut request);
        request
    }
    fn sign(request: &mut ImeTaskRequest) {
        let mut mac = Hmac::<Sha256>::new_from_slice(&[7; 32]).unwrap();
        mac.update(IME_TASK_ORIGIN_PURPOSE);
        // Also permits deliberate invalid-scope test vectors signed by the fixture issuer.
        mac.update(&serde_json::to_vec(&request.origin_capsule.payload).unwrap());
        request.origin_capsule.signature.value = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
    }
    async fn call(
        state: &Arc<crate::AppState>,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:4096")
            .header(
                "authorization",
                format!("Bearer {}", state.auth_token.token()),
            );
        let bytes = match body {
            Some(body) => {
                req = req.header("content-type", "application/json");
                body.to_string()
            }
            None => String::new(),
        };
        let response = crate::build_router(Arc::clone(state))
            .oneshot(req.body(axum::body::Body::from(bytes)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({"raw_error": String::from_utf8_lossy(&bytes)})),
        )
    }

    #[tokio::test]
    async fn signed_intent_is_queued_and_fresh_nonce_replays_same_task() {
        let (state, _temp) = fixture();
        let mut request = body("Synthetic private goal not for status display");
        let (status, first) = call(
            &state,
            "POST",
            "/agent-bridge/ime/intents",
            Some(serde_json::to_value(&request).unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        assert_eq!(first["state"], "queued");
        assert_eq!(first["origin_verified"], true);
        assert_eq!(first["requires_main_ui"], true);
        assert!(first.get("session_id").is_none());
        request.origin_capsule.payload.nonce = "5".repeat(64);
        request.origin_capsule.payload.capsule_id = "6".repeat(32);
        sign(&mut request);
        let (status, replay) = call(
            &state,
            "POST",
            "/agent-bridge/ime/intents",
            Some(serde_json::to_value(request).unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_eq!(replay["duplicate"], true);
        assert_eq!(first["task_id"], replay["task_id"]);
        assert_eq!(state.tasks.list_tasks(None, 100, 0).unwrap().len(), 1);
        let id = first["task_id"].as_str().unwrap();
        let task = state.tasks.get_task(id).unwrap();
        assert_eq!(
            task.workspace_ref.as_deref(),
            Some(state.workspace.to_string_lossy().as_ref())
        );
        assert_eq!(task.policy_profile, "workspace");
        assert_eq!(state.tasks.events(id, 0, 100).unwrap().len(), 2);
        let (status, summary) = call(
            &state,
            "GET",
            &format!("/agent-bridge/ime/tasks/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(summary["last_sequence"], 2);
        assert!(!summary.to_string().contains("Synthetic private"));
        assert!(summary.get("user_goal").is_none());
    }

    #[tokio::test]
    async fn changed_signed_content_conflicts_and_overrides_are_rejected() {
        let (state, _temp) = fixture();
        let original = body("original");
        assert_eq!(
            call(
                &state,
                "POST",
                "/agent-bridge/ime/intents",
                Some(serde_json::to_value(&original).unwrap())
            )
            .await
            .0,
            StatusCode::CREATED
        );
        let changed = body("changed");
        let (status, response) = call(
            &state,
            "POST",
            "/agent-bridge/ime/intents",
            Some(serde_json::to_value(changed).unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{response}");
        let mut override_body = serde_json::to_value(original).unwrap();
        override_body["policy_profile"] = json!("workspace-execute");
        assert_eq!(
            call(
                &state,
                "POST",
                "/agent-bridge/ime/intents",
                Some(override_body)
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(state.tasks.list_tasks(None, 100, 0).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn invalid_signature_scope_expiry_and_prompt_never_create_tasks() {
        let (state, _temp) = fixture();
        let mut vectors = Vec::new();
        let mut request = body("synthetic");
        request.origin_capsule.signature.value = "0".repeat(64);
        vectors.push(request);
        let mut request = body("synthetic");
        request.origin_capsule.payload.scope.mode = "expression_only".into();
        sign(&mut request);
        vectors.push(request);
        let mut request = body("synthetic");
        request.origin_capsule.payload.freshness.issued_at_ms = 1;
        sign(&mut request);
        vectors.push(request);
        let mut request = body("synthetic");
        request.prompt = "tampered".into();
        vectors.push(request);
        let mut request = body("synthetic");
        request.origin_capsule.signature.key_id =
            owo_agent_protocol::EXPRESSION_ORIGIN_KEY_ID.into();
        vectors.push(request);
        for request in vectors {
            let (status, response) = call(
                &state,
                "POST",
                "/agent-bridge/ime/intents",
                Some(serde_json::to_value(request).unwrap()),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
        }
        assert!(state.tasks.list_tasks(None, 100, 0).unwrap().is_empty());
    }

    #[tokio::test]
    async fn disabled_unregistered_and_clock_rollback_fail_closed() {
        let request = body("synthetic");
        let mut gate = ImeTaskGate {
            enabled: false,
            key: Some(Zeroizing::new([7; 32])),
            last_wall_ms: Mutex::new(0),
        };
        assert_eq!(
            gate.verify(&request.origin_capsule, &request.prompt)
                .unwrap_err()
                .1
                 .0["code"],
            "ime/access_disabled"
        );
        gate.enabled = true;
        gate.key = None;
        assert_eq!(
            gate.verify(&request.origin_capsule, &request.prompt)
                .unwrap_err()
                .1
                 .0["code"],
            "ime/origin_not_registered"
        );
        gate.key = Some(Zeroizing::new([7; 32]));
        *gate.last_wall_ms.lock().unwrap() = u64::MAX;
        assert_eq!(
            gate.verify(&request.origin_capsule, &request.prompt)
                .unwrap_err()
                .1
                 .0["code"],
            "ime/origin_clock"
        );
    }

    #[tokio::test]
    async fn status_cursor_reaches_beyond_first_event_page() {
        let (state, _temp) = fixture();
        let task = state
            .tasks
            .create_task(CreateTaskRequest::new("private"))
            .unwrap();
        for _ in 0..215 {
            state
                .tasks
                .store()
                .append_event(
                    task.task_id.as_str(),
                    "fixture.progress",
                    0,
                    json!({}),
                    None,
                )
                .unwrap();
        }
        let (status, summary) = call(
            &state,
            "GET",
            &format!("/agent-bridge/ime/tasks/{}", task.task_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(summary["last_sequence"], 217);
        assert_eq!(summary["pending_approvals"], 0);
        assert!(summary.get("payload").is_none());
    }
}
