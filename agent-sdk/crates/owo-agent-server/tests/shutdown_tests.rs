//! R8 服务端韧性契约测试（最小）：并发 turn 上限、优雅关闭信号、强杀恢复 pid 文件。

use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use owo_agent_server::shutdown::{
    ForceKillRecovery, PidFile, ShutdownGate, TurnBusy, DEFAULT_MAX_CONCURRENT_TURNS,
};
use std::sync::Arc;
use tower::ServiceExt;

// ---------- 单元：ShutdownGate ----------

#[test]
fn gate_enforces_global_concurrency_cap() {
    let gate = ShutdownGate::new(1);
    assert_eq!(gate.max_concurrent(), 1);
    assert_eq!(gate.active_turns(), 0);
    let first = gate.try_acquire_turn().unwrap();
    assert_eq!(gate.active_turns(), 1);
    assert_eq!(
        gate.try_acquire_turn().unwrap_err(),
        TurnBusy::AtCapacity,
        "超出上限必须拒绝"
    );
    drop(first);
    assert_eq!(gate.active_turns(), 0, "permit 释放后计数归零");
    assert!(gate.try_acquire_turn().is_ok(), "释放后应可再次获取");
}

#[tokio::test]
async fn gate_graceful_shutdown_drains_in_flight() {
    let gate = Arc::new(ShutdownGate::new(2));
    let _permit = gate.try_acquire_turn().unwrap();
    assert!(!gate.shutting_down());

    // 请求关闭：拒绝新回合。
    let active = gate.request_shutdown();
    assert_eq!(active, 1);
    assert!(gate.shutting_down());
    assert_eq!(
        gate.try_acquire_turn().unwrap_err(),
        TurnBusy::ShuttingDown,
        "关闭中必须拒绝新回合"
    );

    // 在途未完成 → await_drain 超时返回剩余数。
    let gate_clone = Arc::clone(&gate);
    let remaining = tokio::time::timeout(std::time::Duration::from_secs(3), async move {
        gate_clone
            .await_drain(std::time::Duration::from_millis(100))
            .await
    })
    .await
    .unwrap();
    assert_eq!(remaining, 1, "在途未完成时超时返回剩余数");

    // 完成在途 → drain 归零。
    drop(_permit);
    let gate_clone = Arc::clone(&gate);
    let remaining = tokio::time::timeout(std::time::Duration::from_secs(3), async move {
        gate_clone
            .await_drain(std::time::Duration::from_secs(1))
            .await
    })
    .await
    .unwrap();
    assert_eq!(remaining, 0, "在途完成后 drain 归零");
}

// ---------- 单元：强杀恢复 ----------

#[test]
fn pid_file_cleans_on_drop_and_recovers_stale() {
    let temp = tempfile::tempdir().unwrap();

    // 正常路径：create → drop 自动清理。
    {
        let pid = PidFile::create(temp.path()).unwrap();
        assert!(pid.path().is_file());
    }
    assert!(
        !temp.path().join("server.pid").exists(),
        "Drop 后 pid 文件应被清理"
    );

    // 强杀残留：陈旧 pid（不存在进程）→ 清理并报告。
    std::fs::write(temp.path().join("server.pid"), "999999999").unwrap();
    let recovery = owo_agent_server::shutdown::recover_force_kill(temp.path()).unwrap();
    assert_eq!(
        recovery,
        Some(ForceKillRecovery {
            stale_pid: Some(999_999_999),
            cleaned: true,
        })
    );
    assert!(!temp.path().join("server.pid").exists(), "强杀残留应被清理");

    // 无 pid 文件 → None。
    assert!(owo_agent_server::shutdown::recover_force_kill(temp.path())
        .unwrap()
        .is_none());
}

