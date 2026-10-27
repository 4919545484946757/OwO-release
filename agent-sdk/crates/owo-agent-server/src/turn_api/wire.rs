use std::convert::Infallible;

use axum::response::sse::Event;
use owo_agent_protocol::SseEvent;
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) fn to_sse(event: &owo_agent_core::TurnEvent) -> Option<SseEvent> {
    match event {
        owo_agent_core::TurnEvent::ModelCall => Some(SseEvent::Progress {
            message: "模型调用".to_string(),
        }),
        owo_agent_core::TurnEvent::TokenDelta { delta } => Some(SseEvent::TokenDelta {
            delta: delta.clone(),
        }),
        owo_agent_core::TurnEvent::ReasoningDelta { delta } => Some(SseEvent::ReasoningDelta {
            delta: delta.clone(),
        }),
        owo_agent_core::TurnEvent::PlanUpdate { steps } => Some(SseEvent::PlanUpdate {
            steps: steps.clone(),
        }),
        owo_agent_core::TurnEvent::Compaction { summary } => Some(SseEvent::Compaction {
            summary: summary.clone(),
        }),
        owo_agent_core::TurnEvent::PermissionRequest(request) => {
            let explain = owo_agent_core::permissions::describe_request(request);
            Some(SseEvent::PermissionRequest {
                request_id: request.request_id.clone(),
                tool: request.tool.clone(),
                args: request.args.clone(),
                reason: request.reason.clone(),
                redacted_args: request.redacted_args.clone(),
                level: Some(request.level.label().to_string()),
                risk_note: request.risk_note.clone(),
                explain: Some(explain),
            })
        }
        owo_agent_core::TurnEvent::ToolStart {
            id,
            tool,
            args_preview,
        } => Some(SseEvent::ToolUse {
            id: id.clone(),
            tool: tool.clone(),
            args: args_preview
                .as_ref()
                .map(|preview| Value::String(preview.clone()))
                .unwrap_or(Value::Null),
        }),
        owo_agent_core::TurnEvent::ToolResult {
            id,
            tool,
            ok,
            error,
            preview,
            outputs,
            ..
        } => Some(SseEvent::ToolResult {
            id: id.clone(),
            tool: tool.clone(),
            ok: *ok,
            error: error.clone(),
            // 步骤 chip 展开区展示结果正文（core 已按属性截断到 1600 字符）。
            preview: preview.clone(),
            outputs: outputs.clone(),
        }),
        owo_agent_core::TurnEvent::Final { text } => Some(SseEvent::Final { text: text.clone() }),
    }
}

pub(crate) fn to_event(seq: Option<u64>, sse: SseEvent) -> Result<Event, Infallible> {
    let name = match &sse {
        SseEvent::Progress { .. } => "progress",
        SseEvent::ToolUse { .. } => "tool_use",
        SseEvent::ToolResult { .. } => "tool_result",
        SseEvent::PermissionRequest { .. } => "permission_request",
        SseEvent::Final { .. } => "final",
        SseEvent::TokenDelta { .. } => "token_delta",
        SseEvent::Compaction { .. } => "compaction",
        SseEvent::PermissionResolved { .. } => "permission_resolved",
        SseEvent::ReasoningDelta { .. } => "reasoning_delta",
        SseEvent::PlanUpdate { .. } => "plan_update",
        SseEvent::UserQuestion { .. } => "user_question",
        SseEvent::UserAnswered { .. } => "user_answered",
        SseEvent::TurnFailed { .. } => "turn_failed",
        SseEvent::TurnStats { .. } => "turn_stats",
    };
    // R10：SSE 事件统一携带协议版本 v（见 protocol::SSE_PROTOCOL_VERSION）。
    let mut payload = serde_json::to_value(&sse).unwrap_or_else(|_| json!({}));
    if let serde_json::Value::Object(map) = &mut payload {
        map.insert(
            "v".to_string(),
            json!(owo_agent_protocol::SSE_PROTOCOL_VERSION),
        );
    }
    let data = payload.to_string();
    let event = Event::default().event(name).data(data);
    Ok(match seq {
        Some(seq) => event.id(seq.to_string()),
        None => event,
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct TurnEventsQuery {
    pub(crate) turn_id: String,
    #[serde(default)]
    pub(crate) after_seq: u64,
    pub(crate) limit: Option<usize>,
}

pub(crate) fn is_turn_failed_event(event: &SseEvent) -> bool {
    // 取优合并：远端 engine 的显式 TurnFailed 终态优先；旧的 Progress 前缀
    // 仍识别（历史持久化事件回放兼容）。
    matches!(event, SseEvent::TurnFailed { .. })
        || matches!(event, SseEvent::Progress { message }
            if message.starts_with("turn failed:") || message.starts_with("session save failed:"))
}

#[cfg(test)]
mod media_tests {
    use super::*;
    #[test]
    fn tool_outputs_are_independent_of_the_truncated_human_preview() {
        let output = owo_agent_protocol::ToolOutputArtifact {
            artifact_id: "media-1".into(),
            kind: "image".into(),
            mime_type: "image/png".into(),
            bytes: 1024 * 1024,
            sha256: "abc".into(),
            path: "outputs/image.png".into(),
        };
        let event = owo_agent_core::TurnEvent::ToolResult {
            id: "call-1".into(),
            tool: "generate_image".into(),
            ok: true,
            error: None,
            preview: Some("{truncated".into()),
            outputs: vec![output.clone()],
            command_receipt: None,
        };
        let wire = to_sse(&event).unwrap();
        let serialized = serde_json::to_value(&wire).unwrap();
        assert_eq!(serialized["outputs"][0]["artifact_id"], output.artifact_id);
        let restored: SseEvent = serde_json::from_value(serialized).unwrap();
        assert!(
            matches!(restored, SseEvent::ToolResult { outputs, .. } if outputs == vec![output])
        );
        let legacy: SseEvent = serde_json::from_value(
            json!({"type":"tool_result","id":"old","tool":"read_file","ok":true}),
        )
        .unwrap();
        assert!(matches!(legacy, SseEvent::ToolResult { outputs, .. } if outputs.is_empty()));
    }
}
