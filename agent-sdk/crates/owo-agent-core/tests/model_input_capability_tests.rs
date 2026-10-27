//! Native HTTP admission contracts; no cloud requests or model-name heuristics.
use owo_agent_core::gateway::input_capabilities::{
    declared_image_input, input_capability_bindings_from_env, validate_input_capability_bindings,
    INPUT_CAPABILITIES_ENV,
};
use owo_agent_core::gateway::{
    custom_model_provider, ChatMessage, CircuitBreaker, MessageImage, ModelProvider,
    OpenAiCompatibleConfig, OpenAiCompatibleProvider, ResilientProvider, RetryPolicy, StreamChunk,
};
use owo_agent_protocol::{CustomModelConnection, ModelInputCapabilityBinding};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Peer {
    base: String,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Peer {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let count = hits.clone();
        let received = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let mut bytes = Vec::new();
                let end = loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break None;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(pos + 4);
                    }
                };
                let Some(end) = end else {
                    continue;
                };
                let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                assert!(length < 65536);
                while bytes.len() < end + length {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let body: Value = serde_json::from_slice(&bytes[end..end + length]).unwrap();
                let anthropic = headers.starts_with("POST /v1/messages ")
                    || headers.starts_with("POST /custom/messages ");
                let stream = body["stream"] == true;
                let payload=match (anthropic,stream) {
                    (true,true)=>concat!(
                        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
                        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
                        "data: {\"type\":\"message_stop\"}\n\n").to_string(),
                    (false,true)=>concat!(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
                        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: [DONE]\n\n").to_string(),
                    (true,false)=>json!({"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}).to_string(),
                    (false,false)=>json!({"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}).to_string(),
                };
                received.lock().unwrap().push(body);
                let mime = if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",payload.len());
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Self {
            base,
            hits,
            requests,
            task,
        }
    }
    fn connection(&self, format: &str, declaration: Option<Value>) -> CustomModelConnection {
        let mut value = json!({"model":"selected-model","base_url":self.base,"api_format":format,"api_key":"fixture-only","timeout_secs":3});
        if let Some(binding) = declaration {
            value["input_capabilities"] = binding;
        }
        serde_json::from_value(value).unwrap()
    }
    fn binding(&self, format: &str, support: bool) -> Value {
        json!({"model":"selected-model","base_url":self.base,"api_format":format,"image_input":support})
    }
}
fn images() -> Vec<ChatMessage> {
    vec![
        ChatMessage::user_with_images(
            "old image".into(),
            vec![MessageImage::from_url("data:image/png;base64,QUJD")],
        ),
        ChatMessage::user("new text-only prompt".into()),
    ]
}
async fn invoke(
    provider: &dyn ModelProvider,
    kind: usize,
    model: Option<&str>,
    messages: &[ChatMessage],
) -> Result<(), String> {
    match kind {
        0 => provider
            .complete_with_model(model, messages, &[])
            .await
            .map(|_| ()),
        1 => provider
            .complete_with_model_observed(model, messages, &[])
            .await
            .map(|_| ()),
        2 => provider
            .complete_stream_with_model(model, messages, &[], &mut |_| {})
            .await
            .map(|_| ()),
        3 => provider
            .complete_stream_with_reasoning_and_model(
                model,
                messages,
                &[],
                &mut |_: StreamChunk| {},
            )
            .await
            .map(|_| ()),
        4 => provider
            .complete_stream_with_reasoning_and_model_observed(model, messages, &[], &mut |_| {})
            .await
            .map(|_| ()),
        5 => provider.complete_expression(messages).await.map(|_| ()),
        6 => provider.complete(messages, &[]).await.map(|_| ()),
        7 => provider
            .complete_stream(messages, &[], &mut |_| {})
            .await
            .map(|_| ()),
        8 => provider
            .complete_stream_with_reasoning(messages, &[], &mut |_| {})
            .await
            .map(|_| ()),
        _ => unreachable!(),
    }
}
fn entrypoints(format: &str) -> Vec<usize> {
    // Anthropic deliberately has no dedicated expression implementation; do not fabricate one.
    (0..9)
        .filter(|kind| format == "openai" || *kind != 5)
        .collect()
}
#[tokio::test]
async fn unknown_unsupported_and_stale_bindings_send_zero_network_requests() {
    for format in ["openai", "anthropic"] {
        let peer = Peer::new().await;
        let mut stale = peer.binding(format, true);
        stale["model"] = json!("previous-model");
        let mut states = vec![
            (None, "image_capability_unknown"),
            (Some(peer.binding(format, false)), "image_not_supported"),
            (Some(stale), "capability_binding_mismatch"),
        ];
        for (field, value) in [
            ("base_url", json!("http://127.0.0.1:1/v1")),
            (
                "api_format",
                json!(if format == "openai" {
                    "anthropic"
                } else {
                    "openai"
                }),
            ),
            ("use_full_url", json!(true)),
        ] {
            let mut binding = peer.binding(format, true);
            binding[field] = value;
            states.push((Some(binding), "capability_binding_mismatch"));
        }
        for (binding, code) in states {
            let provider = custom_model_provider(&peer.connection(format, binding)).unwrap();
            for kind in entrypoints(format) {
                let error = invoke(provider.as_ref(), kind, None, &images())
                    .await
                    .unwrap_err();
                assert!(error.contains(code), "{format} / {kind}: {error}");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert_eq!(peer.hits.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn explicit_support_transmits_historical_images_through_every_native_entrypoint() {
    for format in ["openai", "anthropic"] {
        let peer = Peer::new().await;
        let provider =
            custom_model_provider(&peer.connection(format, Some(peer.binding(format, true))))
                .unwrap();
        for kind in entrypoints(format) {
            invoke(provider.as_ref(), kind, Some(" default "), &images())
                .await
                .unwrap();
        }
        let requests = peer.requests.lock().unwrap();
        assert_eq!(requests.len(), entrypoints(format).len());
        for request in requests.iter() {
            assert_eq!(request["model"], "selected-model");
            let content = &request["messages"][0]["content"];
            assert_eq!(
                content[1]["type"],
                if format == "anthropic" {
                    "image"
                } else {
                    "image_url"
                }
            );
            assert!(
                !request.to_string().contains("input_capabilities"),
                "declaration is not a provider wire field"
            );
        }
    }
}
#[tokio::test]
async fn model_override_invalidates_an_old_declaration_without_using_a_fallback() {
    for format in ["openai", "anthropic"] {
        let primary = Peer::new().await;
        let fallback = Peer::new().await;
        let provider = ResilientProvider::new(
            custom_model_provider(&primary.connection(format, Some(primary.binding(format, true))))
                .unwrap(),
            vec![custom_model_provider(
                &fallback.connection(format, Some(fallback.binding(format, true))),
            )
            .unwrap()],
            CircuitBreaker::new(1, std::time::Duration::from_secs(300)),
            RetryPolicy::default(),
        );
        for kind in 0..5 {
            let error = invoke(&provider, kind, Some("different-worker-model"), &images())
                .await
                .unwrap_err();
            assert!(
                error.starts_with("input/capability_binding_mismatch"),
                "{error}"
            );
            assert_eq!(provider.breaker().consecutive_failures(), 0);
        }
        invoke(
            &provider,
            0,
            None,
            &[ChatMessage::user("text still works".into())],
        )
        .await
        .unwrap();
        assert_eq!(primary.hits.load(Ordering::SeqCst), 1);
        assert_eq!(fallback.hits.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn explicit_denial_does_not_break_text_only_calls() {
    for format in ["openai", "anthropic"] {
        let peer = Peer::new().await;
        let provider =
            custom_model_provider(&peer.connection(format, Some(peer.binding(format, false))))
                .unwrap();
        for kind in entrypoints(format) {
            invoke(
                provider.as_ref(),
                kind,
                Some("other-text-model"),
                &[ChatMessage::user("text".into())],
            )
            .await
            .unwrap();
        }
        assert_eq!(peer.hits.load(Ordering::SeqCst), entrypoints(format).len());
    }
}
#[test]
fn declarations_are_exact_bounded_and_cannot_claim_remote_verification() {
    let binding: ModelInputCapabilityBinding = serde_json::from_value(
        json!({"model":"model","base_url":"HTTPS://EXAMPLE.COM:443/v1/","image_input":true}),
    )
    .unwrap();
    assert_eq!(
        declared_image_input(
            "model",
            "https://example.com/v1",
            "openai",
            false,
            None,
            std::slice::from_ref(&binding)
        )
        .unwrap(),
        Some(true)
    );
    for (model, base, format, full) in [
        ("other", "https://example.com/v1", "openai", false),
        ("model", "https://example.com/V1", "openai", false),
        ("model", "https://example.com/v1", "anthropic", false),
        ("model", "https://example.com/v1", "openai", true),
    ] {
        assert_eq!(
            declared_image_input(
                model,
                base,
                format,
                full,
                None,
                std::slice::from_ref(&binding)
            )
            .unwrap(),
            None
        );
        assert!(declared_image_input(model, base, format, full, Some(&binding), &[]).is_err());
    }
    let mut duplicate = binding.clone();
    duplicate.base_url = "https://example.com/v1".into();
    assert!(validate_input_capability_bindings(&[binding.clone(), duplicate]).is_err());
    assert!(validate_input_capability_bindings(&vec![binding.clone(); 65]).is_err());
    for invalid in [
        "https://user:secret@example.com/v1",
        "https://example.com/v1?key=hidden",
        "file:///v1",
        "https://example.com/v1#fragment",
    ] {
        let mut b = binding.clone();
        b.base_url = invalid.into();
        let error = validate_input_capability_bindings(&[b]).unwrap_err();
        assert!(!error.contains("secret") && !error.contains("hidden"));
    }
    let mut fake = serde_json::to_value(binding).unwrap();
    fake["verified"] = json!(true);
    assert!(serde_json::from_value::<ModelInputCapabilityBinding>(fake).is_err());
}
#[test]
fn settings_persist_declarations_and_reject_invalid_configs_before_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = owo_agent_core::Settings {
        model_input_capabilities: Some(vec![serde_json::from_value(
            json!({"model":"model","base_url":"https://example.com/v1","image_input":false}),
        )
        .unwrap()]),
        ..Default::default()
    };
    settings.save(dir.path()).unwrap();
    assert_eq!(
        owo_agent_core::Settings::load(dir.path()).model_input_capabilities,
        settings.model_input_capabilities
    );
    let before = std::fs::read(dir.path().join("settings.json")).unwrap();
    settings.model_input_capabilities.as_mut().unwrap()[0].base_url = "file:///bad".into();
    assert!(settings.save(dir.path()).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("settings.json")).unwrap(),
        before
    );
}
struct RestoreEnv(Option<std::ffi::OsString>);
impl Drop for RestoreEnv {
    fn drop(&mut self) {
        match &self.0 {
            Some(v) => std::env::set_var(INPUT_CAPABILITIES_ENV, v),
            None => std::env::remove_var(INPUT_CAPABILITIES_ENV),
        }
    }
}
#[tokio::test]
async fn host_declarations_apply_to_exact_connections_and_clear_without_granting_other_workers() {
    if std::env::var_os("OWO_INPUT_CAPABILITY_TEST_CHILD").is_none() {
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","host_declarations_apply_to_exact_connections_and_clear_without_granting_other_workers","--test-threads=1","--nocapture"])
            .env("OWO_INPUT_CAPABILITY_TEST_CHILD","1").env(INPUT_CAPABILITIES_ENV,"[]").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let _restore = RestoreEnv(std::env::var_os(INPUT_CAPABILITIES_ENV));
    let peer = Peer::new().await;
    let bindings: Vec<ModelInputCapabilityBinding> =
        serde_json::from_value(json!([peer.binding("openai", true)])).unwrap();
    let settings = owo_agent_core::Settings {
        model_input_capabilities: Some(bindings),
        ..Default::default()
    };
    settings.apply_model_input_capabilities_env();
    let provider = custom_model_provider(&peer.connection("openai", None)).unwrap();
    invoke(provider.as_ref(), 0, None, &images()).await.unwrap();
    assert!(
        invoke(provider.as_ref(), 0, Some("other-worker"), &images())
            .await
            .unwrap_err()
            .contains("image_capability_unknown")
    );
    let settings = owo_agent_core::Settings {
        model_input_capabilities: Some(vec![]),
        ..Default::default()
    };
    settings.apply_model_input_capabilities_env();
    assert!(invoke(provider.as_ref(), 0, None, &images())
        .await
        .unwrap_err()
        .contains("image_capability_unknown"));
    for bad in ["{}", "[{\"verified\":true}]", "not json"] {
        std::env::set_var(INPUT_CAPABILITIES_ENV, bad);
        assert!(input_capability_bindings_from_env()
            .unwrap_err()
            .contains("capability_config_invalid"));
        assert!(invoke(provider.as_ref(), 0, None, &images())
            .await
            .unwrap_err()
            .contains("capability_config_invalid"));
    }
    std::env::set_var(INPUT_CAPABILITIES_ENV, "中".repeat(9000));
    assert!(input_capability_bindings_from_env().is_err());
    assert_eq!(peer.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn concurrent_default_model_updates_never_retarget_admitted_images() {
    if std::env::var_os("OWO_MODEL_SNAPSHOT_TEST_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "concurrent_default_model_updates_never_retarget_admitted_images",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("OWO_MODEL_SNAPSHOT_TEST_CHILD", "1")
            .env(INPUT_CAPABILITIES_ENV, "[]")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let peer = Peer::new().await;
    let binding: ModelInputCapabilityBinding =
        serde_json::from_value(peer.binding("openai", true)).unwrap();
    owo_agent_core::Settings {
        model_input_capabilities: Some(vec![binding]),
        ..Default::default()
    }
    .apply_model_input_capabilities_env();
    std::env::set_var("OPENAI_MODEL", "selected-model");
    let provider = OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
        base_url: peer.base.clone(),
        api_key: "fixture-only".into(),
        model: "selected-model".into(),
        cloud_enabled: false,
    })
    .unwrap();
    invoke(&provider, 0, None, &images()).await.unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let ended = stop.clone();
    let writer = std::thread::spawn(move || {
        while !ended.load(Ordering::Acquire) {
            for model in ["selected-model", "undeclared-text-model"] {
                std::env::set_var("OPENAI_MODEL", model);
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
    });
    let mut unexpected = None;
    for kind in (0..80).map(|i| i % 6) {
        if let Err(error) = invoke(&provider, kind, None, &images()).await {
            if !error.starts_with("input/image_capability_unknown") {
                unexpected = Some(error);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    stop.store(true, Ordering::Release);
    writer.join().unwrap();
    assert_eq!(unexpected, None);
    let requests = peer.requests.lock().unwrap();
    assert!(!requests.is_empty());
    assert!(requests
        .iter()
        .all(|request| request["model"] == "selected-model"));
}

#[test]
fn oversized_or_invalid_persisted_declarations_apply_as_invalid_not_stale_support() {
    if std::env::var_os("OWO_BAD_DECLARATION_TEST_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "oversized_or_invalid_persisted_declarations_apply_as_invalid_not_stale_support",
                "--test-threads=1",
            ])
            .env("OWO_BAD_DECLARATION_TEST_CHILD", "1")
            .env(INPUT_CAPABILITIES_ENV, "[]")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let bindings = (0..64)
        .map(|i| ModelInputCapabilityBinding {
            model: format!("model-{i}"),
            base_url: format!("https://example.com/{}", "x".repeat(1000)),
            api_format: "openai".into(),
            use_full_url: false,
            image_input: true,
        })
        .collect();
    let settings = owo_agent_core::Settings {
        model_input_capabilities: Some(bindings),
        ..Default::default()
    };
    let dir = tempfile::tempdir().unwrap();
    assert!(settings.save(dir.path()).is_err());
    assert!(settings.save_encrypted(dir.path()).is_err());
    settings.apply_model_input_capabilities_env();
    assert_eq!(std::env::var(INPUT_CAPABILITIES_ENV).unwrap(), "null");
    assert!(input_capability_bindings_from_env().is_err());
}

#[tokio::test]
async fn native_preflight_matches_send_admission_without_network_or_breaker_changes() {
    for format in ["openai", "anthropic"] {
        let peer = Peer::new().await;
        for support in [None, Some(false), Some(true)] {
            let binding = support.map(|support| peer.binding(format, support));
            let native = custom_model_provider(&peer.connection(format, binding)).unwrap();
            let provider = ResilientProvider::new(
                native,
                Vec::new(),
                CircuitBreaker::new(1, std::time::Duration::from_secs(300)),
                RetryPolicy::default(),
            );
            assert!(provider
                .validate_image_input(Some("different-text-model"), false)
                .is_ok());
            let result = provider.validate_image_input(None, true);
            match support {
                Some(true) => assert!(result.is_ok()),
                Some(false) => {
                    assert!(result.unwrap_err().starts_with("input/image_not_supported"))
                }
                None => assert!(result
                    .unwrap_err()
                    .starts_with("input/image_capability_unknown")),
            }
            if support.is_some() {
                assert!(provider
                    .validate_image_input(Some("different-worker"), true)
                    .unwrap_err()
                    .starts_with("input/capability_binding_mismatch"));
            }
            assert_eq!(provider.breaker().consecutive_failures(), 0);
        }
        assert_eq!(peer.hits.load(Ordering::SeqCst), 0);
    }
}
