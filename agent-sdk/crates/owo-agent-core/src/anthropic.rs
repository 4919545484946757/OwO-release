//! Anthropic 原生 provider（A1-1，差距文档批次 3）：
//!
//! - `/v1/messages` 协议：system 独立顶层、`tool_use`/`tool_result` 内容块、
//!   `x-api-key` + `anthropic-version` 鉴权；
//! - 流式：`content_block_delta` 的 `text_delta` / `input_json_delta` /
//!   `thinking_delta` 分别映射正文 / 工具参数累积 / 思考通道；
//! - **prompt caching（A1-3）**：`cache_control: {type:"ephemeral"}` 打在
//!   system 尾块与最后一条 user 消息尾块——长会话成本数量级差异的核心；
//! - 多模态（A1-2）：`ChatMessage.images` → `image` 内容块
//!   （http url 直传；`data:image/...;base64,...` 解析为 base64 source）。
//!
//! 选择接线在 [`crate::gateway::DeferredProvider`]：`OWO_PROVIDER=anthropic`
//! 且 `ANTHROPIC_API_KEY` 可用时启用。

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::gateway::{
    build_model_http_client, ChatMessage, MessageImage, ModelCallMetadata, ModelOutput,
    ModelProvider, ObservedModelOutput, StreamChunk, TokenUsage, ToolCall, MODEL_DEFAULT_SENTINEL,
};
use crate::tools::ToolSpec;

use futures_util::StreamExt;

mod sse;

/// Anthropic 未显式指定 max_tokens 时必须下发（协议必填），默认 8192。
const DEFAULT_MAX_TOKENS: u64 = 8192;
/// `ANTHROPIC_VERSION` 缺省值。
const DEFAULT_API_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, PartialEq)]
pub struct AnthropicConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 数据出境开关：false 时拒绝一切云端模型调用。
    pub cloud_enabled: bool,
}

impl AnthropicConfig {
    pub fn from_env() -> Result<Self, String> {
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://api.anthropic.com".to_string());
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                "ANTHROPIC_API_KEY 未设置：使用 Anthropic 原生接入需配置该密钥".to_string()
            })?;
        let model = std::env::var("ANTHROPIC_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "claude-sonnet-4-5".to_string());
        let cloud_enabled = std::env::var("OWO_CLOUD_ENABLED")
            .ok()
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(true);
        Ok(Self {
            base_url,
            api_key,
            model,
            cloud_enabled,
        })
    }
}

pub struct AnthropicProvider {
    client: reqwest::Client,
    direct_client: Option<reqwest::Client>,
    config: AnthropicConfig,
    connection_options: Option<owo_agent_protocol::CustomModelConnection>,
    usage: Mutex<TokenUsage>,
    /// 最近一次请求的缓存命中（token 数）：仅用于日志/诊断，验证 caching 生效。
    last_cache_read: Mutex<u64>,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Result<Self, String> {
        // 客户端不设总超时：SSE 流式响应不能被固定墙钟截断；非流式请求在
        // post_messages 内按请求设置 180s 总超时，流式由逐块空闲超时守护。
        let (client, has_proxy) = if crate::gateway::is_local_endpoint(&config.base_url) {
            (
                reqwest::Client::builder()
                    .no_proxy()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .build()
                    .map_err(|e| e.to_string())?,
                false,
            )
        } else {
            build_model_http_client(10, None)?
        };
        let direct_client = if has_proxy {
            Some(
                reqwest::Client::builder()
                    .no_proxy()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .build()
                    .map_err(|e| format!("直连 HTTP 客户端创建失败：{e}"))?,
            )
        } else {
            None
        };
        Ok(Self {
            client,
            direct_client,
            config,
            connection_options: None,
            usage: Mutex::new(TokenUsage::default()),
            last_cache_read: Mutex::new(0),
        })
    }

    pub fn with_connection_options(
        mut self,
        options: &owo_agent_protocol::CustomModelConnection,
    ) -> Self {
        let mut safe = options.clone();
        safe.api_key = None;
        self.connection_options = Some(safe);
        self
    }

