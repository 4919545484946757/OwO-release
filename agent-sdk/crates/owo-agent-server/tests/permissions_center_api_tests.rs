//! §4.5 权限中心 HTTP 契约测试（`/permissions/overview`、`/permissions/spec`、撤销级联）。
//!
//! 这一层要守住的不是"能不能返回 200"，而是指南 §4.5 的三条语义红线：
//! 1. 维度范围由**服务端展开**（前端不得从档位名猜），所以 overview 必须同时给出
//!    `spec`（人写的）、`effective_spec`（投影补全的）与 `dimensions`（渲染口径）；
//! 2. 结构化配置**只收紧不放宽**，且完全访问必须"范围 + 时长 + 风险"三要素齐；
//! 3. 撤销要能级联（单条 / 按工具 / 按工作区），且"工作区长期"必须真的落盘。
use owo_agent_core::permissions::{Decision, Level, PermissionRequest, Policy};
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use serde_json::{json, Value};
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

fn http_request(
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
    if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

async fn send_json(
    state: &Arc<owo_agent_server::AppState>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let app = build_router(Arc::clone(state));
    let body = body.map(|value| value.to_string());
    let response = app
        .oneshot(http_request(state, method, path, body.as_deref()))
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let parsed = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({ "unparsable": String::from_utf8_lossy(&bytes).to_string() }));
    (status, parsed)
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

fn tight_spec() -> Value {
    json!({
        "filesystem": "workspace_read",
        "command": "allowlisted",
        "network": "deny",
        "persistence": "once",
        "scopes": []
    })
}

#[tokio::test]
async fn overview_expands_dimensions_server_side() {
    let (state, _temp) = test_state().await;
    let (status, body) = send_json(&state, "GET", "/permissions/overview", None).await;
    assert_eq!(status, 200, "总览必须可读：{body}");
    assert_eq!(body["profile"], json!("workspace"), "默认档位如实回显");
    // 没有显式配置时 spec 为 null，但 effective_spec 必须由服务端投影补齐。
    assert_eq!(body["spec"], Value::Null, "未提交过结构化配置");
    assert_eq!(
        body["effective_spec"]["filesystem"],
        json!("workspace_write")
    );
    assert_eq!(body["dimensions"][0]["source"], json!("profile"));
    let keys: Vec<&str> = body["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        vec!["filesystem", "command", "network", "persistence"],
        "四维顺序与 key 是前端渲染的契约，不能漂移"
    );
    for row in body["dimensions"].as_array().unwrap() {
        assert!(
            !row["summary"].as_str().unwrap().is_empty(),
            "每一维都必须有人可读的范围摘要：{row}"
        );
    }
    assert_eq!(body["pending_count"], json!(0), "新进程没有待审批动作");
    assert_eq!(
        body["grants_persisted"],
        json!(true),
        "长期授权必须落在 data_root，否则界面上的「工作区长期」是假的"
    );
    assert_eq!(
        body["scope_literals"]["persistence"],
        json!(["once", "task", "workspace"]),
        "审批动作词表由服务端下发，前后端共用一张表"
    );
    assert!(body["full_access"]["risk_notes"].as_array().unwrap().len() >= 4);
}

#[tokio::test]
async fn legacy_destructive_grant_is_visible_but_marked_inactive() {
    let (state, _temp) = test_state().await;
    let legacy: owo_agent_core::grant_store::Grant = serde_json::from_value(json!({
        "grant_id": "legacy-write-grant",
        "tool_id": "write_file",
        "workspace_id": state.workspace_id(),
        "path_scope": null,
        "host_scope": null,
        "argument_fingerprint": null,
        "created_at": "2026-01-01T00:00:00Z",
        "expires_at": null,
        "remaining_uses": 10,
        "scope": "workspace"
    }))
    .unwrap();
    state.grants.insert(legacy);
    let (status, body) = send_json(&state, "GET", "/permissions/grants", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["grants"][0]["effective"], json!(false));
    assert_eq!(state.grants.list().len(), 1, "旧授权保留给用户显式撤销");
}

#[tokio::test]
async fn approval_actions_persist_only_read_grants_and_keep_inject_one_shot() {
    let (state, temp) = test_state().await;
    // 契约前置：审批响应会按 session_id 取会话工作区来绑定 Grant（§5.4）；这里必须
    // 先落一个真实会话，否则 `load_session` 直接 404，这条契约测试只会空转（实测：
    // 未建会话时首个断言就是 404，后面的 Grant 语义一条都没被验证）。
    {
        let mut session = owo_agent_core::Session::new(&state.workspace, "mock", None);
        session.id = "session-p3".to_string();
        state.store.save(&session).unwrap();
    }
    for (scope, allow, expected_grant_scope) in [
        (Some("once"), true, None),
        (Some("task"), true, Some("task")),
        (Some("workspace"), true, Some("workspace")),
        (None, false, None),
    ] {
        let request = PermissionRequest::new(
            "read_file",
            json!({ "path": format!("{scope:?}.txt") }),
            Level::Read,
            "契约测试",
        );
        let request_id = request.request_id.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        state
            .pending_approvals
            .lock()
            .unwrap()
            .insert(request_id.clone(), (sender, request));
        state
            .pending_approval_sessions
            .lock()
            .unwrap()
            .insert(request_id.clone(), "session-p3".to_string());

        let body = if allow {
            json!({ "allow": true, "scope": scope })
        } else {
            json!({ "allow": false })
        };
        let (status, response) = send_json(
            &state,
            "POST",
            &format!("/session/session-p3/permission/{request_id}"),
            Some(body),
        )
        .await;
        assert_eq!(status, 200, "审批响应应成功：{response}");
        assert_eq!(
            response["allowed"],
            json!(allow),
            "允许/拒绝结果需与 Grant 状态分开回报"
        );
        assert_eq!(
            response["granted"],
            json!(expected_grant_scope.is_some()),
            "granted 只表示成功写入可复用授权"
        );
        assert_eq!(
            receiver.await.unwrap(),
            if allow {
                Decision::Allow
            } else {
                Decision::Deny
            }
        );
        let grants = state.grants.list();
        match expected_grant_scope {
            Some(expected) => assert!(
                grants
                    .iter()
                    .any(|grant| grant.scope.as_deref() == Some(expected)),
                "{expected} 选择必须生成对应 Grant"
            ),
            None => assert_eq!(
                grants.len(),
                if scope == Some("once") { 0 } else { 2 },
                "一次性允许或拒绝不得新增长期授权"
            ),
        }
    }
    assert!(temp.path().join("grants.json").exists());

    // Scope 是用户对风险的再次确认，但所有非只读请求均不得转成自动复用的 Grant。
    for (tool, level, args) in [
        (
            "write_file",
            Level::Write,
            json!({ "path": "blocked-write.txt" }),
        ),
        (
            "run_command",
            Level::Execute,
            json!({ "command": "echo blocked" }),
        ),
    ] {
        let request = PermissionRequest::new(tool, args, level, "非只读审批");
        let request_id = request.request_id.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        state
            .pending_approvals
            .lock()
            .unwrap()
            .insert(request_id.clone(), (sender, request));
        state
            .pending_approval_sessions
            .lock()
            .unwrap()
            .insert(request_id.clone(), "session-p3".to_string());
        let before = state.grants.list().len();
        let (status, response) = send_json(
            &state,
            "POST",
            &format!("/session/session-p3/permission/{request_id}"),
            Some(json!({ "allow": true, "scope": "workspace" })),
        )
        .await;
        assert_eq!(status, 200, "本次审批仍应成功：{response}");
        assert_eq!(receiver.await.unwrap(), Decision::Allow);
        assert_eq!(response["allowed"], json!(true));
        assert_eq!(response["granted"], json!(false));
        assert_eq!(
            state.grants.list().len(),
            before,
            "写入/执行批准不得保存为 Grant"
        );
    }

    let request = PermissionRequest::new(
        "desktop_key",
        json!({ "key": "enter" }),
        Level::Inject,
        "高风险注入动作",
    );
    let request_id = request.request_id.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    state
        .pending_approvals
        .lock()
        .unwrap()
        .insert(request_id.clone(), (sender, request));
    state
        .pending_approval_sessions
        .lock()
        .unwrap()
        .insert(request_id.clone(), "session-p3".to_string());
    let before = state.grants.list().len();
    let (status, response) = send_json(
        &state,
        "POST",
        &format!("/session/session-p3/permission/{request_id}"),
        Some(json!({ "allow": true, "scope": "workspace" })),
    )
    .await;
    assert_eq!(status, 200, "单次审批可放行但不得记为 Grant：{response}");
    assert_eq!(receiver.await.unwrap(), Decision::Allow);
    assert_eq!(response["allowed"], json!(true));
    assert_eq!(response["granted"], json!(false));
    assert_eq!(state.grants.list().len(), before, "Inject 不得生成持久授权");
}

#[tokio::test]
async fn spec_submit_persists_and_switches_source_to_spec() {
    let (state, _temp) = test_state().await;
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({ "spec": tight_spec() })),
    )
    .await;
    assert_eq!(status, 200, "收紧型配置应被接受：{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(
        body["expanded"].as_array().unwrap().len(),
        4,
        "四维展开必须齐全"
    );
    // settings.json 是重启后的唯一真相来源。
    let settings = owo_agent_core::Settings::load(&state.workspace);
    assert!(settings.permission_spec.is_some(), "必须落盘");
    assert_eq!(
        settings.permission_spec.as_ref().unwrap().network,
        owo_agent_core::permission_spec::RuleScope::Deny
    );
    let (status, overview) = send_json(&state, "GET", "/permissions/overview", None).await;
    assert_eq!(status, 200);
    assert_eq!(overview["dimensions"][1]["source"], json!("spec"));
    assert_eq!(overview["dimensions"][1]["effective"], json!("allowlisted"));
    assert_eq!(overview["spec"]["filesystem"], json!("workspace_read"));
    // 档位没被这次提交放宽（Workspace 本就是它所在的位置），收紧则由维度层承担。
    assert_eq!(overview["profile"], json!("workspace"));
}

