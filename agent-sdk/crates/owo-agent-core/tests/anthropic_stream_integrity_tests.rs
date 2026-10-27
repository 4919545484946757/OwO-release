//! Real local HTTP/SSE contracts; no cloud calls or process-environment changes.
use owo_agent_core::gateway::{
    custom_model_provider, ChatMessage, ModelOutput, ModelProvider, StreamChunk,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

const START: &str = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n";
const STOP: &str = "data: {\"type\":\"message_stop\"}\n\n";

enum Step {
    Bytes(Vec<u8>),
    Wait(Arc<Notify>),
    Hold,
}
fn bytes(text: impl AsRef<[u8]>) -> Step {
    Step::Bytes(text.as_ref().to_vec())
}
fn text_delta(text: &str) -> String {
    format!(
        "data: {}\n\n",
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}})
    )
}
fn prompt() -> Vec<ChatMessage> {
    vec![ChatMessage::user("local stream contract".into())]
}

struct Fixture {
    base: String,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl Fixture {
    async fn new(steps: Vec<Step>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
                assert!(request.len() < 8192);
            };
            let headers = std::str::from_utf8(&request[..header_end]).unwrap();
            assert!(headers.starts_with("POST /v1/messages HTTP/"));
            assert!(headers
                .to_ascii_lowercase()
                .contains("x-api-key: fixture-only"));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            assert!(length < 64 * 1024);
            while request.len() < header_end + length {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                request.extend_from_slice(&buf[..n]);
            }
            let body: Value =
                serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
            assert_eq!(body["stream"], true);
            assert_eq!(body["model"], "stream-fixture");
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
            for step in steps {
                match step {
                    Step::Bytes(data) => {
                        // Explicit HTTP chunk boundaries exercise the body decoder, not just feed().
                        let frame = format!("{:x}\r\n", data.len());
                        if socket.write_all(frame.as_bytes()).await.is_err()
                            || socket.write_all(&data).await.is_err()
                            || socket.write_all(b"\r\n").await.is_err()
                        {
                            return; // Expected when a negative case is rejected before EOF.
                        }
                    }
                    Step::Wait(notify) => {
                        tokio::time::timeout(std::time::Duration::from_secs(2), notify.notified())
                            .await
                            .expect(
                                "a complete event must reach its callback before the next bytes",
                            );
                    }
                    Step::Hold => {
                        // Keep HTTP open after the last event; Drop aborts this fixture.
                        std::future::pending::<()>().await;
                    }
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
            let _ = socket.shutdown().await;
        });
        Self {
            base,
            task: Some(task),
        }
    }
    fn provider(&self) -> Arc<dyn ModelProvider> {
        let connection = serde_json::from_value(json!({
            "api_format":"anthropic", "base_url":self.base, "model":"stream-fixture",
            "api_key":"fixture-only", "timeout_secs":1
        }))
        .unwrap();
        custom_model_provider(&connection).unwrap()
    }
    async fn finished(&mut self) {
        self.task.take().unwrap().await.unwrap();
    }
}

#[tokio::test]
async fn stream_error_is_failure_and_never_a_content_delta_or_leaked_credential() {
    let mut fixture = Fixture::new(vec![
        bytes(START), bytes(text_delta("partial")),
        bytes("event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"failure fixture-only\"}}\n\n"),
        bytes(STOP),
    ]).await;
    let mut content = String::new();
    let error = fixture
        .provider()
        .complete_stream_with_model(None, &prompt(), &[], &mut |delta| content.push_str(&delta))
        .await
        .unwrap_err();
    assert!(error.contains("provider/anthropic_stream_error"), "{error}");
    assert!(error.contains("overloaded_error"), "{error}");
    assert!(!error.contains("fixture-only"), "{error}");
    assert_eq!(content, "partial", "provider failures are not model text");
    fixture.finished().await;
}

