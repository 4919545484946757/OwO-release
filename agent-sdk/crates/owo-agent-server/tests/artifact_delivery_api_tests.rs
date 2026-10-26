//! Artifact 下载交付 HTTP 集成测试（七期 · 第三路）。
//!
//! 覆盖：
//! - `GET /artifacts/{id}/content`：JSON 信封（冻结字段 + additive 交付元数据）与
//!   `?raw=true` 原始下载流（Content-Type = media_type、Content-Disposition =
//!   attachment + file_name、字节一致）；
//! - `GET /artifacts/{id}/metadata`：交付元数据（team_id / 哈希 / 字节数 / 校验 /
//!   证据链 / 可选 handoff）+ legacy 记录回退（content_ref 解析哈希、CAS 计算字节数、
//!   artifact_id 首段解析 team_id）；
//! - `GET /projects/{id}/delivery-manifest`：每 kind 最新版本、批准标记、相对下载
//!   路径、评审结论（kind=review）不入清单；
//! - 404 边界（未知产物 / 未知项目）；
//! - 格式门控（真实 TeamCoordinator 契约登记，不依赖模型凭据）：非法 JSON / CSV /
//!   占位 Markdown 登记前拒绝且零落盘（无 Artifact、无 HandoffRecord）；四类格式
//!   合法样例落盘带 validation/sha256/evidence_refs/open_issues，Worker `handoff`
//!   写入真实 HandoffRecord.handoff_note。
//!
//! HTTP 部分直接播种 SQLite + CAS（不经模型）；门控部分走真实 TeamCoordinator。