#[tokio::test]
async fn full_access_requires_confirm_and_duration() {
    let (state, _temp) = test_state().await;
    let wide = json!({
        "filesystem": "workspace_write",
        "command": "unrestricted",
        "network": "unrestricted",
        "persistence": "workspace",
        "scopes": []
    });
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({ "spec": wide })),
    )
    .await;
    assert_eq!(status, 400, "完全访问不能免确认直接生效：{body}");
    assert_eq!(body["error"]["code"], json!("confirmation/required"));
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({ "spec": wide, "confirm": true })),
    )
    .await;
    assert_eq!(
        status, 400,
        "确认了但没给时长也不算三要素齐（时长上限 8h）：{body}"
    );
    assert_eq!(body["error"]["code"], json!("confirmation/required"));
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({ "spec": wide, "confirm": true, "duration_secs": 3600 })),
    )
    .await;
    assert_eq!(status, 200, "三要素齐才允许：{body}");
    assert_eq!(body["duration_secs"], json!(3600));
    assert_eq!(
        body["profile"],
        json!("workspace"),
        "档位不因完全访问被自动抬高（收紧层才是判定入口）"
    );
    // 超上限的时长同样拒绝，避免"永久完全访问"。
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({ "spec": wide, "confirm": true, "duration_secs": 999_999 })),
    )
    .await;
    assert_eq!(status, 400, "时长上限必须硬拦：{body}");
    assert_eq!(body["error"]["code"], json!("validation/failed"));
}

