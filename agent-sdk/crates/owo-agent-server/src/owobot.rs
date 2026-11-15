//! OwO's local gateway to the managed collaboration process.

pub(crate) mod delegation;
mod delivery;
mod runtime;

use crate::{auth_token::AuthToken, AppState};
use axum::body::{to_bytes, Body};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use owo_agent_protocol::{CustomModelConnection, TurnRequest};
use serde::Deserialize;
use serde_json::json;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const COOKIE: &str = "owo_collaboration";
const WORKER_PROOF: &str = "x-owo-owobot-worker";
const TURN_PROOF: &str = "x-owo-owobot-turn";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerRole {
    Lead,
    Worker,
    Reviewer,
}

impl WorkerRole {
    pub(super) const ALL: [Self; 3] = [Self::Lead, Self::Worker, Self::Reviewer];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Lead => "组长",
            Self::Worker => "执行员",
            Self::Reviewer => "评审员",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
        }
    }

    pub(crate) fn instructions(self) -> &'static str {
        match self {
            Self::Lead => "理解任务，查阅当前项目信息。在可用时使用 owobot_delegate 交给执行员，收到结果后请评审员独立核对，最后汇总。",
            Self::Worker => "查阅当前项目材料。需要交付文件时，使用 owobot_stage_delivery 暂存完整文件集，回报修订版本与验收结果。",
            Self::Reviewer => "独立核对任务、项目材料和执行结果。有暂存文件时，用 owobot_read_delivery 读取确切版本，用 owobot_review_delivery 记录结论。",
        }
    }

    pub(crate) fn readers(self) -> &'static [&'static str] {
        match self {
            Self::Lead => &["read_file", "list_dir", "search_files"],
            Self::Worker => &[
                "read_file",
                "list_dir",
                "search_files",
                "grep",
                "find_symbol",
            ],
            Self::Reviewer => &["read_file", "list_dir", "search_files", "grep"],
        }
    }
}

pub(crate) struct WorkerTurn {
    pub(crate) role: WorkerRole,
    pub(crate) model: Option<String>,
    dispatcher: Option<delegation::Delegate>,
    delivery: delivery::Store,
    scope: CollaborationTurn,
    readonly: bool,
}

impl WorkerTurn {
    pub(crate) async fn project_tools(
        self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Vec<Arc<dyn owo_agent_core::tools::Tool>>, (StatusCode, String)> {
        let actor = delivery::Actor {
            role: self.role.key().to_owned(),
            bot_id: self.scope.bot_id,
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            assignment_id: self.scope.assignment_id,
        };
        let thread = self.scope.thread_id;
        let store = self.delivery;
        let mut tools = tokio::task::spawn_blocking(move || {
            store.bind(&thread, actor.clone())?;
            Ok::<_, delivery::DeliveryError>(delivery::tools::for_role(
                &store,
                &thread,
                &actor,
                self.role,
                self.readonly,
            ))
        })
        .await
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "owobot/delivery_unavailable".into(),
            )
        })?
        .map_err(|error| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("owobot/{}", error.code()),
            )
        })?;
        if let Some(dispatcher) = self.dispatcher {
            tools.push(Arc::new(dispatcher));
        }
        Ok(tools)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollaborationTurn {
    bot_id: String,
    group_id: String,
    thread_id: String,
    depth: u32,
    assignment_id: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelSelection {
    model: Option<String>,
    connection: Option<CustomModelConnection>,
}

impl ModelSelection {
    fn validate(mut self) -> GatewayResult<Self> {
        self.model = self.model.take().and_then(|model| {
            let model = model.trim();
            (!model.is_empty() && model != "default").then(|| model.to_owned())
        });
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.len() > 256 || model.chars().any(char::is_control))
        {
            return Err(GatewayError::Refused(
                StatusCode::BAD_REQUEST,
                "OwOBot 模型名称无效",
            ));
        }
        if let Some(connection) = &self.connection {
            if connection.api_key.is_some() {
                return Err(GatewayError::Refused(
                    StatusCode::BAD_REQUEST,
                    "OwOBot 模型连接需使用桌面端凭据引用",
                ));
            }
            if self.model.as_deref() != Some(connection.model.trim()) {
                return Err(GatewayError::Refused(
                    StatusCode::BAD_REQUEST,
                    "OwOBot 模型与连接不匹配",
                ));
            }
            owo_agent_core::gateway::custom_model_provider(connection).map_err(|_| {
                GatewayError::Refused(
                    StatusCode::BAD_REQUEST,
                    "OwOBot 模型连接不可用，请检查桌面端模型设置",
                )
            })?;
        }
        Ok(self)
    }
}

type GatewayResult<T> = Result<T, GatewayError>;

