use crate::tools::ToolSpec;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_MODEL_OUTPUT_TOKENS: u64 = 32_000;
const MAX_MODEL_OUTPUT_TOKENS: u64 = 1_000_000;

/// 内部哨兵：流式结束时 `finish_reason=length` 且只有思考、没有正文/工具调用。
/// 调用方据此自动提升预算并降低思考档后重试一次；不对外暴露。
const REASONING_ONLY_LENGTH: &str = "__owo_reasoning_only_length__";

/// 单条 SSE 行（未处理缓冲区）上限：异常端点可以不发换行地无界推送，
/// 超出即显式失败，避免把内存吃满。
const MAX_STREAM_LINE_BYTES: usize = 1024 * 1024;

use super::config::*;
use super::is_local_endpoint;
use super::message::*;
use super::stream::*;

pub(super) fn parse_tool_calls(
    message: &Value,
    tools: &[ToolSpec],
) -> Result<Option<Vec<ToolCall>>, String> {
    let Some(raw_calls) = message.get("tool_calls") else {
        return Ok(None);
    };
    let calls = raw_calls
        .as_array()
        .ok_or_else(|| "模型响应的 tool_calls 必须是数组".to_string())?;
    if calls.is_empty() {
        return Ok(None);
    }

    let parsed = calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| format!("模型返回的第 {index} 个工具调用缺少有效 id"))?;
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| format!("模型返回的第 {index} 个工具调用缺少有效 function.name"))?;
            let raw_arguments = call
                .pointer("/function/arguments")
                .ok_or_else(|| format!("模型返回的工具 {name} 缺少 arguments"))?;
            let arguments = match raw_arguments {
                Value::String(raw) => serde_json::from_str::<Value>(raw)
                    .map_err(|error| format!("模型返回的工具 {name} 参数不是有效 JSON：{error}"))?,
                Value::Object(_) => raw_arguments.clone(),
                _ => return Err(format!("模型返回的工具 {name} arguments 必须是 JSON 对象")),
            };
            if !arguments.is_object() {
                return Err(format!("模型返回的工具 {name} arguments 必须是 JSON 对象"));
            }
            Ok(ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    validate_tool_calls(parsed, tools).map(Some)
}

