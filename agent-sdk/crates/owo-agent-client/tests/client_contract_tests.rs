//! `owo-agent-client` 契约测试：
//!   * SSE 缓冲的半包/UTF-8 边界（离线）；
//!   * 发现文件读取与陈旧 pid 拒绝；
//!   * 对**假 Daemon**（tokio 原生 TCP，无 server 依赖）的端到端往返。

use owo_agent_client::discovery::{descriptor_path, DaemonDiscovery};
use owo_agent_client::sse::{parse_sse_data_line, JsonSseBuffer, SseBuffer};
use owo_agent_client::{AgentClient, ClientConfig};
use owo_agent_protocol::{DaemonDescriptor, SseEvent};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ---------------------------------------------------------------------------
// 离线：SSE 半包与 UTF-8 边界
// ---------------------------------------------------------------------------

#[test]
fn sse_parses_event_split_across_chunks() {
    let mut buffer = SseBuffer::new();
    buffer.push(b"event: token_delta\ndata: {\"type\":\"token_delta\",\"de");
    assert!(buffer.next_event().is_none(), "半包不得产出事件");
    buffer.push(b"lta\":\"hi\",\"v\":1}\n\n");
    match buffer.next_event() {
        Some(Ok(SseEvent::TokenDelta { delta })) => assert_eq!(delta, "hi"),
        other => panic!("期望 token_delta，得到 {other:?}"),
    }
    assert!(buffer.next_event().is_none());
}

#[test]
fn sse_preserves_multibyte_utf8_split_across_chunks() {
    // "好" = E5 A5 BD；把它切在两个 chunk 之间。
    let mut buffer = SseBuffer::new();
    buffer.push(b"data: {\"type\":\"token_delta\",\"delta\":\"\xe5\xa5");
    buffer.push(b"\xbd\",\"v\":1}\n");
    match buffer.next_event() {
        Some(Ok(SseEvent::TokenDelta { delta })) => assert_eq!(delta, "好"),
        other => panic!("UTF-8 跨 chunk 被损坏：{other:?}"),
    }
}

#[test]
fn turn_sse_buffer_rejects_oversized_partial_frames_and_recovers() {
    let mut buffer = SseBuffer::new();
    buffer.push(b"data: ");
    buffer.push(&vec![b'x'; 1024 * 1024]);
    let error = buffer.next_frame().expect("overflow should be surfaced");
    assert!(error.unwrap_err().to_string().contains("1 MiB"));
    assert!(buffer.next_frame().is_none());

    buffer.push(b"data: {\"type\":\"token_delta\",\"delta\":\"ok\"}\n");
    assert!(matches!(
        buffer.next_event(),
        Some(Ok(SseEvent::TokenDelta { delta })) if delta == "ok"
    ));
}

#[test]
fn sse_keeps_standard_id_cursor_for_replay() {
    let mut buffer = SseBuffer::new();
    buffer
        .push(b"id: 41\nevent: token_delta\ndata: {\"type\":\"token_delta\",\"delta\":\"a\"}\n\n");
    let frame = buffer.next_frame().expect("complete SSE frame").unwrap();
    assert_eq!(frame.id.as_deref(), Some("41"));
    assert!(matches!(frame.event, SseEvent::TokenDelta { ref delta } if delta == "a"));
}