enum GatewayError {
    Unavailable(&'static str),
    Refused(StatusCode, &'static str),
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Unavailable(reason) => (StatusCode::SERVICE_UNAVAILABLE, reason),
            Self::Refused(status, reason) => (status, reason),
        };
        (status, Json(json!({"error": message}))).into_response()
    }
}

pub(crate) struct Collaboration {
    home: Option<PathBuf>,
    client: reqwest::Client,
    browser_session: AuthToken,
    live: Mutex<Option<runtime::Running>>,
    delivery_gate: Arc<std::sync::Mutex<()>>,
}

impl Collaboration {
    pub(crate) fn new() -> Self {
        Self {
            home: std::env::var_os("OWOBOT_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute()),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(3))
                .build()
                .expect("loopback collaboration HTTP client"),
            browser_session: AuthToken::generate(),
            live: Mutex::new(None),
            delivery_gate: Arc::new(std::sync::Mutex::new(())),
        }
    }

    pub(crate) fn web_dir(&self) -> PathBuf {
        self.home
            .as_ref()
            .map(|home| home.join("dist"))
            .unwrap_or_else(|| crate::desktop_web_dir().join("owobot"))
    }

    pub(crate) async fn shutdown(&self) {
        let running = self.live.lock().await.take();
        if let Some(running) = running {
            running.stop().await;
        }
    }

    pub(crate) async fn bind_worker_turn(
        &self,
        headers: &HeaderMap,
        workspace: &FsPath,
        request: &mut TurnRequest,
    ) -> Result<Option<WorkerTurn>, (StatusCode, String)> {
        let denied = || (StatusCode::FORBIDDEN, "owobot/worker_scope".to_string());
        let Some(proof) = headers.get(WORKER_PROOF) else {
            return if headers.contains_key(TURN_PROOF) {
                Err(denied())
            } else {
                Ok(None)
            };
        };
        let proof = proof.to_str().map_err(|_| denied())?;
        let turn = headers
            .get(TURN_PROOF)
            .and_then(|value| value.to_str().ok())
            .filter(|token| token.len() == 48 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(denied)?;
        let mut live = self.live.lock().await;
        let active = live.as_mut().ok_or_else(denied)?;
        if !active.worker_proof.verify(proof) || !active.is_alive() {
            return Err(denied());
        }
        if workspace != active.workspace
            && workspace.canonicalize().ok().as_ref() != Some(&active.workspace)
        {
            return Err(denied());
        }
        if request.read_only == Some(false)
            || request.model_connection.is_some()
            || !request.attachments.is_empty()
            || !request.content_parts.is_empty()
        {
            return Err(denied());
        }

        let scope: CollaborationTurn = self
            .client
            .get(format!("{}/api/internal/owobot-scope", active.base_url))
            .bearer_auth(turn)
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| denied())?
            .json()
            .await
            .map_err(|_| denied())?;
        if scope.group_id != active.group || scope.depth > 3 || scope.thread_id.is_empty() {
            return Err(denied());
        }
        let role = active.role_for(&scope.bot_id).ok_or_else(denied)?;
        request.model_connection = active.model.connection.clone();
        let dispatcher = (role == WorkerRole::Lead
            && scope.depth < 3
            && request.read_only != Some(true))
        .then(|| {
            delegation::Delegate::new(
                self.client.clone(),
                active.base_url.clone(),
                turn.to_owned(),
                active.group.clone(),
                [
                    active.member(WorkerRole::Worker).to_owned(),
                    active.member(WorkerRole::Reviewer).to_owned(),
                ],
            )
        });
        Ok(Some(WorkerTurn {
            role,
            model: active.model.model.clone(),
            dispatcher,
            delivery: active.delivery.clone(),
            scope,
            readonly: request.read_only == Some(true),
        }))
    }

    fn accepts(&self, headers: &HeaderMap) -> bool {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|part| part.trim().split_once('='))
            .any(|(name, value)| name == COOKIE && self.browser_session.verify(value))
    }
}