/// 结构不变量：id 非空且唯一、name 非空、arguments 为 JSON 对象。
///
/// **不检查工具是否在本轮清单内**——「请求了未下发/已热卸载的工具」必须作为
/// 工具错误回喂模型（turn 主循环的 guard 分支），而不是直接终止回合；只有
/// 协议层解析（Provider 从线上响应组装调用）才需要额外核对清单一致性。
pub(crate) fn validate_tool_call_shapes(calls: &[ToolCall]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for call in calls {
        if call.id.trim().is_empty() {
            return Err("模型返回的工具调用缺少有效 id".to_string());
        }
        if call.name.trim().is_empty() {
            return Err(format!("模型返回的工具调用 {} 缺少有效名称", call.id));
        }
        if !ids.insert(call.id.as_str()) {
            return Err(format!("模型返回重复的工具调用 id：{}", call.id));
        }
        if !call.arguments.is_object() {
            return Err(format!(
                "模型返回的工具 {} arguments 必须是 JSON 对象",
                call.name
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_tool_calls(
    calls: Vec<ToolCall>,
    tools: &[ToolSpec],
) -> Result<Vec<ToolCall>, String> {
    validate_tool_call_shapes(&calls)?;
    for call in &calls {
        if !tools.iter().any(|spec| spec.name == call.name) {
            return Err(format!(
                "模型请求了本轮未提供的工具 {}；可用工具列表与响应不一致",
                call.name
            ));
        }
    }
    Ok(calls)
}

/// Validate arguments against the exact schema sent to the model this turn.
pub(crate) fn validate_tool_arguments(arguments: &Value, spec: &ToolSpec) -> Result<(), String> {
    crate::json_schema::validate(arguments, &spec.input_schema, "$")
        .map_err(|error| format!("工具 {} 参数不符合公开 schema：{error}", spec.name))
}

/// OpenAI-compatible `/chat/completions` 客户端（覆盖 OpenAI、DeepSeek、Ollama、多数代理）。
pub struct OpenAiCompatibleProvider {
    pub(super) client: reqwest::Client,
    pub(super) direct_client: Option<reqwest::Client>,
    config: OpenAiCompatibleConfig,
    connection_options: Option<owo_agent_protocol::CustomModelConnection>,
    usage: std::sync::Mutex<TokenUsage>,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, String> {
        // 客户端不设总超时：流式请求不能有"整个响应体"的墙钟上限，否则长回答
        // 会在固定秒数处被 reqwest 腰斩（长程任务实测 Body TimedOut）。非流式
        // 请求在 post_chat 内按请求设置总超时，流式由逐块空闲超时守护。
        let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(10));
        let mut has_proxy = false;
        if is_local_endpoint(&config.base_url) {
            builder = builder.no_proxy();
        }
        // Local model endpoints are isolated from HTTP proxies; proxying loopback
        // breaks local inference and can turn localhost into a remote request.
        if !is_local_endpoint(&config.base_url) {
            for name in [
                "OWO_HTTP_PROXY",
                "HTTPS_PROXY",
                "HTTP_PROXY",
                "https_proxy",
                "http_proxy",
            ] {
                if let Ok(proxy) = std::env::var(name) {
                    if !proxy.trim().is_empty() {
                        let proxy = reqwest::Proxy::all(proxy)
                            .map_err(|e| format!("代理配置无效（{name}）：{e}"))?;
                        builder = builder.proxy(proxy);
                        has_proxy = true;
                        break;
                    }
                }
            }
        }
        let client = builder
            .build()
            .map_err(|e| format!("HTTP 客户端创建失败：{e}"))?;
        let direct_client = if has_proxy {
            Some(
                reqwest::Client::builder()
                    .no_proxy()
                    .connect_timeout(Duration::from_secs(10))
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
            usage: std::sync::Mutex::new(TokenUsage::default()),
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

    fn request_url(&self) -> String {
        if self
            .connection_options
            .as_ref()
            .is_some_and(|o| o.use_full_url)
        {
            self.config.base_url.clone()
        } else {
            format!(
                "{}/chat/completions",
                self.config.base_url.trim_end_matches('/')
            )
        }
    }

    fn request_timeout(&self) -> Duration {
        self.connection_options
            .as_ref()
            .and_then(|o| o.timeout_secs)
            .map(Duration::from_secs)
            .unwrap_or_else(model_request_timeout)
    }

    fn record_usage(&self, usage: &Value) {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens == 0 && parsed.prompt_tokens == 0 && parsed.completion_tokens == 0 {
            return;
        }
        if let Ok(mut current) = self.usage.lock() {
            current.add(&parsed);
        }
    }

    /// 读取环境变量预算并检查当前累计用量是否超限。
    fn usage_budget_check(&self) -> Option<String> {
        let total_cap = std::env::var("OWO_USAGE_TOKEN_BUDGET")
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        let cost_cap = std::env::var("OWO_USAGE_COST_BUDGET_USD")
            .ok()
            .and_then(|value| value.parse::<f64>().ok());
        if total_cap.is_none() && cost_cap.is_none() {
            return None;
        }
        let input_price = std::env::var("OWO_MODEL_INPUT_PRICE_PER_MTOK")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let output_price = std::env::var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let usage = self.usage.lock().map(|usage| *usage).unwrap_or_default();
        budget_violation(&usage, total_cap, cost_cap, input_price, output_price)
    }

    /// 发送请求：优先代理客户端，失败自动切直连重试一次（多轮流式挂起时稳定）。
    ///
    /// `stream=true` 时不设置请求级总超时——reqwest 的 `.timeout()` 覆盖整个响应体
    /// 读取，SSE 长回答会被固定墙钟截断；流式由调用方的逐块空闲超时守护。
    async fn post_chat(
        &self,
        url: &str,
        body: &serde_json::Value,
        stream: bool,
        allow_fallback: bool,
    ) -> Result<reqwest::Response, String> {
        let mut last_error = String::new();
        let attempts: Vec<(&str, &reqwest::Client)> = {
            let mut list = Vec::with_capacity(2);
            let direct_first = std::env::var("OWO_MODEL_DIRECT_FIRST")
                .ok()
                .is_some_and(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes"
                    )
                });
            if direct_first {
                if let Some(direct) = &self.direct_client {
                    list.push(("direct", direct));
                }
            }
            list.push(("proxy", &self.client));
            if !direct_first {
                if let Some(direct) = &self.direct_client {
                    list.push(("direct", direct));
                }
            }
            list
        };
        for (label, client) in attempts
            .into_iter()
            .take(if allow_fallback { 2 } else { 1 })
        {
            let mut request = client.post(url).json(body);
            if !stream {
                request = request.timeout(self.request_timeout());
            }
            let request = if self.config.api_key.is_empty() {
                request
            } else {
                request.bearer_auth(&self.config.api_key)
            };
            // 联调诊断（RUST_LOG=owo_gateway=debug 可见）：请求画像不落任何凭据。
            tracing::debug!(
                target: "owo_gateway",
                url = %url,
                model = %body
                    .get("model")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?"),
                tools = body
                    .get("tools")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0),
                stream = body
                    .get("stream")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                channel = label,
                "模型网关请求"
            );
            match send_response_headers(request, self.request_timeout()).await {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let mut text = bounded_error_body(response).await;
                    if !self.config.api_key.is_empty() {
                        text = text.replace(&self.config.api_key, "[REDACTED]");
                    }
                    tracing::debug!(target: "owo_gateway", status = %status, error_body_bytes = text.len(), "model request rejected");
                    return Err(format!("模型返回 {status}：{text}"));
                }
                Err(error) => {
                    last_error = format!("{label}: {error}");
                }
            }
        }
        Err(format!("模型请求失败：{last_error}"))
    }

    /// 数据出境开关：优先读运行时环境变量（支持设置页即时切换），缺省用启动配置。
    pub(super) fn cloud_enabled(&self) -> bool {
        if is_local_endpoint(&self.config.base_url) {
            return true;
        }
        std::env::var("OWO_CLOUD_ENABLED")
            .ok()
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(self.config.cloud_enabled)
    }

    /// 当前模型：优先读运行时环境变量（支持设置页热切换），缺省用启动配置。
    fn model(&self) -> String {
        if self.connection_options.is_some() {
            return self.config.model.clone();
        }
        std::env::var("OPENAI_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.config.model.clone())
    }

    /// 请求体 `model` 字段取值（M4.2）：请求级覆盖优先；覆盖为空串或 `"default"`
    /// 哨兵时回退 [`model`](Self::model) 解析链——哨兵值绝不泄漏进请求体。
    fn effective_model(&self, model: Option<&str>) -> String {
        match model.map(str::trim) {
            Some(override_model)
                if !override_model.is_empty() && override_model != MODEL_DEFAULT_SENTINEL =>
            {
                override_model.to_string()
            }
            _ => self.model(),
        }
    }

    pub(super) fn request_body(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        stream: bool,
    ) -> Value {
        self.request_body_with_options(model, messages, tools, stream, None, false)
    }

    /// 请求体构造（带内部重试用的覆盖项）：`max_tokens_override` 用于"仅思考触顶"
    /// 后翻倍预算；`force_low_reasoning` 用于同时把 glm-5 系列降到最低思考档。
    fn request_body_with_options(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        stream: bool,
        max_tokens_override: Option<u64>,
        force_low_reasoning: bool,
    ) -> Value {
        let tool_payload: Vec<Value> = tools
            .iter()
            .map(|spec| {
                json!({
                    "type": "function",
                    "function": {
                        "name": spec.name,
                        "description": spec.description,
                        "parameters": spec.input_schema,
                    }
                })
            })
            .collect();
        let messages_payload: Vec<Value> = messages
            .iter()
            .map(|message| {
                let mut wire = json!({
                    "role": message.role,
                    "content": message.content,
                });
                // 多模态（取优合并自远端 engine）：user 消息带图片时，content 转成
                // parts 数组（OpenAI 兼容格式：text + image_url）。无图片时保持纯文本。
                if !message.images.is_empty() {
                    let mut parts: Vec<Value> = Vec::new();
                    if let Some(text) = &message.content {
                        if !text.is_empty() {
                            parts.push(json!({ "type": "text", "text": text }));
                        }
                    }
                    for image in &message.images {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": { "url": image.url },
                        }));
                    }
                    wire["content"] = Value::Array(parts);
                }
                if let Some(tool_call_id) = &message.tool_call_id {
                    wire["tool_call_id"] = Value::String(tool_call_id.clone());
                }
                if let Some(tool_calls) = &message.tool_calls {
                    let wire_calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": serde_json::to_string(&call.arguments)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                }
                            })
                        })
                        .collect();
                    wire["tool_calls"] = Value::Array(wire_calls);
                }
                wire
            })
            .collect();

        let effective_model = self.effective_model(model);
        let mut body = json!({
            "model": effective_model.clone(),
            "messages": messages_payload,
            "stream": stream,
        });
        if !tool_payload.is_empty() {
            body["tools"] = Value::Array(tool_payload);
        }
        if stream {
            body["stream_options"] = json!({ "include_usage": true });
        }
        body["max_tokens"] = Value::from(
            max_tokens_override.unwrap_or_else(|| max_output_tokens_for_model(&effective_model)),
        );
        if let Some(temperature) = self
            .connection_options
            .as_ref()
            .and_then(|o| o.temperature)
            .or_else(temperature_from_env)
        {
            body["temperature"] = json!(temperature);
        }
        // 推理档位只在用户显式选择时才下发（默认请求体与旧版完全一致）；
        // 按有效模型归一化，避免把端点不支持的档位发出去直接 400。
        let effort =
            if force_low_reasoning && effective_model.to_ascii_lowercase().contains("glm-5") {
                Some("low".to_string())
            } else {
                reasoning_effort_for_model(&effective_model)
            };
        if let Some(effort) = effort {
            body["reasoning_effort"] = Value::String(effort);
        }
        body
    }
}