    fn request_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.connection_options
                .as_ref()
                .and_then(|o| o.timeout_secs)
                .unwrap_or(180),
        )
    }

    fn max_tokens() -> u64 {
        std::env::var("OWO_ANTHROPIC_MAX_TOKENS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_TOKENS)
    }

    fn cloud_enabled(&self) -> bool {
        if crate::gateway::is_local_endpoint(&self.config.base_url) {
            return true;
        }
        std::env::var("OWO_CLOUD_ENABLED")
            .ok()
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(self.config.cloud_enabled)
    }

    fn record_usage(&self, usage: &Value) {
        let Some(parsed) = parse_anthropic_usage(usage) else {
            return;
        };
        if let Ok(mut current) = self.usage.lock() {
            current.add(&parsed);
        }
        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if cache_read > 0 {
            if let Ok(mut slot) = self.last_cache_read.lock() {
                *slot = cache_read;
            }
            tracing::info!(
                cache_read_input_tokens = cache_read,
                "Anthropic prompt cache 命中"
            );
        }
    }

    /// 发送 `/v1/messages` 请求：代理优先、失败切直连重试一次（与 OpenAI 通道一致）。
    ///
    /// `stream=true` 时不设置请求级总超时：reqwest 的 `.timeout()` 覆盖整个响应体
    /// 读取，SSE 长回答会被固定墙钟截断（长程任务实测 Body TimedOut）。
    async fn post_messages(&self, body: &Value, stream: bool) -> Result<reqwest::Response, String> {
        let url = if self
            .connection_options
            .as_ref()
            .is_some_and(|o| o.use_full_url)
        {
            self.config.base_url.clone()
        } else {
            let base = self.config.base_url.trim_end_matches('/');
            if base.ends_with("/v1") {
                format!("{base}/messages")
            } else {
                format!("{base}/v1/messages")
            }
        };
        let mut body = body.clone();
        if let Some(temperature) = self.connection_options.as_ref().and_then(|o| o.temperature) {
            body["temperature"] = json!(temperature);
        }
        let mut last_error = String::new();
        let attempts: Vec<(&str, &reqwest::Client)> = {
            let mut list = vec![("proxy", &self.client)];
            if let Some(direct) = &self.direct_client {
                list.push(("direct", direct));
            }
            list
        };
        for (label, client) in attempts {
            let mut request = client
                .post(&url)
                .json(&body)
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", api_version());
            if !stream {
                request = request.timeout(self.request_timeout());
            }
            match tokio::time::timeout(self.request_timeout(), request.send())
                .await
                .map_err(|_| "provider/response_header_timeout".to_string())?
            {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let text = match tokio::time::timeout(
                        self.request_timeout(),
                        read_body_capped(
                            response,
                            MAX_ANTHROPIC_ERROR_BYTES,
                            "Anthropic error response",
                        ),
                    )
                    .await
                    {
                        Ok(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
                        Ok(Err(error)) => error,
                        Err(_) => "provider/error_body_timeout".to_string(),
                    };
                    let text = if self.config.api_key.is_empty() {
                        text
                    } else {
                        text.replace(&self.config.api_key, "[REDACTED]")
                    };
                    return Err(format!("Anthropic 返回 {status}：{text}"));
                }
                Err(error) => {
                    last_error = format!("{label}: {}", error.without_url());
                }
            }
        }
        Err(format!("Anthropic 请求失败：{last_error}"))
    }
}

fn api_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            std::env::var("ANTHROPIC_VERSION")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_string())
        })
        .as_str()
}

/// 图片 → Anthropic `image` 内容块：http(s) URL 直传；data URL 解析出
/// media_type 与 base64 数据（协议只收 base64 source / url source）。
fn image_block(image: &MessageImage) -> Option<Value> {
    let url = image.url.trim();
    if let Some(rest) = url.strip_prefix("data:") {
        // data:image/png;base64,AAAA...
        let (meta, data) = rest.split_once(',')?;
        let media_type = meta
            .strip_prefix("image/")
            .and_then(|suffix| suffix.split(';').next())
            .unwrap_or("png");
        if data.is_empty() {
            return None;
        }
        return Some(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": format!("image/{media_type}"), "data": data },
        }));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(json!({
            "type": "image",
            "source": { "type": "url", "url": url },
        }));
    }
    None
}

/// 缓存断点：给 content 数组的最后一个块打 `cache_control`（返回新数组）。
fn with_cache_break(mut blocks: Vec<Value>) -> Vec<Value> {
    if let Some(last) = blocks.last_mut() {
        if last.is_object() {
            last["cache_control"] = json!({ "type": "ephemeral" });
        }
    }
    blocks
}

