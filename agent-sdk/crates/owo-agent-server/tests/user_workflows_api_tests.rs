//! 用户自定义工作流 HTTP 集成测试（团队模板的用户/社区通道）。
//!
//! 走真实路由（bearer + tower oneshot），模板落 `data_root/workswarm/templates`。
//! 覆盖：创建/更新（越权 origin 拒绝）、校验失败 422 附全部原因、列表（含 valid 与
//! invalid 计数）、只校验不落盘、删除（内置 403 / 不存在 404）、以及"内置模板不可
//! 被用户覆盖"。

use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use serde_json::{json, Value};
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
}

fn fixture() -> Fixture {
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
    Fixture { state, temp }
}

async fn call(
    state: &Arc<owo_agent_server::AppState>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (u16, Value) {
    use axum::http::{header, Method, Request};
    let app = owo_agent_server::build_router(Arc::clone(state));
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let request = match body {
        Some(body) => builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    let resp = app.oneshot(request).await.unwrap();
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

/// 一个合法的"视频复刻"工作流（角色 DAG + 工具白名单 + 写范围 + 验证断言）。
fn valid_workflow(id: &str) -> String {
    json!({
        "template_id": id,
        "name": "视频复刻流水线",
        "mode": "team",
        "applicability": "视频复刻 分镜 配音 合成",
        "source_team_id": null,
        "created_at": "",
        "description": "从素材到成片的多角色流水线",
        "version": "1.0.0",
        "tags": ["video", "media"],
        "roles": [
            {
                "role": "script",
                "assignee": "agent",
                "depends_on": [],
                "verify": "contains:分镜",
                "tools": ["read_file", "search_files", "find_symbol"],
                "write_paths": ["script/"]
            },
            {
                "role": "storyboard",
                "assignee": "agent",
                "depends_on": ["script"],
                "handoff_contract": "每个镜头一行：编号 时长 画面",
                "tools": ["read_file", "write_file"],
                "write_paths": ["storyboard/"]
            },
            {
                "role": "render",
                "assignee": "agent",
                "depends_on": ["storyboard"],
                "verify": "non_empty",
                "tools": ["read_file", "write_file", "mcp__owo_plugin_translate__translate"],
                "write_paths": ["render/"]
            }
        ]
    })
    .to_string()
}

#[tokio::test]
async fn creates_lists_and_deletes_a_user_workflow() {
    let fixture = fixture();
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&valid_workflow("video-recreate")),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["template"]["origin"], json!("user"));
    assert!(
        body["template"]["created_at"].as_str().unwrap().len() > 10,
        "created_at 应被补齐：{body}"
    );

    let (status, listed) = call(&fixture.state, "GET", "/workflows/user", None).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["count"], json!(1));
    assert_eq!(listed["invalid"], json!(0));
    assert_eq!(
        listed["templates"][0]["template"]["template_id"],
        json!("video-recreate")
    );
    assert_eq!(listed["templates"][0]["origin"], json!("user"));
    assert_eq!(listed["templates"][0]["valid"], json!(true));
    // 能力面评审随列表返回（UI 建队前做知情确认）：该工作流含 run_command/MCP/人工节点。
    assert_eq!(listed["templates"][0]["blocked"], json!(false));
    let warnings = listed["templates"][0]["warnings"].as_u64().unwrap_or(0);
    assert!(warnings >= 1, "应至少有 warn 级发现：{listed}");
    let categories: Vec<&str> = listed["templates"][0]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|finding| finding["category"].as_str())
        .collect();
    // 该夹具声明了 MCP 工具（warn）与摘要（info）；没有执行类工具/人工节点。
    assert!(categories.contains(&"mcp"), "{categories:?}");
    assert!(categories.contains(&"summary"), "{categories:?}");
    // 健康摘要：角色/工具计数与风险等级（该夹具含 MCP → medium）。
    let health = &listed["templates"][0]["health"];
    assert_eq!(health["roles"], json!(3));
    assert_eq!(health["risk_level"], json!("medium"), "{health}");
    assert_eq!(health["max_depth"], json!(3), "{health}");
    assert_eq!(health["mcp_tools"].as_array().unwrap().len(), 1, "{health}");
    assert_eq!(health["human_roles"], json!(0), "{health}");

    // 更新（同 id 覆盖）应成功，且仍是用户模板。
    let mut updated: Value = serde_json::from_str(&valid_workflow("video-recreate")).unwrap();
    updated["version"] = json!("1.1.0");
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&updated.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["template"]["version"], json!("1.1.0"));

    // 治理动作必须有审计（社区/用户工作流都属于安全相关面）。
    let events: Vec<String> = fixture
        .state
        .agent
        .audit_log()
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| format!("{}:{}", entry.session_id, entry.event))
        .collect();
    assert!(
        events.contains(&"workflows:workflow_save".to_string()),
        "{events:?}"
    );

    let (status, deleted) = call(
        &fixture.state,
        "DELETE",
        "/workflows/user/video-recreate",
        None,
    )
    .await;
    assert_eq!(status, 200, "{deleted}");
    let events: Vec<String> = fixture
        .state
        .agent
        .audit_log()
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.event.clone())
        .collect();
    assert!(
        events.contains(&"workflow_delete".to_string()),
        "{events:?}"
    );
    assert_eq!(deleted["deleted"], json!("video-recreate"));

    let (_, listed) = call(&fixture.state, "GET", "/workflows/user", None).await;
    assert_eq!(listed["count"], json!(0));

    // 再删同一个 → 404（幂等语义明确）。
    let (status, _) = call(
        &fixture.state,
        "DELETE",
        "/workflows/user/video-recreate",
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn invalid_workflows_are_rejected_with_all_reasons_and_not_persisted() {
    let fixture = fixture();
    let mut broken: Value = serde_json::from_str(&valid_workflow("broken-flow")).unwrap();
    broken["roles"][0]["tools"] = json!(["totally_unknown_tool"]);
    broken["roles"][1]["depends_on"] = json!(["ghost"]);
    broken["roles"][2]["verify"] = json!("looks_good");
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&broken.to_string()),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    let reasons = body["errors"].as_array().unwrap();
    let joined = reasons
        .iter()
        .filter_map(|item| item.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("未知工具"), "{joined}");
    assert!(joined.contains("不存在的角色"), "{joined}");
    assert!(joined.contains("verify 非法"), "{joined}");

    // 422 不得落盘。
    let (_, listed) = call(&fixture.state, "GET", "/workflows/user", None).await;
    assert_eq!(listed["count"], json!(0));
}

