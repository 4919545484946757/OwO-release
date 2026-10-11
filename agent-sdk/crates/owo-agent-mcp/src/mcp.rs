//! MCP（Model Context Protocol）客户端：stdio 与 HTTP 双传输，JSON-RPC 2.0。

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

// McpServerConfig 属插件域（M7 外迁）；这里 `pub use` 使
// `owo_agent_core::mcp::McpServerConfig` 路径继续有效（mcp_tests 等仍用它）。
pub use owo_agent_plugins::McpServerConfig;

use owo_agent_tool_safety::{
    default_manager as default_sandbox_manager, FileScope, IsolationLevel, JobGuard, NetworkPolicy,
    SandboxCommand, SandboxPolicy,
};

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// URL host 提取（去协议/路径/端口，小写）。
fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

/// stdio 超时来源：配置 > 环境变量 > 默认 15 秒。
fn stdio_timeout(config: &McpServerConfig) -> Duration {
    if let Some(ms) = config.timeout_ms {
        return Duration::from_millis(ms);
    }
    if let Ok(ms) = std::env::var("OWO_MCP_STDIO_TIMEOUT_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            if ms > 0 {
                return Duration::from_millis(ms);
            }
        }
    }
    Duration::from_secs(15)
}

const TIMEOUT_ERROR_PREFIX: &str = "MCP_TIMEOUT:";

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// §7.2：MCP annotations 原文（readOnlyHint/destructiveHint 等）；未声明为 None。
    pub annotations: Option<Value>,
}

/// A2-2：MCP 资源（resources/list 条目）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "mimeType")]
    pub mime_type: Option<String>,
}

/// A2-2：MCP 提示模板（prompts/list 条目）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpPrompt {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 模板参数声明（[{name, description?, required?}]，原样透传给模型）。
    #[serde(default)]
    pub arguments: Vec<Value>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    pending: Pending,
    next_id: AtomicU64,
    /// 受限 Job 守卫：drop 时终止 job 内进程（防孤儿；与 kill_on_drop 双保险）。
    _job_guard: Option<JobGuard>,
}

enum Transport {
    Stdio(Box<StdioTransport>),
    Http {
        client: reqwest::Client,
        url: String,
        headers: HeaderMap,
        next_id: AtomicU64,
    },
}

pub struct McpClient {
    transport: Transport,
    tools: Vec<McpTool>,
    /// A2-2：resources/list 缓存（连接时按能力协商拉取）。
    resources: Vec<McpResource>,
    /// A2-2：prompts/list 缓存。
    prompts: Vec<McpPrompt>,
    config: McpServerConfig,
}

impl McpClient {
    /// 启动/连接服务器并完成握手（initialize → initialized → tools/list）。
    pub async fn connect(config: &McpServerConfig) -> Result<Self, String> {
        Self::connect_inner(config.clone()).await
    }