/// ChatMessage 序列 → Anthropic 请求体（A1-1/A1-3）。
///
/// 转换规则：
/// - system 消息 → 顶层 `system` 数组（尾块带缓存断点）；
/// - `role=tool` 的结果消息 → 合并进下一条输出前的 user 消息（协议要求
///   `tool_result` 位于 user 角色、紧跟对应 assistant 的 `tool_use`）；
/// - assistant 的 tool_calls → `tool_use` 块（连同文本块）；
/// - 缓存断点：system 尾块 + 最后一条 user 消息尾块（≤4 断点约束内）。
pub fn build_request_body(
    config: &AnthropicConfig,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    stream: bool,
) -> Value {
    let mut system_parts: Vec<String> = Vec::new();
    let mut wire_messages: Vec<Value> = Vec::new();
    // 待输出的 tool_result 块（连续 tool 消息合并进一条 user 消息）。
    let mut pending_tool_results: Vec<Value> = Vec::new();

    let flush_tool_results = |pending: &mut Vec<Value>, wire: &mut Vec<Value>| {
        if !pending.is_empty() {
            wire.push(json!({ "role": "user", "content": pending.clone() }));
            pending.clear();
        }
    };

    for message in messages {
        match message.role.as_str() {
            "system" => {
                if let Some(text) = message.content.as_deref().filter(|t| !t.is_empty()) {
                    system_parts.push(text.to_string());
                }
            }
            "user" => {
                flush_tool_results(&mut pending_tool_results, &mut wire_messages);
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = message.content.as_deref().filter(|t| !t.is_empty()) {
                    blocks.push(json!({ "type": "text", "text": text }));
                }
                for image in &message.images {
                    if let Some(block) = image_block(image) {
                        blocks.push(block);
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "（空消息）" }));
                }
                wire_messages.push(json!({ "role": "user", "content": blocks }));
            }
            "assistant" => {
                flush_tool_results(&mut pending_tool_results, &mut wire_messages);
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = message.content.as_deref().filter(|t| !t.is_empty()) {
                    blocks.push(json!({ "type": "text", "text": text }));
                }
                for call in message.tool_calls.iter().flatten() {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments,
                    }));
                }
                if !blocks.is_empty() {
                    wire_messages.push(json!({ "role": "assistant", "content": blocks }));
                }
            }
            "tool" => {
                let id = message.tool_call_id.clone().unwrap_or_default();
                let text = message.content.clone().unwrap_or_default();
                pending_tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": [{ "type": "text", "text": text }],
                }));
            }
            _ => {}
        }
    }
    flush_tool_results(&mut pending_tool_results, &mut wire_messages);

    // 缓存断点（A1-3）：最后一条 user 消息的尾块。
    if let Some(last_user) = wire_messages
        .iter_mut()
        .rev()
        .find(|message| message["role"] == "user")
    {
        if let Some(blocks) = last_user
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .cloned()
        {
            last_user["content"] = Value::Array(with_cache_break(blocks));
        }
    }

    let tools_payload: Vec<Value> = tools
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "input_schema": spec.input_schema,
            })
        })
        .collect();

    let mut body = json!({
        "model": config.model,
        "max_tokens": AnthropicProvider::max_tokens(),
        "messages": wire_messages,
        "stream": stream,
    });
    if !system_parts.is_empty() {
        let system_blocks: Vec<Value> = system_parts
            .into_iter()
            .map(|text| json!({ "type": "text", "text": text }))
            .collect();
        // system 尾块带缓存断点：system prompt 通常稳定，缓存收益最大。
        body["system"] = Value::Array(with_cache_break(system_blocks));
    }
    if !tools_payload.is_empty() {
        body["tools"] = Value::Array(tools_payload);
    }
    body
}

/// 非流式响应 → ModelOutput：content blocks 拼接文本与 tool_use。
fn parse_response_content(payload: &Value, tools: &[ToolSpec]) -> Result<ModelOutput, String> {
    if payload.get("type").and_then(Value::as_str) == Some("error") {
        let detail = payload
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("未知错误");
        return Err(format!("Anthropic 错误：{detail}"));
    }
    let blocks = payload
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| "Anthropic 响应缺少 content 数组".to_string())?;
    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(piece) = block.get("text").and_then(Value::as_str) {
                    text.push_str(piece);
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "Anthropic tool_use 缺少有效 id".to_string())?
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| format!("Anthropic tool_use {id} 缺少有效 name"))?
                    .to_string();
                let arguments = block
                    .get("input")
                    .filter(|value| value.is_object())
                    .cloned()
                    .ok_or_else(|| format!("Anthropic 工具 {name} input 必须是 JSON 对象"))?;
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            _ => {}
        }
    }
    if !tool_calls.is_empty() {
        Ok(ModelOutput::ToolCalls(crate::gateway::validate_tool_calls(
            tool_calls, tools,
        )?))
    } else if payload.get("stop_reason").and_then(Value::as_str) == Some("max_tokens") {
        // 截断也要给出可见文本，而不是报「既无文本也无工具调用」。
        Ok(ModelOutput::Text(text))
    } else if !text.is_empty() {
        Ok(ModelOutput::Text(text))
    } else {
        Err("Anthropic 响应既无文本也无工具调用".to_string())
    }
}

/// 流式状态：正文 / 思考 / tool_use 块累积。
/// Anthropic 流式缓冲上限：异常端点可无界推送增量，超出即标记并在产出时显式报错。
const MAX_ANTHROPIC_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_ANTHROPIC_REASONING_BYTES: usize = 8 * 1024 * 1024;
const MAX_ANTHROPIC_TOOL_ARGUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ANTHROPIC_TOOL_BLOCKS: usize = 1024;
const MAX_ANTHROPIC_TOTAL_TOOL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
struct AnthStreamState {
    content: String,
    reasoning: String,
    /// index → (tool_use_id, name, arguments json 累积)。
    tool_blocks: HashMap<u64, (String, String, String)>,
    completed_tool_blocks: HashSet<u64>,
    usage: Option<Value>,
    tool_bytes: usize,
    message_started: bool,
    message_stopped: bool,
    error: Option<String>,
    truncated: bool,
}