/// 推理档位（`reasoning_effort`，取优合并自远端 engine）：读运行时环境变量
/// （设置页保存后即时生效），并按有效模型归一化到端点真正接受的取值。
///
/// 背景（真实模型实测）：`glm-5.x` 系列「始终思考」，只接受 `low`/`high`/`max`；
/// 发送 `minimal`/`medium` 会被端点以 400 拒绝（错误码 1210），导致用户一旦在设置
/// 里选 minimal 就每次请求必失败。这里把设置档位映射到模型支持的档位：
/// minimal/low → low，medium/high → high。其他 OpenAI 兼容端点保持原值透传；
/// 未设置或非法取值返回 None = 不发送该参数。
fn reasoning_effort_for_model(model: &str) -> Option<String> {
    let value = std::env::var("OWO_REASONING_EFFORT")
        .ok()?
        .trim()
        .to_ascii_lowercase();
    if model.to_ascii_lowercase().contains("glm-5") {
        return match value.as_str() {
            "minimal" | "low" => Some("low".to_string()),
            "medium" | "high" => Some("high".to_string()),
            "max" => Some("max".to_string()),
            _ => None,
        };
    }
    if matches!(value.as_str(), "minimal" | "low" | "medium" | "high") {
        Some(value)
    } else {
        None
    }
}