#[tokio::test]
async fn clean_http_eof_cannot_complete_partial_text_or_executable_tool_calls() {
    let tools = owo_agent_core::tools::ToolRegistry::new().specs();
    for partial in [
        text_delta("partial"),
        format!(
            "data: {}\n\n",
            json!({"type":"content_block_start","index":0,"content_block":{
                "type":"tool_use","id":"tool-fixture","name":"read_file","input":{}
            }})
        ),
    ] {
        let mut fixture = Fixture::new(vec![bytes(START), bytes(partial)]).await;
        let error = fixture
            .provider()
            .complete_stream_with_model(None, &prompt(), &tools, &mut |_| {})
            .await
            .unwrap_err();
        assert!(error.contains("provider/stream_incomplete"), "{error}");
        fixture.finished().await;
    }
}

#[tokio::test]
async fn crlf_multiline_data_and_unknown_events_preserve_text_and_reasoning() {
    let mut fixture = Fixture::new(vec![
        bytes(": comment\r\nevent: message_start\r\ndata: {\r\ndata: \"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}\r\ndata: }\r\n\r\n"),
        bytes("data: {\"type\":\"future_event\",\"new_field\":true}\r\n\r\n"),
        bytes("data: {\"type\":\"ping\"}\r\n\r\n"),
        bytes("data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"分析\"}}\r\n\r\n"),
        bytes(text_delta("你好，世界 🌏").replace('\n', "\r\n")),
        bytes("data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":2}}\r\n\r\n"),
        bytes(STOP.replace('\n', "\r\n")),
    ]).await;
    let mut text = String::new();
    let mut reasoning = String::new();
    let observed = fixture
        .provider()
        .complete_stream_with_reasoning_and_model_observed(None, &prompt(), &[], &mut |chunk| {
            match chunk {
                StreamChunk::Content(delta) => text.push_str(&delta),
                StreamChunk::Reasoning(delta) => reasoning.push_str(&delta),
            }
        })
        .await
        .unwrap();
    assert_eq!(text, "你好，世界 🌏");
    assert_eq!(reasoning, "分析");
    assert!(matches!(observed.output, ModelOutput::Text(ref t) if t == &text));
    assert_eq!(observed.metadata.usage.unwrap().total_tokens, 3);
    fixture.finished().await;
}

#[tokio::test]
async fn completed_utf8_events_are_delivered_before_the_next_partial_character_finishes() {
    let delivered = Arc::new(Notify::new());
    let next = text_delta("世界");
    let split = next
        .as_bytes()
        .windows("世".len())
        .position(|w| w == "世".as_bytes())
        .unwrap()
        + 1;
    let mut first = START.as_bytes().to_vec();
    first.extend_from_slice(text_delta("你好").as_bytes());
    first.extend_from_slice(&next.as_bytes()[..split]);
    let mut fixture = Fixture::new(vec![
        bytes(first),
        Step::Wait(Arc::clone(&delivered)),
        bytes(&next.as_bytes()[split..split + 1]),
        bytes(&next.as_bytes()[split + 1..]),
        bytes(STOP),
    ])
    .await;
    let mut content = String::new();
    let output = fixture
        .provider()
        .complete_stream_with_model(None, &prompt(), &[], &mut |delta| {
            content.push_str(&delta);
            if content == "你好" {
                delivered.notify_one();
            }
        })
        .await
        .unwrap();
    assert!(matches!(output, ModelOutput::Text(ref text) if text == "你好世界"));
    assert_eq!(content, "你好世界");
    fixture.finished().await;
}

#[tokio::test]
async fn malformed_json_and_invalid_utf8_fail_instead_of_disappearing_from_the_stream() {
    for invalid in [
        b"data: {broken-json}\n\n".to_vec(),
        [
            b"data: {\"type\":\"ping\",\"bad\":\"".as_slice(),
            &[0xff],
            b"\"}\n\n",
        ]
        .concat(),
    ] {
        let mut fixture = Fixture::new(vec![
            bytes(START),
            bytes(invalid),
            bytes(text_delta("ok")),
            bytes(STOP),
        ])
        .await;
        let error = fixture
            .provider()
            .complete_stream_with_model(None, &prompt(), &[], &mut |_| {})
            .await
            .unwrap_err();
        assert!(
            error.contains("provider/stream_invalid_json")
                || error.contains("provider/stream_invalid_utf8"),
            "{error}"
        );
        fixture.finished().await;
    }
}