impl AnthStreamState {
    /// 消费一条 `data:` 负载；返回本条产生的 (正文增量, 思考增量)。
    fn feed(&mut self, payload: &str) -> Option<(Option<String>, Option<String>)> {
        let payload = payload.trim();
        if payload.is_empty() {
            return None;
        }
        if self.message_stopped || self.error.is_some() || self.truncated {
            return None;
        }
        let value: Value = match serde_json::from_str(payload) {
            Ok(value) => value,
            Err(_) => {
                self.error =
                    Some("provider/stream_invalid_json: invalid Anthropic SSE event".into());
                return None;
            }
        };
        let Some(kind) = value.get("type").and_then(Value::as_str) else {
            self.error = Some("provider/stream_invalid_event: Anthropic event has no type".into());
            return None;
        };
        let index = value.get("index").and_then(Value::as_u64);
        let malformed = match kind {
            "message_start" => !value.get("message").is_some_and(Value::is_object),
            "content_block_start" => {
                index.is_none()
                    || value
                        .pointer("/content_block/type")
                        .and_then(Value::as_str)
                        .is_none()
            }
            "content_block_stop" => index.is_none(),
            "content_block_delta" => {
                index.is_none()
                    || match value.pointer("/delta/type").and_then(Value::as_str) {
                        Some("text_delta") => value
                            .pointer("/delta/text")
                            .and_then(Value::as_str)
                            .is_none(),
                        Some("thinking_delta") => value
                            .pointer("/delta/thinking")
                            .and_then(Value::as_str)
                            .is_none(),
                        Some("input_json_delta") => {
                            value
                                .pointer("/delta/partial_json")
                                .and_then(Value::as_str)
                                .is_none()
                                || index.is_some_and(|index| {
                                    !self.tool_blocks.contains_key(&index)
                                        || self.completed_tool_blocks.contains(&index)
                                })
                        }
                        None => true,
                        _ => false, // Future delta types remain forwards compatible.
                    }
            }
            _ => false,
        };
        if malformed {
            self.error = Some(format!(
                "provider/stream_invalid_event: malformed Anthropic {kind}"
            ));
            return None;
        }
        match kind {
            "message_start" => {
                if self.message_started {
                    self.error =
                        Some("provider/stream_invalid_event: duplicate message_start".into());
                    return None;
                }
                self.message_started = true;
                if let Some(usage) = value.pointer("/message/usage") {
                    merge_anthropic_usage(&mut self.usage, usage);
                }
                None
            }
            "content_block_start" => {
                let index = value.get("index").and_then(Value::as_u64)?;
                let block = value.get("content_block")?;
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if self.tool_blocks.contains_key(&index) {
                        self.error = Some(
                            "provider/stream_invalid_event: duplicate tool block index".into(),
                        );
                        return None;
                    }
                    if self.tool_blocks.len() >= MAX_ANTHROPIC_TOOL_BLOCKS {
                        self.error = Some("provider/stream_tool_blocks_too_large: Anthropic tool block count exceeds 1024".into());
                        return None;
                    }
                    self.tool_bytes = self
                        .tool_bytes
                        .saturating_add(id.len())
                        .saturating_add(name.len());
                    if self.tool_bytes > MAX_ANTHROPIC_TOTAL_TOOL_BYTES {
                        self.truncated = true;
                        return None;
                    }
                    self.tool_blocks.insert(index, (id, name, String::new()));
                }
                None
            }
            "content_block_delta" => {
                let delta = value.get("delta")?;
                match delta.get("type").and_then(Value::as_str)? {
                    "text_delta" => {
                        let piece = delta.get("text").and_then(Value::as_str)?;
                        if self.content.len() + piece.len() > MAX_ANTHROPIC_TEXT_BYTES {
                            self.truncated = true;
                            return None;
                        }
                        self.content.push_str(piece);
                        Some((Some(piece.to_string()), None))
                    }
                    "thinking_delta" => {
                        let piece = delta.get("thinking").and_then(Value::as_str)?;
                        if self.reasoning.len() + piece.len() > MAX_ANTHROPIC_REASONING_BYTES {
                            self.truncated = true;
                            return None;
                        }
                        self.reasoning.push_str(piece);
                        Some((None, Some(piece.to_string())))
                    }
                    "input_json_delta" => {
                        let index = value.get("index").and_then(Value::as_u64)?;
                        if let Some(entry) = self.tool_blocks.get_mut(&index) {
                            let partial = delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            self.tool_bytes = self.tool_bytes.saturating_add(partial.len());
                            if entry.2.len().saturating_add(partial.len())
                                > MAX_ANTHROPIC_TOOL_ARGUMENT_BYTES
                                || self.tool_bytes > MAX_ANTHROPIC_TOTAL_TOOL_BYTES
                            {
                                self.truncated = true;
                            } else {
                                entry.2.push_str(partial);
                            }
                        }
                        None
                    }
                    _ => None,
                }
            }
            "content_block_stop" => {
                if let Some(index) = index.filter(|index| self.tool_blocks.contains_key(index)) {
                    self.completed_tool_blocks.insert(index);
                }
                None
            }
            "message_delta" => {
                if let Some(usage) = value.get("usage") {
                    merge_anthropic_usage(&mut self.usage, usage);
                }
                None
            }
            "message_stop" => {
                if !self.message_started {
                    self.error = Some(
                        "provider/stream_incomplete: message_stop without message_start".into(),
                    );
                } else {
                    self.message_stopped = true;
                }
                None
            }
            "error" => {
                let kind = value
                    .pointer("/error/type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown_error");
                let detail = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("未知错误");
                self.error = Some(format!("provider/anthropic_stream_error: {kind}: {detail}"));
                None
            }
            _ => None,
        }
    }

    fn validate_stream(&self) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if self.truncated {
            return Err("Anthropic 流式响应超过缓冲上限，已拒绝解析".to_string());
        }
        Ok(())
    }

    fn into_output(mut self, tools: &[ToolSpec]) -> Result<ModelOutput, String> {
        self.validate_stream()?;
        if !self.message_stopped {
            return Err(
                "provider/stream_incomplete: Anthropic response ended before message_stop".into(),
            );
        }
        // tool_use 块按 index 排序输出。
        if !self.tool_blocks.is_empty() {
            if self.completed_tool_blocks.len() != self.tool_blocks.len() {
                return Err(
                    "provider/stream_incomplete: Anthropic tool block did not finish".into(),
                );
            }
            let mut calls: Vec<(u64, ToolCall)> = self
                .tool_blocks
                .drain()
                .map(|(index, (id, name, args))| {
                    if id.trim().is_empty() {
                        return Err("Anthropic 流式 tool_use 缺少有效 id".to_string());
                    }
                    if name.trim().is_empty() {
                        return Err(format!("Anthropic 流式 tool_use {id} 缺少有效 name"));
                    }
                    let arguments = if args.trim().is_empty() {
                        Value::Object(serde_json::Map::new())
                    } else {
                        serde_json::from_str::<Value>(&args).map_err(|error| {
                            format!("Anthropic 工具 {name} 参数 JSON 非法：{error}")
                        })?
                    };
                    if !arguments.is_object() {
                        return Err(format!("Anthropic 工具 {name} input 必须是 JSON 对象"));
                    }
                    Ok((
                        index,
                        ToolCall {
                            id,
                            name,
                            arguments,
                        },
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            calls.sort_by_key(|(index, _)| *index);
            let calls = calls.into_iter().map(|(_, call)| call).collect();
            return Ok(ModelOutput::ToolCalls(crate::gateway::validate_tool_calls(
                calls, tools,
            )?));
        }
        Ok(ModelOutput::Text(std::mem::take(&mut self.content)))
    }
}

/// 非流式响应体上限（异常端点可能返回超大 body）。
const MAX_ANTHROPIC_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// 错误响应体上限（错误文本只需可读预览）。
const MAX_ANTHROPIC_ERROR_BYTES: usize = 64 * 1024;

/// 流式读取响应体并在上限处截断。
async fn read_body_capped(
    response: reqwest::Response,
    max: usize,
    context: &str,
) -> Result<Vec<u8>, String> {
    let mut bytes: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("{context}读取失败：{e}"))?;
        if bytes.len() + chunk.len() > max {
            return Err(format!("{context}超过上限（{} KiB），已拒绝", max / 1024));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// A delta's token fields are cumulative, not increments. Preserve start-only fields.
fn merge_anthropic_usage(current: &mut Option<Value>, update: &Value) {
    if !update.is_object() {
        *current = None;
        return;
    }
    let current = current.get_or_insert_with(|| json!({}));
    for field in [
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ] {
        if let Some(value) = update.get(field) {
            current[field] = value.as_u64().map(Value::from).unwrap_or(Value::Null);
        }
    }
}

// Cached input is still input. Invalid/incomplete/overflowing usage remains unknown.
fn parse_anthropic_usage(value: &Value) -> Option<TokenUsage> {
    let input = value.get("input_tokens")?.as_u64()?;
    let output = value.get("output_tokens")?.as_u64()?;
    let cache_tokens = |field| match value.get(field) {
        Some(value) => value.as_u64(),
        None => Some(0),
    };
    let prompt = input
        .checked_add(cache_tokens("cache_creation_input_tokens")?)?
        .checked_add(cache_tokens("cache_read_input_tokens")?)?;
    Some(TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: output,
        total_tokens: prompt.checked_add(output)?,
    })
}

fn anthropic_request_id(response: &reqwest::Response) -> Option<String> {
    ["request-id", "x-request-id"]
        .iter()
        .find_map(|name| response.headers().get(*name))
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

impl AnthropicProvider {
    // Model overrides affect this payload only: shared Worker providers remain immutable.
    fn request_body(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        stream: bool,
    ) -> Value {
        let mut body = build_request_body(&self.config, messages, tools, stream);
        let model = model
            .map(str::trim)
            .filter(|model| !model.is_empty() && *model != MODEL_DEFAULT_SENTINEL)
            .unwrap_or(&self.config.model);
        body["model"] = json!(model);
        body
    }

    async fn complete_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ObservedModelOutput, String> {
        if !self.cloud_enabled() {
            return Err("云端模型已禁用（数据出境开关关闭）".to_string());
        }
        let effective_model = model
            .map(str::trim)
            .filter(|m| !m.is_empty() && *m != MODEL_DEFAULT_SENTINEL)
            .unwrap_or(&self.config.model);
        crate::gateway::input_capabilities::ensure_image_input(
            effective_model,
            &self.config.base_url,
            "anthropic",
            self.connection_options.as_ref(),
            messages,
        )?;
        let started = std::time::Instant::now();
        let body = self.request_body(model, messages, tools, false);
        let response = self.post_messages(&body, false).await?;
        let request_id = anthropic_request_id(&response);
        let bytes =
            read_body_capped(response, MAX_ANTHROPIC_RESPONSE_BYTES, "Anthropic 响应").await?;
        let payload: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("Anthropic 响应解析失败：{e}"))?;
        if let Some(usage) = payload.get("usage") {
            self.record_usage(usage);
        }
        let output = parse_response_content(&payload, tools)?;
        Ok(ObservedModelOutput {
            output,
            metadata: ModelCallMetadata {
                request_id,
                model: body
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                usage: payload.get("usage").and_then(parse_anthropic_usage),
                latency_ms: Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
            },
        })
    }

    async fn stream_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ObservedModelOutput, String> {
        if !self.cloud_enabled() {
            return Err("云端模型已禁用（数据出境开关关闭）".to_string());
        }
        let effective_model = model
            .map(str::trim)
            .filter(|m| !m.is_empty() && *m != MODEL_DEFAULT_SENTINEL)
            .unwrap_or(&self.config.model);
        crate::gateway::input_capabilities::ensure_image_input(
            effective_model,
            &self.config.base_url,
            "anthropic",
            self.connection_options.as_ref(),
            messages,
        )?;
        let started = std::time::Instant::now();
        let body = self.request_body(model, messages, tools, true);
        let response = self.post_messages(&body, true).await?;
        let request_id = anthropic_request_id(&response);
        let mut stream = response.bytes_stream();
        let mut decoder = sse::SseDecoder::default();
        let mut state = AnthStreamState::default();
        let idle_timeout = self
            .connection_options
            .as_ref()
            .and_then(|o| o.timeout_secs)
            .map(std::time::Duration::from_secs)
            .unwrap_or(std::time::Duration::from_secs(60));

        loop {
            let chunk = tokio::time::timeout(idle_timeout, stream.next())
                .await
                .map_err(|_| {
                    format!(
                        "provider/stream_idle_timeout: Anthropic 流式输出空闲超时（{}s 无数据）",
                        idle_timeout.as_secs()
                    )
                })?
                .ok_or_else(|| {
                    "provider/stream_incomplete: Anthropic response ended before message_stop"
                        .to_string()
                })?
                .map_err(|e| format!("Anthropic 流式读取失败：{}", e.without_url()))?;
            let stopped = decoder
                .feed(&chunk, &mut |payload| {
                    if let Some((content_delta, reasoning_delta)) = state.feed(payload) {
                        if let Some(piece) = reasoning_delta {
                            on_chunk(StreamChunk::Reasoning(piece));
                        }
                        if let Some(piece) = content_delta {
                            on_chunk(StreamChunk::Content(piece));
                        }
                    }
                    state.validate_stream()?;
                    Ok(state.message_stopped)
                })
                .map_err(|error| {
                    // A provider may echo credentials inside an SSE error message.
                    let error = if self.config.api_key.is_empty() {
                        error
                    } else {
                        error.replace(&self.config.api_key, "[REDACTED]")
                    };
                    error.chars().take(2048).collect::<String>()
                })?;
            if stopped {
                break;
            }
        }
        if let Some(usage) = state.usage.clone() {
            self.record_usage(&usage);
        }
        let usage = state.usage.as_ref().and_then(parse_anthropic_usage);
        let output = state.into_output(tools)?;
        Ok(ObservedModelOutput {
            output,
            metadata: ModelCallMetadata {
                request_id,
                model: body
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                usage,
                latency_ms: Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
            },
        })
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn validate_image_input(&self, model: Option<&str>, has_images: bool) -> Result<(), String> {
        let model = model
            .map(str::trim)
            .filter(|m| !m.is_empty() && *m != "default")
            .unwrap_or(&self.config.model);
        crate::gateway::input_capabilities::ensure_image_presence(
            model,
            &self.config.base_url,
            "anthropic",
            self.connection_options.as_ref(),
            has_images,
        )
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.complete_with_model(None, messages, tools).await
    }

    async fn complete_with_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.complete_observed(model, messages, tools)
            .await
            .map(|observed| observed.output)
    }

    async fn complete_with_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ObservedModelOutput, String> {
        self.complete_observed(model, messages, tools).await
    }

    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        self.complete_stream_with_model(None, messages, tools, on_delta)
            .await
    }

    async fn complete_stream_with_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        let mut forward = |chunk| {
            if let StreamChunk::Content(text) = chunk {
                on_delta(text);
            }
        };
        self.stream_observed(model, messages, tools, &mut forward)
            .await
            .map(|observed| observed.output)
    }

    async fn complete_stream_with_reasoning(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.complete_stream_with_reasoning_and_model(None, messages, tools, on_chunk)
            .await
    }

    async fn complete_stream_with_reasoning_and_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.stream_observed(model, messages, tools, on_chunk)
            .await
            .map(|observed| observed.output)
    }

    async fn complete_stream_with_reasoning_and_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ObservedModelOutput, String> {
        self.stream_observed(model, messages, tools, on_chunk).await
    }

    fn usage_snapshot(&self) -> TokenUsage {
        self.usage.lock().map(|usage| *usage).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> AnthropicConfig {
        AnthropicConfig {
            base_url: "https://api.anthropic.com".to_string(),
            api_key: "sk-test".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            cloud_enabled: true,
        }
    }

    #[test]
    fn request_body_extracts_system_and_merges_tool_results() {
        let messages = vec![
            ChatMessage::system("系统提示".to_string()),
            ChatMessage::user("看看文件".to_string()),
            ChatMessage::assistant_tool_calls(vec![ToolCall {
                id: "tu_1".to_string(),
                name: "read_file".to_string(),
                arguments: json!({ "path": "a.txt" }),
            }]),
            ChatMessage::tool("tu_1".to_string(), "文件内容".to_string()),
            ChatMessage::assistant_text("读完了".to_string()),
        ];
        let tools = vec![ToolSpec {
            name: "read_file".to_string(),
            description: "读文件".to_string(),
            input_schema: json!({ "type": "object" }),
            effect: None,
        }];
        let body = build_request_body(&config(), &messages, &tools, true);
        assert_eq!(body["stream"], json!(true));
        // system 提取到顶层，且尾块带缓存断点。
        let system = body["system"].as_array().expect("system 应为数组");
        assert_eq!(system.last().unwrap()["cache_control"]["type"], "ephemeral");
        // tool_result 合并进 user 消息。
        let wire = body["messages"].as_array().unwrap();
        assert_eq!(wire.len(), 4, "tool 结果并入 user，不单独占一条");
        assert_eq!(wire[2]["content"][0]["type"], "tool_result");
        assert_eq!(wire[2]["content"][0]["tool_use_id"], "tu_1");
        // assistant 的 tool_use 块。
        assert_eq!(wire[1]["content"][0]["type"], "tool_use");
        assert_eq!(wire[1]["content"][0]["name"], "read_file");
        // tools → input_schema。
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        // 最后一条 user（tool_result 宿主）尾块带缓存断点。
        let last_user = wire.iter().rev().find(|m| m["role"] == "user").unwrap();
        assert_eq!(
            last_user["content"].as_array().unwrap().last().unwrap()["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn anthropic_request_uses_the_same_registry_tool_contracts() {
        let specs = crate::tools::ToolRegistry::new().specs();
        let body = build_request_body(&config(), &[], &specs, false);
        let sent = body["tools"].as_array().unwrap();
        assert_eq!(sent.len(), specs.len());
        for (entry, spec) in sent.iter().zip(&specs) {
            assert_eq!(entry["name"], spec.name);
            assert_eq!(entry["description"], spec.description);
            assert_eq!(entry["input_schema"], spec.input_schema);
            assert_eq!(entry["input_schema"]["type"], "object");
        }
        let verification = sent
            .iter()
            .find(|entry| entry["name"] == "verification_plan")
            .unwrap();
        let requirement = &verification["input_schema"]["properties"]["plan"]["properties"]
            ["requirements"]["items"];
        let actual_validator_ids: std::collections::BTreeSet<_> = requirement["properties"]
            ["validator_id"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let expected_validator_ids: std::collections::BTreeSet<_> =
            crate::verification::workspace_validator_contracts()
                .iter()
                .map(|contract| contract.validator_id)
                .chain(std::iter::once(
                    crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID,
                ))
                .collect();
        assert_eq!(actual_validator_ids, expected_validator_ids);
        assert_eq!(requirement["properties"]["arguments"]["type"], "object");
        assert!(requirement.get("anyOf").is_none());
    }

    #[test]
    fn images_convert_to_image_blocks() {
        let messages = vec![ChatMessage::user_with_images(
            "这是什么".to_string(),
            vec![
                MessageImage::from_url("https://example.com/a.png"),
                MessageImage::from_url("data:image/jpeg;base64,QUJD"),
            ],
        )];
        let body = build_request_body(&config(), &messages, &[], false);
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "url");
        assert_eq!(blocks[2]["source"]["type"], "base64");
        assert_eq!(blocks[2]["source"]["media_type"], "image/jpeg");
        assert_eq!(blocks[2]["source"]["data"], "QUJD");
    }

    #[test]
    fn stream_state_accumulates_text_tools_and_usage() {
        let mut state = AnthStreamState::default();
        state.feed(r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":5}}}"#);
        state.feed(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#);
        let text_delta = state.feed(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你好"}}"#);
        assert_eq!(text_delta.unwrap().0.as_deref(), Some("你好"));
        state.feed(r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_9","name":"grep"}}"#);
        state.feed(r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"pa"}}"#);
        state.feed(r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"ttern\":\"x\"}"}}"#);
        state.feed(r#"{"type":"content_block_stop","index":1}"#);
        state.feed(r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#);
        state.feed(r#"{"type":"message_stop"}"#);
        let output = state
            .into_output(&crate::tools::ToolRegistry::new().specs())
            .unwrap();
        match output {
            ModelOutput::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "tu_9");
                assert_eq!(calls[0].name, "grep");
                assert_eq!(calls[0].arguments["pattern"], "x");
            }
            other => panic!("应为 ToolCalls：{other:?}"),
        }
    }

    #[test]
    fn oversized_anthropic_tool_arguments_are_rejected() {
        let mut state = AnthStreamState::default();
        state.feed(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tu_big","name":"write_file"}}"#);
        let payload = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": "x".repeat(5 * 1024 * 1024)}
        })
        .to_string();
        state.feed(&payload);
        let error = state
            .into_output(&crate::tools::ToolRegistry::new().specs())
            .unwrap_err();
        assert!(error.contains("缓冲上限"), "{error}");
    }

    #[test]
    fn non_stream_response_parses_blocks() {
        let payload = json!({
            "content": [
                { "type": "text", "text": "分析如下" },
                { "type": "tool_use", "id": "tu_2", "name": "run_command", "input": { "command": "ls" } }
            ],
            "stop_reason": "tool_use"
        });
        let output =
            parse_response_content(&payload, &crate::tools::ToolRegistry::new().specs()).unwrap();
        match output {
            ModelOutput::ToolCalls(calls) => {
                assert_eq!(calls[0].name, "run_command");
                assert_eq!(calls[0].arguments["command"], "ls");
            }
            other => panic!("应为 ToolCalls：{other:?}"),
        }
    }

    #[test]
    fn malformed_anthropic_tool_uses_are_rejected_instead_of_defaulted() {
        let tools = crate::tools::ToolRegistry::new().specs();
        let missing_id = json!({
            "content": [{
                "type": "tool_use",
                "id": "",
                "name": "read_file",
                "input": {"path": "README.md"}
            }],
            "stop_reason": "tool_use"
        });
        assert!(parse_response_content(&missing_id, &tools)
            .unwrap_err()
            .contains("有效 id"));

        let invalid_input = json!({
            "content": [{
                "type": "tool_use",
                "id": "toolu_1",
                "name": "read_file",
                "input": null
            }],
            "stop_reason": "tool_use"
        });
        assert!(parse_response_content(&invalid_input, &tools)
            .unwrap_err()
            .contains("JSON 对象"));

        let mut state = AnthStreamState::default();
        state.feed(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_2","name":"read_file"}}"#);
        state.feed(r#"{"type":"message_start","message":{}}"#);
        state.feed(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{bad"}}"#);
        state.feed(r#"{"type":"content_block_stop","index":0}"#);
        state.feed(r#"{"type":"message_stop"}"#);
        assert!(state.into_output(&tools).unwrap_err().contains("JSON 非法"));
    }

    #[test]
    fn error_payload_maps_to_message() {
        let payload =
            json!({ "type": "error", "error": { "type": "rate_limit_error", "message": "限流" } });
        let error = parse_response_content(&payload, &crate::tools::ToolRegistry::new().specs())
            .unwrap_err();
        assert!(error.contains("限流"), "{error}");
    }
}
