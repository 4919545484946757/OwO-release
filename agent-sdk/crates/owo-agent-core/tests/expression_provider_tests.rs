//! Wire-level evidence for the tool-free expression transport and fixed budget.
use owo_agent_core::gateway::{
    ChatMessage, ModelProvider, OpenAiCompatibleConfig, OpenAiCompatibleProvider,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
async fn fixture(
    message: Value,
    finish: &str,
) -> (OpenAiCompatibleProvider, tokio::task::JoinHandle<Value>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let payload = json!({"choices":[{"message":message,"finish_reason":finish}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}).to_string();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        let mut chunk = [0; 4096];
        let (offset, length) = loop {
            let size = socket.read(&mut chunk).await.unwrap();
            assert!(size > 0);
            data.extend_from_slice(&chunk[..size]);
            if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                break (pos + 4, length);
            }
        };
        while data.len() < offset + length {
            let size = socket.read(&mut chunk).await.unwrap();
            assert!(size > 0);
            data.extend_from_slice(&chunk[..size]);
        }
        let request: Value = serde_json::from_slice(&data[offset..offset + length]).unwrap();
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", payload.len(), payload);
        socket.write_all(response.as_bytes()).await.unwrap();
        request
    });
    let provider = OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
        base_url: base,
        api_key: String::new(),
        model: "fixture".into(),
        cloud_enabled: false,
    })
    .unwrap();
    (provider, server)
}
#[tokio::test]
async fn expression_wire_has_no_tools_and_a_fixed_token_limit() {
    let (provider, server) = fixture(json!({"content":"您好"}), "stop").await;
    assert_eq!(
        provider
            .complete_expression(&[ChatMessage::user("改写你好".into())])
            .await
            .unwrap(),
        "您好"
    );
    let body = server.await.unwrap();
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    assert_eq!(body["max_tokens"], 1024);
    assert_eq!(body["stream"], false);
    assert_eq!(provider.usage_snapshot().total_tokens, 2);
}
#[tokio::test]
async fn expression_rejects_tool_calls_truncation_and_oversize() {
    for (message, finish) in [
        (
            json!({"content":"","tool_calls":[{"id":"1","type":"function","function":{"name":"read_file","arguments":"{}"}}]}),
            "tool_calls",
        ),
        (json!({"content":"truncated"}), "length"),
        (json!({"content":"中".repeat(4097)}), "stop"),
    ] {
        let (provider, server) = fixture(message, finish).await;
        assert!(provider
            .complete_expression(&[ChatMessage::user("test".into())])
            .await
            .is_err());
        server.await.unwrap();
    }
}
