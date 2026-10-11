//! 传输后端抽象：CloudTransport + Mock/HTTP 实现与凭据读取（从 cloud_exec.rs 拆出）。

use super::*;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

// ============================================================================
// v0.2：传输后端抽象（CloudTransport）+ 任务队列/状态机/持久化 + 进度事件
// ============================================================================
// 全链路契约：仓库快照 → 隔离执行 → 进度事件 → diff 回传 → 审阅/revert。
// 传输后端二选一：MockRemoteTransport（不联网，测试/本地冒烟）与 HttpTransport
// （HTTP 远端；协议契约见下，供主控后续在 server 侧接入）。
//
// HTTP 远端协议契约（POST/GET 均为 application/json）：
//   POST  {base}/cloud/tasks            body: CloudTaskSpec → { "id": "<remote_id>" }
//   GET   {base}/cloud/tasks/{id}       → { "state": "queued|running|succeeded|failed|canceled", "error"?: string }
//   GET   {base}/cloud/tasks/{id}/result→ CloudTaskResult
//   POST  {base}/cloud/tasks/{id}/cancel→ { "ok": true }
// 凭据：仅经环境变量 OWO_CLOUD_TOKEN / OWO_CLOUD_API_KEY 读取，放入请求头
// Authorization: Bearer <token>；任何结构体/持久化文件不存储凭据。

/// 远端任务状态（传输层视角）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteStatus {
    Queued,
    Running,
    Succeeded,
    Failed(String),
    Canceled,
}

/// 传输后端抽象：submit → status → fetch_result → cancel。
#[async_trait::async_trait]
pub trait CloudTransport: Send + Sync {
    fn kind(&self) -> &'static str;
    /// 提交任务到远端，返回远端句柄（remote_id）。
    async fn submit(&self, spec: &CloudTaskSpec) -> Result<String, String>;
    async fn status(&self, remote_id: &str) -> Result<RemoteStatus, String>;
    async fn fetch_result(&self, remote_id: &str) -> Result<CloudTaskResult, String>;
    async fn cancel(&self, remote_id: &str) -> Result<(), String>;
}

/// 从环境变量读取远端凭据（OWO_CLOUD_TOKEN 优先，回退 OWO_CLOUD_API_KEY）。
/// 只读进请求头，绝不落盘。
pub fn cloud_token_from_env() -> Option<String> {
    std::env::var("OWO_CLOUD_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::env::var("OWO_CLOUD_API_KEY")
                .ok()
                .filter(|v| !v.is_empty())
        })
}

/// 不联网的远端替身：本地临时目录充当远端工作区，复用 v0.1 的隔离执行逻辑。
/// 语义：submit 只登记；fetch_result 时才实际执行（模拟远端异步，延迟可控）。
pub struct MockRemoteTransport {
    executor: tokio::sync::Mutex<LocalSimExecutor>,
}

impl MockRemoteTransport {
    pub fn new(scratch_root: PathBuf) -> Self {
        Self {
            executor: tokio::sync::Mutex::new(LocalSimExecutor::new(scratch_root)),
        }
    }
}

#[async_trait::async_trait]
impl CloudTransport for MockRemoteTransport {
    fn kind(&self) -> &'static str {
        "mock"
    }

    async fn submit(&self, spec: &CloudTaskSpec) -> Result<String, String> {
        let mut executor = self.executor.lock().await;
        let remote_id = executor.submit(spec.clone())?;
        // 模拟远端异步执行：提交即执行完毕，result 就绪（status 即可见终态）。
        executor.run(&remote_id).await?;
        Ok(remote_id)
    }

    async fn status(&self, remote_id: &str) -> Result<RemoteStatus, String> {
        let executor = self.executor.lock().await;
        let task = executor
            .tasks
            .get(remote_id)
            .ok_or_else(|| format!("远端任务不存在：{remote_id}"))?;
        Ok(match &task.result {
            None => RemoteStatus::Running,
            Some(r) if r.exit_code == Some(0) => RemoteStatus::Succeeded,
            Some(_) => RemoteStatus::Failed("非零退出码".to_string()),
        })
    }

    async fn fetch_result(&self, remote_id: &str) -> Result<CloudTaskResult, String> {
        let executor = self.executor.lock().await;
        let task = executor
            .tasks
            .get(remote_id)
            .ok_or_else(|| format!("远端任务不存在：{remote_id}"))?;
        task.result
            .clone()
            .ok_or_else(|| format!("远端任务尚无结果：{remote_id}"))
    }

    async fn cancel(&self, remote_id: &str) -> Result<(), String> {
        let mut executor = self.executor.lock().await;
        executor
            .tasks
            .remove(remote_id)
            .map(|mut task| {
                if let Some(temp_dir) = task.temp_dir.take() {
                    let _ = std::fs::remove_dir_all(&temp_dir);
                }
            })
            .ok_or_else(|| format!("远端任务不存在：{remote_id}"))
    }
}