#[cfg(windows)]
#[test]
fn process_liveness_distinguishes_current_and_impossible_pid() {
    assert!(
        owo_agent_server::shutdown::process_alive(std::process::id()),
        "当前测试进程必须被识别为存活"
    );
    assert!(
        !owo_agent_server::shutdown::process_alive(u32::MAX),
        "不可能的 pid 不应阻塞陈旧 pid 文件恢复"
    );
}

#[cfg(windows)]
#[test]
fn exited_child_with_open_handle_is_not_alive() {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    assert!(child.wait().unwrap().success());
    // The Child still owns the original process handle after wait().
    assert!(!owo_agent_server::shutdown::process_alive(pid));
    drop(child);
}

/// pid 复用防护：pid 活着 ≠ 上一代服务活着。
///
/// Windows 会把死掉核心的 pid 分给无关进程；若陈旧 pid 判定只看存活，桌面壳
/// 的新核心会等满 40s 后按双开冲突拒绝启动，且每次重启都复现（锁死不自愈）。
#[cfg(windows)]
#[test]
fn pid_reuse_guard_requires_owo_agent_image() {
    let me = std::process::id();
    assert!(
        owo_agent_server::shutdown::process_alive(me),
        "前置条件：当前测试进程是活的"
    );
    assert!(
        !owo_agent_server::shutdown::process_is_previous_server(me),
        "测试进程映像名不是 owo-agent，不能被当成上一代服务（否则 pid 复用会锁死启动）"
    );
    assert!(
        !owo_agent_server::shutdown::process_is_previous_server(u32::MAX),
        "不可能的 pid 必须判死"
    );
    // 陈旧 pid 指向"活着的无关进程"时，recover_force_kill 必须放行而不是报双开冲突。
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("server.pid"), me.to_string()).unwrap();
    let recovery = owo_agent_server::shutdown::recover_force_kill(temp.path())
        .expect("被复用的 pid 不应触发双开冲突");
    assert_eq!(
        recovery,
        Some(ForceKillRecovery {
            stale_pid: Some(me),
            cleaned: true,
        }),
        "被复用的 pid 应按陈旧锁清理"
    );
    assert!(!temp.path().join("server.pid").exists());
}

// ---------- 路由面：/server/status + /server/shutdown ----------

struct IdleProvider;

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("IdleProvider 不应被调用".to_string())
    }
}

async fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    (state, temp)
}

fn request(
    state: &Arc<owo_agent_server::AppState>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> axum::http::Request<axum::body::Body> {
    use axum::http::{header, Method, Request};
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

#[tokio::test]
async fn server_status_reports_gate_and_storage() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "GET", "/server/status", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        status["shutdown_gate"]["max_concurrent_turns"], DEFAULT_MAX_CONCURRENT_TURNS as u64,
        "默认并发上限应上报"
    );
    assert_eq!(status["shutdown_gate"]["active_turns"], 0);
    assert_eq!(status["shutdown_gate"]["shutting_down"], false);
    assert_eq!(status["storage"]["read_only"], false);
}

#[tokio::test]
async fn shutdown_requires_confirm_and_blocks_new_turns() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // 无 confirm → 400。
    let no_confirm = app
        .clone()
        .oneshot(request(&state, "POST", "/server/shutdown", Some("{}")))
        .await
        .unwrap();
    assert_eq!(no_confirm.status().as_u16(), 400, "缺少二次确认应 400");

    // confirm → 200 且停止接收。
    let confirmed = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/server/shutdown",
            Some(r#"{"confirm":true}"#),
        ))
        .await
        .unwrap();
    assert_eq!(confirmed.status().as_u16(), 200);
    assert!(state.shutdown_gate.shutting_down());

    // 关闭中新回合 → 503。
    let created = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/session",
            Some(r#"{"workspace":".","model":"idle"}"#),
        ))
        .await
        .unwrap();
    let session_id: String = {
        let bytes = axum::body::to_bytes(created.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    };
    let turn = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{session_id}/turn"),
            Some(r#"{"prompt":"hi"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(turn.status().as_u16(), 503, "关闭中拒绝新回合（503）");
}