/// Resolve the response budget against the effective model, then use the configured default.
/// Model-specific values allow one endpoint to host models with different output limits.
fn max_output_tokens_for_model(model: &str) -> u64 {
    let by_model = std::env::var("OWO_MODEL_OUTPUT_TOKENS_BY_MODEL")
        .ok()
        .and_then(|value| serde_json::from_str::<serde_json::Map<String, Value>>(&value).ok())
        .and_then(|values| values.get(model).and_then(Value::as_u64))
        .filter(|value| (1..=MAX_MODEL_OUTPUT_TOKENS).contains(value));
    if let Some(value) = by_model {
        return value;
    }
    std::env::var("OWO_MODEL_MAX_OUTPUT_TOKENS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (1..=MAX_MODEL_OUTPUT_TOKENS).contains(value))
        .unwrap_or(DEFAULT_MODEL_OUTPUT_TOKENS)
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    async fn complete_expression(&self, messages: &[ChatMessage]) -> Result<String, String> {
        let result = self.complete_once(None, messages, &[], Some(1024)).await?;
        match result.output {
            ModelOutput::Text(text) if !text.trim().is_empty() && text.chars().count() <= 4096 => {
                Ok(text)
            }
            _ => Err("provider/expression_invalid_output".into()),
        }
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
        self.complete_with_model_observed(model, messages, tools)
            .await
            .map(|observed| observed.output)
    }

    async fn complete_with_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ObservedModelOutput, String> {
        self.complete_once(model, messages, tools, None).await
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
        // 兼容入口：只转发正文增量（思考通道经 reasoning 变体消费）。
        let mut forward = |chunk: StreamChunk| {
            if let StreamChunk::Content(text) = chunk {
                on_delta(text);
            }
        };
        self.stream_completion(model, messages, tools, &mut forward)
            .await
    }

    async fn complete_stream_with_reasoning(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.stream_completion(None, messages, tools, on_chunk)
            .await
    }

    async fn complete_stream_with_reasoning_and_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.stream_completion(model, messages, tools, on_chunk)
            .await
    }

    async fn complete_stream_with_reasoning_and_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ObservedModelOutput, String> {
        self.stream_completion_observed(model, messages, tools, on_chunk)
            .await
    }

    fn usage_snapshot(&self) -> TokenUsage {
        self.usage.lock().map(|usage| *usage).unwrap_or_default()
    }
}

