//! OwO's local gateway to the managed collaboration process.

mod runtime;

use crate::{auth_token::AuthToken, AppState};
use axum::body::{to_bytes, Body};
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const COOKIE: &str = "owo_collaboration";

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

pub(crate) async fn open(State(state): State<Arc<AppState>>) -> Response {
    open_desktop(&state)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn open_desktop(state: &AppState) -> GatewayResult<Response> {
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
    let group = &live.as_ref().expect("sidecar started").group;
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