#[tokio::test]
async fn oversized_unterminated_sse_frame_is_rejected_before_the_peer_closes() {
    let mut oversized = b"data: ".to_vec();
    oversized.extend(std::iter::repeat_n(b'x', 1024 * 1024));
    let fixture = Fixture::new(vec![bytes(START), bytes(oversized), Step::Hold]).await;
    let error = fixture
        .provider()
        .complete_stream_with_model(None, &prompt(), &[], &mut |_| {})
        .await
        .unwrap_err();
    assert!(error.contains("provider/stream_frame_too_large"), "{error}");
}

#[tokio::test]
async fn message_stop_finishes_without_waiting_for_the_http_connection_to_close() {
    let fixture = Fixture::new(vec![
        bytes(START),
        bytes(text_delta("done")),
        bytes(STOP),
        Step::Hold,
    ])
    .await;
    let output = tokio::time::timeout(
        std::time::Duration::from_millis(800),
        fixture
            .provider()
            .complete_stream_with_model(None, &prompt(), &[], &mut |_| {}),
    )
    .await
    .expect("terminal event must finish before the one-second transport idle timeout")
    .unwrap();
    assert!(matches!(output, ModelOutput::Text(ref text) if text == "done"));
}

#[tokio::test]
async fn incomplete_terminal_event_and_ping_only_stream_are_not_successful_messages() {
    for content in [
        format!(
            "{START}{}data: {{\"type\":\"message_stop\"}}",
            text_delta("partial")
        ),
        "data: {\"type\":\"ping\"}\n\n".to_string(),
    ] {
        let mut fixture = Fixture::new(vec![bytes(content)]).await;
        let error = fixture
            .provider()
            .complete_stream_with_model(None, &prompt(), &[], &mut |_| {})
            .await
            .unwrap_err();
        assert!(error.contains("provider/stream_incomplete"), "{error}");
        fixture.finished().await;
    }
}

#[tokio::test]
async fn tool_block_count_is_bounded_even_when_each_event_is_small() {
    let mut steps = vec![bytes(START)];
    for index in 0..1025 {
        steps.push(bytes(format!(
            "data: {}\n\n",
            json!({"type":"content_block_start","index":index,"content_block":{
                "type":"tool_use","id":format!("tool-{index}"),"name":"read_file","input":{}
            }})
        )));
    }
    let mut fixture = Fixture::new(steps).await;
    let error = fixture
        .provider()
        .complete_stream_with_model(None, &prompt(), &[], &mut |_| {})
        .await
        .unwrap_err();
    assert!(
        error.contains("provider/stream_tool_blocks_too_large"),
        "{error}"
    );
    fixture.finished().await;
}

#[tokio::test]
async fn total_tool_memory_is_bounded_across_individually_allowed_blocks() {
    let mut steps = vec![bytes(START)];
    let partial = " ".repeat(512 * 1024);
    for index in 0..5 {
        steps.push(bytes(format!(
            "data: {}\n\n",
            json!({"type":"content_block_start","index":index,"content_block":{
                "type":"tool_use","id":format!("tool-{index}"),"name":"read_file","input":{}
            }})
        )));
        // Each call remains below 4 MiB; combined arguments exceed 16 MiB.
        for _ in 0..7 {
            steps.push(bytes(format!(
                "data: {}\n\n",
                json!({"type":"content_block_delta","index":index,"delta":{
                    "type":"input_json_delta","partial_json":partial
                }})
            )));
        }
    }
    let mut fixture = Fixture::new(steps).await;
    let error = fixture
        .provider()
        .complete_stream_with_model(None, &prompt(), &[], &mut |_| {})
        .await
        .unwrap_err();
    assert!(error.contains("缓冲上限"), "{error}");
    fixture.finished().await;
}