async fn send_response_headers(
    request: reqwest::RequestBuilder,
    deadline: Duration,
) -> Result<reqwest::Response, String> {
    match tokio::time::timeout(deadline, request.send()).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Err(format!("model transport failed: {}", error.without_url())),
        Err(_) => Err(format!(
            "模型响应头超时（provider/response_header_timeout，{}s）",
            deadline.as_secs()
        )),
    }
}

async fn bounded_error_body(mut response: reqwest::Response) -> String {
    const MAX_ERROR_BODY: usize = 64 * 1024;
    let body = tokio::time::timeout(model_request_timeout(), async move {
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| error.without_url())?
        {
            let remaining = MAX_ERROR_BODY.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if chunk.len() > remaining {
                bytes.extend_from_slice(b"\n[error body truncated]");
                break;
            }
        }
        Ok::<_, reqwest::Error>(String::from_utf8_lossy(&bytes).into_owned())
    })
    .await;
    match body {
        Ok(Ok(text)) => text,
        Ok(Err(_)) => "provider/error_body_read_failed".to_string(),
        Err(_) => "provider/error_body_timeout".to_string(),
    }
}

fn temperature_from_env() -> Option<f64> {
    std::env::var("OWO_MODEL_TEMPERATURE")
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .filter(|value| value.is_finite() && (0.0..=2.0).contains(value))
}

fn parse_timeout_secs(raw: Option<&str>, default_secs: u64) -> u64 {
    raw.and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1..=600).contains(value))
        .unwrap_or(default_secs)
}

fn duration_from_env(name: &str, default_secs: u64) -> Duration {
    Duration::from_secs(parse_timeout_secs(
        std::env::var(name).ok().as_deref(),
        default_secs,
    ))
}

fn model_request_timeout() -> Duration {
    let value = std::env::var("OWO_MODEL_REQUEST_TIMEOUT_SECS")
        .ok()
        .or_else(|| std::env::var("OWO_MODEL_TIMEOUT_SECS").ok());
    Duration::from_secs(
        value
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (1..=3600).contains(value))
            .unwrap_or(240),
    )
}

fn model_stream_idle_timeout() -> Duration {
    duration_from_env("OWO_MODEL_STREAM_IDLE_TIMEOUT_SECS", 60)
}

