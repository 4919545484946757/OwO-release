//! `AgentClient`：认证头 + JSON 往返 + 结构化错误的核心 HTTP 门面。

use crate::error::{ClientError, Result};
use owo_agent_protocol::DaemonDescriptor;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;

/// 客户端连接配置。
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub base_url: String,
    pub token: Option<String>,
    /// 单次请求超时（不含 SSE 流本身）。
    pub timeout: Duration,
}

impl ClientConfig {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token,
            timeout: Duration::from_secs(30),
        }
    }
}

/// 唯一 Daemon 客户端（CLI/TUI/桌面共用同一实现）。
#[derive(Debug, Clone)]
pub struct AgentClient {
    http: reqwest::Client,
    stream_http: reqwest::Client,
    base_url: String,
    token: Option<String>,
    descriptor: Option<DaemonDescriptor>,
}

impl AgentClient {
    pub fn new(config: ClientConfig) -> Result<Self> {
        let local = reqwest::Url::parse(&config.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            });
        let mut http_builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(config.timeout);
        let mut stream_builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(5));
        if local {
            http_builder = http_builder.no_proxy();
            stream_builder = stream_builder.no_proxy();
        }
        let http = http_builder
            .build()
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        // Long-running bodies have no total timeout. Header sends remain bounded separately.
        let stream_http = stream_builder
            .build()
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Ok(Self {
            http,
            stream_http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            token: config.token,
            descriptor: None,
        })
    }

    /// 附带发现描述符（诊断/展示用）。
    pub fn with_descriptor(mut self, descriptor: DaemonDescriptor) -> Self {
        self.descriptor = Some(descriptor);
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn descriptor(&self) -> Option<&DaemonDescriptor> {
        self.descriptor.as_ref()
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let builder = self.http.request(method, url);
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    fn stream_request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let builder = self.stream_http.request(method, url);
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self
            .request(reqwest::Method::GET, path)
            .send()
            .await
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Self::decode(response).await
    }

    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = self
            .request(reqwest::Method::POST, path)
            .json(body)
            .send()
            .await
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Self::decode(response).await
    }

    /// POST 一个空对象（服务端无 body 提取器的端点，如 abort）。
    pub async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self
            .request(reqwest::Method::POST, path)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Self::decode(response).await
    }

    /// DELETE 并解码响应（资源删除类端点，如用户工作流/草稿）。
    pub async fn delete_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self
            .request(reqwest::Method::DELETE, path)
            .send()
            .await
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Self::decode(response).await
    }

    /// 发起长期 GET 流（如团队事件 SSE）；关闭请求级超时，由调用方控制生命周期。
    pub(crate) fn get_stream(&self, path: &str) -> reqwest::RequestBuilder {
        self.stream_request(reqwest::Method::GET, path)
    }

    /// 发起流式请求（SSE）：成功返回未读取 body 的 `Response`，失败读回错误体。
    pub(crate) fn post_stream<B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> reqwest::RequestBuilder {
        self.stream_request(reqwest::Method::POST, path).json(body)
    }

    pub(crate) async fn send_stream(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        let response = tokio::time::timeout(Duration::from_secs(30), request.send())
            .await
            .map_err(|_| ClientError::Transport("client/response_header_timeout".into()))?
            .map_err(|error| ClientError::Transport(error.without_url().to_string()))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(ClientError::Status { status, body });
        }
        Ok(response)
    }

    async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(ClientError::Status {
                status: status.as_u16(),
                body: text,
            });
        }
        serde_json::from_str(&text)
            .map_err(|error| ClientError::Protocol(format!("{error}：{text}")))
    }
}
