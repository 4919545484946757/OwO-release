//! 意图网关进程：命名管道 `\.\pipe\OwO.Agent.External.v1` ↔ 本机 Agent Daemon。
//!
//! 启动顺序：解析 data_root → 读 `runtime/daemon.json` 得端口 → 准备隔离工作区 →
//! 运行管道服务。token 在首次请求时从 `<data_root>/auth/token` 读取。
//!
//! 环境变量：`OWO_AGENT_DATA`（数据根）、`OWO_GATEWAY_DAEMON_URL`（覆盖 Daemon 地址，
//! 仅允许 127.0.0.1/localhost 回环）、`RUST_LOG`。

#[cfg(windows)]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    use std::sync::Arc;
    use std::time::Duration;

    use owo_intent_gateway::http::{
        daemon_base_url, gateway_workspace, resolve_data_root, token_path,
    };
    use owo_intent_gateway::{
        run_pipe_server, ImeBridge, ImeState, OwoHttpClient, DEFAULT_OP_TIMEOUT_MS,
        DEFAULT_PIPE_NAME, DEFAULT_SESSION_TTL,
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let data_root = resolve_data_root();
    let base_url = match std::env::var("OWO_GATEWAY_DAEMON_URL") {
        Ok(url) => url,
        Err(_) => match daemon_base_url(&data_root) {
            Ok(url) => url,
            Err(error) => {
                tracing::error!(%error, "找不到运行中的 Agent Daemon（先启动 owo-agent serve）");
                return std::process::ExitCode::from(3);
            }
        },
    };
    let workspace = gateway_workspace(&data_root);
    if let Err(error) = std::fs::create_dir_all(&workspace) {
        tracing::error!(%error, path = %workspace.display(), "无法创建网关工作区");
        return std::process::ExitCode::from(4);
    }

    let state = Arc::new(ImeState::new(DEFAULT_SESSION_TTL));
    let client = match OwoHttpClient::new(base_url, token_path(&data_root)) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            tracing::error!(%error, "Invalid Daemon origin");
            return std::process::ExitCode::from(2);
        }
    };
    let base_url = client.base_url().to_string();
    let bridge = ImeBridge::new(
        state.clone(),
        client,
        workspace.to_string_lossy().into_owned(),
        base_url.clone(),
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let gc_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            gc_state.gc();
        }
    });
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(true);
    });

    tracing::info!(daemon = %base_url, "意图网关启动");
    match run_pipe_server(
        DEFAULT_PIPE_NAME,
        Duration::from_millis(DEFAULT_OP_TIMEOUT_MS),
        bridge,
        shutdown_rx,
    )
    .await
    {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "管道服务退出");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("owo-intent-gateway 仅支持 Windows（命名管道）");
    std::process::exit(1);
}