pub(crate) async fn open(
    State(state): State<Arc<AppState>>,
    model: Option<Json<ModelSelection>>,
) -> Response {
    open_desktop(&state, model.map(|Json(model)| model).unwrap_or_default())
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn open_desktop(state: &AppState, model: ModelSelection) -> GatewayResult<Response> {
    let model = model.validate()?;
    let collaboration = &state.owobot;
    let home = collaboration
        .home
        .as_deref()
        .filter(|path| {
            path.join("node.exe").is_file() && path.join("dist-server/index.js").is_file()
        })
        .filter(|_| collaboration.web_dir().join("index.html").is_file())
        .ok_or(GatewayError::Unavailable("当前安装缺少 OwOBot 运行组件"))?;
    let mut live = collaboration.live.lock().await;
    let needs_start = match live.as_mut() {
        Some(running) => !running.is_alive(),
        None => true,
    };
    if needs_start {
        live.take();
        *live = Some(
            runtime::start(
                &collaboration.client,
                home,
                &state.data_root,
                &state.workspace,
                Arc::clone(&collaboration.delivery_gate),
            )
            .await?,
        );
    }
    let active = live.as_mut().expect("sidecar started");
    active.model = model;
    let group = &active.group;
    let cookie = format!(
        "{COOKIE}={}; Path=/owobot/api/; HttpOnly; SameSite=Strict",
        collaboration.browser_session.token(),
    );
    let mut response = Json(json!({"url": "/owobot/", "groupId": group})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("generated cookie is ASCII"),
    );
    Ok(response)
}

async fn browser_delivery(
    state: &AppState,
    method: &Method,
    headers: &HeaderMap,
) -> Result<delivery::Store, Response> {
    let collaboration = &state.owobot;
    if !collaboration.accepts(headers) {
        return Err(GatewayError::Refused(
            StatusCode::UNAUTHORIZED,
            "Please open OwOBot from the desktop",
        )
        .into_response());
    }
    if !same_origin(method, headers) {
        return Err(GatewayError::Refused(
            StatusCode::FORBIDDEN,
            "OwOBot delivery needs a same-origin desktop request",
        )
        .into_response());
    }
    let live = collaboration.live.lock().await;
    live.as_ref()
        .map(|running| running.delivery.clone())
        .ok_or_else(|| GatewayError::Unavailable("Please reopen OwOBot").into_response())
}

fn delivery_error(error: delivery::DeliveryError) -> Response {
    let status = match error {
        delivery::DeliveryError::Invalid(_) => StatusCode::BAD_REQUEST,
        delivery::DeliveryError::Conflict(_) => StatusCode::CONFLICT,
        delivery::DeliveryError::Missing => StatusCode::NOT_FOUND,
        delivery::DeliveryError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (status, Json(json!({"error": error.code()}))).into_response()
}

#[derive(Deserialize)]
pub(crate) struct DeliveryLookup {
    revision: Option<u64>,
}

pub(crate) async fn read_delivery(
    State(state): State<Arc<AppState>>,
    Path(thread): Path<String>,
    Query(lookup): Query<DeliveryLookup>,
    headers: HeaderMap,
) -> Response {
    let store = match browser_delivery(&state, &Method::GET, &headers).await {
        Ok(store) => store,
        Err(response) => return response,
    };
    match tokio::task::spawn_blocking(move || store.inspect(&thread, lookup.revision)).await {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(error)) => delivery_error(error),
        Err(_) => delivery_error(delivery::DeliveryError::Unavailable),
    }
}

pub(crate) async fn decide_delivery(
    State(state): State<Arc<AppState>>,
    Path(thread): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let store = match browser_delivery(&state, &parts.method, &parts.headers).await {
        Ok(store) => store,
        Err(response) => return response,
    };
    let Ok(bytes) = to_bytes(body, 4096).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let Ok(decision) = serde_json::from_slice::<delivery::DecisionRequest>(&bytes) else {
        return delivery_error(delivery::DeliveryError::Invalid(
            "invalid_delivery_decision",
        ));
    };
    let revision = decision.revision;
    let action = decision.action;
    let task_id = format!("owobot-{thread}");
    let lease = crate::workswarm_api::acquire_workspace_delivery_lease(&store.workspace).await;
    let result = tokio::task::spawn_blocking(move || {
        let _lease = lease;
        let result = store.decide(&thread, decision);
        if !matches!(&result, Ok(view) if view.replayed) {
            if let Ok(mut log) = state.agent.audit_log().lock() {
                log.record(
                    &task_id,
                    "owobot_delivery_decision",
                    None,
                    Some(result.is_ok()),
                    json!({"revision":revision,"action":action,
                        "error":result.as_ref().err().map(delivery::DeliveryError::code)})
                    .to_string(),
                );
            }
            crate::flush_audit(&state);
        }
        result
    })
    .await;
    match result {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(error)) => delivery_error(error),
        Err(_) => delivery_error(delivery::DeliveryError::Unavailable),
    }
}