#[tokio::test]
async fn validate_endpoint_is_dry_run() {
    let fixture = fixture();
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user/validate",
        Some(&valid_workflow("dry-run")),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert!(body["errors"].as_array().unwrap().is_empty());

    let mut broken: Value = serde_json::from_str(&valid_workflow("dry-run")).unwrap();
    broken["roles"][0]["write_paths"] = json!(["../outside"]);
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user/validate",
        Some(&broken.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(false));
    assert!(!body["errors"].as_array().unwrap().is_empty());

    // 只校验：列表仍为空。
    let (_, listed) = call(&fixture.state, "GET", "/workflows/user", None).await;
    assert_eq!(listed["count"], json!(0));
}

#[tokio::test]
async fn builtin_templates_cannot_be_overwritten_or_deleted_by_users() {
    let fixture = fixture();
    // 先安装一个内置模板（走目录安装路由），再用用户路由覆盖同 id。
    let (status, catalog) = call(&fixture.state, "GET", "/teams/templates/catalog", None).await;
    assert_eq!(status, 200, "{catalog}");
    let first = catalog["catalog"][0]["template"]["template_id"]
        .as_str()
        .expect("内置目录应有模板")
        .to_string();
    let (status, _) = call(
        &fixture.state,
        "POST",
        &format!("/teams/templates/catalog/{first}/install"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200);

    let mut forged: Value = serde_json::from_str(&valid_workflow(&first)).unwrap();
    forged["name"] = json!("伪造的内置模板");
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&forged.to_string()),
    )
    .await;
    assert_eq!(status, 409, "内置模板不得被用户覆盖：{body}");

    let (status, body) = call(
        &fixture.state,
        "DELETE",
        &format!("/workflows/user/{first}"),
        None,
    )
    .await;
    assert_eq!(status, 403, "内置模板不得删除：{body}");
}