#[test]
fn sse_ignores_non_data_lines_and_parses_final() {
    assert!(parse_sse_data_line("event: final").is_none());
    assert!(parse_sse_data_line(": keep-alive").is_none());
    let parsed = parse_sse_data_line("data: {\"type\":\"final\",\"text\":\"done\",\"v\":1}");
    match parsed {
        Some(Ok(SseEvent::Final { text })) => assert_eq!(text, "done"),
        other => panic!("期望 final，得到 {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 离线：发现文件
// ---------------------------------------------------------------------------

fn descriptor(pid: u32) -> DaemonDescriptor {
    DaemonDescriptor {
        pid,
        port: 4096,
        instance_id: String::new(),
        api_version: "0.7".to_string(),
        build_id: "abc".to_string(),
        started_at: "2026-09-21T00:00:00Z".to_string(),
        data_root: "Agent-deadbeef".to_string(),
    }
}

#[test]
fn discovery_reads_live_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("runtime")).unwrap();
    std::fs::write(
        descriptor_path(dir.path()),
        serde_json::to_string(&descriptor(std::process::id())).unwrap(),
    )
    .unwrap();
    let discovery = DaemonDiscovery::read(dir.path()).expect("read");
    assert_eq!(discovery.base_url(), "http://127.0.0.1:4096");
    discovery.validate_api_version("0.7").expect("version ok");
    assert!(discovery.validate_api_version("9.9").is_err());
}

#[test]
fn discovery_rejects_stale_dead_pid() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("runtime")).unwrap();
    std::fs::write(
        descriptor_path(dir.path()),
        serde_json::to_string(&descriptor(0)).unwrap(),
    )
    .unwrap();
    assert!(
        DaemonDiscovery::read(dir.path()).is_err(),
        "pid 0 必须视为陈旧"
    );
}

#[test]
fn discovery_missing_file_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let error = DaemonDiscovery::read(dir.path()).unwrap_err();
    assert!(matches!(error, owo_agent_client::ClientError::NotFound(_)));
}

// ---------------------------------------------------------------------------
// 端到端：假 Daemon（原生 TCP，客户端不依赖 server）
// ---------------------------------------------------------------------------

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn read_request(socket: &mut TcpStream) -> (String, String, String) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let read = socket.read(&mut tmp).await.unwrap_or(0);
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..read]);
        if find_subslice(&buf, b"\r\n\r\n").is_some() {
            break;
        }
    }
    let header_end = find_subslice(&buf, b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let header_block = head.clone();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("").to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let content_length = lines
        .filter_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(str::to_string)
        })
        .next()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let read = socket.read(&mut tmp).await.unwrap_or(0);
        if read == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..read]);
    }
    (method, path, header_block)
}