pub(crate) async fn proxy(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    request: Request,
) -> Response {
    forward(&state.owobot, &path, request)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn forward(
    collaboration: &Collaboration,
    path: &str,
    request: Request,
) -> GatewayResult<Response> {
    let (parts, body) = request.into_parts();
    if !collaboration.accepts(&parts.headers) {
        return Err(GatewayError::Refused(
            StatusCode::UNAUTHORIZED,
            "请从 OwO 打开协作工作台",
        ));
    }
    if !same_origin(&parts.method, &parts.headers) {
        return Err(GatewayError::Refused(
            StatusCode::FORBIDDEN,
            "OwOBot 仅接受桌面工作台请求",
        ));
    }

    let (base_url, session) = {
        let mut live = collaboration.live.lock().await;
        let running = live
            .as_mut()
            .ok_or(GatewayError::Unavailable("请重新打开 OwOBot"))?;
        if !running.is_alive() {
            return Err(GatewayError::Unavailable("OwOBot 已停止，请重新打开"));
        }
        if !route_allowed(&parts.method, path, &running.group) {
            return Err(GatewayError::Refused(
                StatusCode::FORBIDDEN,
                "该操作不属于 OwOBot 协作入口",
            ));
        }
        (running.base_url.clone(), running.session.clone())
    };

    let query = parts
        .uri
        .query()
        .map(|value| format!("?{value}"))
        .unwrap_or_default();
    let mut upstream = collaboration
        .client
        .request(
            parts.method.clone(),
            format!("{base_url}/api/{path}{query}"),
        )
        .bearer_auth(&session);
    if let Some(value) = parts.headers.get("last-event-id") {
        upstream = upstream.header("last-event-id", value);
    }
    if parts.method == Method::POST {
        let bytes = to_bytes(body, 1024 * 1024)
            .await
            .map_err(|_| GatewayError::Refused(StatusCode::PAYLOAD_TOO_LARGE, "OwOBot 请求过大"))?;
        upstream = upstream
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes);
    }
    const HEAD_LIMIT: Duration = Duration::from_secs(16);
    let response = tokio::time::timeout(HEAD_LIMIT, upstream.send())
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or(GatewayError::Unavailable("OwOBot 本机服务未响应"))?;
    let status = response.status();
    if status.is_redirection() {
        return Err(GatewayError::Unavailable("OwOBot 返回了非预期跳转"));
    }
    let content_type = if path == "events" && status.is_success() {
        "text/event-stream"
    } else {
        "application/json"
    };
    let mut reply = Response::new(Body::from_stream(response.bytes_stream()));
    *reply.status_mut() = status;
    reply
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    reply.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(reply)
}

fn same_origin(method: &Method, headers: &HeaderMap) -> bool {
    if let Some(site) = headers.get("sec-fetch-site") {
        if site != "same-origin" {
            return false;
        }
    }
    match headers.get(header::ORIGIN) {
        None => method == Method::GET,
        Some(origin) => headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok())
            .is_some_and(|host| origin == format!("http://{host}").as_str()),
    }
}

fn route_allowed(method: &Method, path: &str, group: &str) -> bool {
    let segments: Vec<_> = path.split('/').collect();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || segment.len() > 128
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }) {
        return false;
    }

    match (method, segments.as_slice()) {
        (&Method::GET, ["bots" | "events" | "instances" | "search"]) => true,
        (&Method::GET, ["threads", _, "messages"]) => true,
        (&Method::GET, ["groups", id]) => *id == group,
        (&Method::POST, ["groups", id, "messages" | "read" | "interrupt" | "tasks"]) => {
            *id == group
        }
        (&Method::POST, ["groups", id, "tasks", _]) => *id == group,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_gateway_only_exposes_project_chat_surface() {
        assert!(route_allowed(
            &Method::POST,
            "groups/current/messages",
            "current"
        ));
        assert!(route_allowed(&Method::GET, "events", "current"));
        assert!(route_allowed(
            &Method::GET,
            "threads/abc/messages",
            "current"
        ));
        assert!(!route_allowed(
            &Method::POST,
            "groups/other/messages",
            "current"
        ));
        assert!(!route_allowed(&Method::GET, "groups/other", "current"));
        assert!(!route_allowed(&Method::POST, "auth/pairing", "current"));
        assert!(!route_allowed(&Method::GET, "config", "current"));
        assert!(!route_allowed(
            &Method::GET,
            "threads/../messages",
            "current"
        ));
        assert!(!route_allowed(
            &Method::GET,
            "threads/%2e%2e/messages",
            "current"
        ));
    }

    #[test]
    fn cookies_and_origin_are_bound_to_the_collaboration_gateway() {
        let collaboration = Collaboration::new();
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:4096"));
        assert!(!collaboration.accepts(&headers));
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!(
                "another=123; {COOKIE}={}",
                collaboration.browser_session.token(),
            ))
            .unwrap(),
        );
        assert!(collaboration.accepts(&headers));
        assert!(same_origin(&Method::GET, &headers));
        assert!(!same_origin(&Method::POST, &headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:4097"),
        );
        assert!(!same_origin(&Method::GET, &headers));
        assert!(!same_origin(&Method::POST, &headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:4096"),
        );
        assert!(same_origin(&Method::POST, &headers));
    }
}
