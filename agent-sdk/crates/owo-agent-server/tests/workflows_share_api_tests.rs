//! 工作流社区共享（导出/评审/导入）HTTP 集成测试。
//!
//! 覆盖：导出用户工作流 → 评测（findings 分类）→ 导入到"另一台机器"（另一个 data root）
//! 变成 community 来源；以及安全门禁：篡改包（sha256 不符）、非法包（kind/schema）、
//! 校验不过的包（DAG 成环）导入被拒且不落盘、内置模板不可导出、同 id 冲突 409。

use owo_agent_core::gateway::{ChatMessage, ModelOutput, ModelProvider};
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_core::ToolSpec;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

struct IdleProvider;

#[async_trait::async_trait]
impl ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
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
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&data_root).unwrap();
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
        data_root,
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
    let bytes = axum::body::to_bytes(resp.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!(null))
    };
    (status, value)
}

/// 一个包含"执行/联网 + 人工节点"的用户工作流（导出后应有 warn/info 发现）。
fn workflow_json(id: &str) -> String {
    json!({
        "template_id": id,
        "name": "视频复刻流水线",
        "mode": "team",
        "applicability": "视频复刻 分镜 配音 合成",
        "description": "素材 → 分镜 → 合成 → 人工验收",
        "version": "1.2.0",
        "tags": ["video"],
        "roles": [
            {
                "role": "script",
                "assignee": "agent",
                "depends_on": [],
                "verify": "contains:分镜",
                "tools": ["read_file", "write_file", "run_command"],
                "write_paths": ["script/"]
            },
            {
                "role": "render",
                "assignee": "agent",
                "depends_on": ["script"],
                "tools": ["read_file", "web_fetch"],
                "write_paths": ["render/"]
            },
            {
                "role": "acceptance",
                "assignee": "human",
                "depends_on": ["render"],
                "handoff_contract": "验收结论"
            }
        ]
    })
    .to_string()
}

