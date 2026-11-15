//! OwO's local gateway to the managed collaboration process.

pub(crate) mod delegation;
mod runtime;

use crate::{auth_token::AuthToken, AppState};
use axum::body::{to_bytes, Body};
use axum::extract::{Path, Request, State};
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

    pub(crate) fn instructions(self) -> &'static str {
        match self {
            Self::Lead => "理解任务，查阅当前项目信息。在可用时使用 owobot_delegate 交给执行员，收到结果后请评审员独立核对，最后汇总。",
            Self::Worker => "查阅当前项目材料，提供执行结果与交付建议。",
            Self::Reviewer => "独立核对任务要求、项目材料和执行结果，记录评审意见。",
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
    pub(crate) dispatcher: Option<delegation::Delegate>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollaborationTurn {
    bot_id: String,
    group_id: String,
    thread_id: String,
    depth: u32,
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
