//! 命名管道服务端集成测试（E1.2 验收）。
//!
//! 覆盖：帧回显往返、超长帧拒绝、超时断开、断连后服务端继续接受新连接（多实例）。

#![cfg(windows)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use owo_intent_gateway::frame::{read_frame, write_frame};
use owo_intent_gateway::pipe::{run_pipe_server, FrameHandler};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::sync::watch;

/// 回显处理器：原样返回载荷；记录处理次数。
struct EchoHandler {
    handled: AtomicUsize,
}

#[async_trait::async_trait]
impl FrameHandler for EchoHandler {
    async fn handle(&self, payload: Vec<u8>) -> Option<Vec<u8>> {
        self.handled.fetch_add(1, Ordering::SeqCst);
        Some(payload)
    }
}

/// 慢处理器：处理前睡眠，用于超时测试。
struct SlowHandler {
    delay: Duration,
}

#[async_trait::async_trait]
impl FrameHandler for SlowHandler {
    async fn handle(&self, payload: Vec<u8>) -> Option<Vec<u8>> {
        tokio::time::sleep(self.delay).await;
        Some(payload)
    }
}

fn unique_pipe(tag: &str) -> String {
    format!(r"\\.\pipe\owo-ime-test-{}-{}", tag, std::process::id())
}

/// 启动服务端，返回 shutdown 发送端（测试结束置 true 停止服务）。
fn spawn_server(
    pipe_name: &str,
    timeout: Duration,
    handler: Arc<dyn FrameHandler>,
) -> watch::Sender<bool> {
    let (tx, rx) = watch::channel(false);
    let pipe_name = pipe_name.to_string();
    tokio::spawn(async move {
        let _ = run_pipe_server(&pipe_name, timeout, handler, rx).await;
    });
    tx
}