#[tokio::test]
async fn export_review_and_import_round_trip_between_machines() {
    // A 机器：创建用户工作流并导出。
    let machine_a = fixture();
    let (status, created) = call(
        &machine_a.state,
        "POST",
        "/workflows/user",
        Some(&workflow_json("video-recreate")),
    )
    .await;
    assert_eq!(status, 200, "{created}");

    let (status, exported) = call(
        &machine_a.state,
        "POST",
        "/workflows/user/video-recreate/export",
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200, "{exported}");
    assert_eq!(exported["manifest"]["kind"], json!("owo-workflow"));
    assert_eq!(exported["manifest"]["role_count"], json!(3));
    assert_eq!(exported["manifest"]["version"], json!("1.2.0"));
    assert!(exported["sha256"].as_str().unwrap().len() == 64);
    // 导出预审：执行/联网工具与人工节点要可见。
    let findings = exported["review"]["findings"].as_array().unwrap();
    assert!(findings
        .iter()
        .any(|item| item["category"] == json!("capability")));
    assert!(findings
        .iter()
        .any(|item| item["category"] == json!("human_in_loop")));
    assert_eq!(exported["review"]["blocked"], json!(false));
    let package_b64 = exported["package_b64"].as_str().unwrap().to_string();

    // B 机器：先评审（不落盘）→ 再导入（community 来源）。
    let machine_b = fixture();
    let (status, reviewed) = call(
        &machine_b.state,
        "POST",
        "/workflows/import/review",
        Some(&json!({ "package_b64": package_b64 }).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{reviewed}");
    assert_eq!(reviewed["review"]["blocked"], json!(false));
    let (_, before) = call(&machine_b.state, "GET", "/workflows/user", None).await;
    assert_eq!(before["count"], json!(0), "评审不得落盘");

    let (status, imported) = call(
        &machine_b.state,
        "POST",
        "/workflows/import",
        Some(&json!({ "package_b64": package_b64 }).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{imported}");
    assert_eq!(imported["template"]["origin"], json!("community"));
    assert_eq!(imported["template"]["version"], json!("1.2.0"));
    // 导入成功写审计：只记 id/sha256/计数，不落配置正文。
    let import_entries: Vec<(Option<bool>, String)> = machine_b
        .state
        .agent
        .audit_log()
        .lock()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| entry.event == "workflow_import")
        .map(|entry| (entry.approved, entry.detail.clone()))
        .collect();
    assert_eq!(import_entries.len(), 1, "应有一条导入审计");
    assert_eq!(import_entries[0].0, Some(true));
    assert!(
        import_entries[0].1.contains("sha256="),
        "{import_entries:?}"
    );
    let (_, listed) = call(&machine_b.state, "GET", "/workflows/user", None).await;
    assert_eq!(listed["count"], json!(1));
    assert_eq!(listed["templates"][0]["origin"], json!("community"));
    assert_eq!(listed["templates"][0]["valid"], json!(true));
    // 导入溯源：sha256 + 导入时间 + 当时的发现数。
    let provenance = &listed["templates"][0]["provenance"];
    assert_eq!(
        provenance["sha256"].as_str().unwrap(),
        exported["sha256"].as_str().unwrap(),
        "{provenance}"
    );
    assert!(provenance["imported_at"].as_str().unwrap().len() > 10);
    assert!(
        provenance["findings"].as_u64().unwrap() >= 1,
        "{provenance}"
    );

    // 删除后溯源清空（再导入再记录，避免残留旧 sha256）。
    let (status, _) = call(
        &machine_b.state,
        "DELETE",
        "/workflows/user/video-recreate",
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        owo_agent_core::community_provenance::load_community_imports(&machine_b.state.data_root)
            .is_empty(),
        "删除模板应同时清掉导入溯源"
    );
    let (_, sources) = call(&machine_b.state, "GET", "/workflows/provenance", None).await;
    assert_eq!(
        sources["count"],
        json!(0),
        "删除后来源清单应为空：{sources}"
    );
}

#[tokio::test]
async fn import_review_reports_diff_against_local_workflow() {
    // 本地已有 v1；导入 v2 包时评审响应应带上"与本地同名工作流的差异"。
    let machine = fixture();
    let mut v1: serde_json::Value = serde_json::from_str(&workflow_json("video-recreate")).unwrap();
    v1["version"] = json!("1.0.0");
    v1["roles"][0]["tools"] = json!(["read_file", "write_file"]);
    let (status, _) = call(
        &machine.state,
        "POST",
        "/workflows/user",
        Some(&v1.to_string()),
    )
    .await;
    assert_eq!(status, 200);

    // v2：版本提升、impl 角色新增 run_command、写范围放宽（用另一个 data root 的导出包模拟社区更新）。
    let other = fixture();
    let mut v2 = v1.clone();
    v2["version"] = json!("1.1.0");
    v2["roles"][0]["tools"] = json!(["read_file", "write_file", "run_command"]);
    v2["roles"][0]["write_paths"] = json!([]);
    let (status, _) = call(
        &other.state,
        "POST",
        "/workflows/user",
        Some(&v2.to_string()),
    )
    .await;
    assert_eq!(status, 200);
    let (status, exported) = call(
        &other.state,
        "POST",
        "/workflows/user/video-recreate/export",
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200, "{exported}");
    let package = json!({ "package_b64": exported["package_b64"] }).to_string();

    // 1) 评审（不落盘）带差异与风险
    let (status, reviewed) = call(
        &machine.state,
        "POST",
        "/workflows/import/review",
        Some(&package),
    )
    .await;
    assert_eq!(status, 200, "{reviewed}");
    let diff = &reviewed["diff_vs_local"];
    assert_eq!(diff["local_origin"], json!("user"));
    assert_eq!(diff["local_version"], json!("1.0.0"));
    assert_eq!(diff["incoming_version"], json!("1.1.0"));
    let summary = diff["summary"].as_array().unwrap();
    let joined = summary
        .iter()
        .filter_map(|line| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("元数据 version：1.0.0 → 1.1.0"), "{joined}");
    assert!(joined.contains("[风险]"), "{joined}");
    assert!(joined.contains("run_command"), "{joined}");
    assert!(joined.contains("工作区任意写"), "{joined}");

    // 2) 导入响应同样带差异；且本地被覆盖为 community + 新版本
    let (status, imported) =
        call(&machine.state, "POST", "/workflows/import", Some(&package)).await;
    assert_eq!(status, 200, "{imported}");
    assert_eq!(imported["template"]["origin"], json!("community"));
    assert_eq!(imported["template"]["version"], json!("1.1.0"));
    assert_eq!(
        imported["diff_vs_local"]["incoming_version"],
        json!("1.1.0")
    );

    // 3) 本地没有同名模板时 diff_vs_local 为 null
    let fresh = fixture();
    let (status, reviewed) = call(
        &fresh.state,
        "POST",
        "/workflows/import/review",
        Some(&package),
    )
    .await;
    assert_eq!(status, 200, "{reviewed}");
    assert!(reviewed["diff_vs_local"].is_null(), "{reviewed}");
}

#[tokio::test]
async fn tampered_invalid_and_blocked_packages_are_rejected_without_persisting() {
    let machine = fixture();
    let (_, created) = call(
        &machine.state,
        "POST",
        "/workflows/user",
        Some(&workflow_json("share-source")),
    )
    .await;
    assert_eq!(created["ok"], json!(true));
    let (_, exported) = call(
        &machine.state,
        "POST",
        "/workflows/user/share-source/export",
        Some("{}"),
    )
    .await;
    let package_b64 = exported["package_b64"].as_str().unwrap().to_string();

    // 1) 篡改包（改模板但不改 sha256）→ 422。
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&package_b64)
        .unwrap();
    let mut package: Value = serde_json::from_slice(&raw).unwrap();
    package["template"]["name"] = json!("被篡改的名字");
    let tampered = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_string(&package).unwrap().as_bytes());
    let (status, body) = call(
        &machine.state,
        "POST",
        "/workflows/import",
        Some(&json!({ "package_b64": tampered }).to_string()),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("校验失败"),
        "{body}"
    );

    // 2) kind 不符 → 422。
    package["template"]["name"] = json!("视频复刻流水线");
    package["kind"] = json!("owo-skill");
    let wrong_kind = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_string(&package).unwrap().as_bytes());
    let (status, body) = call(
        &machine.state,
        "POST",
        "/workflows/import",
        Some(&json!({ "package_b64": wrong_kind }).to_string()),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(body["error"].as_str().unwrap().contains("kind"), "{body}");

    // 3) 校验不过（DAG 成环 + 未知工具）→ blocked 且不落盘。
    //
    // 注意：sha256 覆盖的是**规范化 JSON**（服务端按 TeamTemplate 序列化），所以手工组包
    // 必须走同一序列化路径（先反序列化成 TeamTemplate 再 to_string），否则会先在
    // 校验阶段被 sha256 拦下（这也是真实产品行为：建议直接用 export 生成包）。
    let mut broken: Value = serde_json::from_str(&workflow_json("broken-package")).unwrap();
    broken["roles"][0]["depends_on"] = json!(["render"]);
    broken["roles"][1]["depends_on"] = json!(["script"]);
    broken["roles"][0]["tools"] = json!(["not_a_tool"]);
    let broken_template: owo_agent_protocol::TeamTemplate =
        serde_json::from_value(broken.clone()).unwrap();
    let canonical = serde_json::to_string(&broken_template).unwrap();
    let hash = owo_agent_core::CasStore::hash_of(canonical.as_bytes());
    let broken_value: Value = serde_json::from_str(&canonical).unwrap();
    let mut blocked_package: Value = serde_json::from_slice(&raw).unwrap();
    blocked_package["kind"] = json!("owo-workflow");
    blocked_package["template"] = broken_value;
    blocked_package["sha256"] = json!(hash);
    let blocked_b64 = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_string(&blocked_package).unwrap().as_bytes());
    let (status, body) = call(
        &machine.state,
        "POST",
        "/workflows/import",
        Some(&json!({ "package_b64": blocked_b64 }).to_string()),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["blocked"], json!(true));
    assert!(!body["findings"].as_array().unwrap().is_empty());
    // 被门禁拒绝也要审计（approved=false，便于安全复盘）。
    let blocked_entries: Vec<(Option<bool>, String)> = machine
        .state
        .agent
        .audit_log()
        .lock()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| entry.event == "workflow_import_blocked")
        .map(|entry| (entry.approved, entry.detail.clone()))
        .collect();
    assert_eq!(blocked_entries.len(), 1, "应有一条被拒审计");
    assert_eq!(blocked_entries[0].0, Some(false));
    assert!(
        blocked_entries[0].1.contains("门禁拒绝"),
        "{blocked_entries:?}"
    );
    let (_, listed) = call(&machine.state, "GET", "/workflows/user", None).await;
    // 只有最开始的 share-source 一个；坏包没有落盘。
    assert_eq!(listed["count"], json!(1), "{listed}");
}

