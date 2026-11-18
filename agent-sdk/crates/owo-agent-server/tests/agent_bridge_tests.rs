//! 输入法 / 桌宠协调协议 v1 契约（Agent V2 Phase 7）。
//!
//! 覆盖：版本化消息、来源权限、message_id 幂等、状态/审批通知、通知开关，
//! 以及「审批只能在主界面完成」的硬约束。

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

async fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let state = test_state_at(temp.path()).await;
    (state, temp)
}

async fn test_state_at(data: &std::path::Path) -> Arc<owo_agent_server::AppState> {
    let workspace = data.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&data.join("index.db")).unwrap();
    Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        data.join("traces"),
        data.to_path_buf(),
        workspace,
    ))
}

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
        .header(header::HOST, "127.0.0.1:4096")
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

async fn json_of(response: axum::response::Response) -> (u16, serde_json::Value) {
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    )
}

fn message(kind: &str, source: &str, message_id: &str, payload: serde_json::Value) -> String {
    serde_json::json!({
        "protocol_version": 1,
        "message_id": message_id,
        "timestamp": "2026-11-04T10:00:00Z",
        "source": source,
        "correlation_id": "corr-1",
        "kind": kind,
        "payload": payload,
    })
    .to_string()
}

async fn post_message(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    body: &str,
) -> (u16, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(request(state, "POST", "/agent-bridge/messages", Some(body)))
        .await
        .unwrap();
    json_of(response).await
}

#[tokio::test]
async fn intent_submit_is_queued_and_idempotent_per_message_id() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let body = message(
        "intent.submit",
        "ime",
        "m-1",
        serde_json::json!({"user_goal": "把选中的文字整理成要点"}),
    );

    let (status, first) = post_message(&app, &state, &body).await;
    assert_eq!(status, 201, "{first}");
    assert_eq!(first["kind"], "intent.accepted");
    assert_eq!(first["protocol_version"], 1);
    let task_id = first["task_id"].as_str().unwrap().to_string();
    assert_eq!(first["state"], "queued");
    assert_eq!(first["open_route"], format!("/agent-tasks/{task_id}"));
    assert_ne!(first["duplicate"], true);

    let (status, replay) = post_message(&app, &state, &body).await;
    assert_eq!(status, 200, "{replay}");
    assert_eq!(replay["duplicate"], true, "重复 message_id 必须识别");
    assert_eq!(
        replay["task_id"], first["task_id"],
        "重放不得创建第二个任务"
    );

    // 输入法只能提交只读倾向档位；请求执行档位不会生效。
    let (status, _) = post_message(
        &app,
        &state,
        &message(
            "intent.submit",
            "ime",
            "m-2",
            serde_json::json!({"user_goal": "x", "policy_profile": "workspace-execute"}),
        ),
    )
    .await;
    assert_eq!(status, 201);
    let detail = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}"),
            None,
        ))
        .await
        .unwrap();
    let (_, detail) = json_of(detail).await;
    assert_eq!(detail["task"]["policy_profile"], "workspace");
}

#[tokio::test]
async fn protocol_version_source_and_timestamp_are_enforced() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let mut body: serde_json::Value = serde_json::from_str(&message(
        "intent.submit",
        "ime",
        "v-1",
        serde_json::json!({"user_goal": "x"}),
    ))
    .unwrap();
    body["protocol_version"] = serde_json::json!(2);
    let (status, response) = post_message(&app, &state, &body.to_string()).await;
    assert_eq!(status, 400, "{response}");
    assert_eq!(response["code"], "UNSUPPORTED_PROTOCOL_VERSION");

    let mut body: serde_json::Value = serde_json::from_str(&message(
        "intent.submit",
        "ime",
        "v-2",
        serde_json::json!({"user_goal": "x"}),
    ))
    .unwrap();
    body["source"] = serde_json::json!("unknown-app");
    let (status, response) = post_message(&app, &state, &body.to_string()).await;
    assert_eq!(status, 400, "{response}");
    assert_eq!(response["code"], "INVALID_SOURCE");

    let mut body: serde_json::Value = serde_json::from_str(&message(
        "intent.submit",
        "ime",
        "v-3",
        serde_json::json!({"user_goal": "x"}),
    ))
    .unwrap();
    body["timestamp"] = serde_json::json!("yesterday");
    let (status, response) = post_message(&app, &state, &body.to_string()).await;
    assert_eq!(status, 400, "{response}");
    assert_eq!(response["code"], "INVALID_REQUEST");
}

#[tokio::test]
async fn approvals_require_the_main_ui_and_sources_are_limited() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let (status, response) = post_message(
        &app,
        &state,
        &message(
            "task.approval.decided",
            "pet",
            "a-1",
            serde_json::json!({"task_id": "t-1", "approval_id": "x", "decision": "approved"}),
        ),
    )
    .await;
    assert_eq!(status, 403, "{response}");
    assert_eq!(response["code"], "APPROVAL_REQUIRES_MAIN_UI");

    let (status, response) = post_message(
        &app,
        &state,
        &message(
            "task.pause",
            "ime",
            "a-2",
            serde_json::json!({"task_id": "t-1"}),
        ),
    )
    .await;
    assert_eq!(status, 403, "{response}");
    assert_eq!(response["code"], "SOURCE_NOT_ALLOWED");
}

