//! Native gateway routing contract: inspect requests received by a local HTTP peer.
//! No cloud calls, model-name heuristics, or environment mutation.
use owo_agent_core::gateway::{
    custom_model_provider, ChatMessage, MessageImage, ModelOutput, ModelProvider, StreamChunk,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    base: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(n, 0, "request ended before headers");
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                    assert!(bytes.len() <= 8192, "bounded request headers");
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                assert!(headers.starts_with("POST /v1/messages HTTP/"));
                assert!(headers
                    .to_ascii_lowercase()
                    .contains("x-api-key: fixture-only"));
                let len = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .expect("content length");
                assert!(len <= 512 * 1024, "bounded fixture input");
                while bytes.len() < header_end + len {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let request: Value =
                    serde_json::from_slice(&bytes[header_end..header_end + len]).unwrap();
                let streaming = request["stream"] == true;
                let rejected = request["model"] == "reject-model";
                captured.lock().unwrap().push(request);
                let (status, mime, body) = if rejected {
                    (
                        "400 Bad Request",
                        "application/json",
                        json!({"error":{"message":"unsupported model"}}).to_string(),
                    )
                } else if streaming {
                    ("200 OK", "text/event-stream", concat!(
                        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
                        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"trace\"}}\n\n",
                        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
                        "data: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n\n",
                        "data: {\"type\":\"message_stop\"}\n\n"
                    ).to_string())
                } else {
                    (
                        "200 OK",
                        "application/json",
                        json!({
                            "content":[{"type":"text","text":"ok"}],
                            "stop_reason":"end_turn", "usage":{"input_tokens":1,"output_tokens":1}
                        })
                        .to_string(),
                    )
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        Self {
            base,
            requests,
            task,
        }
    }
    fn provider(&self) -> Arc<dyn ModelProvider> {
        let connection = serde_json::from_value(json!({
            "model":"base-model", "base_url":self.base, "api_format":"anthropic",
            "api_key":"fixture-only", "timeout_secs":3
        }))
        .unwrap();
        custom_model_provider(&connection).unwrap()
    }
    fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}
fn prompt() -> Vec<ChatMessage> {
    vec![ChatMessage::user("fixture".into())]
}
fn assert_ok(output: ModelOutput) {
    assert!(matches!(output, ModelOutput::Text(ref text) if text == "ok"));
}

#[tokio::test]
async fn nonstream_override_and_sentinel_use_the_requested_model_without_mutation() {
    let fixture = Fixture::new().await;
    let provider = fixture.provider();
    let messages = prompt();
    assert_ok(
        provider
            .complete_with_model(Some("  vision-worker  "), &messages, &[])
            .await
            .unwrap(),
    );
    for model in [Some("default"), Some(" default "), Some("   "), None] {
        assert_ok(
            provider
                .complete_with_model(model, &messages, &[])
                .await
                .unwrap(),
        );
    }
    assert_ok(provider.complete(&messages, &[]).await.unwrap());
    let observed = provider
        .complete_with_model_observed(Some("observed-worker"), &messages, &[])
        .await
        .unwrap();
    assert_ok(observed.output);
    let requests = fixture.requests();
    let models: Vec<_> = requests
        .iter()
        .map(|r| r["model"].as_str().unwrap())
        .collect();
    assert_eq!(
        models,
        [
            "vision-worker",
            "base-model",
            "base-model",
            "base-model",
            "base-model",
            "base-model",
            "observed-worker"
        ]
    );
    assert!(requests.iter().all(|r| r["stream"] == false));
}

#[tokio::test]
async fn every_stream_entry_preserves_model_selection_and_incremental_channels() {
    let fixture = Fixture::new().await;
    let provider = fixture.provider();
    let messages = prompt();
    let mut text = String::new();
    assert_ok(
        provider
            .complete_stream_with_model(Some("stream-worker"), &messages, &[], &mut |s| {
                text.push_str(&s)
            })
            .await
            .unwrap(),
    );
    assert_eq!(text, "ok");
    let mut chunks = Vec::new();
    assert_ok(
        provider
            .complete_stream_with_reasoning_and_model(
                Some("reasoning-worker"),
                &messages,
                &[],
                &mut |s| chunks.push(s),
            )
            .await
            .unwrap(),
    );
    assert!(chunks.contains(&StreamChunk::Reasoning("trace".into())));
    assert!(chunks.contains(&StreamChunk::Content("ok".into())));
    let observed = provider
        .complete_stream_with_reasoning_and_model_observed(
            Some("observed-stream"),
            &messages,
            &[],
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_ok(observed.output);
    assert_ok(
        provider
            .complete_stream_with_reasoning(&messages, &[], &mut |_| {})
            .await
            .unwrap(),
    );
    assert_ok(
        provider
            .complete_stream_with_model(Some(" default "), &messages, &[], &mut |_| {})
            .await
            .unwrap(),
    );
    assert_ok(
        provider
            .complete_stream(&messages, &[], &mut |_| {})
            .await
            .unwrap(),
    );
    let requests = fixture.requests();
    let models: Vec<_> = requests
        .iter()
        .map(|r| r["model"].as_str().unwrap())
        .collect();
    assert_eq!(
        models,
        [
            "stream-worker",
            "reasoning-worker",
            "observed-stream",
            "base-model",
            "base-model",
            "base-model"
        ]
    );
    assert!(requests.iter().all(|r| r["stream"] == true));
}

#[tokio::test]
async fn concurrent_worker_models_keep_separate_payloads_and_image_blocks() {
    let fixture = Fixture::new().await;
    let provider = fixture.provider();
    let first = [ChatMessage::user_with_images(
        "first".into(),
        vec![MessageImage::from_url("data:image/png;base64,QUJD")],
    )];
    let second = [ChatMessage::user("second".into())];
    let (left, right) = tokio::join!(
        provider.complete_with_model(Some("image-worker"), &first, &[]),
        provider.complete_with_model(Some("text-worker"), &second, &[])
    );
    assert_ok(left.unwrap());
    assert_ok(right.unwrap());
    let requests = fixture.requests();
    assert_eq!(requests.len(), 2);
    let image = requests
        .iter()
        .find(|r| r["model"] == "image-worker")
        .unwrap();
    assert_eq!(image["messages"][0]["content"][0]["text"], "first");
    assert_eq!(image["messages"][0]["content"][1]["type"], "image");
    assert_eq!(image["messages"][0]["content"][1]["source"]["data"], "QUJD");
    let text = requests
        .iter()
        .find(|r| r["model"] == "text-worker")
        .unwrap();
    assert_eq!(text["messages"][0]["content"][0]["text"], "second");
    assert_eq!(text["messages"][0]["content"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn rejected_override_does_not_fall_back_to_the_configured_model() {
    let fixture = Fixture::new().await;
    let provider = fixture.provider();
    let error = provider
        .complete_with_model(Some("reject-model"), &prompt(), &[])
        .await
        .unwrap_err();
    assert!(error.contains("400"));
    assert!(!error.contains("fixture-only"));
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "reject-model");
}
