use serde_json::Value;
use std::collections::HashMap;

use super::message::*;
#[derive(Debug, Default, Clone, PartialEq)]
pub struct StreamDelta {
    pub request_id: Option<String>,
    pub model: Option<String>,
    pub finish_reason: Option<String>,
    pub content: Option<String>,
    /// 思考通道增量（`reasoning_content`，GLM/DeepSeek 约定）；不写入对话历史。
    pub reasoning: Option<String>,
    /// 原始 tool_calls 增量片段（JSON 值）。
    pub tool_call_fragments: Vec<Value>,
    /// 末尾 usage 块（OpenAI-compatible 流式响应在最后一条 data 中给出）。
    pub usage: Option<TokenUsage>,
}

/// 解析一条 `data:` 负载。空负载/心跳返回 None。
pub fn parse_sse_payload(payload: &str) -> Option<StreamDelta> {
    let payload = payload.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    // usage 尾帧允许 choices=[] 或缺省；不能先要求 choices/0/delta。
    let delta = value.pointer("/choices/0/delta").unwrap_or(&Value::Null);
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    let content = delta
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // 思考通道（取优合并自远端 engine）：GLM/DeepSeek 的 reasoning_content。
    let reasoning = delta
        .get("reasoning_content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let tool_call_fragments = delta
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let usage = value
        .get("usage")
        .map(parse_usage_value)
        .filter(|usage| usage.total_tokens > 0 || usage.prompt_tokens > 0);
    let request_id = value.get("id").and_then(Value::as_str).map(str::to_string);
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    if content.is_none()
        && reasoning.is_none()
        && tool_call_fragments.is_empty()
        && usage.is_none()
        && request_id.is_none()
        && model.is_none()
        && finish_reason.is_none()
    {
        return None;
    }
    Some(StreamDelta {
        request_id,
        model,
        finish_reason,
        content,
        reasoning,
        tool_call_fragments,
        usage,
    })
}

/// 单个工具调用累计参数上限：异常/被劫持端点可能无界推送 arguments，
/// 超出即标记截断并在构建时显式报错（不静默解析半截 JSON）。
const MAX_TOOL_CALL_ARGUMENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
    truncated: bool,
}

pub(super) fn accumulate_tool_fragments(
    accumulators: &mut HashMap<usize, ToolCallAccumulator>,
    fragments: &[Value],
) {
    for fragment in fragments {
        let Some(index) = fragment.get("index").and_then(Value::as_u64) else {
            continue;
        };
        let index = index as usize;
        let entry = accumulators.entry(index).or_default();
        if let Some(id) = fragment.get("id").and_then(Value::as_str) {
            entry.id = id.to_string();
        }
        if let Some(name) = fragment.pointer("/function/name").and_then(Value::as_str) {
            entry.name = name.to_string();
        }
        if let Some(arguments) = fragment
            .pointer("/function/arguments")
            .and_then(Value::as_str)
        {
            if entry.arguments.len() + arguments.len() > MAX_TOOL_CALL_ARGUMENT_BYTES {
                entry.truncated = true;
            } else {
                entry.arguments.push_str(arguments);
            }
        }
    }
}