/// 带重试的客户端连接（首次 open 可能遇到 ERROR_PIPE_BUSY）。
async fn connect(pipe_name: &str) -> NamedPipeClient {
    for _ in 0..50 {
        match ClientOptions::new().open(pipe_name) {
            Ok(client) => return client,
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    panic!("管道 {pipe_name} 连接超时");
}

#[tokio::test]
async fn echo_roundtrip() {
    let pipe_name = unique_pipe("echo");
    let handler = Arc::new(EchoHandler {
        handled: AtomicUsize::new(0),
    });
    let shutdown = spawn_server(&pipe_name, Duration::from_secs(5), handler.clone());

    let mut client = connect(&pipe_name).await;
    write_frame(&mut client, b"hello-owo").await.unwrap();
    let response = read_frame(&mut client).await.unwrap();
    assert_eq!(response, b"hello-owo");
    assert_eq!(handler.handled.load(Ordering::SeqCst), 1);

    let _ = shutdown.send(true);
}

#[tokio::test]
async fn bad_frame_rejected_and_server_survives() {
    let pipe_name = unique_pipe("badframe");
    let handler = Arc::new(EchoHandler {
        handled: AtomicUsize::new(0),
    });
    let shutdown = spawn_server(&pipe_name, Duration::from_secs(5), handler.clone());

    // 发 0 长度帧：服务端应拒绝（不回包、连接关闭），且 handler 不被调用。
    let mut bad_client = connect(&pipe_name).await;
    use tokio::io::AsyncWriteExt;
    bad_client.write_all(&0_u32.to_le_bytes()).await.unwrap();
    bad_client.flush().await.unwrap();
    // 服务端断开后，读取应失败（EOF / 断管）。
    assert!(
        read_frame(&mut bad_client).await.is_err(),
        "非法帧必须被拒绝"
    );
    assert_eq!(handler.handled.load(Ordering::SeqCst), 0);

    // 服务端仍可接受新连接并正常服务。
    let mut client = connect(&pipe_name).await;
    write_frame(&mut client, b"still-alive").await.unwrap();
    let response = read_frame(&mut client).await.unwrap();
    assert_eq!(response, b"still-alive");
    assert_eq!(handler.handled.load(Ordering::SeqCst), 1);

    let _ = shutdown.send(true);
}

#[tokio::test]
async fn op_timeout_disconnects() {
    let pipe_name = unique_pipe("timeout");
    let handler = Arc::new(SlowHandler {
        delay: Duration::from_millis(800),
    });
    let shutdown = spawn_server(&pipe_name, Duration::from_millis(150), handler);

    let mut client = connect(&pipe_name).await;
    write_frame(&mut client, b"please-wait").await.unwrap();
    // 服务端 150ms 超时断开 → 客户端读失败。
    let result = read_frame(&mut client).await;
    assert!(result.is_err(), "超时后连接必须断开而不是挂起");

    let _ = shutdown.send(true);
}

#[tokio::test]
async fn concurrent_connections() {
    let pipe_name = unique_pipe("concurrent");
    let handler = Arc::new(EchoHandler {
        handled: AtomicUsize::new(0),
    });
    let shutdown = spawn_server(&pipe_name, Duration::from_secs(5), handler.clone());

    let mut tasks = Vec::new();
    for index in 0..3 {
        let pipe_name = pipe_name.clone();
        tasks.push(tokio::spawn(async move {
            let mut client = connect(&pipe_name).await;
            let payload = format!("payload-{index}");
            write_frame(&mut client, payload.as_bytes()).await.unwrap();
            let response = read_frame(&mut client).await.unwrap();
            assert_eq!(response, payload.as_bytes());
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(handler.handled.load(Ordering::SeqCst), 3);

    let _ = shutdown.send(true);
}

#[tokio::test]
async fn shutdown_stops_server() {
    let pipe_name = unique_pipe("shutdown");
    let handler = Arc::new(EchoHandler {
        handled: AtomicUsize::new(0),
    });
    let shutdown = spawn_server(&pipe_name, Duration::from_secs(5), handler);

    // 首次连接验证服务可用。
    let mut client = connect(&pipe_name).await;
    write_frame(&mut client, b"before-shutdown").await.unwrap();
    assert_eq!(read_frame(&mut client).await.unwrap(), b"before-shutdown");
    drop(client);

    shutdown.send(true).unwrap();
    // 等待循环退出并释放管道：之后应完全连不上。
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut connected = false;
    for _ in 0..5 {
        if ClientOptions::new().open(&pipe_name).is_ok() {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!connected, "shutdown 后不得再接受连接");
}

#[tokio::test]
async fn occupied_pipe_name_is_rejected_without_joining_existing_server() {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = unique_pipe("occupied");
    let occupied = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .unwrap();
    let handler = Arc::new(EchoHandler {
        handled: AtomicUsize::new(0),
    });
    // Keep this sender alive so shutdown is not an alternate reason to return.
    let (shutdown_tx, shutdown) = watch::channel(false);
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        run_pipe_server(&name, Duration::from_secs(1), handler.clone(), shutdown),
    )
    .await
    .expect("occupied endpoint must reject immediately, not start serving");
    assert!(matches!(
        result,
        Err(owo_intent_gateway::pipe::PipeError::Io(_))
    ));
    assert_eq!(handler.handled.load(Ordering::SeqCst), 0);
    drop(shutdown_tx);
    drop(occupied);
}

#[tokio::test]
async fn endpoint_remains_owned_between_fast_reconnecting_clients() {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = unique_pipe("continuous-ownership");
    let handler = Arc::new(EchoHandler {
        handled: AtomicUsize::new(0),
    });
    let shutdown = spawn_server(&name, Duration::from_secs(1), handler.clone());
    for i in 0..20 {
        let mut client = connect(&name).await;
        write_frame(&mut client, b"owned").await.unwrap();
        assert_eq!(read_frame(&mut client).await.unwrap(), b"owned");
        drop(client);
        // Yield to let the just-completed connection release its handle.
        tokio::task::yield_now().await;
        assert!(
            ServerOptions::new()
                .first_pipe_instance(true)
                .create(&name)
                .is_err(),
            "endpoint lost ownership after connection {i}"
        );
    }
    assert_eq!(handler.handled.load(Ordering::SeqCst), 20);
    shutdown.send(true).unwrap();
}
