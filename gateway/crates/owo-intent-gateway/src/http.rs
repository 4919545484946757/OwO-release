//! agent-server HTTP 回环客户端（Bearer 鉴权 + 401 刷新重试一次）。
//!
//! 契约与 LingXi `crates/owo-bridge` 同源（openapi.json 冻结）：
//! - `GET /auth/token` → `{token}`；token 懒加载 + 缓存；
//! - 401 → 重新引导刷新一次（引擎重启轮换 token 场景）；
//! - turn 端点是长流：**不设全局请求超时**，仅连接超时 5s。

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
}

/// 回环 HTTP 客户端。
pub struct OwoHttpClient {
    http: reqwest::Client,
    base_url: String,
    token: tokio::sync::RwLock<Option<String>>,
}

impl OwoHttpClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            // 不设全局 timeout：turn 端点是分钟级 SSE 长流。
            .build()
            .expect("reqwest client 构造失败");
        Self {
            http,
            base_url: base_url.into(),
            token: tokio::sync::RwLock::new(None),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // ────────────────────────── 鉴权 ──────────────────────────

    async fn bootstrap_token(&self) -> Result<String, BridgeError> {
        let response = self
            .http
            .get(format!("{}/auth/token", self.base_url))
            .timeout(Duration::from_secs(5))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        let value: serde_json::Value = response.json().await?;
        value
            .get("token")
            .and_then(serde_json::Value::as_str)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .ok_or_else(|| BridgeError::Protocol("auth/token 响应缺少非空 token 字段".to_string()))
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

    /// `POST /session/{id}/turn` → SSE 长流响应（不设总超时）。
    pub async fn turn_stream(
        &self,
        session_id: &str,
        prompt: &str,
    ) -> Result<reqwest::Response, BridgeError> {
        let body = TurnRequest {
            model_connection: None,
            prompt: prompt.to_string(),
            attachments: Vec::new(),
            read_only: None,
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
