//! Real local sockets and the production Agent/ToolHost/web_fetch path.
//! No cloud provider or broad auto-approval; the fixture permits one exact URL only.
use async_trait::async_trait;
use owo_agent_core::permissions::{Approver, Decision, PermissionRequest, Policy};
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::{
    Agent, ChatMessage, ModelOutput, ModelProvider, Session, ToolCall, ToolSpec, TurnEvent,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct SyntheticFetch {
    url: String,
    calls: AtomicUsize,
}
#[async_trait]
impl ModelProvider for SyntheticFetch {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(ModelOutput::ToolCalls(vec![ToolCall {
                id: "synthetic-fetch".into(),
                name: "web_fetch".into(),
                arguments: json!({"url":self.url,"max_bytes":1024}),
            }]))
        } else {
            Ok(ModelOutput::Text(
                "synthetic boundary check finished".into(),
            ))
        }
    }
}
struct ExactSyntheticUrl {
    url: String,
    approvals: AtomicUsize,
}
#[async_trait]
impl Approver for ExactSyntheticUrl {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        if request.tool == "web_fetch"
            && request.args.get("url").and_then(|v| v.as_str()) == Some(&self.url)
        {
            self.approvals.fetch_add(1, Ordering::SeqCst);
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}
#[tokio::test]
async fn ipv6_literal_fetch_cannot_reach_loopback_even_after_exact_url_approval() {
    // Process-local fixture controls only; never change user/machine environment.
    for name in [
        "OWO_HTTP_PROXY",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "OWO_WEB_FETCH_ALLOW_PRIVATE",
    ] {
        std::env::remove_var(name);
    }
    for (bind, host) in [("[::1]:0", "[::1]"), ("127.0.0.1:0", "[::ffff:127.0.0.1]")] {
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .expect("local IPv6/IPv4 test listener must be available");
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://{host}:{port}/synthetic-private-data");
        let connections = Arc::new(AtomicUsize::new(0));
        let seen = connections.clone();
        let server = tokio::spawn(async move {
            if let Ok(Ok((mut socket, _))) =
                tokio::time::timeout(Duration::from_millis(500), listener.accept()).await
            {
                seen.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0u8; 4096];
                let _ =
                    tokio::time::timeout(Duration::from_secs(2), socket.read(&mut buffer)).await;
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nsynthetic").await;
            }
        });
        let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.work-logs/web-fetch-boundary")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&workspace).unwrap();
        let provider = Arc::new(SyntheticFetch {
            url: url.clone(),
            calls: AtomicUsize::new(0),
        });
        let agent = Agent::new(
            provider,
            ToolRegistry::new(),
            Policy::new(&workspace),
            Default::default(),
        );
        let approver = ExactSyntheticUrl {
            url,
            approvals: AtomicUsize::new(0),
        };
        let mut session = Session::new(&workspace, "synthetic", None);
        let mut outcomes = Vec::new();
        let mut on_event = |event: &TurnEvent| {
            if let TurnEvent::ToolResult { tool, ok, .. } = event {
                if tool == "web_fetch" {
                    outcomes.push(*ok);
                }
            }
        };
        agent
            .run_turn(
                &mut session,
                "run the exact synthetic fetch boundary check",
                &approver,
                &AtomicBool::new(false),
                &mut on_event,
            )
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(
            approver.approvals.load(Ordering::SeqCst),
            1,
            "fixture must reach the approved real tool path"
        );
        assert_eq!(
            connections.load(Ordering::SeqCst),
            0,
            "private IPv6 literal must be rejected BEFORE connecting"
        );
        assert_eq!(
            outcomes,
            vec![false],
            "fetch must publish an explicit refusal"
        );
        let resolved = workspace.canonicalize().unwrap();
        let expected_parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap()
            .join(".work-logs/web-fetch-boundary");
        assert!(resolved.starts_with(&expected_parent) && resolved != expected_parent);
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