#[tokio::test]
async fn spec_validation_rejects_escapes_and_empty_custom() {
    let (state, _temp) = test_state().await;
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({
            "spec": {
                "filesystem": "custom",
                "command": "allowlisted",
                "network": "deny",
                "persistence": "once",
                "scopes": []
            }
        })),
    )
    .await;
    assert_eq!(status, 400, "custom 空范围等于没配置：{body}");
    assert_eq!(body["error"]["code"], json!("validation/failed"));
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({
            "spec": {
                "filesystem": "custom",
                "command": "allowlisted",
                "network": "deny",
                "persistence": "once",
                "scopes": ["path:../../secrets/**"]
            }
        })),
    )
    .await;
    assert_eq!(status, 400, "范围必须工作区相对：{body}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("工作区相对"));
    let (status, _) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({
            "spec": {
                "filesystem": "everything",
                "command": "allowlisted",
                "network": "deny",
                "persistence": "once",
                "scopes": []
            }
        })),
    )
    .await;
    assert!(
        (400..500).contains(&status),
        "未知识别字面量必须被拒（serde 422 或校验 400 都算），不能静默降级"
    );
}

#[tokio::test]
async fn read_only_mode_rejects_loosening_spec() {
    let (state, _temp) = test_state().await;
    state
        .agent
        .set_permission_profile(owo_agent_core::PermissionProfile::ReadOnly);
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({ "spec": tight_spec() })),
    )
    .await;
    assert_eq!(
        status, 400,
        "只读模式下提交可写配置必须显式失败，而不是存起来以后生效：{body}"
    );
    assert_eq!(body["error"]["code"], json!("conflict/read_only"));
    // 等价只读的收紧配置则允许（它不会放宽任何东西）。
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/spec",
        Some(json!({
            "spec": {
                "filesystem": "none",
                "command": "deny",
                "network": "deny",
                "persistence": "once",
                "scopes": []
            }
        })),
    )
    .await;
    assert_eq!(status, 200, "收紧到只读等价配置应接受：{body}");
    assert_eq!(body["profile"], json!("read_only"));
}