#[tokio::test]
async fn community_workflow_with_risk_requires_explicit_acknowledgement_before_team_create() {
    // A 机器导出含风险面的工作流（run_command + 人工节点）→ B 机器导入为 community，
    // 然后尝试建队：未确认 → 409（宿主侧兜底，不只依赖客户端提示）；确认后 → 202。
    let machine_a = fixture();
    let (status, _) = call(
        &machine_a.state,
        "POST",
        "/workflows/user",
        Some(&workflow_json("community-risk")),
    )
    .await;
    assert_eq!(status, 200);
    let (status, exported) = call(
        &machine_a.state,
        "POST",
        "/workflows/user/community-risk/export",
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200, "{exported}");
    let package = json!({ "package_b64": exported["package_b64"] }).to_string();

    let machine_b = fixture();
    let (status, imported) = call(
        &machine_b.state,
        "POST",
        "/workflows/import",
        Some(&package),
    )
    .await;
    assert_eq!(status, 200, "{imported}");
    assert_eq!(imported["template"]["origin"], json!("community"));

    // 未确认 → 409，且返回风险面与逐条 findings。
    let (status, blocked) = call(
        &machine_b.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "跑一次社区工作流",
                "mode": "team",
                "template_id": "community-risk"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 409, "{blocked}");
    assert_eq!(blocked["risk_level"], json!("high"));
    assert!(blocked["warnings"].as_u64().unwrap_or(0) >= 1);
    assert!(!blocked["findings"].as_array().unwrap().is_empty());
    assert!(
        blocked["error"]
            .as_str()
            .unwrap()
            .contains("acknowledge_risk"),
        "{blocked}"
    );

    // 确认后 → 202（建队成功；后台运行在测试环境无模型，会在后台失败，不影响本断言）。
    let (status, created) = call(
        &machine_b.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "跑一次社区工作流",
                "mode": "team",
                "template_id": "community-risk",
                "acknowledge_risk": true
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

    // 用户自己保存的同形工作流不受门禁影响（只针对社区来源）。
    let own = fixture();
    let (status, _) = call(
        &own.state,
        "POST",
        "/workflows/user",
        Some(&workflow_json("own-risk")),
    )
    .await;
    assert_eq!(status, 200);
    let (status, created) = call(
        &own.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "跑自己的高风险工作流",
                "mode": "team",
                "template_id": "own-risk"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "用户自己的工作流不应被社区门禁拦截：{created}");
}

#[tokio::test]
async fn workflow_stats_aggregate_runs_by_template() {
    // 用同一条用户工作流建两次队（离线环境会快速失败）→ 统计应聚合成 runs=2。
    let machine = fixture();
    let (status, _) = call(
        &machine.state,
        "POST",
        "/workflows/user",
        Some(&workflow_json("stats-flow")),
    )
    .await;
    assert_eq!(status, 200);

    // 先看空统计（还没跑过）。
    let (status, empty) = call(&machine.state, "GET", "/workflows/stats", None).await;
    assert_eq!(status, 200, "{empty}");
    assert_eq!(empty["count"], json!(0));
    assert_eq!(empty["summary"]["total_runs"], json!(0));

    // 建两次队后立即取消：终态确定为 cancelled（不依赖后台运行的时序）。
    for _ in 0..2 {
        let (status, created) = call(
            &machine.state,
            "POST",
            "/teams",
            Some(
                &json!({
                    "objective": "统计演示",
                    "mode": "team",
                    "template_id": "stats-flow"
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
        let (status, _) = call(
            &machine.state,
            "POST",
            &format!("/teams/{team_id}/steer"),
            Some(r#"{"command":"cancel"}"#),
        )
        .await;
        assert_eq!(status, 200, "取消失败：{team_id}");
    }
    // 动态组队（无 template_id）不参与工作流统计。
    let (status, _) = call(
        &machine.state,
        "POST",
        "/teams",
        Some(&json!({ "objective": "动态组队", "mode": "team" }).to_string()),
    )
    .await;
    assert_eq!(status, 202);

    let (status, stats) = call(&machine.state, "GET", "/workflows/stats", None).await;
    assert_eq!(status, 200, "{stats}");
    assert_eq!(stats["count"], json!(1), "{stats}");
    let item = &stats["templates"][0];
    assert_eq!(item["template_id"], json!("stats-flow"));
    assert_eq!(item["name"], json!("视频复刻流水线"));
    assert_eq!(item["origin"], json!("user"));
    assert_eq!(item["runs"], json!(2), "{stats}");
    assert_eq!(item["cancelled"], json!(2), "{stats}");
    // 只有"失败"运行才带 last_failure；全取消 → null（不自造原因）。
    assert!(item["last_failure"].is_null(), "{stats}");
    // 只有取消（没有结论）不算"需要修复"，避免误报。
    assert_eq!(item["attention"].as_array().unwrap().len(), 0, "{stats}");
    assert_eq!(item["last_run_failed"], json!(false));
    // 只有终态参与成功率；两次都取消 → 0%。
    assert_eq!(item["success_rate"], json!(0.0), "{stats}");
    assert!(item["risk_level"].is_string(), "{stats}");
    assert!(!item["last_run_at"].as_str().unwrap().is_empty());
    assert_eq!(stats["summary"]["total_runs"], json!(2));
    assert_eq!(stats["summary"]["succeeded"], json!(0));
    // 统计口径：created+running+awaiting_human+终态 == runs（不漏计）。
    let accounted = item["created"].as_u64().unwrap_or(0)
        + item["running"].as_u64().unwrap_or(0)
        + item["awaiting_human"].as_u64().unwrap_or(0)
        + item["succeeded"].as_u64().unwrap_or(0)
        + item["failed"].as_u64().unwrap_or(0)
        + item["cancelled"].as_u64().unwrap_or(0);
    assert_eq!(accounted, item["runs"].as_u64().unwrap_or(0), "{stats}");
}

#[tokio::test]
async fn workflow_stats_expose_last_failure_from_audit_for_failed_runs() {
    // 离线环境下团队必然失败（无模型）：有界轮询到终态后，统计应给出失败定位。
    let machine = fixture();
    let (status, _) = call(
        &machine.state,
        "POST",
        "/workflows/user",
        Some(&workflow_json("failing-flow")),
    )
    .await;
    assert_eq!(status, 200);
    let (status, created) = call(
        &machine.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "必然失败",
                "mode": "team",
                "template_id": "failing-flow"
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
        let (_, detail) = call(&machine.state, "GET", &format!("/teams/{team_id}"), None).await;
        final_status = detail["team"]["status"].as_str().unwrap_or("").to_string();
        if matches!(final_status.as_str(), "failed" | "succeeded" | "cancelled") {
            break;
        }
    }
    assert_eq!(
        final_status, "failed",
        "离线环境应快速失败；实际 {final_status}"
    );

    let (status, stats) = call(&machine.state, "GET", "/workflows/stats", None).await;
    assert_eq!(status, 200, "{stats}");
    let item = &stats["templates"][0];
    assert_eq!(item["failed"], json!(1), "{stats}");
    // 从未成功 + 最近一次失败 → 需要修复提示（CLI/面板据此高亮）。
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
    assert!(
        attention
            .iter()
            .any(|reason| reason.contains("最近一次运行失败")),
        "{attention:?}"
    );
    assert_eq!(item["last_run_failed"], json!(true));
    let failure = &item["last_failure"];
    assert!(!failure.is_null(), "失败运行应给出定位：{stats}");
    assert_eq!(failure["team_id"], json!(team_id));
    assert!(
        failure["event"].as_str().unwrap().contains("fail"),
        "{stats}"
    );
    assert!(!failure["detail"].as_str().unwrap().is_empty(), "{stats}");
}

#[tokio::test]
async fn workflow_stats_support_sort_order_and_limit() {
    let machine = fixture();
    // 两条工作流：alpha 跑 3 次（全取消）、beta 跑 1 次（取消）。
    for (template_id, runs) in [("stats-alpha", 3usize), ("stats-beta", 1usize)] {
        let (status, _) = call(
            &machine.state,
            "POST",
            "/workflows/user",
            Some(&workflow_json(template_id)),
        )
        .await;
        assert_eq!(status, 200);
        for _ in 0..runs {
            let (status, created) = call(
                &machine.state,
                "POST",
                "/teams",
                Some(
                    &json!({
                        "objective": "排序演示",
                        "mode": "team",
                        "template_id": template_id
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
            call(
                &machine.state,
                "POST",
                &format!("/teams/{team_id}/steer"),
                Some(r#"{"command":"cancel"}"#),
            )
            .await;
        }
    }

    // 缺省：按 runs 倒序 → alpha 在前。
    let (status, default_stats) = call(&machine.state, "GET", "/workflows/stats", None).await;
    assert_eq!(status, 200, "{default_stats}");
    assert_eq!(default_stats["sort"], json!("runs"));
    assert_eq!(default_stats["total"], json!(2));
    assert_eq!(
        default_stats["templates"][0]["template_id"],
        json!("stats-alpha"),
        "{default_stats}"
    );

    // runs 升序 → beta 在前。
    let (_, ascending) = call(
        &machine.state,
        "GET",
        "/workflows/stats?sort=runs&order=asc",
        None,
    )
    .await;
    assert_eq!(ascending["order"], json!("asc"));
    assert_eq!(
        ascending["templates"][0]["template_id"],
        json!("stats-beta")
    );

    // limit=1 → 截断标记 + total 仍为 2。
    let (_, limited) = call(&machine.state, "GET", "/workflows/stats?limit=1", None).await;
    assert_eq!(limited["count"], json!(1));
    assert_eq!(limited["total"], json!(2));
    assert_eq!(limited["truncated"], json!(true));
    assert_eq!(limited["templates"][0]["template_id"], json!("stats-alpha"));
    // summary 覆盖全部工作流（分页只影响 templates）：alpha 3 次 + beta 1 次 = 4。
    assert_eq!(limited["summary"]["total_runs"], json!(4), "{limited}");

    // 按成功率排序（都是 0%）→ 同值按 id 稳定排序。
    let (_, by_rate) = call(
        &machine.state,
        "GET",
        "/workflows/stats?sort=success_rate&order=asc",
        None,
    )
    .await;
    assert_eq!(by_rate["sort"], json!("success_rate"));
    assert_eq!(by_rate["templates"][0]["template_id"], json!("stats-alpha"));
}

#[tokio::test]
async fn builtin_templates_cannot_be_exported_and_id_conflicts_are_reported() {
    let machine = fixture();
    // 安装内置模板后尝试导出 → 403。
    let (_, catalog) = call(&machine.state, "GET", "/teams/templates/catalog", None).await;
    let builtin_id = catalog["catalog"][0]["template"]["template_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = call(
        &machine.state,
        "POST",
        &format!("/teams/templates/catalog/{builtin_id}/install"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200);
    let (status, body) = call(
        &machine.state,
        "POST",
        &format!("/workflows/user/{builtin_id}/export"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 403, "{body}");

    // 未知模板导出 → 404。
    let (status, _) = call(
        &machine.state,
        "POST",
        "/workflows/user/no-such-workflow/export",
        Some("{}"),
    )
    .await;
    assert_eq!(status, 404);

    // 冲突：用内置 id 制造一个"用户包"再导入 → 409。
    let (_, exported_source) = call(
        &machine.state,
        "POST",
        "/workflows/user",
        Some(&workflow_json("conflict-source")),
    )
    .await;
    assert_eq!(exported_source["ok"], json!(true));
    let (_, exported) = call(
        &machine.state,
        "POST",
        "/workflows/user/conflict-source/export",
        Some("{}"),
    )
    .await;
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(exported["package_b64"].as_str().unwrap())
        .unwrap();
    let mut package: Value = serde_json::from_slice(&raw).unwrap();
    let mut forged: Value = serde_json::from_str(&json!(package["template"]).to_string()).unwrap();
    forged["template_id"] = json!(builtin_id);
    // 同前：按规范化 JSON 重算 sha256（先反序列化再序列化）。
    let forged_template: owo_agent_protocol::TeamTemplate =
        serde_json::from_value(forged.clone()).unwrap();
    let canonical = serde_json::to_string(&forged_template).unwrap();
    package["template"] = serde_json::from_str::<Value>(&canonical).unwrap();
    package["sha256"] = json!(owo_agent_core::CasStore::hash_of(canonical.as_bytes()));
    let forged_b64 = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_string(&package).unwrap().as_bytes());
    let (status, body) = call(
        &machine.state,
        "POST",
        "/workflows/import",
        Some(&json!({ "package_b64": forged_b64 }).to_string()),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("内置模板冲突"),
        "{body}"
    );
}