pub(super) fn build_tool_calls(
    accumulators: &mut HashMap<usize, ToolCallAccumulator>,
) -> Result<Option<Vec<ToolCall>>, String> {
    if accumulators.is_empty() {
        return Ok(None);
    }
    if let Some(accum) = accumulators.values().find(|accum| accum.truncated) {
        return Err(format!(
            "模型返回的工具 {} 参数超过上限（{} MiB），已拒绝解析",
            accum.name,
            MAX_TOOL_CALL_ARGUMENT_BYTES / (1024 * 1024)
        ));
    }
    let mut calls: Vec<(usize, ToolCall)> = accumulators
        .drain()
        .map(|(index, accum)| {
            let arguments = if accum.arguments.trim().is_empty() {
                Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(&accum.arguments).map_err(|error| {
                    format!("模型返回的工具 {} 参数不是有效 JSON：{error}", accum.name)
                })?
            };
            Ok((
                index,
                ToolCall {
                    id: if accum.id.is_empty() {
                        format!("call_{index}")
                    } else {
                        accum.id
                    },
                    name: accum.name,
                    arguments,
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    calls.sort_by_key(|(index, _)| *index);
    Ok(Some(calls.into_iter().map(|(_, call)| call).collect()))
}

pub(super) fn append_utf8_chunk(buffer: &mut String, pending: &mut Vec<u8>, chunk: &[u8]) {
    pending.extend_from_slice(chunk);
    match String::from_utf8(std::mem::take(pending)) {
        Ok(text) => buffer.push_str(&text),
        Err(error) => {
            let bytes = error.into_bytes();
            let valid = std::str::from_utf8(&bytes)
                .map(|_| bytes.len())
                .unwrap_or_else(|error| error.valid_up_to());
            buffer.push_str(std::str::from_utf8(&bytes[..valid]).unwrap_or_default());
            pending.extend_from_slice(&bytes[valid..]);
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct StreamState {
    pub(super) buffer: String,
    pub(super) utf8_pending: Vec<u8>,
    pub(super) content: String,
    pub(super) tool_call_accumulators: HashMap<usize, ToolCallAccumulator>,
    pub(super) saw_sse: bool,
    pub(super) request_id: Option<String>,
    pub(super) response_model: Option<String>,
    pub(super) finish_reason: Option<String>,
    pub(super) saw_done: bool,
    pub(super) semantic_progress: bool,
}

impl StreamState {
    pub(super) fn validate_resource_bounds(&self) -> Result<(), String> {
        if self.buffer.len().saturating_add(self.utf8_pending.len()) > 1024 * 1024 {
            return Err("provider/stream_frame_too_large".into());
        }
        if self.content.len() > 32 * 1024 * 1024 {
            return Err("provider/stream_content_too_large".into());
        }
        let argument_bytes = self
            .tool_call_accumulators
            .values()
            .fold(0usize, |size, call| {
                size.saturating_add(call.arguments.len())
                    .saturating_add(call.id.len())
                    .saturating_add(call.name.len())
            });
        if self.tool_call_accumulators.len() > 1024 || argument_bytes > 16 * 1024 * 1024 {
            return Err("provider/stream_tool_arguments_too_large".into());
        }
        Ok(())
    }
}

pub(super) fn consume_stream_buffer(
    state: &mut StreamState,
    on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
) -> Option<TokenUsage> {
    let mut usage = None;
    while let Some(newline) = state.buffer.find('\n') {
        let line = state.buffer[..newline].trim().to_string();
        state.buffer.drain(..=newline);
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        state.saw_sse = true;
        if payload.trim() == "[DONE]" {
            state.saw_done = true;
            state.semantic_progress = true;
            continue;
        }
        if let Some(delta) = parse_sse_payload(payload) {
            state.semantic_progress |= delta.content.is_some()
                || delta.reasoning.is_some()
                || delta.tool_call_fragments.iter().any(|fragment| {
                    ["/id", "/function/name", "/function/arguments"]
                        .iter()
                        .any(|path| {
                            fragment
                                .pointer(path)
                                .and_then(Value::as_str)
                                .is_some_and(|text| !text.is_empty())
                        })
                })
                || delta.usage.is_some()
                || delta.finish_reason.is_some();
            if delta.request_id.is_some() {
                state.request_id = delta.request_id;
            }
            if delta.model.is_some() {
                state.response_model = delta.model;
            }
            if delta.finish_reason.is_some() {
                state.finish_reason = delta.finish_reason;
            }
            if delta.usage.is_some() {
                usage = delta.usage;
            }
            if let Some(delta_content) = delta.content {
                state.content.push_str(&delta_content);
                on_chunk(StreamChunk::Content(delta_content));
            }
            // 思考通道（GLM/DeepSeek）：与正文分开回调，由上层决定展示方式。
            if let Some(reasoning) = delta.reasoning {
                on_chunk(StreamChunk::Reasoning(reasoning));
            }
            accumulate_tool_fragments(
                &mut state.tool_call_accumulators,
                &delta.tool_call_fragments,
            );
        }
    }
    usage
}

#[cfg(test)]
mod progress_boundary_tests {
    use super::*;
    #[test]
    fn heartbeat_and_metadata_do_not_count_as_model_progress() {
        let mut state = StreamState {
            buffer: ": keep-alive\ndata: {\"id\":\"request-1\",\"model\":\"fixture\"}\n".into(),
            ..Default::default()
        };
        consume_stream_buffer(&mut state, &mut |_| {});
        assert!(!state.semantic_progress);
        state.buffer =
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n".into();
        consume_stream_buffer(&mut state, &mut |_| {});
        assert!(state.semantic_progress);
    }
    #[test]
    fn unterminated_frames_invalid_utf8_and_accumulated_output_are_bounded() {
        let mut state = StreamState {
            buffer: "x".repeat(1024 * 1024 + 1),
            ..Default::default()
        };
        assert_eq!(
            state.validate_resource_bounds().unwrap_err(),
            "provider/stream_frame_too_large"
        );
        state.buffer.clear();
        state.utf8_pending = vec![255; 1024 * 1024 + 1];
        assert!(state.validate_resource_bounds().is_err());
        state.utf8_pending.clear();
        state.content = "x".repeat(32 * 1024 * 1024 + 1);
        assert_eq!(
            state.validate_resource_bounds().unwrap_err(),
            "provider/stream_content_too_large"
        );
    }
}