use axum::http::{header, HeaderMap};
use owo_agent_core::artifact_pipeline::{file_name_of, media_type_of};
use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::permissions::Policy;
use owo_agent_core::project_space_store::ProjectSpaceStoreBackend;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::workswarm_output::{
    WorkerArtifactV1, WorkerEvidenceV1, WorkerOutputStatus, WorkerOutputV1,
};
use owo_agent_core::{
    Agent, CasStore, Goal, GoalRunState, Plan, RunMeta, SqliteProjectSpaceStore, StepSpec,
    TeamCoordinator, TeamTemplateRegistry, WorkSwarmError,
};
use owo_agent_protocol::{
    Artifact, ArtifactClassification, ArtifactValidation, ProjectSpace, ProjectSpaceStatus,
    ReviewState, TeamMode, TeamRun, TeamRunStatus,
};
use owo_agent_server::{build_router, AppState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

const JSON_CONTENT: &str = "{\"ok\": true, \"items\": [1, 2, 3]}";
const CSV_V1_CONTENT: &str = "name,note\nold,初版\n";
const CSV_V2_CONTENT: &str = "name,note\na,更新\nc,新增\n";
const RESEARCH_CONTENT: &str = "# 研究报告\n\n结论：证据链可用，来源可溯源。\n";
const LEGACY_CONTENT: &str = "验证记录正文（legacy 登记样例）。";

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

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn request(
    state: &Arc<AppState>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> axum::http::Request<axum::body::Body> {
    use axum::http::{header as h, Method, Request};
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path)
        .header(
            h::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if let Some(b) = body {
        builder = builder.header(h::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

async fn call(
    state: &Arc<AppState>,
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (u16, Value) {
    let resp = app
        .clone()
        .oneshot(request(state, method, path, body))
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

/// 原始响应（下载流断言用：状态码 + 响应头 + 字节体）。
async fn call_raw(
    state: &Arc<AppState>,
    app: &axum::Router,
    path: &str,
) -> (u16, HeaderMap, Vec<u8>) {
    let resp = app
        .clone()
        .oneshot(request(state, "GET", path, None))
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

/// 构造登记态产物（CAS 落内容，additive 交付元数据按管线口径推导）。
fn make_artifact(
    artifact_id: &str,
    kind: &str,
    format: &str,
    content: &str,
    cas: &CasStore,
    review_state: ReviewState,
) -> Artifact {
    let hash = cas.put(content.as_bytes()).unwrap();
    Artifact {
        artifact_id: artifact_id.to_string(),
        kind: kind.to_string(),
        version: 1,
        producer: format!("m-{}", artifact_id.split(':').nth(1).unwrap_or("builder")),
        content_ref: format!("cas://sha256:{hash}"),
        schema_ref: None,
        source_refs: vec![],
        classification: ArtifactClassification::Private,
        review_state,
        supersedes_artifact_id: None,
        created_at: now(),
        team_id: artifact_id
            .split(':')
            .next()
            .unwrap_or_default()
            .to_string(),
        task_id: Some(format!("task-{artifact_id}")),
        attempt_id: Some(format!("attempt-{artifact_id}")),
        format: format.to_string(),
        media_type: media_type_of(format).to_string(),
        file_name: file_name_of(kind, format),
        sha256: hash,
        size_bytes: content.len() as u64,
        evidence_refs: vec![],
        open_issues: vec![],
        validation: None,
        handoff: None,
    }
}

/// 播种：项目空间 + 四类格式样例（research 已批准带证据链）+ plan 版本链 + legacy
/// 记录 + critic 评审结论（过程产物）。数据落 `data_root/workswarm/{space.db,cas}`，
/// 与 AppState 的懒连接同物理目录。
async fn seeded_state() -> (Arc<AppState>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let data_root = temp.path().to_path_buf();
    let workspace = data_root.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let workswarm_dir = data_root.join("workswarm");
    std::fs::create_dir_all(&workswarm_dir).unwrap();
    let cas = CasStore::new(workswarm_dir.join("cas")).unwrap();
    let store = SqliteProjectSpaceStore::open(&workswarm_dir.join("space.db")).unwrap();

    let project_id = "ps-delivery";
    store
        .save_project_space(&ProjectSpace {
            project_id: project_id.to_string(),
            goal_id: None,
            team_id: Some("team-d1".to_string()),
            tasks: vec![],
            artifacts: vec![],
            decisions: vec![],
            approvals: vec![],
            discussions: vec![],
            activity_stream: vec![],
            delivery_manifest_ref: None,
            rework_tasks: vec![],
            version: 1,
            status: ProjectSpaceStatus::Active,
            created_at: now(),
            updated_at: now(),
        })
        .await
        .unwrap();

    // 研究产物：已批准 + 证据链 + 交接说明 + 校验结果（研究交付可溯源验收样例）。
    let mut research = make_artifact(
        "team-d1:researcher:v1",
        "research",
        "research",
        RESEARCH_CONTENT,
        &cas,
        ReviewState::Approved,
    );
    research.evidence_refs = vec!["https://example.com/sources (调查来源)".to_string()];
    research.validation = Some(ArtifactValidation {
        format: "research".to_string(),
        valid: true,
        reason: None,
    });
    research.handoff = Some("交接：请评审研究结论".to_string());
    store.save_artifact(&research, project_id).await.unwrap();

    // JSON 产物：校验结果随落盘。
    let mut json_artifact = make_artifact(
        "team-d1:builder:v1",
        "document",
        "json",
        JSON_CONTENT,
        &cas,
        ReviewState::Draft,
    );
    json_artifact.validation = Some(ArtifactValidation {
        format: "json".to_string(),
        valid: true,
        reason: None,
    });
    store
        .save_artifact(&json_artifact, project_id)
        .await
        .unwrap();

    // CSV 版本链：v1 → v2（清单应取 v2）。
    let csv_v1 = make_artifact(
        "team-d1:planner:v1",
        "plan",
        "csv",
        CSV_V1_CONTENT,
        &cas,
        ReviewState::Draft,
    );
    store.save_artifact(&csv_v1, project_id).await.unwrap();
    let mut csv_v2 = make_artifact(
        "team-d1:planner:v2",
        "plan",
        "csv",
        CSV_V2_CONTENT,
        &cas,
        ReviewState::Draft,
    );
    csv_v2.version = 2;
    csv_v2.supersedes_artifact_id = Some("team-d1:planner:v1".to_string());
    store.save_artifact(&csv_v2, project_id).await.unwrap();

    // legacy 记录：additive 交付字段全空（服务端回退 content_ref / CAS / artifact_id）。
    let mut legacy = make_artifact(
        "team-legacy:controller:v1",
        "verification",
        "text",
        LEGACY_CONTENT,
        &cas,
        ReviewState::Draft,
    );
    legacy.team_id = String::new();
    legacy.media_type = String::new();
    legacy.file_name = String::new();
    legacy.sha256 = String::new();
    legacy.size_bytes = 0;
    store.save_artifact(&legacy, project_id).await.unwrap();

    // critic 评审结论（过程产物，不入交付清单）。
    let review = make_artifact(
        "team-d1:critic:v1",
        "review",
        "markdown",
        "## 评审\n\n通过，无阻断问题。",
        &cas,
        ReviewState::PendingReview,
    );
    store.save_artifact(&review, project_id).await.unwrap();

    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let session_store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(AppState::new(
        agent,
        session_store,
        workspace.join("traces"),
        data_root,
        workspace,
    ));
    (state, temp)
}

// ---------------------------------------------------------------------------
// content
// ---------------------------------------------------------------------------

#[tokio::test]
async fn artifact_content_envelope_and_raw_download() {
    let (state, _temp) = seeded_state().await;
    let app = build_router(state.clone());

    // JSON 信封：冻结字段 + additive 交付元数据。
    let (status, body) = call(
        &state,
        &app,
        "GET",
        "/artifacts/team-d1:builder:v1/content",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["artifact_id"], "team-d1:builder:v1");
    assert_eq!(body["format"], "json");
    assert_eq!(body["sha256"], CasStore::hash_of(JSON_CONTENT.as_bytes()));
    assert_eq!(body["size_bytes"], JSON_CONTENT.len() as u64);
    assert_eq!(body["content"], JSON_CONTENT);
    assert_eq!(body["kind"], "document");
    assert_eq!(body["media_type"], "application/json");
    assert_eq!(body["file_name"], "document.json");
    assert_eq!(body["validation"]["format"], "json");
    assert_eq!(body["validation"]["valid"], true);

    // 原始下载流：?raw=true → 正确 media type + 文件名 + 字节一致。
    let (status, headers, bytes) = call_raw(
        &state,
        &app,
        "/artifacts/team-d1:builder:v1/content?raw=true",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/json; charset=utf-8"
    );
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"document.json\""
    );
    assert_eq!(bytes, JSON_CONTENT.as_bytes());

    // 研究产物：text/markdown 下载 + 证据链与交接说明随交付透出。
    let (status, headers, bytes) = call_raw(
        &state,
        &app,
        "/artifacts/team-d1:researcher:v1/content?raw=true",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/markdown; charset=utf-8"
    );
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"research.md\""
    );
    assert_eq!(bytes, RESEARCH_CONTENT.as_bytes());

    let (status, body) = call(
        &state,
        &app,
        "GET",
        "/artifacts/team-d1:researcher:v1/content",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["file_name"], "research.md");
    assert_eq!(
        body["evidence_refs"][0],
        "https://example.com/sources (调查来源)"
    );
    assert_eq!(body["open_issues"], json!([]));
    assert_eq!(body["handoff"], "交接：请评审研究结论");
}

// ---------------------------------------------------------------------------
// metadata
// ---------------------------------------------------------------------------

#[tokio::test]
async fn artifact_metadata_shape_and_legacy_fallback() {
    let (state, _temp) = seeded_state().await;
    let app = build_router(state.clone());

    // 冻结字段 + 可选 handoff（Worker 提供交接说明时出现）。
    let (status, body) = call(
        &state,
        &app,
        "GET",
        "/artifacts/team-d1:researcher:v1/metadata",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["artifact_id"], "team-d1:researcher:v1");
    assert_eq!(body["team_id"], "team-d1");
    assert_eq!(body["task_id"], "task-team-d1:researcher:v1");
    assert_eq!(body["attempt_id"], "attempt-team-d1:researcher:v1");
    assert_eq!(body["kind"], "research");
    assert_eq!(body["format"], "research");
    assert_eq!(body["version"], 1);
    assert_eq!(
        body["sha256"],
        CasStore::hash_of(RESEARCH_CONTENT.as_bytes())
    );
    assert_eq!(body["size_bytes"], RESEARCH_CONTENT.len() as u64);
    assert_eq!(body["validation"]["format"], "research");
    assert_eq!(body["validation"]["valid"], true);
    assert_eq!(
        body["evidence_refs"][0],
        "https://example.com/sources (调查来源)"
    );
    assert_eq!(body["handoff"], "交接：请评审研究结论");
    assert_eq!(body["review_state"], "approved");

    // 无交接说明的产物：handoff 可选键不出现。
    let (status, body) = call(
        &state,
        &app,
        "GET",
        "/artifacts/team-d1:builder:v1/metadata",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("handoff").is_none(), "{body}");

    // legacy 记录：team_id 回退 artifact_id 首段；哈希回退 content_ref、
    // 字节数回退 CAS 实际内容。
    let (status, body) = call(
        &state,
        &app,
        "GET",
        "/artifacts/team-legacy:controller:v1/metadata",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["team_id"], "team-legacy");
    assert_eq!(body["format"], "text");
    assert_eq!(body["sha256"], CasStore::hash_of(LEGACY_CONTENT.as_bytes()));
    assert_eq!(body["size_bytes"], LEGACY_CONTENT.len() as u64);
    assert_eq!(body["validation"], json!(null));
}

// ---------------------------------------------------------------------------
// delivery-manifest
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delivery_manifest_lists_latest_per_kind() {
    let (state, _temp) = seeded_state().await;
    let app = build_router(state.clone());

    let (status, body) = call(
        &state,
        &app,
        "GET",
        "/projects/ps-delivery/delivery-manifest",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["project_id"], "ps-delivery");
    assert!(!body["generated_at"].as_str().unwrap().is_empty());
    let entries = body["manifest"].as_array().unwrap();
    // document / plan(v2) / research / verification 各一条；review 过程产物不入清单。
    assert_eq!(entries.len(), 4, "{body}");

    let by_id = |id: &str| {
        entries
            .iter()
            .find(|e| e["artifact_id"] == json!(id))
            .unwrap_or_else(|| panic!("清单缺少 {id}：{body}"))
    };

    // plan 取最新版本，被取代的 v1 不入清单。
    let plan = by_id("team-d1:planner:v2");
    assert_eq!(plan["kind"], "plan");
    assert_eq!(plan["format"], "csv");
    assert_eq!(plan["version"], 2);
    assert_eq!(plan["approved"], false);
    assert_eq!(plan["content_url"], "/artifacts/team-d1:planner:v2/content");
    assert!(entries
        .iter()
        .all(|e| e["artifact_id"] != json!("team-d1:planner:v1")));

    // 批准标记 + 校验结果 + 证据引用（最终交付清单验收样例）。
    let research = by_id("team-d1:researcher:v1");
    assert_eq!(research["approved"], true);
    assert_eq!(research["validation"]["valid"], true);
    assert_eq!(
        research["evidence_refs"][0],
        "https://example.com/sources (调查来源)"
    );
    assert_eq!(
        research["sha256"],
        CasStore::hash_of(RESEARCH_CONTENT.as_bytes())
    );

    // legacy 记录：哈希/字节数由 content_ref + CAS 回退解析。
    let legacy = by_id("team-legacy:controller:v1");
    assert_eq!(
        legacy["sha256"],
        CasStore::hash_of(LEGACY_CONTENT.as_bytes())
    );
    assert_eq!(legacy["size_bytes"], LEGACY_CONTENT.len() as u64);
    assert_eq!(legacy["approved"], false);
}

// ---------------------------------------------------------------------------
// 404 边界
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_artifact_and_project_return_404() {
    let (state, _temp) = seeded_state().await;
    let app = build_router(state.clone());

    for path in [
        "/artifacts/team-x:ghost:v1/content",
        "/artifacts/team-x:ghost:v1/content?raw=true",
        "/artifacts/team-x:ghost:v1/metadata",
    ] {
        let (status, body) = call(&state, &app, "GET", path, None).await;
        assert_eq!(status, 404, "{path} → {body}");
        assert!(
            body["error"].as_str().unwrap().contains("产物不存在"),
            "{body}"
        );
    }

    let (status, body) = call(
        &state,
        &app,
        "GET",
        "/projects/ps-unknown/delivery-manifest",
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("项目空间不存在"),
        "{body}"
    );
}

// ---------------------------------------------------------------------------
// 格式门控（真实 TeamCoordinator 契约登记；不依赖模型凭据）
// ---------------------------------------------------------------------------

/// 播种门控夹具：TeamRun + ProjectSpace + GoalRunState 侧车 + RunMeta 侧车
/// + 真实 TeamCoordinator（`<run_dir>/<team_id>.json` 计划含 s-builder 步骤）。
async fn gate_coordinator(
    temp: &tempfile::TempDir,
) -> (
    TeamCoordinator,
    Arc<SqliteProjectSpaceStore>,
    String,
    String,
) {
    let workswarm_dir = temp.path().join("workswarm");
    std::fs::create_dir_all(&workswarm_dir).unwrap();
    let run_dir = workswarm_dir.join("runs");
    let store = Arc::new(SqliteProjectSpaceStore::open(&workswarm_dir.join("space.db")).unwrap());

    let team_id = "team-gate";
    let project_id = "ps-gate";
    store
        .save_team_run(&TeamRun {
            team_id: team_id.to_string(),
            goal_id: None,
            mode: TeamMode::Team,
            members: vec![],
            task_graph_ref: None,
            project_space_id: Some(project_id.to_string()),
            template_id: None,
            shared_context_refs: vec![],
            budget: json!({}),
            human_policy: None,
            strategy_decision: None,
            status: TeamRunStatus::Running,
            created_at: now(),
            updated_at: now(),
        })
        .await
        .unwrap();
    store
        .save_project_space(&ProjectSpace {
            project_id: project_id.to_string(),
            goal_id: None,
            team_id: Some(team_id.to_string()),
            tasks: vec![],
            artifacts: vec![],
            decisions: vec![],
            approvals: vec![],
            discussions: vec![],
            activity_stream: vec![],
            delivery_manifest_ref: None,
            rework_tasks: vec![],
            version: 1,
            status: ProjectSpaceStatus::Active,
            created_at: now(),
            updated_at: now(),
        })
        .await
        .unwrap();

    // GoalRunState 侧车（persist 为 pub；文件名 = run_id）。
    let mut plan = Plan::new("plan-gate", team_id);
    plan.add_step(StepSpec::new("s-builder", "m-builder"));
    let mut run_state = GoalRunState::new(Goal::new(team_id, "门控验收目标"), plan);
    run_state.run_id = team_id.to_string();
    run_state.persist(&run_dir).unwrap();

    // RunMeta 侧车（save/load 为私有方法：按落盘格式直接写文件）。
    let meta = RunMeta {
        team_id: team_id.to_string(),
        correlation_id: "corr-gate".to_string(),
        roles: vec![],
        template_source: None,
        template_id: None,           // 八期一路 additive：动态组队为 None
        budgets: Default::default(), // 八期一路 additive：角色 → 调用预算
        parallel: false,             // 十一期 additive：非并行模式
    };
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join(format!("{team_id}-meta.json")),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();

    let templates = Arc::new(TeamTemplateRegistry::new(workswarm_dir.join("templates")));
    let cas = CasStore::new(workswarm_dir.join("cas")).unwrap();
    // 四路集成修复（备案）：`TeamCoordinator::new` 首参为 `Arc<dyn ProjectSpaceStoreBackend>`；
    // 泛型 `Arc::clone(&store)` 参数位不做 unsize 强转（E0308），按 workswarm_tests.rs 同款
    // `as Arc<dyn ...>` 显式强转（与 workswarm_recovery/responsiveness_tests 一致）。
    let coordinator = TeamCoordinator::new(
        Arc::clone(&store) as Arc<dyn ProjectSpaceStoreBackend>,
        templates,
        cas,
        run_dir,
    );
    (
        coordinator,
        store,
        team_id.to_string(),
        project_id.to_string(),
    )
}

#[tokio::test]
async fn contract_gate_rejects_invalid_and_registers_valid_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let (coordinator, store, team_id, project_id) = gate_coordinator(&temp).await;

    // 1) 非法 JSON（裹围栏 + 语法坏）：登记前拒绝，零落盘。
    let invalid_json = WorkerOutputV1 {
        status: WorkerOutputStatus::Done,
        summary: "交付 JSON".to_string(),
        artifact: Some(WorkerArtifactV1 {
            kind: "document".to_string(),
            format: "json".to_string(),
            content: "```json\n{\"broken\": }".to_string(),
        }),
        evidence: vec![],
        open_issues: vec![],
        handoff: Some("不应被登记".to_string()),
        review_result: None,
    };
    let err = coordinator
        .register_step_output_contract(
            &team_id,
            "m-builder",
            "builder",
            "s-builder",
            &invalid_json,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, WorkSwarmError::Run(m) if m.starts_with("artifact_invalid:")),
        "{err:?}"
    );
    assert!(store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .list_handoffs_by_project(&project_id)
        .await
        .unwrap()
        .is_empty());

    // 2) 非法 CSV（列数不一致）：登记前拒绝，零落盘。
    let invalid_csv = WorkerOutputV1 {
        status: WorkerOutputStatus::Done,
        summary: "交付 CSV".to_string(),
        artifact: Some(WorkerArtifactV1 {
            kind: "plan".to_string(),
            format: "csv".to_string(),
            content: "name,note\n只有一列\n".to_string(),
        }),
        evidence: vec![],
        open_issues: vec![],
        handoff: None,
        review_result: None,
    };
    let err = coordinator
        .register_step_output_contract(
            &team_id,
            "m-builder",
            "builder",
            "s-builder",
            &invalid_csv,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, WorkSwarmError::Run(m) if m.starts_with("artifact_invalid:")),
        "{err:?}"
    );
    assert!(store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap()
        .is_empty());

    // 3) 合法 JSON：落盘带 validation/sha256/evidence_refs/open_issues，
    //    Worker handoff 写入真实 HandoffRecord。
    let handoff_note = "请评审字段命名后接力";
    let valid_json = WorkerOutputV1 {
        status: WorkerOutputStatus::Done,
        summary: "交付配置 JSON".to_string(),
        artifact: Some(WorkerArtifactV1 {
            kind: "document".to_string(),
            format: "json".to_string(),
            content: "{\"ok\": true}".to_string(),
        }),
        evidence: vec![WorkerEvidenceV1 {
            source: "https://example.com/spec".to_string(),
            note: Some("接口规范".to_string()),
        }],
        open_issues: vec!["边界用例未覆盖".to_string()],
        handoff: Some(handoff_note.to_string()),
        review_result: None,
    };
    let artifact = coordinator
        .register_step_output_contract(
            &team_id,
            "m-builder",
            "builder",
            "s-builder",
            &valid_json,
            None,
        )
        .await
        .unwrap();
    assert_eq!(artifact.artifact_id, format!("{team_id}:builder:v1"));
    assert_eq!(artifact.kind, "document");
    assert_eq!(artifact.format, "json");
    assert_eq!(artifact.media_type, "application/json");
    assert_eq!(artifact.file_name, "document.json");
    assert_eq!(artifact.sha256, CasStore::hash_of(b"{\"ok\": true}"));
    assert_eq!(artifact.size_bytes, "{\"ok\": true}".len() as u64);
    let validation = artifact.validation.as_ref().unwrap();
    assert_eq!(validation.format, "json");
    assert!(validation.valid);
    assert_eq!(
        artifact.evidence_refs,
        vec!["https://example.com/spec (接口规范)".to_string()]
    );
    assert_eq!(artifact.open_issues, vec!["边界用例未覆盖".to_string()]);
    assert_eq!(artifact.handoff.as_deref(), Some(handoff_note));

    let handoffs = store.list_handoffs_by_project(&project_id).await.unwrap();
    assert_eq!(handoffs.len(), 1);
    assert_eq!(handoffs[0].handoff_note.as_deref(), Some(handoff_note));
    assert_eq!(
        handoffs[0].output_artifact_refs,
        vec![artifact.artifact_id.clone()]
    );
    assert!(handoffs[0]
        .evidence_refs
        .contains(&"https://example.com/spec (接口规范)".to_string()));

    // 4) 合法 CSV（表头 + 数据行）：登记，版本链推进到 v2。
    let valid_csv = WorkerOutputV1 {
        status: WorkerOutputStatus::Done,
        summary: "交付清单 CSV".to_string(),
        artifact: Some(WorkerArtifactV1 {
            kind: "plan".to_string(),
            format: "csv".to_string(),
            content: "name,note\na,更新\nc,新增\n".to_string(),
        }),
        evidence: vec![],
        open_issues: vec![],
        handoff: None,
        review_result: None,
    };
    let csv_artifact = coordinator
        .register_step_output_contract(
            &team_id,
            "m-builder",
            "builder",
            "s-builder",
            &valid_csv,
            None,
        )
        .await
        .unwrap();
    assert_eq!(csv_artifact.artifact_id, format!("{team_id}:builder:v2"));
    assert_eq!(csv_artifact.format, "csv");
    assert_eq!(csv_artifact.media_type, "text/csv");
    assert_eq!(csv_artifact.file_name, "plan.csv");
    assert_eq!(csv_artifact.validation.as_ref().unwrap().format, "csv");
    assert!(csv_artifact.validation.as_ref().unwrap().valid);

    // 5) 合法 research（声明 text + kind=research → 按 research 证据链规则）。
    let valid_research = WorkerOutputV1 {
        status: WorkerOutputStatus::Done,
        summary: "研究结论".to_string(),
        artifact: Some(WorkerArtifactV1 {
            kind: "research".to_string(),
            format: "text".to_string(),
            content: "# 调研\n\n结论正文，非占位。".to_string(),
        }),
        evidence: vec![WorkerEvidenceV1 {
            source: "docs/notes.md".to_string(),
            note: None,
        }],
        open_issues: vec![],
        handoff: None,
        review_result: None,
    };
    let research_artifact = coordinator
        .register_step_output_contract(
            &team_id,
            "m-builder",
            "builder",
            "s-builder",
            &valid_research,
            None,
        )
        .await
        .unwrap();
    assert_eq!(research_artifact.format, "research");
    assert_eq!(research_artifact.media_type, "text/markdown");
    assert_eq!(research_artifact.file_name, "research.md");
    let validation = research_artifact.validation.as_ref().unwrap();
    assert_eq!(validation.format, "research");
    assert!(validation.valid);
    assert_eq!(
        research_artifact.evidence_refs,
        vec!["docs/notes.md".to_string()]
    );

    // 6) 占位 Markdown（TBD）：登记前拒绝，零落盘。
    let before = store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap()
        .len();
    let placeholder_markdown = WorkerOutputV1 {
        status: WorkerOutputStatus::Done,
        summary: "交付方案".to_string(),
        artifact: Some(WorkerArtifactV1 {
            kind: "document".to_string(),
            format: "markdown".to_string(),
            content: "TBD".to_string(),
        }),
        evidence: vec![],
        open_issues: vec![],
        handoff: None,
        review_result: None,
    };
    let err = coordinator
        .register_step_output_contract(
            &team_id,
            "m-builder",
            "builder",
            "s-builder",
            &placeholder_markdown,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, WorkSwarmError::Run(m) if m.starts_with("artifact_invalid:")),
        "{err:?}"
    );
    assert_eq!(
        store
            .list_artifacts_by_project(&project_id)
            .await
            .unwrap()
            .len(),
        before
    );

    // 7) 合法 Markdown：登记（版本推进到 v4）。
    let valid_markdown = WorkerOutputV1 {
        status: WorkerOutputStatus::Done,
        summary: "交付方案".to_string(),
        artifact: Some(WorkerArtifactV1 {
            kind: "document".to_string(),
            format: "markdown".to_string(),
            content: "## 方案\n\n正文完整，无占位词。".to_string(),
        }),
        evidence: vec![],
        open_issues: vec![],
        handoff: None,
        review_result: None,
    };
    let markdown_artifact = coordinator
        .register_step_output_contract(
            &team_id,
            "m-builder",
            "builder",
            "s-builder",
            &valid_markdown,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        markdown_artifact.artifact_id,
        format!("{team_id}:builder:v4")
    );
    assert_eq!(markdown_artifact.format, "markdown");
    assert_eq!(markdown_artifact.file_name, "document.md");
    assert!(markdown_artifact.validation.as_ref().unwrap().valid);

    // 终态：4 个合法产物 + 4 条交接记录；被拒产物全部零落盘。
    let artifacts = store.list_artifacts_by_project(&project_id).await.unwrap();
    assert_eq!(artifacts.len(), 4);
    let handoffs = store.list_handoffs_by_project(&project_id).await.unwrap();
    assert_eq!(handoffs.len(), 4);
    assert!(handoffs.iter().all(|h| !h.output_artifact_refs.is_empty()));
}