fn route(method: &str, path: &str, headers: &str) -> (u16, &'static str, Vec<u8>) {
    if method == "GET" && path.starts_with("/health") {
        return (
            200,
            "application/json",
            br#"{"healthy":true,"version":"0.1.0","api_version":"0.7","auto_approve":false,"pid":1,"stage":"ready","build_id":"abc"}"#.to_vec(),
        );
    }
    if method == "GET" && path.starts_with("/sessions") {
        return (200, "application/json", b"[]".to_vec());
    }
    if method == "GET" && path == "/workspace/trust" {
        // 审查 N1：工作区信任状态（未信任 + 受限模式 + 危险配置清单）。
        return (
            200,
            "application/json",
            r#"{"workspace":"/ws","trusted":false,"gated":true,"changed":false,"reasons":["settings.hooks：1 条"],"note":"未信任：已跳过工作区 hooks"}"#
                .as_bytes()
                .to_vec(),
        );
    }
    if method == "POST" && path == "/workspace/trust" {
        return (
            200,
            "application/json",
            r#"{"ok":true,"trusted":true,"gated":false,"changed":false,"reasons":[],"note":"已信任"}"#
                .as_bytes()
                .to_vec(),
        );
    }
    if method == "POST" && path.starts_with("/session/") && path.ends_with("/goal") {
        let body = if path.contains("goal-clear") {
            r#"{"session_id":"sess-1","action":"clear","goal":null}"#
        } else {
            r#"{"session_id":"sess-1","action":"set","goal":{"objective":"契约目标","iterations":0,"done":false,"updated_at":"2026-10-18T00:00:00Z"}}"#
        };
        return (200, "application/json", body.as_bytes().to_vec());
    }
    if method == "GET" && path == "/mcp" {
        // CLI `owo-agent mcp list` 的在线路径。
        return (
            200,
            "application/json",
            r#"{"servers":[{"name":"translate","transport":"stdio","command":"python","args":["server.py"],"connected":true}],"total":1}"#
                .as_bytes()
                .to_vec(),
        );
    }
    if method == "POST" && path == "/mcp/add" {
        return (
            200,
            "application/json",
            r#"{"ok":true,"name":"translate","connected":true,"tools":2}"#
                .as_bytes()
                .to_vec(),
        );
    }
    if method == "POST" && path == "/mcp/remove" {
        return (
            200,
            "application/json",
            r#"{"ok":true,"name":"translate"}"#.as_bytes().to_vec(),
        );
    }
    if method == "POST" && path == "/teams" {
        // `owo-agent workflow run` 的建队路径（202 + 成员）。
        return (
            202,
            "application/json",
            r#"{"team_id":"team-1","mode":"team","members":[{"role":"plan","assignee":"agent"}]}"#
                .as_bytes()
                .to_vec(),
        );
    }
    if method == "GET" && path == "/teams/team-1" {
        return (
            200,
            "application/json",
            r#"{"team_id":"team-1","status":"running"}"#.as_bytes().to_vec(),
        );
    }
    if method == "POST" && path == "/teams/team-1/steer" {
        return (
            200,
            "application/json",
            r#"{"ok":true,"command":"cancel"}"#.as_bytes().to_vec(),
        );
    }
    if method == "POST" && path.starts_with("/session/") && path.ends_with("/turn") {
        let body = concat!(
            "event: progress\ndata: {\"type\":\"progress\",\"message\":\"模型调用\",\"v\":1}\n\n",
            "event: token_delta\ndata: {\"type\":\"token_delta\",\"delta\":\"你\",\"v\":1}\n\n",
            "event: token_delta\ndata: {\"type\":\"token_delta\",\"delta\":\"好\",\"v\":1}\n\n",
            "event: final\ndata: {\"type\":\"final\",\"text\":\"你好\",\"v\":1}\n\n",
            "event: turn_stats\ndata: {\"type\":\"turn_stats\",\"steps\":1,\"duration_ms\":7,\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5,\"cost_usd\":0.0,\"completion_status\":\"accepted\"}\n\n"
        );
        return (200, "text/event-stream", body.as_bytes().to_vec());
    }
    if method == "GET" && path == "/teams/team/events/cursor-check" {
        let received = headers.lines().any(|line| {
            line.to_ascii_lowercase()
                .starts_with("last-event-id: 2026-10-02t00:00:00z#4")
        });
        let body = if received {
            b"data: {\"type\":\"cursor\",\"accepted\":true}\n\n".to_vec()
        } else {
            b"data: {\"type\":\"cursor\",\"accepted\":false}\n\n".to_vec()
        };
        return (200, "text/event-stream", body);
    }
    if method == "GET" && path == "/teams/team/events" {
        return (
            200,
            "text/event-stream",
            concat!(
                "id: 7\nevent: progress\ndata: {\"type\":\"progress\",\"progress\":{\"current_steps\":[]}}\n\n",
                "id: 8\nevent: state\ndata: {\"type\":\"state\",\"status\":\"Succeeded\"}\n\n"
            )
            .as_bytes()
            .to_vec(),
        );
    }
    if method == "POST" && path.contains("/answer/") {
        return (200, "application/json", br#"{"ok":true}"#.to_vec());
    }
    if method == "POST" && path.ends_with("/abort") {
        return (200, "application/json", br#"{"ok":true}"#.to_vec());
    }
    if method == "POST" && path == "/session" {
        return (
            200,
            "application/json",
            br#"{"id":"sess-1","workspace":"ws","model":"m","created_at":"2026-01-01T00:00:00Z","updated_at":"","title":null,"archived":false,"pinned":false,"parent_id":null,"fork_point":null}"#.to_vec(),
        );
    }
    (
        404,
        "application/json",
        br#"{"error":"not found"}"#.to_vec(),
    )
}

async fn spawn_fake_daemon() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (method, path, headers) = read_request(&mut socket).await;
                let (status, content_type, body) = route(&method, &path, &headers);
                let reason = if status == 200 { "OK" } else { "Not Found" };
                let head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.flush().await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{}", address.port())
}

async fn spawn_replay_daemon(interrupted: bool, failed: bool, split_final_stats: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let replay_page_requests = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let replay_page_requests = Arc::clone(&replay_page_requests);
            tokio::spawn(async move {
                let (method, path, _headers) = read_request(&mut socket).await;
                let (content_type, extra_headers, body) = if method == "POST"
                    && path.ends_with("/turn")
                {
                    (
                        "text/event-stream",
                        "X-Owo-Turn-Id: turn-1\r\n",
                        concat!(
                            "id: 1\nevent: progress\ndata: {\"type\":\"progress\",\"message\":\"start\"}\n\n",
                            "id: 2\nevent: token_delta\ndata: {\"type\":\"token_delta\",\"delta\":\"partial\"}\n\n"
                        )
                        .as_bytes()
                        .to_vec(),
                    )
                } else if method == "GET" && path.contains("/turn/events?") {
                    let page_request_index = replay_page_requests.fetch_add(1, Ordering::Relaxed);
                    let (events, state, next_after_seq, active) = if interrupted {
                        (serde_json::json!([]), "interrupted", 2, false)
                    } else if failed {
                        (
                            serde_json::json!([{
                                "session_id": "sess-1",
                                "turn_id": "turn-1",
                                "seq": 3,
                                "created_at": "2026-09-22T00:00:00Z",
                                "payload": {"type": "final", "text": "saved response"}
                            }, {
                                "session_id": "sess-1",
                                "turn_id": "turn-1",
                                "seq": 4,
                                "created_at": "2026-09-22T00:00:01Z",
                                "payload": {
                                    "type": "turn_stats",
                                    "steps": 1,
                                    "duration_ms": 7,
                                    "prompt_tokens": 3,
                                    "completion_tokens": 2,
                                    "total_tokens": 5,
                                    "cost_usd": 0.0,
                                    "completion_status": "accepted"
                                }
                            }, {
                                "session_id": "sess-1",
                                "turn_id": "turn-1",
                                "seq": 5,
                                "created_at": "2026-09-22T00:00:02Z",
                                "payload": {
                                    "type": "turn_failed",
                                    "message": "session save failed",
                                    "completion_status": "unverified"
                                }
                            }]),
                            "failed",
                            5,
                            false,
                        )
                    } else if split_final_stats && page_request_index == 0 {
                        (
                            serde_json::json!([{
                                "session_id": "sess-1",
                                "turn_id": "turn-1",
                                "seq": 3,
                                "created_at": "2026-09-22T00:00:00Z",
                                "payload": {"type": "final", "text": "resumed"}
                            }]),
                            "active",
                            3,
                            true,
                        )
                    } else {
                        (
                            serde_json::json!([{
                                "session_id": "sess-1",
                                "turn_id": "turn-1",
                                "seq": 3,
                                "created_at": "2026-09-22T00:00:00Z",
                                "payload": {"type": "final", "text": "resumed"}
                            }, {
                                "session_id": "sess-1",
                                "turn_id": "turn-1",
                                "seq": 4,
                                "created_at": "2026-09-22T00:00:01Z",
                                "payload": {
                                    "type": "turn_stats",
                                    "steps": 1,
                                    "duration_ms": 7,
                                    "prompt_tokens": 3,
                                    "completion_tokens": 2,
                                    "total_tokens": 5,
                                    "cost_usd": 0.0,
                                    "completion_status": "accepted"
                                }
                            }]),
                            "completed",
                            4,
                            false,
                        )
                    };
                    (
                        "application/json",
                        "",
                        serde_json::json!({
                            "events": events,
                            "active": active,
                            "state": state,
                            "next_after_seq": next_after_seq
                        })
                        .to_string()
                        .into_bytes(),
                    )
                } else {
                    ("application/json", "", b"{}".to_vec())
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.flush().await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{}", address.port())
}

#[tokio::test]
async fn client_resumes_turn_from_persisted_events_after_stream_eof() {
    let base_url = spawn_replay_daemon(false, false, false).await;
    let client = AgentClient::new(ClientConfig::new(base_url, None)).expect("client");
    let mut stream = client.open_turn("sess-1", "hello").await.expect("turn");
    let mut events = Vec::new();
    stream
        .drive(|event| events.push(event))
        .await
        .expect("drive");
    assert!(matches!(events.first(), Some(SseEvent::Progress { .. })));
    assert!(matches!(events.get(1), Some(SseEvent::TokenDelta { delta }) if delta == "partial"));
    assert!(matches!(events.get(2), Some(SseEvent::Final { text }) if text == "resumed"));
    assert!(matches!(
        events.get(3),
        Some(SseEvent::TurnStats {
            completion_status: owo_agent_protocol::CompletionStatusV1::Accepted,
            ..
        })
    ));
    assert_eq!(events.len(), 4);
}

#[tokio::test]
async fn client_continues_replay_when_final_and_turn_stats_are_on_separate_pages() {
    let base_url = spawn_replay_daemon(false, false, true).await;
    let client = AgentClient::new(ClientConfig::new(base_url, None)).expect("client");
    let mut stream = client.open_turn("sess-1", "hello").await.expect("turn");
    let mut events = Vec::new();
    stream
        .drive(|event| events.push(event))
        .await
        .expect("drive replay pages");
    assert!(matches!(events.get(2), Some(SseEvent::Final { text }) if text == "resumed"));
    assert!(matches!(
        events.get(3),
        Some(SseEvent::TurnStats {
            completion_status: owo_agent_protocol::CompletionStatusV1::Accepted,
            ..
        })
    ));
    assert_eq!(events.len(), 4);
}

#[tokio::test]
async fn client_delivers_replayed_turn_failure_without_losing_its_completion_status() {
    let base_url = spawn_replay_daemon(false, true, false).await;
    let client = AgentClient::new(ClientConfig::new(base_url, None)).expect("client");
    let mut stream = client.open_turn("sess-1", "hello").await.expect("turn");
    let mut events = Vec::new();
    stream
        .drive(|event| events.push(event))
        .await
        .expect("TurnFailed is a terminal event, not a missing-details protocol error");
    assert!(matches!(
        events.get(3),
        Some(SseEvent::TurnStats {
            completion_status: owo_agent_protocol::CompletionStatusV1::Accepted,
            ..
        })
    ));
    assert!(matches!(
        events.get(4),
        Some(SseEvent::TurnFailed {
            completion_status: owo_agent_protocol::CompletionStatusV1::Unverified,
            ..
        })
    ));
    assert_eq!(
        events.len(),
        5,
        "post-stats failure must not be hidden by early stream termination"
    );
}

#[tokio::test]
async fn client_reports_interrupted_turn_after_replaying_partial_output() {
    let base_url = spawn_replay_daemon(true, false, false).await;
    let client = AgentClient::new(ClientConfig::new(base_url, None)).expect("client");
    let mut stream = client.open_turn("sess-1", "hello").await.expect("turn");
    let mut events = Vec::new();
    let result = stream.drive(|event| events.push(event)).await;
    assert!(matches!(events.last(), Some(SseEvent::TokenDelta { delta }) if delta == "partial"));
    assert!(
        matches!(result, Err(owo_agent_client::ClientError::Protocol(message)) if message.contains("中断"))
    );
}

#[tokio::test]
async fn workspace_trust_round_trips_against_fake_daemon() {
    // 客户端/路由配对守卫：路径或方法写错时（例如漏了 /workspace 前缀）这里立刻失败。
    let base_url = spawn_fake_daemon().await;
    let client = AgentClient::new(ClientConfig::new(base_url, Some("test-token".to_string())))
        .expect("client");

    let status = client.workspace_trust().await.expect("trust status");
    assert_eq!(status["trusted"], serde_json::json!(false));
    assert_eq!(status["gated"], serde_json::json!(true));
    assert_eq!(
        status["reasons"][0],
        serde_json::json!("settings.hooks：1 条")
    );

    let granted = client.set_workspace_trust(true).await.expect("grant trust");
    assert_eq!(granted["ok"], serde_json::json!(true));
    assert_eq!(granted["trusted"], serde_json::json!(true));
}

#[tokio::test]
async fn session_goal_round_trips_against_fake_daemon() {
    // 客户端/路由配对守卫：/session/{id}/goal 的路径与方法写错时这里立刻失败。
    let base_url = spawn_fake_daemon().await;
    let client = AgentClient::new(ClientConfig::new(base_url, Some("test-token".to_string())))
        .expect("client");

    let set = client
        .session_goal("sess-1", "set", Some("契约目标"))
        .await
        .expect("set goal");
    assert_eq!(set["action"], serde_json::json!("set"));
    assert_eq!(set["goal"]["objective"], serde_json::json!("契约目标"));
    assert_eq!(set["goal"]["iterations"], serde_json::json!(0));

    let cleared = client
        .session_goal("goal-clear", "clear", None)
        .await
        .expect("clear goal");
    assert!(cleared["goal"].is_null());
}

#[tokio::test]
async fn team_create_status_cancel_round_trip_against_fake_daemon() {
    // `owo-agent workflow run` 的客户端/路由配对守卫。
    let base_url = spawn_fake_daemon().await;
    let client = AgentClient::new(ClientConfig::new(base_url, Some("test-token".to_string())))
        .expect("client");

    let created = client
        .create_team("用工作流建队", "team", Some("video-recreate"), true)
        .await
        .expect("create team");
    assert_eq!(created["team_id"], serde_json::json!("team-1"));
    assert_eq!(created["members"].as_array().unwrap().len(), 1);

    let status = client.get_team("team-1").await.expect("get team");
    assert_eq!(status["status"], serde_json::json!("running"));

    let cancelled = client.cancel_team("team-1").await.expect("cancel team");
    assert_eq!(cancelled["ok"], serde_json::json!(true));
}

#[tokio::test]
async fn mcp_management_round_trips_against_fake_daemon() {
    // 客户端/路由配对守卫：/mcp、/mcp/add、/mcp/remove 的路径与方法写错时立刻失败。
    let base_url = spawn_fake_daemon().await;
    let client = AgentClient::new(ClientConfig::new(base_url, Some("test-token".to_string())))
        .expect("client");

    let listed = client.mcp_list().await.expect("mcp list");
    assert_eq!(listed["total"], serde_json::json!(1));
    assert_eq!(listed["servers"][0]["name"], serde_json::json!("translate"));

    let added = client
        .mcp_add(
            "translate",
            "stdio",
            "python",
            &["server.py".to_string()],
            None,
        )
        .await
        .expect("mcp add");
    assert_eq!(added["ok"], serde_json::json!(true));
    assert_eq!(added["connected"], serde_json::json!(true));
    assert_eq!(added["tools"], serde_json::json!(2));

    let removed = client.mcp_remove("translate").await.expect("mcp remove");
    assert_eq!(removed["name"], serde_json::json!("translate"));
}

#[tokio::test]
async fn client_round_trips_against_fake_daemon() {
    let base_url = spawn_fake_daemon().await;
    let client = AgentClient::new(ClientConfig::new(base_url, Some("test-token".to_string())))
        .expect("client");

    let health = client.health().await.expect("health");
    assert!(health.healthy);
    assert_eq!(health.api_version, "0.7");

    assert!(client.list_sessions().await.expect("list").is_empty());

    let session = client.create_session("ws").await.expect("create");
    assert_eq!(session.id, "sess-1");

    let mut stream = client.open_turn(&session.id, "hi").await.expect("turn");
    let mut deltas = Vec::new();
    let mut final_text = None;
    let mut completion_status = None;
    stream
        .drive(|event| match event {
            SseEvent::TokenDelta { delta } => deltas.push(delta),
            SseEvent::Final { text } => final_text = Some(text),
            SseEvent::TurnStats {
                completion_status: status,
                ..
            } => completion_status = Some(status),
            _ => {}
        })
        .await
        .expect("drive");
    assert_eq!(deltas, vec!["你".to_string(), "好".to_string()]);
    assert_eq!(final_text.as_deref(), Some("你好"));
    assert_eq!(
        completion_status,
        Some(owo_agent_protocol::CompletionStatusV1::Accepted)
    );

    let answer = client
        .answer_question(&session.id, "q-1", "用户补充信息")
        .await
        .expect("answer question");
    assert_eq!(answer["ok"], true);

    let cancelled = client.cancel_turn(&session.id).await.expect("cancel");
    assert_eq!(cancelled["ok"], true);
}

#[tokio::test]
async fn client_sends_last_event_id_when_reopening_stream() {
    let base_url = spawn_fake_daemon().await;
    let client = AgentClient::new(ClientConfig::new(base_url, None)).expect("client");
    let mut stream = client
        .open_event_stream_after(
            "/teams/team/events/cursor-check",
            Some("2026-10-02T00:00:00Z#4"),
        )
        .await
        .expect("reopen team events");
    let event = stream
        .next_event()
        .await
        .expect("cursor response")
        .expect("valid SSE");
    assert_eq!(event["accepted"], true);
}

#[tokio::test]
async fn client_reads_team_event_stream_incrementally() {
    let base_url = spawn_fake_daemon().await;
    let client = AgentClient::new(ClientConfig::new(base_url, None)).expect("client");
    let mut stream = client
        .open_event_stream("/teams/team/events")
        .await
        .expect("open team events");

    let progress = stream
        .next_event()
        .await
        .expect("progress frame")
        .expect("valid frame");
    assert_eq!(progress["type"], "progress");
    assert_eq!(progress["progress"]["current_steps"], serde_json::json!([]));
    assert_eq!(stream.last_event_id(), Some("7"));

    let state = stream
        .next_event()
        .await
        .expect("state frame")
        .expect("valid frame");
    assert_eq!(state["type"], "state");
    assert_eq!(state["status"], "Succeeded");
    assert_eq!(stream.last_event_id(), Some("8"));
    assert!(
        stream.next_event().await.is_none(),
        "EOF should terminate the stream"
    );
}

#[test]
fn json_sse_buffer_handles_fragmented_utf8_and_multiline_frames() {
    let mut buffer = JsonSseBuffer::new();
    buffer.push(b": keepalive\r\ndata: {\"type\":\"progress\",\"text\":\"\xe4\xb8");
    assert!(buffer.next_event().is_none());
    buffer.push(b"\xad\"}\r\n\r\n");
    assert_eq!(buffer.next_event().unwrap().unwrap()["text"], "中");

    buffer.push(b"data: {\"type\":\"progress\",\n");
    buffer.push(b"data: \"n\":2}\n\n");
    assert_eq!(buffer.next_event().unwrap().unwrap()["n"], 2);
}

#[test]
fn json_sse_buffer_flushes_eof_frame_and_rejects_oversized_input() {
    let mut buffer = JsonSseBuffer::new();
    buffer.push(b"data: {\"ok\":true}");
    buffer.finish();
    assert_eq!(buffer.next_event().unwrap().unwrap()["ok"], true);
    assert!(buffer.next_event().is_none());

    let mut oversized = JsonSseBuffer::new();
    oversized.push(&vec![b'x'; 1024 * 1024 + 1]);
    assert!(oversized.next_event().unwrap().is_err());
}
