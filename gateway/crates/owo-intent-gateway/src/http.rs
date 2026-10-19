//! agent-server HTTP 回环客户端（Bearer 鉴权 + 401 刷新重试一次）。
//!
//! - token 来源：Daemon 写入的 `<data_root>/auth/token`（用户级 ACL），懒加载 + 缓存；
//!   不调用公开的 `GET /auth/token`（任何本机进程都能调用，不是本用户凭据边界）；
//! - 401 → 重读 token 文件一次（引擎重启轮换 token 场景）；
//! - turn 端点是长流：**不设全局请求超时**，仅连接超时 5s。

use std::path::{Path, PathBuf};
use std::time::Duration;

use owo_agent_protocol::{CreateSessionRequest, FileDiff, SessionInfo, TurnRequest};
use reqwest::RequestBuilder;

/// 普通请求超时（非流式端点）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// 连接超时。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 错误正文截断长度。
const ERROR_BODY_LIMIT: usize = 500;

/// HTTP 客户端错误。
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("HTTP 请求失败：{0}")]
    Http(#[from] reqwest::Error),
    #[error("agent-server 返回 {status}：{body}")]
    Status { status: u16, body: String },
    #[error("响应解析失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("协议异常：{0}")]
    Protocol(String),
    #[error("读取 token 失败：{0}")]
    Token(String),
}

/// 解析 Agent 数据根：`OWO_AGENT_DATA` → `%LOCALAPPDATA%\OwO\Agent` → `data/agent`
/// （与 `owo-agent-client::discovery::resolve_data_root` 同一约定）。
pub fn resolve_data_root() -> PathBuf {
    std::env::var("OWO_AGENT_DATA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("LOCALAPPDATA")
                .ok()
                .map(|dir| PathBuf::from(dir).join("OwO").join("Agent"))
        })
        .unwrap_or_else(|| PathBuf::from("data/agent"))
}

/// token 文件路径：`<data_root>/auth/token`。
pub fn token_path(data_root: &Path) -> PathBuf {
    data_root.join("auth").join("token")
}

/// 网关会话专用的空工作区：`<data_root>/gateway/workspace`（不指向用户项目）。
pub fn gateway_workspace(data_root: &Path) -> PathBuf {
    data_root.join("gateway").join("workspace")
}

/// 从 `<data_root>/runtime/daemon.json` 读出回环地址 `http://127.0.0.1:<port>`。
pub fn daemon_base_url(data_root: &Path) -> Result<String, BridgeError> {
    let path = data_root.join("runtime").join("daemon.json");
    let bytes = std::fs::read(&path)
        .map_err(|error| BridgeError::Protocol(format!("{}（{error}）", path.display())))?;
    let descriptor: owo_agent_protocol::DaemonDescriptor = serde_json::from_slice(&bytes)?;
    if descriptor.port == 0 {
        return Err(BridgeError::Protocol(format!(
            "{} 端口为 0",
            path.display()
        )));
    }
    Ok(format!("http://127.0.0.1:{}", descriptor.port))
}

fn read_token_file(path: &Path) -> Result<String, BridgeError> {
    let token = std::fs::read_to_string(path)
        .map_err(|error| BridgeError::Token(format!("{}（{error}）", path.display())))?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(BridgeError::Token(format!("{} 为空", path.display())));
    }
    Ok(token)
}