/// 非流式模型响应体的硬上限（异常端点/本地代理可能返回超大 body）。
/// 可用 `OWO_MODEL_MAX_RESPONSE_BYTES` 覆盖（1KB..=64MB），默认 16MB。
fn max_non_stream_response_bytes() -> usize {
    std::env::var("OWO_MODEL_MAX_RESPONSE_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1024..=64 * 1024 * 1024).contains(value))
        .unwrap_or(16 * 1024 * 1024)
}

/// 模型请求体上限：防止上下文/附件病态膨胀把请求打成不可控大小。
/// 可用 `OWO_MODEL_MAX_REQUEST_BYTES` 覆盖（1KB..=512MB），默认 64MB。
fn max_request_body_bytes() -> usize {
    std::env::var("OWO_MODEL_MAX_REQUEST_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1024..=512 * 1024 * 1024).contains(value))
        .unwrap_or(64 * 1024 * 1024)
}

pub(super) fn ensure_request_body_size(body: &Value) -> Result<(), String> {
    let max = max_request_body_bytes();
    let size = serde_json::to_vec(body)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    if size > max {
        return Err(format!(
            "模型请求体过大（{} KiB，上限 {} KiB）：请缩减上下文或附件",
            size / 1024,
            max / 1024
        ));
    }
    Ok(())
}

impl OpenAiCompatibleProvider {
    /// 流式补全唯一实现：正文与思考通道统一经 `on_chunk` 回调（类型区分）。
    async fn stream_completion(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.stream_completion_observed(model, messages, tools, on_chunk)
            .await
            .map(|observed| observed.output)
    }

    async fn stream_completion_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ObservedModelOutput, String> {
        if !self.cloud_enabled() {
            return Err("云端模型已禁用（数据出境开关关闭）".to_string());
        }
        if let Some(reason) = self.usage_budget_check() {
            return Err(reason);
        }
        // 真实模型实测：glm-5.x「始终思考」，复杂任务会把整个输出预算烧在思考上
        // （finish_reason=length 且正文为空），整轮直接失败。这里检测"仅思考触顶"并
        // 自动重试一次：输出预算翻倍（封顶 MAX），同时把推理档位降到模型最低档，
        // 让模型有机会真正产出正文/工具调用。正文未产生，重试不会重复用户可见内容。
        match self
            .stream_once(model, messages, tools, on_chunk, None, false)
            .await
        {
            Err(error) if error == REASONING_ONLY_LENGTH => {
                let base = max_output_tokens_for_model(&self.effective_model(model));
                let boosted = base.saturating_mul(2).min(MAX_MODEL_OUTPUT_TOKENS);
                match self
                    .stream_once(model, messages, tools, on_chunk, Some(boosted), true)
                    .await
                {
                    Err(error) if error == REASONING_ONLY_LENGTH => Err(format!(
                        "模型输出达到 max_tokens 上限（finish_reason=length，仅思考、无正文/工具调用）；已自动把输出预算提升到 {boosted} 并降低推理档位后仍失败，请提高 OWO_MODEL_MAX_OUTPUT_TOKENS 或缩小单次任务"
                    )),
                    other => other,
                }
            }
            other => other,
        }
    }

    /// 单次流式请求（含 finish_reason 检查）。仅思考触顶返回内部哨兵错误，
    /// 由调用方决定是否提升预算重试；正文/工具调用触顶仍按原错误返回。
    async fn stream_once(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
        max_tokens_override: Option<u64>,
        force_low_reasoning: bool,
    ) -> Result<ObservedModelOutput, String> {
        let body = self.request_body_with_options(
            model,
            messages,
            tools,
            true,
            max_tokens_override,
            force_low_reasoning,
        );
        ensure_request_body_size(&body)?;
        let url = self.request_url();
        let request_model = body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);
        let response = self.post_chat(&url, &body, true, true).await?;
        let request_id = ["x-request-id", "request-id", "openai-request-id"]
            .iter()
            .find_map(|name| response.headers().get(*name))
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        let mut stream = response.bytes_stream();
        let mut request_usage = None;
        let mut state = StreamState {
            request_id,
            ..StreamState::default()
        };

        let idle_timeout = self
            .connection_options
            .as_ref()
            .and_then(|o| o.timeout_secs)
            .map(Duration::from_secs)
            .unwrap_or_else(model_stream_idle_timeout);
        let mut last_progress = tokio::time::Instant::now();
        while let Some(chunk) = tokio::time::timeout(
            idle_timeout.saturating_sub(last_progress.elapsed()),
            stream.next(),
        )
        .await
        .map_err(|_| {
            format!(
                "provider/stream_progress_timeout: no model progress for {}s",
                idle_timeout.as_secs()
            )
        })? {
            let chunk = chunk.map_err(|e| format!("流式读取失败：{e:?}"))?;
            if chunk.len() > 8 * 1024 * 1024 {
                return Err("provider/stream_chunk_too_large".into());
            }
            state.semantic_progress = false;
            append_utf8_chunk(&mut state.buffer, &mut state.utf8_pending, &chunk);
            if state
                .buffer
                .split('\n')
                .any(|line| line.len() > MAX_STREAM_LINE_BYTES)
            {
                return Err("provider/stream_frame_too_large".into());
            }
            if let Some(usage) = consume_stream_buffer(&mut state, on_chunk) {
                request_usage = Some(usage);
                self.record_usage(&json!({
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                }));
                // R9：流式路径每块检查预算，超限立即停轮并返回可读错误。
                if let Some(reason) = self.usage_budget_check() {
                    return Err(reason);
                }
            }
            state.validate_resource_bounds()?;
            if state.semantic_progress {
                last_progress = tokio::time::Instant::now();
            }
            if state.saw_done {
                break;
            }
        }

        if !state.utf8_pending.is_empty() {
            state
                .buffer
                .push_str(&String::from_utf8_lossy(&state.utf8_pending));
        }
        if !state.buffer.trim().is_empty() {
            state.buffer.push('\n');
            if let Some(usage) = consume_stream_buffer(&mut state, on_chunk) {
                request_usage = Some(usage);
                self.record_usage(&json!({
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                }));
                if let Some(reason) = self.usage_budget_check() {
                    return Err(reason);
                }
            }
        }

        if !state.saw_sse {
            return Err("模型流式响应为空或不是 SSE 格式".to_string());
        }
        if state.finish_reason.as_deref() == Some("length") {
            let has_content = !state.content.trim().is_empty();
            let has_tool_calls = !state.tool_call_accumulators.is_empty();
            if !has_content && !has_tool_calls {
                return Err(REASONING_ONLY_LENGTH.to_string());
            }
            return Err("模型输出达到 max_tokens 上限（finish_reason=length），请提高 OWO_MODEL_MAX_OUTPUT_TOKENS 或缩小单次任务".to_string());
        }
        if !state.saw_done
            && !matches!(
                state.finish_reason.as_deref(),
                Some("stop" | "tool_calls" | "function_call" | "content_filter")
            )
        {
            return Err("模型流式响应未收到结束标记，且缺少正常 finish_reason".to_string());
        }

        let output = if let Some(tool_calls) = build_tool_calls(&mut state.tool_call_accumulators)?
        {
            ModelOutput::ToolCalls(validate_tool_calls(tool_calls, tools)?)
        } else {
            ModelOutput::Text(state.content)
        };
        Ok(ObservedModelOutput {
            output,
            metadata: ModelCallMetadata {
                request_id: state.request_id,
                model: state.response_model.or(request_model),
                usage: request_usage,
                latency_ms: None,
            },
        })
    }
}

#[cfg(test)]
mod timeout_config_tests {
    use super::parse_timeout_secs;

    #[test]
    fn timeout_values_are_bounded_and_fall_back_safely() {
        assert_eq!(parse_timeout_secs(None, 240), 240);
        assert_eq!(parse_timeout_secs(Some("90"), 240), 90);
        assert_eq!(parse_timeout_secs(Some("0"), 240), 240);
        assert_eq!(parse_timeout_secs(Some("601"), 240), 240);
        assert_eq!(parse_timeout_secs(Some("invalid"), 240), 240);
    }
}

#[cfg(test)]
mod stream_wait_contract_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn response_header_timeout_is_independent_of_stream_body() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            std::future::pending::<()>().await;
        });
        let request = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{address}/chat/completions"))
            .json(&json!({}));
        let result = send_response_headers(request, Duration::from_millis(50)).await;
        server.abort();
        assert!(result
            .unwrap_err()
            .contains("provider/response_header_timeout"));
    }

    #[tokio::test]
    async fn done_marker_completes_without_waiting_for_connection_eof() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:X}\r\n{}\r\n",
                body.len(), body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            std::future::pending::<()>().await;
        });
        let provider = OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
            base_url: format!("http://{address}"),
            api_key: String::new(),
            model: "fixture-model".into(),
            cloud_enabled: false,
        })
        .unwrap();
        let mut chunks = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            provider.stream_completion_observed(
                None,
                &[ChatMessage::user("fixture".to_string())],
                &[],
                &mut |chunk| chunks.push(chunk),
            ),
        )
        .await;
        server.abort();
        let result = result.expect("DONE must not require EOF").unwrap();
        assert!(matches!(result.output, ModelOutput::Text(ref text) if text == "ok"));
        assert!(!chunks.is_empty());
    }
}