    async fn connect_inner(config: McpServerConfig) -> Result<Self, String> {
        let transport = if config.transport == "http" {
            let url = config
                .url
                .clone()
                .ok_or_else(|| "HTTP MCP 服务器缺少 url".to_string())?;
            // 静态扫描 allowlist（R9）：network_allowlist 非空时 URL host 必须命中。
            if !config.network_allowlist.is_empty() {
                let host =
                    url_host(&url).ok_or_else(|| format!("HTTP MCP URL 无法解析域名：{url}"))?;
                let allowed = config.network_allowlist.iter().any(|entry| {
                    let entry_host =
                        if entry.starts_with("http://") || entry.starts_with("https://") {
                            url_host(entry).unwrap_or_default()
                        } else {
                            entry.split(':').next().unwrap_or(entry).to_lowercase()
                        };
                    entry_host == host
                });
                if !allowed {
                    return Err(format!(
                        "HTTP MCP 服务器 {host} 不在网络白名单（{}），拒绝连接",
                        config.network_allowlist.join("、")
                    ));
                }
            }
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|error| format!("HTTP 客户端创建失败：{error}"))?;
            let mut headers = HeaderMap::new();
            headers.insert(
                ACCEPT,
                HeaderValue::from_static("application/json, text/event-stream"),
            );
            Transport::Http {
                client,
                url,
                headers,
                next_id: AtomicU64::new(1),
            }
        } else {
            // 沙箱门卫（X01）：stdio 子进程统一经 SandboxManager 授权；
            // 策略：只读系统作用域 + 回环网络 + Job 级隔离（可显式降级，审计记录）。
            let policy = SandboxPolicy {
                name: format!("mcp:{}", config.name),
                file_scope: FileScope::WorkspacePlusReadonlySystem,
                network_policy: NetworkPolicy::Loopback,
                cpu_ms: None,
                mem_mb: Some(1024),
                ttl_secs: None,
                require_isolation: IsolationLevel::JobOnly,
                allow_degraded: true,
                // MCP 服务器可能需要子进程（如插件调 powershell/node），放宽进程数上限。
                active_process_limit: Some(32),
                ..SandboxPolicy::default()
            };
            let sandbox_command =
                SandboxCommand::new(&config.command, policy.clone()).with_args(config.args.clone());
            let manager = default_sandbox_manager();
            {
                let mut manager = manager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                manager.guard(&sandbox_command).map_err(|error| {
                    format!("MCP 服务器被沙箱拒绝（{}）：{error}", config.command)
                })?;
            }

            let mut command = Command::new(&config.command);
            command
                .args(&config.args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit())
                // 子进程随客户端释放（客户端被 drop/关机时不留孤儿进程）。
                .kill_on_drop(true);
            let mut child = command
                .spawn()
                .map_err(|error| format!("MCP 服务器启动失败（{}）：{error}", config.command))?;
            // 启动后挂入受限 Job；挂接失败 = 显式拒绝（终止进程，不留非受限子进程）。
            let mut job_guard: Option<JobGuard> = None;
            if let Some(pid) = child.id() {
                // attach 在同步块内完成（guard 不跨 await，保证 future Send）。
                let attach_result = {
                    let mut manager = manager
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    manager.attach_pid(&policy, pid)
                };
                match attach_result {
                    Ok(guard) => job_guard = Some(guard),
                    Err(error) => {
                        let _ = child.kill().await;
                        let _wait_status = child.wait().await;
                        return Err(format!(
                            "MCP 服务器无法挂入沙箱 Job（{}）：{error}",
                            config.command
                        ));
                    }
                }
            }
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| "MCP 服务器无 stdin".to_string())?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| "MCP 服务器无 stdout".to_string())?;

            let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
            let reader_pending = Arc::clone(&pending);
            tokio::spawn(async move {
                // 有界按行读取：不受信 MCP 服务器可能不发换行地推送超长数据，
                // `lines()` 会整行分配导致内存无界增长。这里逐字节累积并在超限后
                // 丢弃到下一个换行，只保留有界内存。
                const MAX_MCP_LINE_BYTES: usize = 16 * 1024 * 1024;
                let mut reader = BufReader::new(stdout);
                let mut line: Vec<u8> = Vec::new();
                let mut dropping = false;
                let mut chunk = [0u8; 8192];
                loop {
                    let read = match AsyncReadExt::read(&mut reader, &mut chunk).await {
                        Ok(0) => break,
                        Ok(read) => read,
                        Err(_) => break,
                    };
                    for &byte in &chunk[..read] {
                        if byte == b'\n' {
                            if !dropping {
                                if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                                    if let Some(id) = value.get("id").and_then(Value::as_u64) {
                                        if let Ok(mut map) = reader_pending.lock() {
                                            if let Some(sender) = map.remove(&id) {
                                                let _ = sender.send(value);
                                            }
                                        }
                                    }
                                }
                            }
                            line.clear();
                            dropping = false;
                        } else if dropping {
                            continue;
                        } else if line.len() < MAX_MCP_LINE_BYTES {
                            line.push(byte);
                        } else {
                            line.clear();
                            dropping = true;
                        }
                    }
                }
            });
            Transport::Stdio(Box::new(StdioTransport {
                child,
                stdin,
                pending,
                next_id: AtomicU64::new(1),
                _job_guard: job_guard,
            }))
        };

        let mut client = Self {
            transport,
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
            config: config.clone(),
        };
        let initialize = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "owo-agent", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        let server_version = initialize
            .pointer("/serverInfo/name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        let tools_result = client.request("tools/list", json!({})).await?;
        client.tools = tools_result
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| {
                        Some(McpTool {
                            name: tool.get("name")?.as_str()?.to_string(),
                            description: tool
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            input_schema: tool.get("inputSchema").cloned().unwrap_or(Value::Null),
                            annotations: tool
                                .get("annotations")
                                .cloned()
                                .filter(|value| value.is_object()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        // A2-2：resources/prompts 按能力协商条件拉取——服务器未声明能力时
        // 不发送对应请求（防老服务器对未知方法报错导致连接失败）。
        let capabilities = initialize
            .get("capabilities")
            .cloned()
            .unwrap_or(Value::Null);
        if capabilities.get("resources").is_some() {
            if let Err(error) = client.reload_resources().await {
                tracing::warn!(?error, server = %config.name, "MCP resources/list 失败");
            }
        }
        if capabilities.get("prompts").is_some() {
            if let Err(error) = client.reload_prompts().await {
                tracing::warn!(?error, server = %config.name, "MCP prompts/list 失败");
            }
        }
        tracing::info!(
            "MCP 服务器 {}（{server_version}，{}）已连接，工具 {} 个、资源 {} 个、模板 {} 个",
            config.name,
            config.transport,
            client.tools.len(),
            client.resources.len(),
            client.prompts.len()
        );
        Ok(client)
    }

    pub fn tools(&self) -> Vec<McpTool> {
        self.tools.clone()
    }

    /// A2-2：已发现的资源列表。
    pub fn resources(&self) -> Vec<McpResource> {
        self.resources.clone()
    }

    /// A2-2：已发现的提示模板列表。
    pub fn prompts(&self) -> Vec<McpPrompt> {
        self.prompts.clone()
    }

    /// A2-2：重新拉取 resources/list（刷新缓存；失败返回错误、保留旧缓存）。
    pub async fn reload_resources(&mut self) -> Result<usize, String> {
        let result = self.request("resources/list", json!({})).await?;
        self.resources = result
            .get("resources")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(McpResource {
                            uri: item.get("uri")?.as_str()?.to_string(),
                            name: item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            description: item
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            mime_type: item
                                .get("mimeType")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(self.resources.len())
    }

    /// A2-2：读取资源（resources/read），返回原始 result（contents 数组等）。
    pub async fn read_resource(&mut self, uri: &str) -> Result<Value, String> {
        self.request("resources/read", json!({ "uri": uri })).await
    }

    /// A2-2：重新拉取 prompts/list（刷新缓存；失败返回错误、保留旧缓存）。
    pub async fn reload_prompts(&mut self) -> Result<usize, String> {
        let result = self.request("prompts/list", json!({})).await?;
        self.prompts = result
            .get("prompts")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(McpPrompt {
                            name: item.get("name")?.as_str()?.to_string(),
                            description: item
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            arguments: item
                                .get("arguments")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(self.prompts.len())
    }

    /// A2-2：获取提示模板（prompts/get），返回原始 result（messages 数组）。
    pub async fn get_prompt(
        &mut self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<Value, String> {
        let mut params = json!({ "name": name });
        if let Some(arguments) = arguments {
            params["arguments"] = arguments;
        }
        self.request("prompts/get", params).await
    }

    /// stdio 子进程是否仍在运行（HTTP 传输恒为 true——无进程可查）。
    pub fn is_running(&mut self) -> bool {
        match &mut self.transport {
            Transport::Stdio(transport) => transport
                .child
                .try_wait()
                .map(|status| status.is_none())
                .unwrap_or(false),
            Transport::Http { .. } => true,
        }
    }

    /// 服务器名（工具命名空间基座）。
    pub fn server_name(&self) -> &str {
        &self.config.name
    }

    /// One tool request per call. Recovery never implies that a timed-out side effect did not happen.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        if let Err(error) = &result {
            if error.starts_with(TIMEOUT_ERROR_PREFIX)
                && matches!(self.transport, Transport::Stdio(_))
            {
                // Restore the transport for future calls, but never replay this unknown-outcome request.
                if let Err(recovery_error) = self.reconnect().await {
                    return Err(format!(
                        "{error}; mcp/outcome_unknown; recovery_failed: {recovery_error}"
                    ));
                }
                return Err(format!(
                    "{error}; mcp/outcome_unknown; transport_recovered; request_not_replayed"
                ));
            }
        }
        let result = result?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if is_error {
            Err(text)
        } else {
            Ok(json!({
                "text": text,
                "content": result.get("content").cloned().unwrap_or(Value::Null)
            }))
        }
    }

    /// stdio 超时后重建传输并重做握手。
    async fn reconnect(&mut self) -> Result<(), String> {
        let rebuilt = Self::connect_inner(self.config.clone()).await?;
        *self = rebuilt;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<(), String> {
        if matches!(&self.transport, Transport::Stdio(_)) {
            let _ = self.notify("exit", json!({})).await;
        }
        match &mut self.transport {
            Transport::Stdio(transport) => {
                let child = &mut transport.child;
                child.kill().await.map_err(|error| error.to_string())?;
                let _ = child.wait().await;
            }
            Transport::Http { .. } => {
                // HTTP 传输无显式 exit；连接随客户端释放。
            }
        }
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match &mut self.transport {
            Transport::Stdio(transport) => {
                let timeout = stdio_timeout(&self.config);
                request_stdio(transport, timeout, method, params).await
            }
            Transport::Http {
                client,
                url,
                headers,
                next_id,
            } => request_http(client, url, headers, next_id, method, params).await,
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        match &mut self.transport {
            Transport::Stdio(transport) => {
                let stdin = &mut transport.stdin;
                let message = json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": params,
                });
                let payload = serde_json::to_string(&message).map_err(|error| error.to_string())?;
                stdin
                    .write_all(payload.as_bytes())
                    .await
                    .map_err(|error| format!("MCP 写入失败：{error}"))?;
                stdin
                    .write_all(b"\n")
                    .await
                    .map_err(|error| format!("MCP 写入失败：{error}"))?;
                stdin
                    .flush()
                    .await
                    .map_err(|error| format!("MCP 刷新失败：{error}"))
            }
            Transport::Http {
                client,
                url,
                headers,
                ..
            } => {
                let message = json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": params,
                });
                let _ = client
                    .post(url.as_str())
                    .headers(headers.clone())
                    .json(&message)
                    .send()
                    .await;
                Ok(())
            }
        }
    }
}

async fn request_stdio(
    transport: &mut StdioTransport,
    timeout: Duration,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let stdin = &mut transport.stdin;
    let pending = &transport.pending;
    let next_id = &transport.next_id;
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = oneshot::channel();
    pending
        .lock()
        .map_err(|_| "MCP 响应表锁中毒".to_string())?
        .insert(id, sender);
    let message = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let payload = serde_json::to_string(&message).map_err(|error| error.to_string())?;
    stdin
        .write_all(payload.as_bytes())
        .await
        .map_err(|error| format!("MCP 写入失败：{error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("MCP 写入失败：{error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("MCP 刷新失败：{error}"))?;
    let response = match tokio::time::timeout(timeout, receiver).await {
        Ok(result) => result.map_err(|_| "MCP 服务器已关闭".to_string())?,
        Err(_) => {
            // 超时：清掉挂起的响应槽，终止挂死进程，便于下次调用自动重连。
            let _ = pending.lock().map(|mut map| map.remove(&id));
            let _ = transport.child.start_kill();
            return Err(format!("{TIMEOUT_ERROR_PREFIX} stdio 请求超时：{method}"));
        }
    };
    extract_result(response)
}

/// MCP HTTP 响应体上限（不受信/异常服务器不整块读进内存）。
const MAX_MCP_HTTP_BODY_BYTES: usize = 16 * 1024 * 1024;
/// MCP HTTP 错误响应体上限（错误文本只需可读预览）。
const MAX_MCP_HTTP_ERROR_BYTES: usize = 64 * 1024;

/// 流式读取响应体并在上限处截断。
async fn read_body_capped(
    response: reqwest::Response,
    max: usize,
    context: &str,
) -> Result<Vec<u8>, String> {
    let mut bytes: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("{context}读取失败：{error}"))?;
        if bytes.len() + chunk.len() > max {
            return Err(format!(
                "{context}超过上限（{} MiB），已拒绝",
                max / (1024 * 1024)
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn request_http(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    next_id: &AtomicU64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let message = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let response = client
        .post(url)
        .headers(headers.clone())
        .json(&message)
        .send()
        .await
        .map_err(|error| format!("MCP HTTP 请求失败：{error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let bytes = read_body_capped(response, MAX_MCP_HTTP_ERROR_BYTES, "MCP HTTP 错误响应")
            .await
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        return Err(format!("MCP HTTP 返回 {status}：{text}"));
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    if content_type.contains("text/event-stream") {
        let bytes = read_body_capped(response, MAX_MCP_HTTP_BODY_BYTES, "MCP SSE 响应").await?;
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            if let Some(payload) = line.trim().strip_prefix("data:") {
                if payload.trim() == "[DONE]" {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(payload) {
                    match extract_result(value) {
                        Ok(result) => return Ok(result),
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        Err(format!("MCP SSE 无有效响应：{method}"))
    } else {
        let bytes = read_body_capped(response, MAX_MCP_HTTP_BODY_BYTES, "MCP HTTP 响应").await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("MCP HTTP 响应解析失败：{error}"))?;
        extract_result(value)
    }
}

fn extract_result(value: Value) -> Result<Value, String> {
    if let Some(error) = value.get("error") {
        return Err(format!("MCP 错误：{error}"));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

/// 已连接 MCP 客户端的进程级生命周期注册表（M3 热卸载收尾）。
///
/// 记录每个服务器的客户端句柄，支撑：插件禁用/服务器移除时进程级 kill
/// （stdio 子进程立即终止，不留孤儿进程）；服务器退出时统一清理。
#[derive(Clone, Default)]
pub struct McpRegistry {
    clients: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<McpClient>>>>>,
}

impl McpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录客户端（重复插入按名覆盖，旧客户端先关闭，避免泄漏）。
    pub fn insert(&self, name: &str, client: Arc<tokio::sync::Mutex<McpClient>>) {
        if let Ok(mut clients) = self.clients.try_lock() {
            if let Some(previous) = clients.insert(name.to_string(), client) {
                tokio::spawn(async move {
                    let mut guard = previous.lock().await;
                    let _ = guard.shutdown().await;
                });
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<tokio::sync::Mutex<McpClient>>> {
        self.clients
            .try_lock()
            .ok()
            .and_then(|clients| clients.get(name).cloned())
    }

    /// 已注册服务器名。
    pub fn names(&self) -> Vec<String> {
        self.clients
            .try_lock()
            .map(|clients| {
                let mut names: Vec<String> = clients.keys().cloned().collect();
                names.sort();
                names
            })
            .unwrap_or_default()
    }

    /// 客户端是否仍在运行（stdio 子进程存活；未注册返回 false）。
    pub fn is_running(&self, name: &str) -> bool {
        match self.get(name) {
            Some(client) => client
                .try_lock()
                .map(|mut client| client.is_running())
                .unwrap_or(false),
            None => false,
        }
    }

    /// 断开并终止服务器：stdio 子进程 kill + wait；HTTP 客户端直接移除。
    pub async fn shutdown(&self, name: &str) -> Result<(), String> {
        let client = {
            let mut clients = self.clients.lock().await;
            clients.remove(name)
        };
        match client {
            Some(client) => {
                let mut guard = client.lock().await;
                guard.shutdown().await
            }
            None => Err(format!("MCP 服务器 {name} 未连接")),
        }
    }

    /// 关闭全部已连接服务器（进程退出前调用，防止遗留 stdio 子进程）。
    pub async fn shutdown_all(&self) -> Vec<(String, String)> {
        let names = self.names();
        let mut errors = Vec::new();
        for name in names {
            if let Err(error) = self.shutdown(&name).await {
                errors.push((name, error));
            }
        }
        errors
    }
}