/// Validate the parsed authority before reading credentials or making any request.
/// Pin localhost to IPv4 loopback rather than trusting local DNS/hosts resolution.
fn validate_daemon_url(value: &str) -> Result<String, BridgeError> {
    let invalid =
        || BridgeError::Protocol("Daemon URL must be a plain HTTP loopback origin".into());
    let mut url = reqwest::Url::parse(value).map_err(|_| invalid())?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
        || url.port_or_known_default() == Some(0)
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    url.set_host(Some("127.0.0.1")).map_err(|_| invalid())?;
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// 回环 HTTP 客户端。
pub struct OwoHttpClient {
    http: reqwest::Client,
    base_url: String,
    token_file: PathBuf,
    token: tokio::sync::RwLock<Option<String>>,
}

impl OwoHttpClient {
    /// `token_file` 一般为 [`token_path`]`(data_root)`。
    pub fn new(
        base_url: impl Into<String>,
        token_file: impl Into<PathBuf>,
    ) -> Result<Self, BridgeError> {
        let base_url = validate_daemon_url(&base_url.into())?;
        let http = reqwest::Client::builder()
            // Daemon traffic and bearer credentials must never traverse an HTTP proxy.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            // 不设全局 timeout：turn 端点是分钟级 SSE 长流。
            .build()
            .map_err(BridgeError::Http)?;
        Ok(Self {
            http,
            base_url,
            token_file: token_file.into(),
            token: tokio::sync::RwLock::new(None),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // ────────────────────────── 鉴权 ──────────────────────────

    async fn bootstrap_token(&self) -> Result<String, BridgeError> {
        let path = self.token_file.clone();
        tokio::task::spawn_blocking(move || read_token_file(&path))
            .await
            .map_err(|error| BridgeError::Token(error.to_string()))?
    }

    async fn auth_token(&self) -> Result<String, BridgeError> {
        if let Some(token) = self.token.read().await.clone() {
            return Ok(token);
        }
        let token = self.bootstrap_token().await?;
        *self.token.write().await = Some(token.clone());
        Ok(token)
    }

    async fn refresh_token(&self) -> Result<String, BridgeError> {
        let token = self.bootstrap_token().await?;
        *self.token.write().await = Some(token.clone());
        Ok(token)
    }

    /// 带鉴权请求；401 时刷新 token 重试一次。
    async fn authed<F>(&self, build: F) -> Result<reqwest::Response, BridgeError>
    where
        F: Fn(&reqwest::Client, &str) -> RequestBuilder,
    {
        let token = self.auth_token().await?;
        let response = build(&self.http, &token).send().await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let token = self.refresh_token().await?;
            return Ok(build(&self.http, &token).send().await?);
        }
        Ok(response)
    }

    // ────────────────────────── 健康与会话 ──────────────────────────

    /// `GET /health`（公开端点）。
    pub async fn health(&self) -> Result<serde_json::Value, BridgeError> {
        let response = self
            .http
            .get(format!("{}/health", self.base_url))
            .timeout(Duration::from_secs(3))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(response.json().await?)
    }

    /// `POST /session`。
    pub async fn create_session(&self, workspace: &str) -> Result<SessionInfo, BridgeError> {
        let body = CreateSessionRequest {
            workspace: workspace.to_string(),
            model: None,
            system_prompt: None,
        };
        let response = self
            .authed(|http, token| {
                http.post(format!("{}/session", self.base_url))
                    .bearer_auth(token)
                    .timeout(REQUEST_TIMEOUT)
                    .json(&body)
            })
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(response.json().await?)
    }

    // ────────────────────────── 回合 ──────────────────────────

    pub async fn expression(&self, prompt: &str) -> Result<String, BridgeError> {
        let body = owo_agent_protocol::ExpressionRequest {
            prompt: prompt.into(),
        };
        let response = self
            .authed(|http, token| {
                http.post(format!("{}/expression", self.base_url))
                    .bearer_auth(token)
                    .timeout(Duration::from_secs(65))
                    .json(&body)
            })
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 32 * 1024 {
                return Err(BridgeError::Protocol("expression output too large".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let result: owo_agent_protocol::ExpressionResponse = serde_json::from_slice(&bytes)?;
        if result.text.trim().is_empty() || result.text.chars().count() > 4096 {
            return Err(BridgeError::Protocol("invalid expression output".into()));
        }
        Ok(result.text)
    }

    /// `POST /session/{id}/turn` → SSE 长流响应（不设总超时）。
    ///
    /// `read_only = true` 时宿主策略层只放行只读工具，且不能被会话权限放宽。
    pub async fn turn_stream(
        &self,
        session_id: &str,
        prompt: &str,
        read_only: bool,
    ) -> Result<reqwest::Response, BridgeError> {
        let body = TurnRequest {
            model_connection: None,
            prompt: prompt.to_string(),
            attachments: Vec::new(),
            read_only: Some(read_only),
            turn_id: None,
        };
        let response = self
            .authed(|http, token| {
                http.post(format!("{}/session/{session_id}/turn", self.base_url))
                    .bearer_auth(token)
                    .header("accept", "text/event-stream")
                    .json(&body)
            })
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(response)
    }

    /// `POST /session/{id}/abort`。
    pub async fn abort(&self, session_id: &str) -> Result<(), BridgeError> {
        let response = self
            .authed(|http, token| {
                http.post(format!("{}/session/{session_id}/abort", self.base_url))
                    .bearer_auth(token)
                    .timeout(REQUEST_TIMEOUT)
            })
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(())
    }

    /// `POST /session/{id}/permission/{rid}`。
    pub async fn respond_permission(
        &self,
        session_id: &str,
        request_id: &str,
        allow: bool,
        remember: bool,
    ) -> Result<(), BridgeError> {
        let response = self
            .authed(|http, token| {
                http.post(format!(
                    "{}/session/{session_id}/permission/{request_id}",
                    self.base_url
                ))
                .bearer_auth(token)
                .timeout(REQUEST_TIMEOUT)
                .json(&serde_json::json!({ "allow": allow, "remember": remember }))
            })
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(())
    }

    // ────────────────────────── 改动审阅 ──────────────────────────

    /// `GET /session/{id}/diff`。
    pub async fn diff(&self, session_id: &str) -> Result<Vec<FileDiff>, BridgeError> {
        let response = self
            .authed(|http, token| {
                http.get(format!("{}/session/{session_id}/diff", self.base_url))
                    .bearer_auth(token)
                    .timeout(REQUEST_TIMEOUT)
            })
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(response.json().await?)
    }

    /// `POST /session/{id}/revert`。
    pub async fn revert(&self, session_id: &str) -> Result<(), BridgeError> {
        let response = self
            .authed(|http, token| {
                http.post(format!("{}/session/{session_id}/revert", self.base_url))
                    .bearer_auth(token)
                    .timeout(REQUEST_TIMEOUT)
            })
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        Ok(())
    }
}

async fn status_error(response: reqwest::Response) -> BridgeError {
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(ERROR_BODY_LIMIT)
        .collect();
    BridgeError::Status { status, body }
}

#[cfg(test)]
mod origin_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn rejects_authority_bypass_and_non_origin_urls_without_echoing_input() {
        for value in [
            "http://localhost:80@evil.example",
            "http://127.0.0.1:80@evil.example",
            "http://user:fixture-secret@localhost:9000",
            "http://localhost.evil.example:9000",
            "https://127.0.0.1:9000",
            "http://127.0.0.1:0",
            "http://127.0.0.1:9000/expression",
            "http://127.0.0.1:9000/?token=fixture-secret",
            "http://127.0.0.1:9000/#fixture-secret",
            "not a URL",
        ] {
            let error = OwoHttpClient::new(value, "unused")
                .err()
                .expect("must reject");
            assert!(!error.to_string().contains("fixture-secret"));
        }
        assert_eq!(
            reqwest::Url::parse("http://localhost:80@evil.example")
                .unwrap()
                .host_str(),
            Some("evil.example")
        );
    }

    #[test]
    fn accepts_origin_and_pins_localhost_without_dns() {
        for (value, expected) in [
            ("http://localhost:9000/", "http://127.0.0.1:9000"),
            ("http://127.0.0.1:9000", "http://127.0.0.1:9000"),
            ("http://localhost", "http://127.0.0.1"),
        ] {
            assert_eq!(
                OwoHttpClient::new(value, "unused").unwrap().base_url(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn redirects_never_replay_expression_even_on_same_origin() {
        use axum::{
            http::{header::LOCATION, StatusCode},
            routing::any,
            Router,
        };
        for status in [301, 302, 303, 307, 308] {
            let sink = Arc::new(AtomicUsize::new(0));
            let sink_count = sink.clone();
            let app = Router::new()
                .route(
                    "/expression",
                    any(move || async move {
                        (StatusCode::from_u16(status).unwrap(), [(LOCATION, "/sink")])
                    }),
                )
                .route(
                    "/sink",
                    any(move || {
                        let count = sink_count.clone();
                        async move {
                            count.fetch_add(1, Ordering::SeqCst);
                            StatusCode::INTERNAL_SERVER_ERROR
                        }
                    }),
                );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = OwoHttpClient::new(
                format!("http://{}", listener.local_addr().unwrap()),
                "unused",
            )
            .unwrap();
            // Synthetic credential only; no token file or model is accessed.
            *client.token.write().await = Some("synthetic-origin-test".into());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let result = client.expression("synthetic expression").await;
            server.abort();
            assert!(
                matches!(result, Err(BridgeError::Status { status: actual, .. }) if actual == status)
            );
            assert_eq!(sink.load(Ordering::SeqCst), 0);
        }
    }
}