/// HTTP 远端传输（协议契约见模块头注释）。凭据只经请求头，不存储。
pub struct HttpTransport {
    base_url: String,
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new(base_url: String) -> Result<Self, String> {
        // reqwest 内置 default-tls，http/https 均可用（https 由 reqwest 完成 TLS）。
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(format!(
                "base_url 必须以 http:// 或 https:// 开头：{base_url}"
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| format!("HTTP 客户端初始化失败：{e}"))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&CloudTaskSpec>,
    ) -> Result<reqwest::Response, String> {
        let url = format!("{}{}", self.base_url, path);
        let mut builder = self.client.request(
            reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?,
            &url,
        );
        if let Some(token) = cloud_token_from_env() {
            builder = builder.bearer_auth(token);
        }
        let response = match body {
            Some(spec) => builder
                .json(spec)
                .send()
                .await
                .map_err(|e| format!("HTTP {method} {url} 失败（请检查远端地址/网络）：{e}"))?,
            None => builder
                .send()
                .await
                .map_err(|e| format!("HTTP {method} {url} 失败（请检查远端地址/网络）：{e}"))?,
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(format!("HTTP {method} {url} 返回 {status}"));
        }
        Ok(response)
    }
}

/// 远端响应体上限：不受信远端可能返回超大 body，先流式读取并在上限处截断。
const MAX_REMOTE_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

async fn read_json_capped<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    context: &str,
) -> Result<T, String> {
    let mut bytes: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("{context}读取失败：{e}"))?;
        if bytes.len() + chunk.len() > MAX_REMOTE_RESPONSE_BYTES {
            return Err(format!(
                "{context}超过上限（{} MiB），已拒绝解析",
                MAX_REMOTE_RESPONSE_BYTES / (1024 * 1024)
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("{context}解析失败：{e}"))
}

#[async_trait::async_trait]
impl CloudTransport for HttpTransport {
    fn kind(&self) -> &'static str {
        "http"
    }

    async fn submit(&self, spec: &CloudTaskSpec) -> Result<String, String> {
        let response = self.call("POST", "/cloud/tasks", Some(spec)).await?;
        let value: serde_json::Value = read_json_capped(response, "远端响应").await?;
        value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("远端响应缺少 id：{value}"))
    }

    async fn status(&self, remote_id: &str) -> Result<RemoteStatus, String> {
        let response = self
            .call("GET", &format!("/cloud/tasks/{remote_id}"), None)
            .await?;
        let value: serde_json::Value = read_json_capped(response, "远端响应").await?;
        let state = value
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        Ok(match state {
            "queued" => RemoteStatus::Queued,
            "running" => RemoteStatus::Running,
            "succeeded" => RemoteStatus::Succeeded,
            "canceled" => RemoteStatus::Canceled,
            "failed" => RemoteStatus::Failed(
                value
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("远端执行失败")
                    .to_string(),
            ),
            other => return Err(format!("远端返回未知状态：{other}")),
        })
    }

    async fn fetch_result(&self, remote_id: &str) -> Result<CloudTaskResult, String> {
        let response = self
            .call("GET", &format!("/cloud/tasks/{remote_id}/result"), None)
            .await?;
        read_json_capped(response, "远端结果").await
    }

    async fn cancel(&self, remote_id: &str) -> Result<(), String> {
        let response = self
            .call("POST", &format!("/cloud/tasks/{remote_id}/cancel"), None)
            .await?;
        let _: serde_json::Value = read_json_capped(response, "远端响应").await?;
        Ok(())
    }
}