impl OpenAiCompatibleProvider {
    async fn complete_once(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        output_cap: Option<u64>,
    ) -> Result<ObservedModelOutput, String> {
        let started = std::time::Instant::now();
        if !self.cloud_enabled() {
            return Err("云端模型已禁用（数据出境开关关闭）".to_string());
        }
        if let Some(reason) = self.usage_budget_check() {
            return Err(reason);
        }
        let body = match output_cap {
            Some(cap) => {
                self.request_body_with_options(model, messages, tools, false, Some(cap), false)
            }
            None => self.request_body(model, messages, tools, false),
        };
        ensure_request_body_size(&body)?;
        let request_model = body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);
        let url = self.request_url();
        let response = self
            .post_chat(&url, &body, false, output_cap.is_none())
            .await?;
        let request_id = ["x-request-id", "request-id", "openai-request-id"]
            .iter()
            .find_map(|name| response.headers().get(*name))
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        // 非流式响应也设硬上限：异常端点/代理返回超大 body 时不能整块读进内存。
        let max_bytes = if output_cap.is_some() {
            64 * 1024
        } else {
            max_non_stream_response_bytes()
        };
        let mut bytes: Vec<u8> = Vec::with_capacity(64 * 1024);
        let mut oversized = false;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("读取模型响应失败：{e}"))?;
            if bytes.len() + chunk.len() > max_bytes {
                oversized = true;
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        if oversized {
            return Err(format!(
                "模型响应超过上限（{max_bytes} bytes），已拒绝解析；可用 OWO_MODEL_MAX_RESPONSE_BYTES 调整"
            ));
        }
        let payload: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("模型响应解析失败：{e}"))?;
        let usage_value = payload.get("usage");
        self.record_usage(usage_value.unwrap_or(&Value::Null));
        let usage = usage_value
            .filter(|value| {
                let has_prompt = value
                    .get("prompt_tokens")
                    .or_else(|| value.get("prompt_eval_count"))
                    .and_then(Value::as_u64)
                    .is_some();
                let has_completion = value
                    .get("completion_tokens")
                    .or_else(|| value.get("eval_count"))
                    .and_then(Value::as_u64)
                    .is_some();
                has_prompt && has_completion
            })
            .map(parse_usage_value);
        let response_model = payload
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);
        let message = payload
            .pointer("/choices/0/message")
            .ok_or_else(|| "响应缺少 choices[0].message".to_string())?;
        // 非流式同样要拒绝截断输出：finish_reason=length 时 content/tool_calls 都可能是
        // 半截内容，静默当最终答案会把"被截断的 JSON/补丁"带进后续流程。
        if payload
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            == Some("length")
        {
            return Err("模型输出达到 max_tokens 上限（finish_reason=length），请提高 OWO_MODEL_MAX_OUTPUT_TOKENS 或缩小单次任务".to_string());
        }
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string);
        let tool_calls = parse_tool_calls(message, tools)?;
        let output = if let Some(tool_calls) = tool_calls {
            ModelOutput::ToolCalls(tool_calls)
        } else if let Some(content) = content {
            ModelOutput::Text(content)
        } else {
            return Err("模型响应既无文本也无工具调用".to_string());
        };
        Ok(ObservedModelOutput {
            output,
            metadata: ModelCallMetadata {
                request_id,
                model: response_model.or(request_model),
                usage,
                latency_ms: Some(started.elapsed().as_millis() as u64),
            },
        })
    }
}
