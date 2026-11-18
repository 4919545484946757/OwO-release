//! 执行工作区 HTTP 契约（Agent V2 Phase 6）。
//!
//! 覆盖：Git Worktree 创建 → 任务写入隔离 → Diff 审阅 → 应用 / 回滚 / 拒绝 →
//! 释放；非 Git 工作区与未实现类型明确拒绝；任务以租约 path 为 workspace_ref 时
//! 真的在隔离目录里建会话。

use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tower::ServiceExt;

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

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git 必须可执行");
    assert!(
        output.status.success(),
        "git {} 失败：{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(dir: &Path) {
    git(dir, &["init"]);
    git(dir, &["config", "user.email", "owo@test.local"]);
    git(dir, &["config", "user.name", "OwO Test"]);
    std::fs::write(dir.join("app.txt"), "base\n").unwrap();
    git(dir, &["add", "app.txt"]);
    git(dir, &["commit", "-m", "base"]);
}

async fn test_state(with_git: bool) -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    if with_git {
        init_repo(&workspace);
    }
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    // 会话库与 traces 放在仓库外，保证 git status 只包含任务自己的改动。
    let store = SqliteSessionStore::open(&temp.path().join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        temp.path().join("traces"),
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
        .header(header::HOST, "127.0.0.1:4096")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

async fn json_of(response: axum::response::Response) -> (u16, serde_json::Value) {
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    )
}

async fn create_lease(
    app: &axum::Router,
    state: &Arc<owo_agent_server::AppState>,
    kind: &str,
) -> (u16, serde_json::Value) {
    let body = format!(r#"{{"kind":"{kind}"}}"#);
    let response = app
        .clone()
        .oneshot(request(state, "POST", "/workspaces", Some(&body)))
        .await
        .unwrap();
    json_of(response).await
}

#[tokio::test]
async fn worktree_lease_reviews_applies_rolls_back_and_releases() {
    if !git_available() {
        return;
    }
    let (state, temp) = test_state(true).await;
    let app = build_router(Arc::clone(&state));
    let repo = state.workspace.clone();

    let (status, created) = create_lease(&app, &state, "git_worktree").await;
    assert_eq!(status, 201, "{created}");
    let workspace = &created["workspace"];
    let id = workspace["workspace_id"].as_str().unwrap().to_string();
    let lease_path = std::path::PathBuf::from(workspace["path"].as_str().unwrap());
    assert!(lease_path.is_dir());
    assert_ne!(lease_path, repo);

    std::fs::write(lease_path.join("app.txt"), "base\ntask-change\n").unwrap();
    std::fs::write(lease_path.join("new.txt"), "created\n").unwrap();

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/workspaces/{id}/diff"),
            None,
        ))
        .await
        .unwrap();
    let (status, diff) = json_of(response).await;
    assert_eq!(status, 200, "{diff}");
    assert_eq!(diff["empty"], false);
    let files: Vec<&str> = diff["diff"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|file| file["path"].as_str())
        .collect();
    assert!(files.contains(&"app.txt"));
    assert!(files.contains(&"new.txt"));
    assert_eq!(
        std::fs::read_to_string(repo.join("app.txt")).unwrap(),
        "base\n",
        "创建租约/审阅 Diff 不得修改原工作区"
    );

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/workspaces/{id}/apply"),
            Some("{}"),
        ))
        .await
        .unwrap();
    let (status, applied) = json_of(response).await;
    assert_eq!(status, 200, "{applied}");
    assert_eq!(
        std::fs::read_to_string(repo.join("app.txt")).unwrap(),
        "base\ntask-change\n"
    );

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/workspaces/{id}/rollback"),
            Some("{}"),
        ))
        .await
        .unwrap();
    let (status, rolled) = json_of(response).await;
    assert_eq!(status, 200, "{rolled}");
    assert_eq!(
        std::fs::read_to_string(repo.join("app.txt")).unwrap(),
        "base\n"
    );
    assert!(!repo.join("new.txt").exists());

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "DELETE",
            &format!("/workspaces/{id}"),
            None,
        ))
        .await
        .unwrap();
    let (status, released) = json_of(response).await;
    assert_eq!(status, 200, "{released}");
    assert_eq!(released["workspace"]["state"], "released");
    assert!(!lease_path.exists(), "释放后 worktree 目录必须删除");
    drop(temp);
}