#[tokio::test]
async fn revoke_cascades_three_granularities() {
    let (state, _temp) = test_state().await;
    let workspace_id = state.workspace_id();
    let make = |tool: &str| {
        let request = PermissionRequest::new(tool, json!({ "path": "a.txt" }), Level::Read, "测试");
        state
            .grants
            .grant_from_scope(
                &request,
                &workspace_id,
                owo_agent_core::grant_store::GrantScope::Workspace,
            )
            .unwrap()
    };
    let one = make("read_file");
    let grant_id = one.grant_id.clone();
    state.grants.insert(one);
    state.grants.insert(make("read_file"));
    state.grants.insert(make("list_dir"));
    assert_eq!(state.grants.list().len(), 3);

    // 单条：既有条款不变（不存在仍 404）。
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/grants/revoke",
        Some(json!({ "grant_id": grant_id })),
    )
    .await;
    assert_eq!(status, 200, "单条撤销：{body}");
    assert_eq!(body["scope"], json!("grant"));
    assert_eq!(body["revoked"], json!(1));
    let (status, _) = send_json(
        &state,
        "POST",
        "/permissions/grants/revoke",
        Some(json!({ "grant_id": grant_id })),
    )
    .await;
    assert_eq!(status, 404, "重复撤销仍按不存在处理");

    // 按工具。
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/grants/revoke",
        Some(json!({ "tool_id": "read_file" })),
    )
    .await;
    assert_eq!(status, 200, "按工具撤销：{body}");
    assert_eq!(body["revoked"], json!(1));
    assert_eq!(body["remaining"], json!(1));

    // 空 body 不等于"撤销全部"。
    let (status, _) = send_json(
        &state,
        "POST",
        "/permissions/grants/revoke",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400, "空请求绝不能扩大撤销作用面");

    // 按工作区全部。
    let (status, body) = send_json(
        &state,
        "POST",
        "/permissions/grants/revoke",
        Some(json!({ "all": true })),
    )
    .await;
    assert_eq!(status, 200, "按工作区撤销：{body}");
    assert_eq!(body["scope"], json!("workspace"));
    assert_eq!(state.grants.list().len(), 0, "全部撤销后不留条目");
    let (_, overview) = send_json(&state, "GET", "/permissions/overview", None).await;
    assert_eq!(overview["grant_count"], json!(0));
}

#[tokio::test]
async fn long_lived_grants_survive_state_rebuild() {
    // 「工作区长期」是权限中心最容易被做假的承诺：只在内存里活一次进程，
    // 就等于骗用户说"以后不会再问"。这里跨过 AppState 重建验一次。
    let (state, temp) = test_state().await;
    let workspace_id = state.workspace_id();
    let request =
        PermissionRequest::new("read_file", json!({ "path": "a.txt" }), Level::Read, "测试");
    let grant = state
        .grants
        .grant_from_scope(
            &request,
            &workspace_id,
            owo_agent_core::grant_store::GrantScope::Workspace,
        )
        .unwrap();
    let grant_id = grant.grant_id.clone();
    state.grants.insert(grant);
    assert!(
        temp.path().join("grants.json").exists(),
        "长期授权必须写到 data_root"
    );
    let store = SqliteSessionStore::open(&state.workspace.join("index2.db")).unwrap();
    let revived = Arc::new(owo_agent_server::AppState::new(
        Agent::new(
            Arc::new(IdleProvider),
            ToolRegistry::new(),
            Policy::new(&state.workspace),
            Default::default(),
        ),
        store,
        state.workspace.join("traces2"),
        temp.path().to_path_buf(),
        state.workspace.clone(),
    ));
    let grants = revived.grants.list();
    assert_eq!(grants.len(), 1, "重建后仍认得这条长期授权");
    assert_eq!(grants[0].grant_id, grant_id, "是同一张授权，不是新发的");
    let (_, overview) = send_json(&revived, "GET", "/permissions/overview", None).await;
    assert_eq!(overview["grants"][0]["long_lived"], json!(true));
    assert_eq!(overview["grants"][0]["scope"], json!("workspace"));
}
