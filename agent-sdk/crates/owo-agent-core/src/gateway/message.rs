use crate::tools::ToolSpec;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// 图片输入单元（多模态，取优合并自远端 engine）：URL（http/https）或 base64 data URL。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageImage {
    pub url: String,
}

impl MessageImage {
    pub fn from_url(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// 图片输入（多模态）：content 保持纯文本，provider 层在 images 非空且角色为
    /// user 时把 wire 内容转成 parts 数组（OpenAI: image_url）。
    /// 附加可选字段：老会话记录缺省视为空，向前兼容。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<MessageImage>,
}

impl ChatMessage {
    pub fn system(content: String) -> Self {
        Self {
            role: "system".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn user(content: String) -> Self {
        Self {
            role: "user".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }
    /// 带图片的用户消息（多模态：截图/贴图进主对话上下文）。
    pub fn user_with_images(content: String, images: Vec<MessageImage>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            images,
        }
    }

    pub fn assistant_text(content: String) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            images: Vec::new(),
        }
    }

    pub fn tool(tool_call_id: String, content: String) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(other.prompt_tokens);
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(other.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }

    /// 回合增量 = 当前快照 − 回合前快照（saturating）。
    pub fn saturating_sub(&self, other: &TokenUsage) -> TokenUsage {
        TokenUsage {
            prompt_tokens: self.prompt_tokens.saturating_sub(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_sub(other.completion_tokens),
            total_tokens: self.total_tokens.saturating_sub(other.total_tokens),
        }
    }

    /// 成本估算（美元）：价格按每百万 token 计，默认 0（未知价格不估算）。
    pub fn cost_estimate_usd(&self, input_per_mtok: f64, output_per_mtok: f64) -> f64 {
        self.prompt_tokens as f64 / 1_000_000.0 * input_per_mtok
            + self.completion_tokens as f64 / 1_000_000.0 * output_per_mtok
    }
}

/// 用量预算熔断：返回超限原因；未配置预算时返回 None。
///
/// 累计 token 上限（`OWO_USAGE_TOKEN_BUDGET`）与累计成本上限（美元，
/// `OWO_USAGE_COST_BUDGET_USD`，需配合单价环境变量）任一超限即熔断。
pub fn budget_violation(
    usage: &TokenUsage,
    total_tokens_cap: Option<u64>,
    cost_cap_usd: Option<f64>,
    input_price_per_mtok: f64,
    output_price_per_mtok: f64,
) -> Option<String> {
    if let Some(cap) = total_tokens_cap {
        if usage.total_tokens >= cap {
            return Some(format!(
                "模型用量预算已超限：累计 {} tokens ≥ 上限 {}",
                usage.total_tokens, cap
            ));
        }
    }
    if let Some(cap) = cost_cap_usd {
        let cost = usage.cost_estimate_usd(input_price_per_mtok, output_price_per_mtok);
        if cost >= cap {
            return Some(format!(
                "模型成本预算已超限：累计 ${cost:.6} ≥ 上限 ${cap:.6}"
            ));
        }
    }
    None
}

/// 从模型响应 usage 字段提取 token 用量（兼容 OpenAI/DeepSeek 与 Ollama 字段）。
pub fn parse_usage_value(usage: &Value) -> TokenUsage {
    if !usage.is_object() {
        return TokenUsage::default();
    }
    let prompt = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("prompt_eval_count").and_then(Value::as_u64))
        .unwrap_or(0);
    let completion = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("eval_count").and_then(Value::as_u64))
        .unwrap_or(0);
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(prompt.saturating_add(completion));
    TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelOutput {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// 流式增量块：正文（对用户可见的回答）或思考（深度思考过程，不写入对话历史）。
#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    Content(String),
    Reasoning(String),
}

/// Metadata returned for one provider request; contains no prompt or response content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallMetadata {
    pub request_id: Option<String>,
    pub model: Option<String>,
    pub usage: Option<TokenUsage>,
    /// Wall time spent in the provider call, recorded by instrumentation wrappers.
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObservedModelOutput {
    pub output: ModelOutput,
    pub metadata: ModelCallMetadata,
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Dedicated bounded text request. Providers must opt in; never fall back to an Agent turn.
    async fn complete_expression(&self, _messages: &[ChatMessage]) -> Result<String, String> {
        Err("provider/expression_unsupported".into())
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String>;

    /// 流式补全：文本增量经 `on_delta` 回调；返回最终输出。
    /// 默认实现退化为非流式。
    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        let output = self.complete(messages, tools).await?;
        if let ModelOutput::Text(text) = &output {
            on_delta(text.clone());
        }
        Ok(output)
    }

    /// 按请求覆盖模型（M4.2 会话级/档位路由）：`Some(model)` 时请求体使用该模型，
    /// `None`/空串/`"default"` 哨兵回退 Provider 自身解析链。默认实现忽略覆盖并
    /// 退化为 [`complete`](Self::complete)（测试桩/非 OpenAI Provider 零改动兼容）。
    async fn complete_with_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        let _ = model;
        self.complete(messages, tools).await
    }

    /// One non-streaming request with attributable metadata. Providers without native
    /// request accounting keep compatibility and report unknown metadata.
    async fn complete_with_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ObservedModelOutput, String> {
        let output = self.complete_with_model(model, messages, tools).await?;
        Ok(ObservedModelOutput {
            output,
            metadata: ModelCallMetadata::default(),
        })
    }

    /// 流式版按请求覆盖（语义同 [`complete_with_model`](Self::complete_with_model)）。
    /// 带思考通道的流式补全：正文与思考增量统一经 `on_chunk` 回调（类型区分）。
    /// 默认实现委托 `complete_stream`（不支持的 provider 自动兼容，思考块缺失）。
    async fn complete_stream_with_reasoning(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        let mut forward = |text: String| on_chunk(StreamChunk::Content(text));
        self.complete_stream(messages, tools, &mut forward).await
    }

    /// 带模型覆盖的思考通道流式补全（会话 `/model` 与思考展示并存）。
    /// 默认实现忽略思考通道，委托 `complete_stream_with_model`。
    async fn complete_stream_with_reasoning_and_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        let mut forward = |text: String| on_chunk(StreamChunk::Content(text));
        self.complete_stream_with_model(model, messages, tools, &mut forward)
            .await
    }

    async fn complete_stream_with_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        let _ = model;
        self.complete_stream(messages, tools, on_delta).await
    }

    /// 带单次请求元数据的流式模型调用。兼容 Provider 默认委托原接口并返回未知元数据；
    /// 原生支持的 Provider 应返回本次响应的 request id、请求模型和 usage。
    async fn complete_stream_with_reasoning_and_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ObservedModelOutput, String> {
        let output = self
            .complete_stream_with_reasoning_and_model(model, messages, tools, on_chunk)
            .await?;
        Ok(ObservedModelOutput {
            output,
            metadata: ModelCallMetadata::default(),
        })
    }

    /// 累计 token 用量快照（供回合增量统计；未实现的 Provider 返回零）。
    fn usage_snapshot(&self) -> TokenUsage {
        TokenUsage::default()
    }
}