#[tokio::test]
async fn rejecting_a_lease_never_touches_the_original_workspace() {
    if !git_available() {
        return;
    }
    let (state, _temp) = test_state(true).await;
    let app = build_router(Arc::clone(&state));
    let repo = state.workspace.clone();

    let (status, created) = create_lease(&app, &state, "git_worktree").await;
    assert_eq!(status, 201);
    let id = created["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();
    let lease_path = std::path::PathBuf::from(created["workspace"]["path"].as_str().unwrap());
    std::fs::write(lease_path.join("app.txt"), "rejected\n").unwrap();

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/workspaces/{id}/reject"),
            Some("{}"),
        ))
        .await
        .unwrap();
    let (status, rejected) = json_of(response).await;
    assert_eq!(status, 200, "{rejected}");
    assert_eq!(rejected["workspace"]["state"], "rejected");
    assert_eq!(
        std::fs::read_to_string(repo.join("app.txt")).unwrap(),
        "base\n"
    );
    assert!(!lease_path.exists());
}

#[tokio::test]
async fn unsupported_and_non_git_workspaces_are_explicit() {
    let (state, _temp) = test_state(false).await;
    let app = build_router(Arc::clone(&state));

    let (status, unsupported) = create_lease(&app, &state, "local_sandbox").await;
    assert_eq!(status, 501, "{unsupported}");
    assert_eq!(unsupported["code"], "WORKSPACE_UNSUPPORTED");

    let (status, remote) = create_lease(&app, &state, "remote_workspace").await;
    assert_eq!(status, 501, "{remote}");
    assert_eq!(remote["code"], "WORKSPACE_UNSUPPORTED");

    let (status, invalid) = create_lease(&app, &state, "container_magic").await;
    assert_eq!(status, 400, "{invalid}");
    assert_eq!(invalid["code"], "INVALID_REQUEST");

    if git_available() {
        let (status, not_git) = create_lease(&app, &state, "git_worktree").await;
        assert_eq!(status, 400, "{not_git}");
        assert_eq!(not_git["code"], "WORKSPACE_NOT_GIT");
    }

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            "/workspaces/no-such-workspace",
            None,
        ))
        .await
        .unwrap();
    let (status, missing) = json_of(response).await;
    assert_eq!(status, 404, "{missing}");
    assert_eq!(missing["code"], "WORKSPACE_NOT_FOUND");
}

#[tokio::test]
async fn task_with_lease_workspace_ref_runs_inside_the_worktree() {
    if !git_available() {
        return;
    }
    let (state, _temp) = test_state(true).await;
    let app = build_router(Arc::clone(&state));

    let (status, created) = create_lease(&app, &state, "git_worktree").await;
    assert_eq!(status, 201);
    let workspace_id = created["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();
    let lease_path = created["workspace"]["path"].as_str().unwrap().to_string();

    let body = serde_json::json!({
        "user_goal": "在隔离工作区里执行",
        "execution_strategy": "single_agent",
        "workspace_ref": lease_path,
    })
    .to_string();
    let response = app
        .clone()
        .oneshot(request(&state, "POST", "/agent-tasks", Some(&body)))
        .await
        .unwrap();
    let (status, task_body) = json_of(response).await;
    assert_eq!(status, 201, "{task_body}");
    let task_id = task_body["task"]["task_id"].as_str().unwrap().to_string();
    assert_eq!(task_body["task"]["workspace_ref"], lease_path);

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/agent-tasks/{task_id}/start"),
            Some("{}"),
        ))
        .await
        .unwrap();
    let (status, _) = json_of(response).await;
    assert_eq!(status, 200);

    for _ in 0..400 {
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "GET",
                &format!("/agent-tasks/{task_id}"),
                None,
            ))
            .await
            .unwrap();
        let (_, detail) = json_of(response).await;
        let state_text = detail["task"]["state"].as_str().unwrap_or_default();
        if state_text == "failed" {
            break;
        }
        assert_ne!(state_text, "succeeded", "模型不可用时不得伪装成功");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/agent-tasks/{task_id}/events?limit=100"),
            None,
        ))
        .await
        .unwrap();
    let (_, events) = json_of(response).await;
    let session_workspace = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_type"] == "task.session.created")
        .and_then(|event| event["payload"]["workspace"].as_str())
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        session_workspace, lease_path,
        "任务必须在租约工作区里建会话，而不是用户原工作区"
    );

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "DELETE",
            &format!("/workspaces/{workspace_id}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
}