#[tokio::test]
async fn pet_can_pause_and_cancel_and_status_messages_report_state() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let (_, created) = post_message(
        &app,
        &state,
        &message(
            "intent.submit",
            "ime",
            "p-1",
            serde_json::json!({"user_goal": "待取消的任务"}),
        ),
    )
    .await;
    let task_id = created["task_id"].as_str().unwrap().to_string();

    let (status, cancelled) = post_message(
        &app,
        &state,
        &message(
            "task.cancel",
            "pet",
            "p-2",
            serde_json::json!({"task_id": task_id, "reason": "桌宠取消"}),
        ),
    )
    .await;
    assert_eq!(status, 200, "{cancelled}");
    assert_eq!(cancelled["state"], "cancelled");

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/agent-bridge/tasks/{task_id}/status"),
            None,
        ))
        .await
        .unwrap();
    let (status, status_body) = json_of(response).await;
    assert_eq!(status, 200, "{status_body}");
    assert_eq!(status_body["notifications_enabled"], true);
    assert_eq!(status_body["state"], "cancelled");
    let kinds: Vec<&str> = status_body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"task.status.changed"));
    let changed = status_body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "task.status.changed")
        .unwrap();
    assert_eq!(changed["protocol_version"], 1);
    assert_eq!(changed["payload"]["terminal"], true);
    assert!(changed["payload"]["last_sequence"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn pet_notifications_can_be_turned_off() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let (_, created) = post_message(
        &app,
        &state,
        &message(
            "intent.submit",
            "ime",
            "n-1",
            serde_json::json!({"user_goal": "通知开关任务"}),
        ),
    )
    .await;
    let task_id = created["task_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/agent-bridge/preferences",
            Some(r#"{"pet_notifications":false}"#),
        ))
        .await
        .unwrap();
    let (status, prefs) = json_of(response).await;
    assert_eq!(status, 200, "{prefs}");
    assert_eq!(prefs["pet_notifications"], false);

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/agent-bridge/tasks/{task_id}/status"),
            None,
        ))
        .await
        .unwrap();
    let (_, status_body) = json_of(response).await;
    assert_eq!(status_body["notifications_enabled"], false);
    assert_eq!(status_body["messages"].as_array().unwrap().len(), 0);

    let (status, opened) = post_message(
        &app,
        &state,
        &message(
            "task.open",
            "pet",
            "n-2",
            serde_json::json!({"task_id": task_id}),
        ),
    )
    .await;
    assert_eq!(status, 200, "{opened}");
    assert_eq!(opened["requires_main_ui"], true, "审批/详情必须回主界面");
    assert_eq!(opened["open_route"], format!("/agent-tasks/{task_id}"));
}

#[tokio::test]
async fn missing_task_status_is_404_and_empty_goal_is_rejected() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            "/agent-bridge/tasks/no-such-task/status",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);

    let (status, response) = post_message(
        &app,
        &state,
        &message(
            "intent.submit",
            "ime",
            "e-1",
            serde_json::json!({"user_goal": "   "}),
        ),
    )
    .await;
    assert_eq!(status, 400, "{response}");
    assert_eq!(response["code"], "EMPTY_GOALS");
}

#[tokio::test]
async fn intent_replay_survives_store_and_bridge_recreation() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let body = message(
        "intent.submit",
        "ime",
        "durable-1",
        serde_json::json!({"user_goal": "synthetic admission"}),
    );
    let (status, first) = post_message(&app, &state, &body).await;
    assert_eq!(status, 201, "{first}");
    drop(app);
    drop(state);
    let state = test_state_at(temp.path()).await;
    let app = build_router(Arc::clone(&state));
    let (status, replay) = post_message(&app, &state, &body).await;
    assert_eq!(status, 200, "{replay}");
    assert_eq!(replay["duplicate"], true);
    assert_eq!(replay["task_id"], first["task_id"]);
    assert_eq!(state.tasks.list_tasks(None, 100, 0).unwrap().len(), 1);
    assert_eq!(
        state
            .tasks
            .events(first["task_id"].as_str().unwrap(), 0, 100)
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn reused_intent_id_with_changed_payload_is_a_conflict() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let body = message(
        "intent.submit",
        "ime",
        "conflict-1",
        serde_json::json!({"user_goal": "original"}),
    );
    assert_eq!(post_message(&app, &state, &body).await.0, 201);
    let body = message(
        "intent.submit",
        "ime",
        "conflict-1",
        serde_json::json!({"user_goal": "changed"}),
    );
    let (status, response) = post_message(&app, &state, &body).await;
    assert_eq!(status, 409, "{response}");
    assert_eq!(response["code"], "IDEMPOTENCY_CONFLICT");
    assert_eq!(state.tasks.list_tasks(None, 100, 0).unwrap().len(), 1);
}

#[tokio::test]
async fn intent_id_cannot_replay_an_unrelated_cached_open_response() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let body = message(
        "task.open",
        "ime",
        "scoped-id",
        serde_json::json!({"task_id": "synthetic"}),
    );
    assert_eq!(post_message(&app, &state, &body).await.0, 200);
    let body = message(
        "intent.submit",
        "ime",
        "scoped-id",
        serde_json::json!({"user_goal": "synthetic"}),
    );
    let (status, response) = post_message(&app, &state, &body).await;
    assert_eq!(status, 201, "{response}");
    assert_eq!(response["kind"], "intent.accepted");
}
