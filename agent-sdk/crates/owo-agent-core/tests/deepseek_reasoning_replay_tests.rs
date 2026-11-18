//! Local HTTP protocol regressions. Cloud-model acceptance is recorded separately.
use owo_agent_core::gateway::{ModelProvider, OpenAiCompatibleConfig, OpenAiCompatibleProvider};
use owo_agent_core::permissions::{AutoApprover, Policy};
use owo_agent_core::session::{JsonSessionStore, SessionStore};
use owo_agent_core::{Agent, AgentConfig, Session, ToolRegistry};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_request(socket: &mut tokio::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    let head = loop {
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        assert!(bytes.len() < 8192);
    };
    let length = std::str::from_utf8(&bytes[..head])
        .unwrap()
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap();
    assert!(length < 256 * 1024);
    while bytes.len() < head + length {
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    serde_json::from_slice(&bytes[head..head + length]).unwrap()
}
async fn respond(socket: &mut tokio::net::TcpStream, body: &str, mime: &str) {
    let headers=format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
    socket.write_all(headers.as_bytes()).await.unwrap();
    socket.write_all(body.as_bytes()).await.unwrap();
}
fn provider(base: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
        base_url: base,
        api_key: String::new(),
        model: "deepseek-flash".into(),
        cloud_enabled: false,
    })
    .unwrap()
}
fn frame(delta: Value, finish: Option<&str>, model: &str) -> String {
    format!(
        "data: {}\n\n",
        json!({"model":model,"choices":[{"delta":delta,"finish_reason":finish}]})
    )
}

#[tokio::test]
async fn streaming_tool_text_and_durable_history_replay_exact_reasoning_only_to_bound_model() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for index in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let body = read_request(&mut socket).await;
            let messages = body["messages"].as_array().unwrap();
            let assistants = messages
                .iter()
                .filter(|m| m["role"] == "assistant")
                .collect::<Vec<_>>();
            if index == 1 {
                assert_eq!(assistants[0]["reasoning_content"], "tool-thought");
                assert_eq!(assistants[0]["content"], "");
                assert_eq!(assistants[0]["tool_calls"][0]["id"], "read-1");
            }
            if index == 2 {
                assert_eq!(assistants[0]["reasoning_content"], "tool-thought");
                assert_eq!(assistants[1]["reasoning_content"], "answer-thought-1");
                assert_eq!(assistants[1]["content"], "answer-1");
            }
            if index == 3 {
                assert_eq!(body["model"], "other-model");
                assert!(assistants
                    .iter()
                    .all(|m| m.get("reasoning_content").is_none()));
            }
            let model = if index == 3 {
                "other-model"
            } else {
                "deepseek-flash"
            };
            let mut response = frame(
                json!({"reasoning_content":if index==0 {"tool-"}else{"answer-"}}),
                None,
                model,
            );
            response += &frame(
                json!({"reasoning_content":if index==0 {"thought"}else if index==1 {"thought-1"}else{"thought-2"}}),
                None,
                model,
            );
            if index == 0 {
                response += &frame(
                    json!({"tool_calls":[{"index":0,"id":"read-1","function":{"name":"read_file","arguments":"{\"path\":\"data.txt\"}"}}]}),
                    Some("tool_calls"),
                    model,
                );
            } else {
                response += &frame(
                    json!({"content":format!("answer-{index}")}),
                    Some("stop"),
                    model,
                );
            }
            response += "data: [DONE]\n\n";
            respond(&mut socket, &response, "text/event-stream").await;
        }
    });
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("data.txt"), "real file").unwrap();
    let mut registry = ToolRegistry::new();
    registry.retain_names(&["read_file".into()]);
    let agent = Agent::new(
        Arc::new(provider(base)),
        registry,
        Policy::new(workspace.path()),
        AgentConfig::default(),
    );
    let mut session = Session::new(workspace.path(), "deepseek-flash", None);
    let abort = AtomicBool::new(false);
    let approval = AutoApprover { allow: false };
    let first = agent
        .run_turn(&mut session, "read file", &approval, &abort, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(first.final_text.as_deref(), Some("answer-1"));
    assert!(session
        .messages
        .iter()
        .filter_map(|m| m.content.as_deref())
        .all(|text| !text.contains("tool-thought")));
    let store = JsonSessionStore::new(data.path());
    store.save(&session).unwrap();
    let mut restored = store.load(&session.id).unwrap();
    agent
        .run_turn(
            &mut restored,
            "continue after reload",
            &approval,
            &abort,
            &mut |_| {},
        )
        .await
        .unwrap();
    restored.model_override = Some("other-model".into());
    agent
        .run_turn(
            &mut restored,
            "switch model",
            &approval,
            &abort,
            &mut |_| {},
        )
        .await
        .unwrap();
    server.await.unwrap();
    // Old JSON sessions remain readable and do not fabricate protocol content.
    let mut legacy = serde_json::to_value(&session).unwrap();
    for message in legacy["messages"].as_array_mut().unwrap() {
        message.as_object_mut().unwrap().remove("reasoning_replay");
    }
    let legacy: Session = serde_json::from_value(legacy).unwrap();
    assert!(legacy.messages.iter().all(|m| m.reasoning_replay.is_none()));
}

#[tokio::test]
async fn non_streaming_observation_keeps_provider_continuation_separate_from_answer() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let body = read_request(&mut socket).await;
        assert_eq!(body["stream"], false);
        let response=json!({"model":"deepseek-flash","choices":[{"message":{"content":"answer","reasoning_content":"exact continuation"},"finish_reason":"stop"}]}).to_string();
        respond(&mut socket, &response, "application/json").await;
    });
    let output = provider(base)
        .complete_with_model_observed(None, &[], &[])
        .await
        .unwrap();
    assert_eq!(
        output.output,
        owo_agent_core::ModelOutput::Text("answer".into())
    );
    let replay = output.reasoning_replay.unwrap();
    assert_eq!(replay.model, "deepseek-flash");
    assert_eq!(replay.content, "exact continuation");
    server.await.unwrap();
}
