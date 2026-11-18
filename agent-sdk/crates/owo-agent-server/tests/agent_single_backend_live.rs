//! Single Agent 真实执行后端的端到端验收（默认忽略：会真的调用模型并产生费用）。
//!
//! 这条测试走的是**产品路径**：HTTP `/agent-tasks` → Task Runtime 分发 → 真实
//! Agent 回合（含工具宿主）→ 提交执行事实 → 独立验证。
//!
//! 运行（凭据只经环境变量注入，不要写进任何文件）：
//!
//! ```powershell
//! $env:OPENAI_API_KEY = [Environment]::GetEnvironmentVariable('OPENAI_API_KEY','User')
//! cargo test -p owo-agent-server --test agent_single_backend_live -- --ignored --nocapture
//! ```
//!
//! 不在 CI 默认运行：CI 没有模型凭据，也不应该为契约测试产生云端调用。

use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use std::sync::Arc;
use tower::ServiceExt;

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
        .header(header::HOST, "127.0.0.1:4096")
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

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "真实模型调用：需显式运行并提供 OPENAI_API_KEY / 本地兼容端点"]
async fn single_agent_task_runs_a_real_turn() {
    // 没有凭据时明确跳过，而不是失败成一个看不懂的错误。
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("跳过：未设置 OPENAI_API_KEY（本测试不做云端调用）");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let provider: Arc<dyn owo_agent_core::ModelProvider> =
        Arc::new(owo_agent_core::gateway::ResilientProvider::from_env().expect("模型配置不可用"));
    let agent = Agent::new(
        provider,
        ToolRegistry::with_all_builtin_capabilities(),
        Policy::new(&workspace),
        Default::default(),
    );
    let data_root = temp.path().join("data");
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        SqliteSessionStore::open(&workspace.join("index.db")).unwrap(),
        data_root.join("traces"),
        data_root,
        workspace.clone(),
    ));
    let app = build_router(Arc::clone(&state));

    let created = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/agent-tasks",
            Some(
                r#"{"user_goal":"在当前工作区根目录创建 hello.txt，内容为 hello owo。完成后用一句话说明你做了什么。","execution_strategy":"single_agent"}"#,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(created.status().as_u16(), 201);
    let created_body = json_body(created).await;
    let task_id = created_body["task"]["task_id"]
        .as_str()
        .unwrap()
        .to_string();
    println!("任务：{task_id}");

    let started = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/agent-tasks/{task_id}/start"),
            Some("{}"),
        ))
        .await
        .unwrap();
    let started_body = json_body(started).await;
    println!("分发：{started_body}");
    assert_eq!(started_body["dispatched"], true);

    let mut state_text = String::new();
    for _ in 0..600 {
        let detail = app
            .clone()
            .oneshot(request(
                &state,
                "GET",
                &format!("/agent-tasks/{task_id}"),
                None,
            ))
            .await
            .unwrap();
        let detail_body = json_body(detail).await;
        state_text = detail_body["task"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if state_text != "running" {
            let events = app
                .clone()
                .oneshot(request(
                    &state,
                    "GET",
                    &format!("/agent-tasks/{task_id}/events?limit=200"),
                    None,
                ))
                .await
                .unwrap();
            let events_body = json_body(events).await;
            println!("最终状态：{state_text}");
            println!("详情：{detail_body}");
            println!("事件：{events_body}");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_ne!(state_text, "running", "真实回合应在时限内结束");
    assert_ne!(
        state_text, "failed",
        "真实执行不应因为后端接线问题失败：{state_text}"
    );
    assert!(
        workspace.join("hello.txt").is_file(),
        "Agent 应在工作区创建 hello.txt"
    );
}