#[tokio::test]
async fn list_reports_run_attention_only_for_failing_workflows() {
    let fixture = fixture();
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&valid_workflow("att-list")),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // 从未运行：列表返回 attention=[]、runs=0，且 needs_attention=0。
    let (status, listed) = call(&fixture.state, "GET", "/workflows/user", None).await;
    assert_eq!(status, 200, "{listed}");
    let item = listed["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["template"]["template_id"] == json!("att-list"))
        .expect("新建工作流应出现在列表");
    assert_eq!(item["attention"].as_array().unwrap().len(), 0, "{listed}");
    assert_eq!(item["runs"], json!(0), "{listed}");
    assert_eq!(listed["needs_attention"], json!(0), "{listed}");

    // 造一条失败运行（离线环境团队必然失败，与统计测试同口径）。
    let (status, created) = call(
        &fixture.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "列表告警演示",
                "mode": "team",
                "template_id": "att-list"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "{created}");
    let team_id = created["team_id"]
        .as_str()
        .or_else(|| created["team"]["team_id"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(!team_id.is_empty(), "{created}");
    let mut final_status = String::new();
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (_, detail) = call(&fixture.state, "GET", &format!("/teams/{team_id}"), None).await;
        final_status = detail["team"]["status"].as_str().unwrap_or("").to_string();
        if matches!(final_status.as_str(), "failed" | "succeeded" | "cancelled") {
            break;
        }
    }
    assert_eq!(
        final_status, "failed",
        "离线环境应快速失败；实际 {final_status}"
    );

    let (status, listed) = call(&fixture.state, "GET", "/workflows/user", None).await;
    assert_eq!(status, 200, "{listed}");
    let item = listed["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["template"]["template_id"] == json!("att-list"))
        .expect("工作流仍应在列表");
    let attention: Vec<&str> = item["attention"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(
        attention.iter().any(|reason| reason.contains("从未成功")),
        "{attention:?}"
    );
    assert_eq!(item["runs"], json!(1), "{listed}");
    assert_eq!(listed["needs_attention"], json!(1), "{listed}");
}

#[tokio::test]
async fn last_run_is_null_before_any_run_and_reuses_objective_after() {
    let fixture = fixture();
    let (status, _) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&valid_workflow("last-run-flow")),
    )
    .await;
    assert_eq!(status, 200);

    // 从未运行 → last_run 为 null（不编造目标）。
    let (status, empty) = call(
        &fixture.state,
        "GET",
        "/workflows/user/last-run-flow/last-run",
        None,
    )
    .await;
    assert_eq!(status, 200, "{empty}");
    assert_eq!(empty["template_id"], json!("last-run-flow"));
    assert!(empty["last_run"].is_null(), "{empty}");

    // 跑一次（离线必失败）→ 暴露 team_id/status/可复用 objective。
    let (status, created) = call(
        &fixture.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "复现上次失败：改哪里",
                "mode": "team",
                "template_id": "last-run-flow"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "{created}");
    let team_id = created["team_id"]
        .as_str()
        .or_else(|| created["team"]["team_id"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(!team_id.is_empty(), "{created}");
    let mut final_status = String::new();
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (_, detail) = call(&fixture.state, "GET", &format!("/teams/{team_id}"), None).await;
        final_status = detail["team"]["status"].as_str().unwrap_or("").to_string();
        if matches!(final_status.as_str(), "failed" | "succeeded" | "cancelled") {
            break;
        }
    }
    assert_eq!(final_status, "failed", "离线环境应快速失败");

    let (status, last) = call(
        &fixture.state,
        "GET",
        "/workflows/user/last-run-flow/last-run",
        None,
    )
    .await;
    assert_eq!(status, 200, "{last}");
    let run = last["last_run"].as_object().expect("应有最近一次运行");
    assert_eq!(run["team_id"], json!(team_id), "{last}");
    assert_eq!(run["status"], json!("failed"), "{last}");
    // 目标可从运行快照读回（rerun 复用它），不是占位符。
    assert_eq!(run["objective"], json!("复现上次失败：改哪里"), "{last}");
    assert!(
        !run["created_at"].as_str().unwrap_or("").is_empty(),
        "{last}"
    );

    // 不存在的工作流 id 也只是 last_run=null（路由存在、不 404 掩盖）。
    let (status, missing) = call(
        &fixture.state,
        "GET",
        "/workflows/user/no-such-flow/last-run",
        None,
    )
    .await;
    assert_eq!(status, 200, "{missing}");
    assert!(missing["last_run"].is_null(), "{missing}");
}

#[tokio::test]
async fn origin_is_normalized_and_rejects_unknown_values() {
    let fixture = fixture();
    let mut workflow: Value = serde_json::from_str(&valid_workflow("origin-check")).unwrap();
    workflow["origin"] = json!("community");
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&workflow.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["template"]["origin"], json!("community"));

    workflow["origin"] = json!("hacked");
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&workflow.to_string()),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(body["error"].as_str().unwrap().contains("origin"));
}