#[tokio::test]
async fn stalled_stream_reports_the_actual_connection_idle_timeout() {
    let fixture = Fixture::new(vec![bytes(START), Step::Hold]).await;
    let error = fixture
        .provider()
        .complete_stream_with_model(None, &prompt(), &[], &mut |_| {})
        .await
        .unwrap_err();
    assert!(error.contains("provider/stream_idle_timeout"), "{error}");
    assert!(error.contains("1s"), "{error}");
    assert!(!error.contains("60s"), "{error}");
}

#[tokio::test]
async fn executable_tool_calls_require_well_formed_deltas_and_completed_blocks() {
    let tools = owo_agent_core::tools::ToolRegistry::new().specs();
    let start = format!(
        "data: {}\n\n",
        json!({
            "type":"content_block_start","index":0,
            "content_block":{"type":"tool_use","id":"read-fixture","name":"read_file","input":{}}
        })
    );
    for mode in [
        "complete",
        "no-block-stop",
        "bad-argument-type",
        "orphan-argument",
    ] {
        let index = if mode == "orphan-argument" { 9 } else { 0 };
        let arguments = if mode == "bad-argument-type" {
            json!(7)
        } else {
            json!("{\"path\":\"README.md\"}")
        };
        let mut steps = vec![
            bytes(START),
            bytes(&start),
            bytes(format!(
                "data: {}\n\n",
                json!({"type":"content_block_delta","index":index,
                "delta":{"type":"input_json_delta","partial_json":arguments}})
            )),
        ];
        if mode != "no-block-stop" {
            steps.push(bytes(
                "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            ));
        }
        steps.push(bytes(STOP));
        let mut fixture = Fixture::new(steps).await;
        let result = fixture
            .provider()
            .complete_stream_with_model(None, &prompt(), &tools, &mut |_| {})
            .await;
        if mode == "complete" {
            let ModelOutput::ToolCalls(calls) = result.unwrap() else {
                panic!("tool calls expected")
            };
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "read-fixture");
            assert_eq!(calls[0].name, "read_file");
            assert_eq!(calls[0].arguments, json!({"path":"README.md"}));
        } else {
            let error = result.unwrap_err();
            let expected = if mode == "no-block-stop" {
                "provider/stream_incomplete"
            } else {
                "provider/stream_invalid_event"
            };
            assert!(error.contains(expected), "{mode}: {error}");
        }
        fixture.finished().await;
    }
}

#[tokio::test]
async fn idle_recovery_preserves_network_policy_without_repeating_partial_text() {
    use owo_agent_core::gateway::{CircuitBreaker, ResilientProvider, RetryPolicy};
    for (partial, retry_network) in [(false, true), (true, true), (false, false)] {
        let mut primary_steps = vec![bytes(START)];
        if partial {
            primary_steps.push(bytes(text_delta("partial")));
        }
        primary_steps.push(Step::Hold);
        let primary = Fixture::new(primary_steps).await;
        let mut fallback = Fixture::new(vec![
            bytes(START),
            bytes(text_delta("recovered")),
            bytes(STOP),
        ])
        .await;
        // Only an explicitly supplied fallback can be used. No connection is inferred.
        let gateway = ResilientProvider::new(
            primary.provider(),
            vec![fallback.provider()],
            CircuitBreaker::default(),
            RetryPolicy {
                max_retries: 0,
                retry_network,
                ..Default::default()
            },
        );
        let mut content = String::new();
        let result = gateway
            .complete_stream_with_reasoning_and_model_observed(None, &prompt(), &[], &mut |chunk| {
                if let StreamChunk::Content(delta) = chunk {
                    content.push_str(&delta);
                }
            })
            .await;
        if !partial && retry_network {
            let observed = result.unwrap();
            assert!(matches!(observed.output, ModelOutput::Text(ref text) if text == "recovered"));
            assert_eq!(content, "recovered");
            fallback.finished().await;
        } else {
            assert!(result.unwrap_err().contains("provider/stream_idle_timeout"));
            assert_eq!(content, if partial { "partial" } else { "" });
            assert!(!fallback.task.as_ref().unwrap().is_finished());
        }
    }
}
