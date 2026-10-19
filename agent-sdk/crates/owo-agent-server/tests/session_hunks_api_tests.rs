//! 会话逐 hunk 审阅 HTTP 集成测试（G13 服务端面）。
//!
//! 全部走真实路由（bearer + tower oneshot），会话与写前快照直接注入 store
//! （模拟"Agent 已写过文件"），不依赖模型凭据。覆盖：
//! - `GET /session/{id}/hunks` 的 hunk 视图与不可审阅原因；
//! - `POST /session/{id}/hunks/apply` 的按序号接受/拒绝、写盘结果与剩余 hunk；
//! - 安全边界：路径越界 400、用户改过文件 409、序号越界 422、accept+reject 422、
//!   无写前快照 422、未知文件 404、缺选择 422。

use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::{Agent, SessionStore};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

struct IdleProvider;

#[async_trait::async_trait]
impl ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("IdleProvider 不应被调用".to_string())
    }
}

struct Fixture {
    state: Arc<owo_agent_server::AppState>,
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    workspace: PathBuf,
    session_id: String,
}

/// 注入"Agent 已写过文件"的会话：写前快照 = before，磁盘 = after。
fn fixture(before: &str, after: &str) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let file = workspace.join("fixture.txt");
    std::fs::write(&file, after).unwrap();

    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let mut session = store.create(&workspace, "mock", None).unwrap();
    let snapshot = owo_agent_core::session::SnapshotEntry {
        original_b64: Some(base64_encode(before)),
        expected_after_sha256: Some(owo_agent_core::CasStore::hash_of(after.as_bytes())),
        turn: 0,
    };
    session
        .snapshots
        .insert(file.to_string_lossy().to_string(), snapshot);
    store.save(&session).unwrap();

    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace.clone(),
    ));
    Fixture {
        state,
        temp,
        workspace,
        session_id: session.id,
    }
}

fn base64_encode(value: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
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

async fn call(
    state: &Arc<owo_agent_server::AppState>,
    fixture: &Fixture,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (u16, Value) {
    let app = owo_agent_server::build_router(Arc::clone(state));
    let path = path.replace("{id}", &fixture.session_id);
    let resp = app
        .oneshot(request(state, method, &path, body))
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!(null))
    };
    (status, value)
}

const BEFORE: &str = "1\n2\n3\n4\n5\n6\n7\n8\n";
const AFTER: &str = "1\n2\nX\n4\n5\n6\nY\n8\n";

#[tokio::test]
async fn hunks_view_lists_reviewable_files_and_reasons() {
    let fixture = fixture(BEFORE, AFTER);
    let (status, body) = call(&fixture.state, &fixture, "GET", "/session/{id}/hunks", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["reviewable"], json!(1));
    let file = &body["files"][0];
    assert_eq!(file["path"], json!("fixture.txt"));
    assert_eq!(file["kind"], json!("modified"));
    assert_eq!(file["hunks"].as_array().unwrap().len(), 2);
    assert_eq!(file["hunks"][0]["index"], json!(0));
    assert_eq!(file["hunks"][0]["old_start"], json!(2));
}

#[tokio::test]
async fn apply_accepts_single_hunk_and_reports_remaining() {
    let fixture = fixture(BEFORE, AFTER);
    // 只接受第 2 个 hunk（index=1）→ 第 1 处回到写前内容。
    let (status, body) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"fixture.txt","accept":[1]}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["accepted"], json!([1]));
    assert_eq!(body["rejected"], json!([0]));
    assert_eq!(body["remaining"], json!(1));
    assert_eq!(body["changed"], json!(true));
    let on_disk = std::fs::read_to_string(fixture.workspace.join("fixture.txt")).unwrap();
    assert_eq!(
        on_disk, "1\n2\n3\n4\n5\n6\nY\n8\n",
        "被拒绝的 hunk 必须还原为写前内容"
    );
}

#[tokio::test]
async fn apply_reject_all_restores_snapshot_content() {
    let fixture = fixture(BEFORE, AFTER);
    let (status, body) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"fixture.txt","reject":[0,1]}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["accepted"], json!([]));
    assert_eq!(body["remaining"], json!(0));
    let on_disk = std::fs::read_to_string(fixture.workspace.join("fixture.txt")).unwrap();
    assert_eq!(on_disk, BEFORE);
    // 全部还原后会话 diff 应为空。
    let (_, diff) = call(&fixture.state, &fixture, "GET", "/session/{id}/diff", None).await;
    assert_eq!(diff.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn apply_is_idempotent_when_selection_matches_current_content() {
    let fixture = fixture(BEFORE, AFTER);
    let (status, body) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"fixture.txt","accept":[0,1]}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["changed"],
        json!(false),
        "全接受等于当前内容，不应写盘"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.join("fixture.txt")).unwrap(),
        AFTER
    );
}

#[tokio::test]
async fn apply_refuses_out_of_range_unknown_and_empty_selections() {
    let fixture = fixture(BEFORE, AFTER);
    let (status, body) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"fixture.txt","accept":[9]}"#),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    let (status, _) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"fixture.txt"}"#),
    )
    .await;
    assert_eq!(status, 422, "缺少 accept/reject 必须拒绝");

    let (status, _) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"fixture.txt","accept":[0],"reject":[1]}"#),
    )
    .await;
    assert_eq!(status, 422, "accept 与 reject 不能同时给出");

    let (status, _) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"no-such.txt","accept":[0]}"#),
    )
    .await;
    assert_eq!(status, 404, "会话 diff 里没有的文件必须 404");
}

#[tokio::test]
async fn apply_refuses_to_clobber_user_edits_and_escaped_paths() {
    let fixture = fixture(BEFORE, AFTER);
    // 用户改过文件 → 409（绝不覆盖）。
    std::fs::write(fixture.workspace.join("fixture.txt"), "user edit\n").unwrap();
    let (status, body) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"fixture.txt","accept":[1]}"#),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.join("fixture.txt")).unwrap(),
        "user edit\n"
    );

    // 路径越界 → 400。
    let (status, _) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"../escape.txt","accept":[0]}"#),
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn files_without_write_snapshot_are_not_hunk_reviewable() {
    // 无写前快照（original_b64=None）→ 不可逐 hunk 审阅（422），并在视图中给出原因。
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let file = workspace.join("new.txt");
    std::fs::write(&file, "hello\n").unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let mut session = store.create(&workspace, "mock", None).unwrap();
    let snapshot = owo_agent_core::session::SnapshotEntry {
        original_b64: None,
        expected_after_sha256: Some(owo_agent_core::CasStore::hash_of(b"hello\n")),
        turn: 0,
    };
    session
        .snapshots
        .insert(file.to_string_lossy().to_string(), snapshot);
    store.save(&session).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace.clone(),
    ));
    let fixture = Fixture {
        state,
        temp,
        workspace,
        session_id: session.id,
    };

    let (status, body) = call(&fixture.state, &fixture, "GET", "/session/{id}/hunks", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["reviewable"], json!(0));
    assert!(body["files"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("没有写前快照"));

    let (status, _) = call(
        &fixture.state,
        &fixture,
        "POST",
        "/session/{id}/hunks/apply",
        Some(r#"{"path":"new.txt","accept":[0]}"#),
    )
    .await;
    assert_eq!(status, 422, "无写前快照时不得逐 hunk 应用");
}
