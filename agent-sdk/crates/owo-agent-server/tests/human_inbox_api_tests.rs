//! Human Inbox 集成测试（八期 · 第三路：统一 Human Inbox 后端与直接处理；
//! 九期 · 第二路：主键升级 + ChangeSet 扫描拆分实弹覆盖）。
//!
//! 覆盖（全部使用 IdleProvider + 直接播种 store/run_dir，不依赖模型凭据）：
//! 1. 四类待办统一列表：human_result（人节点）/ step_retry（失败步骤）/
//!    artifact_review（PendingReview 产物）三类实弹源进同一列表；
//!    change_set 为八期二路 ChangeSet（直读 store sidecar，九期起独立于团队扫描）；
//! 2. claim 互斥：双用户竞争仅一人成功；同人重复领取幂等；release 仅领取者；
//! 3. resolve step_retry：真实分派 steer retry（步骤重置）+ 同参重放零副作用 + 异键 409；
//! 4. resolve artifact_review：真实产生评审记录（approve → Approved）+ 重放幂等；
//! 5. resolve human_result：真实录入人节点结果（步骤 Succeeded + Artifact 落盘）；
//! 6. 已解决待办不再出现在列表；resolve 后 last_error 不残留；
//! 7. 九期 ChangeSet 扫描拆分：succeeded 团队 / 超 24 团队上限 / run state 损坏
//!    均不吞 pending ChangeSet；同 step_id 跨团队出独立待办；
//! 8. 九期发生版本：重试轮次分配新 occurrence（不被旧 resolved 项吞掉）；
//! 9. resolve change_set：真实分派 accept（保留文件）/ reject（恢复删除新建文件）+
//!    幂等重放。
//!
//! 存储层持久化/CAS 语义的单元测试在 `human_inbox_store.rs` 内 `#[cfg(test)]`。

use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::goal::StepRecord;
use owo_agent_core::plan::{Plan, StepSpec, StepStatus};
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::workswarm::{RoleSpec, RunMeta};
use owo_agent_core::{Agent, Goal, GoalRunState};
use owo_agent_protocol::{
    Artifact, ArtifactClassification, ChangeSet, ChangeSetFileHash, ChangeSetStatus, MemberHealth,
    ProjectSpace, ProjectSpaceStatus, ReviewState, RuntimeBinding, TeamMember, TeamMode, TeamRun,
    TeamRunStatus,
};
use owo_agent_server::build_router;
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

async fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        owo_agent_core::permissions::Policy::new(&workspace),
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
    if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

async fn call(
    state: &Arc<owo_agent_server::AppState>,
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
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

fn now() -> String {
    "2026-08-30T00:00:00Z".to_string()
}

fn human_member(member_id: &str, role: &str) -> TeamMember {
    TeamMember {
        member_id: member_id.to_string(),
        role: role.to_string(),
        runtime_binding: RuntimeBinding::Human {
            user_id: "u-seed".to_string(),
        },
        capabilities: vec![],
        tool_scope: vec![],
        read_scope: vec![],
        write_scope: vec![],
        budget: json!({}),
        handoff_contract: None,
        health: MemberHealth::Active,
    }
}

fn agent_member(member_id: &str, role: &str) -> TeamMember {
    TeamMember {
        member_id: member_id.to_string(),
        role: role.to_string(),
        runtime_binding: RuntimeBinding::Agent {
            agent_id: "a-seed".to_string(),
        },
        capabilities: vec![],
        tool_scope: vec![],
        read_scope: vec![],
        write_scope: vec![],
        budget: json!({}),
        handoff_contract: None,
        health: MemberHealth::Active,
    }
}

async fn seed_team_run(
    state: &owo_agent_server::AppState,
    team_id: &str,
    project_id: &str,
    status: TeamRunStatus,
    members: Vec<TeamMember>,
) {
    let coordinator = state.workswarm.coordinator().unwrap();
    let team = TeamRun {
        team_id: team_id.to_string(),
        goal_id: None,
        mode: TeamMode::Team,
        members,
        task_graph_ref: None,
        project_space_id: Some(project_id.to_string()),
        template_id: None,
        shared_context_refs: vec![],
        budget: json!({}),
        human_policy: None,
        strategy_decision: None,
        status,
        created_at: now(),
        updated_at: now(),
    };
    coordinator.store().save_team_run(&team).await.unwrap();
}

/// 播种 GoalRunState 侧车（`<run_dir>/<team_id>.json`，run_id = team_id）。
fn seed_run_state(
    state: &owo_agent_server::AppState,
    team_id: &str,
    steps: &[(&str, &str)],
    records: &[(&str, StepStatus, u32, Option<&str>)],
) {
    let coordinator = state.workswarm.coordinator().unwrap();
    let mut plan = Plan::new(format!("plan-{team_id}"), team_id);
    for (step_id, worker) in steps {
        plan.add_step(StepSpec::new(*step_id, *worker));
    }
    let mut run_state = GoalRunState::new(Goal::new(team_id, "inbox 验收目标"), plan);
    run_state.run_id = team_id.to_string();
    for (step_id, status, attempts, error) in records {
        run_state.records.insert(
            (*step_id).to_string(),
            StepRecord {
                step_id: (*step_id).to_string(),
                status: *status,
                attempts: *attempts,
                attempt_id: None,
                output: None,
                error: error.map(|e| e.to_string()),
                skip_reason: None,
                phase_epoch: None,
                validation_receipts: Vec::new(),
            },
        );
    }
    run_state.persist(coordinator.run_dir()).unwrap();
}

/// 播种 RunMeta 侧车（人节点分派依赖 roles[worker] → assignee=human）。
fn seed_run_meta(state: &owo_agent_server::AppState, team_id: &str, worker: &str) {
    let coordinator = state.workswarm.coordinator().unwrap();
    // 成员 id 匹配规则：role_spec_of_member 按 `m-{role}` 对应（m-h ↔ role="h"）。
    let meta = RunMeta {
        team_id: team_id.to_string(),
        correlation_id: format!("corr-{team_id}"),
        roles: vec![RoleSpec {
            node_id: None,
            role: "h".to_string(),
            assignee: "human".to_string(),
            worker: Some(worker.to_string()),
            ..Default::default()
        }],
        template_source: None,
        template_id: None,
        budgets: std::collections::BTreeMap::new(),
        parallel: false,
    };
    std::fs::write(
        coordinator.run_dir().join(format!("{team_id}-meta.json")),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();
}

async fn seed_artifact(state: &owo_agent_server::AppState, project_id: &str, artifact: Artifact) {
    let coordinator = state.workswarm.coordinator().unwrap();
    coordinator
        .store()
        .save_artifact(&artifact, project_id)
        .await
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn make_artifact(
    artifact_id: &str,
    project_team: &str,
    version: u32,
    review_state: ReviewState,
) -> Artifact {
    Artifact {
        artifact_id: artifact_id.to_string(),
        kind: "document".to_string(),
        version,
        producer: "m-w".to_string(),
        content_ref: format!("cas://sha256:seed-{artifact_id}"),
        schema_ref: None,
        source_refs: vec![],
        classification: ArtifactClassification::Public,
        review_state,
        supersedes_artifact_id: None,
        created_at: now(),
        team_id: project_team.to_string(),
        task_id: None,
        attempt_id: None,
        format: "markdown".to_string(),
        media_type: "text/markdown".to_string(),
        file_name: String::new(),
        sha256: String::new(),
        size_bytes: 0,
        evidence_refs: vec![],
        open_issues: vec![],
        validation: None,
        handoff: None,
    }
}

/// 播种三源夹具：
/// - team-a（Running）：s-1 Failed（agent 成员）→ step_retry；
/// - team-b（AwaitingHuman）：s-9 Running（human 成员）→ human_result；
/// - ps-review：a-1 PendingReview → artifact_review。
///
/// 播种项目空间（record_human_result/apply_steer 的 load_bundle 依赖其存在）。
async fn seed_project_space(state: &owo_agent_server::AppState, project_id: &str) {
    let coordinator = state.workswarm.coordinator().unwrap();
    let space = ProjectSpace {
        project_id: project_id.to_string(),
        goal_id: None,
        team_id: None,
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
    };
    coordinator
        .store()
        .save_project_space(&space)
        .await
        .unwrap();
}

async fn seed_all_sources(state: &owo_agent_server::AppState) {
    seed_project_space(state, "ps-a").await;
    seed_team_run(
        state,
        "team-a",
        "ps-a",
        TeamRunStatus::Running,
        vec![agent_member("m-impl", "implementer")],
    )
    .await;
    seed_run_state(
        state,
        "team-a",
        &[("s-1", "m-impl")],
        &[("s-1", StepStatus::Failed, 1, Some("boom"))],
    );
    seed_artifact(
        state,
        "ps-a",
        make_artifact("a-1", "team-a", 2, ReviewState::PendingReview),
    )
    .await;

    seed_project_space(state, "ps-b").await;
    seed_team_run(
        state,
        "team-b",
        "ps-b",
        TeamRunStatus::AwaitingHuman,
        vec![human_member("m-h", "reviewer")],
    )
    .await;
    seed_run_state(
        state,
        "team-b",
        &[("s-9", "m-h")],
        &[("s-9", StepStatus::Running, 1, None)],
    );
    seed_run_meta(state, "team-b", "m-h");
    seed_artifact(
        state,
        "ps-b",
        make_artifact("a-2", "team-b", 1, ReviewState::Approved),
    )
    .await;
}

fn item_of(body: &Value, item_id: &str) -> Option<Value> {
    body["items"]
        .as_array()?
        .iter()
        .find(|i| i["item_id"].as_str() == Some(item_id))
        .cloned()
}

// ---------------------------------------------------------------------------
// 1) 统一列表
// ---------------------------------------------------------------------------

#[tokio::test]
async fn inbox_lists_all_live_sources() {
    let (state, _temp) = test_state().await;
    seed_all_sources(&state).await;
    let app = build_router(Arc::clone(&state));

    let (status, body) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert_eq!(status, 200, "{body}");

    let retry = item_of(&body, "step_retry:team-a:s-1:1").expect("step_retry 条目缺失");
    assert_eq!(retry["kind"], "step_retry");
    assert_eq!(retry["status"], "open");
    assert_eq!(retry["team_id"], "team-a");
    assert_eq!(retry["detail"]["step_id"], "s-1");
    assert_eq!(retry["detail"]["attempts"], 1);
    assert_eq!(retry["detail"]["error"], "boom");

    let human = item_of(&body, "human_result:team-b:s-9:1").expect("human_result 条目缺失");
    assert_eq!(human["kind"], "human_result");
    assert_eq!(human["team_id"], "team-b");
    assert_eq!(human["detail"]["step_status"], "Running");

    let review = item_of(&body, "artifact_review:team-a:a-1:1").expect("artifact_review 条目缺失");
    assert_eq!(review["kind"], "artifact_review");
    assert_eq!(review["project_id"], "ps-a");
    assert_eq!(review["detail"]["review_state"], "PendingReview");

    assert_eq!(body["counts"]["step_retry"], 1);
    assert_eq!(body["counts"]["human_result"], 1);
    assert_eq!(body["counts"]["artifact_review"], 1);
    assert_eq!(body["counts"]["change_set"], 0, "二路 seam 未接线前恒 0");
}

// ---------------------------------------------------------------------------
// 2) claim 互斥 / release
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claim_is_exclusive_and_release_requires_claimer() {
    let (state, _temp) = test_state().await;
    seed_all_sources(&state).await;
    let app = build_router(Arc::clone(&state));
    let path = "/human/inbox/step_retry:team-a:s-1:1/claim";

    let (status, body) = call(&state, &app, "POST", path, Some(r#"{ "user": "alice" }"#)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["item"]["status"], "claimed");
    assert_eq!(body["item"]["assignee"], "alice");
    assert!(body["item"]["claimed_at"].is_u64());

    // 另一用户领取 → 409（同一待办仅允许一个用户处理）。
    let (status, body) = call(&state, &app, "POST", path, Some(r#"{ "user": "bob" }"#)).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["claimed_by"], "alice");

    // 同人重复领取 → 幂等 200。
    let (status, body) = call(&state, &app, "POST", path, Some(r#"{ "user": "alice" }"#)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["item"]["assignee"], "alice");

    // 非领取者释放 → 409。
    let (status, _) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:team-a:s-1:1/release",
        Some(r#"{ "user": "bob" }"#),
    )
    .await;
    assert_eq!(status, 409);

    // 领取者释放 → open。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:team-a:s-1:1/release",
        Some(r#"{ "user": "alice" }"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["item"]["status"], "open");
    assert_eq!(body["item"]["assignee"], Value::Null);
}

// ---------------------------------------------------------------------------
// 3) resolve step_retry：真实分派 + 幂等
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_step_retry_resets_step_and_is_idempotent() {
    let (state, _temp) = test_state().await;
    seed_all_sources(&state).await;
    let app = build_router(Arc::clone(&state));
    let coordinator = state.workswarm.coordinator().unwrap();

    let body = json!({ "user": "alice", "note": "收口重试" }).to_string();
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:team-a:s-1:1/resolve",
        Some(&body),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["resolved"], true);
    assert_eq!(body["replayed"], false);
    assert_eq!(body["result"]["action"], "step_retry");

    // 真实分派：步骤不再处于 Failed（被 retry 重置）。
    let run_state = coordinator.load_run_state("team-a").unwrap();
    let record = run_state.records.get("s-1").unwrap();
    assert!(
        matches!(record.status, StepStatus::Ready | StepStatus::Pending),
        "retry 后步骤应被重置，实际 {:?}",
        record.status
    );

    // 同参重放（派生幂等键相同）→ replayed true，不二次分派。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:team-a:s-1:1/resolve",
        Some(&json!({ "user": "alice", "note": "收口重试" }).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["replayed"], true, "{body}");

    // 异键重复解决 → 409。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:team-a:s-1:1/resolve",
        Some(
            &json!({ "user": "alice", "idempotency_key": "other-key", "note": "另一把钥匙" })
                .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 409, "{body}");

    // 已解决待办不再出现在统一列表。
    let (_, list) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert!(
        item_of(&list, "step_retry:team-a:s-1:1").is_none(),
        "{list}"
    );
}

// ---------------------------------------------------------------------------
// 4) resolve artifact_review：真实评审记录
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_artifact_review_creates_review_record() {
    let (state, _temp) = test_state().await;
    seed_all_sources(&state).await;
    let app = build_router(Arc::clone(&state));
    let coordinator = state.workswarm.coordinator().unwrap();

    let body = json!({
        "user": "alice",
        "decision": "approve",
        "reviewer": "critic-1",
        "comment": "收口批准"
    })
    .to_string();
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/artifact_review:team-a:a-1:1/resolve",
        Some(&body),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["resolved"], true);
    assert_eq!(body["result"]["action"], "artifact_review");
    assert_eq!(body["result"]["replayed"], false);

    // 真实评审记录：产物转 Approved。
    let artifact = coordinator.store().get_artifact("a-1").await.unwrap();
    assert_eq!(artifact.review_state, ReviewState::Approved);

    // 同参重放 → inbox 层 replayed（缓存结果，不重复分派）。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/artifact_review:team-a:a-1:1/resolve",
        Some(
            &json!({
                "user": "alice",
                "decision": "approve",
                "reviewer": "critic-1",
                "comment": "收口批准"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["replayed"], true);

    // 列表中不再出现。
    let (_, list) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert!(
        item_of(&list, "artifact_review:team-a:a-1:1").is_none(),
        "{list}"
    );
}

// ---------------------------------------------------------------------------
// 5) resolve human_result：真实录入
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_human_result_records_artifact() {
    let (state, _temp) = test_state().await;
    seed_all_sources(&state).await;
    let app = build_router(Arc::clone(&state));
    let coordinator = state.workswarm.coordinator().unwrap();

    let body = json!({ "user": "u-seed", "result": "人工复核通过：源对比结论成立" }).to_string();
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/human_result:team-b:s-9:1/resolve",
        Some(&body),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["resolved"], true);
    assert_eq!(body["result"]["action"], "human_result");
    let artifact_id = body["result"]["artifact"]["artifact_id"]
        .as_str()
        .expect("人节点结果应登记 Artifact")
        .to_string();
    assert!(!artifact_id.is_empty());

    // 步骤置 Succeeded。
    let run_state = coordinator.load_run_state("team-b").unwrap();
    assert_eq!(
        run_state.records.get("s-9").unwrap().status,
        StepStatus::Succeeded
    );

    // 列表中不再出现。
    let (_, list) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert!(
        item_of(&list, "human_result:team-b:s-9:1").is_none(),
        "{list}"
    );
}

// ---------------------------------------------------------------------------
// 6) 边界：404 / ChangeSet seam / 空结果校验
// ---------------------------------------------------------------------------

#[tokio::test]
async fn inbox_edges_404_change_set_seam_and_validation() {
    let (state, _temp) = test_state().await;
    seed_all_sources(&state).await;
    let app = build_router(Arc::clone(&state));

    // 未知待办：claim/resolve 均 404。
    let (status, _) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:nope/claim",
        Some(r#"{ "user": "alice" }"#),
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:nope/resolve",
        Some(r#"{ "user": "alice" }"#),
    )
    .await;
    assert_eq!(status, 404);

    // GET 单条：不存在 404；存在（claim 后）返回记录。
    let (status, _) = call(
        &state,
        &app,
        "GET",
        "/human/inbox/step_retry:team-a:s-1:1",
        None,
    )
    .await;
    assert_eq!(status, 200);

    // change_set：无 sidecar 时计数为 0（不误报）；有 pending ChangeSet 必出待办
    // 的实弹覆盖见下方九期新增测试（succeeded 团队 / 超上限 / 损坏 run state）。
    let (_, list) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert_eq!(list["counts"]["change_set"], 0);

    // 空 user 校验。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:team-a:s-1:1/claim",
        Some(r#"{ "user": "  " }"#),
    )
    .await;
    assert_eq!(status, 400, "{body}");

    // human_result resolve 缺 result → 400，且条目回到 open（可重试）。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/human_result:team-b:s-9:1/resolve",
        Some(r#"{ "user": "u-seed" }"#),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    let (_, list) = call(&state, &app, "GET", "/human/inbox", None).await;
    let item = item_of(&list, "human_result:team-b:s-9:1").expect("分派失败应回滚为可重试");
    assert_eq!(item["status"], "open", "{item}");
}

// ---------------------------------------------------------------------------
// 7) 九期（二路）：ChangeSet 扫描拆分 + 发生版本 + Inbox 实弹 accept/reject
// ---------------------------------------------------------------------------

/// 播种 ChangeSet sidecar（`<run_dir>/<team_id>-change-sets.json`，八期二路存储）。
fn seed_change_sets(state: &owo_agent_server::AppState, team_id: &str, records: &[ChangeSet]) {
    let coordinator = state.workswarm.coordinator().unwrap();
    std::fs::create_dir_all(coordinator.run_dir()).unwrap();
    std::fs::write(
        coordinator
            .run_dir()
            .join(format!("{team_id}-change-sets.json")),
        serde_json::to_string_pretty(records).unwrap(),
    )
    .unwrap();
}

/// 构造「新建文件」型 ChangeSet：执行前文件不存在（base sha=None，可恢复），
/// 执行后落盘 content（result sha 用真实 CAS 哈希，保证 reject 恢复可比对）。
fn created_file_change_set(team_id: &str, cs_id: &str, rel: &str, content: &[u8]) -> ChangeSet {
    ChangeSet {
        change_set_id: cs_id.to_string(),
        team_id: team_id.to_string(),
        step_id: "s-impl".to_string(),
        attempt_id: None,
        role: "implementer".to_string(),
        base_hashes: vec![ChangeSetFileHash {
            path: rel.to_string(),
            sha256: None,
            content_available: true,
        }],
        result_hashes: vec![ChangeSetFileHash {
            path: rel.to_string(),
            sha256: Some(owo_agent_core::cas_store::CasStore::hash_of(content)),
            content_available: false,
        }],
        changed_files: vec![rel.to_string()],
        diff_ref: None,
        status: ChangeSetStatus::PendingReview,
        created_at: now(),
        decision: None,
        conflicts: vec![],
    }
}

/// succeeded 团队的 pending ChangeSet 仍必须出现在 /human/inbox（九期拆分核心要求）。
#[tokio::test]
async fn succeeded_team_pending_change_set_still_in_inbox() {
    let (state, _temp) = test_state().await;
    seed_team_run(
        &state,
        "team-succ",
        "ps-succ",
        TeamRunStatus::Succeeded,
        vec![],
    )
    .await;
    let ws = state.workspace.clone();
    std::fs::create_dir_all(ws.join("out")).unwrap();
    std::fs::write(ws.join("out/fix.md"), b"generated").unwrap();
    seed_change_sets(
        &state,
        "team-succ",
        &[created_file_change_set(
            "team-succ",
            "cs-team-succ-s-impl-1",
            "out/fix.md",
            b"generated",
        )],
    );
    let app = build_router(Arc::clone(&state));

    let (status, body) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert_eq!(status, 200, "{body}");
    let item = item_of(&body, "change_set:team-succ:cs-team-succ-s-impl-1:1")
        .expect("succeeded 团队的 pending ChangeSet 必须出现在待办");
    assert_eq!(item["kind"], "change_set");
    assert_eq!(item["team_id"], "team-succ");
    assert_eq!(item["detail"]["status"], "PendingReview");
    assert_eq!(body["counts"]["change_set"], 1, "{body}");
}

/// 超 24 团队扫描上限 / run state 损坏 / succeeded 团队——三种旧扫描会吞掉
/// pending ChangeSet 的场景，拆分后全部照常出待办（26 个团队全量断言）。
#[tokio::test]
async fn change_set_scan_ignores_team_cap_and_broken_run_state() {
    let (state, _temp) = test_state().await;
    let coordinator = state.workswarm.coordinator().unwrap();
    for i in 1..=26 {
        let team_id = format!("team-cap-{i:02}");
        // 全部团队不播 run state（团队扫描 load 失败 → 全部跳过）；
        // team-cap-01 为 succeeded（旧扫描直接 continue）。
        let status = if i == 1 {
            TeamRunStatus::Succeeded
        } else {
            TeamRunStatus::Running
        };
        seed_team_run(&state, &team_id, &format!("ps-cap-{i:02}"), status, vec![]).await;
        seed_change_sets(
            &state,
            &team_id,
            &[created_file_change_set(
                &team_id,
                &format!("cs-{team_id}-1"),
                "out/x.md",
                format!("content-{i}").as_bytes(),
            )],
        );
    }
    // team-cap-02 的 run state 文件损坏（加载失败路径）。
    std::fs::write(coordinator.run_dir().join("team-cap-02.json"), "{ not json").unwrap();

    let app = build_router(Arc::clone(&state));
    let (_, body) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert_eq!(
        body["counts"]["change_set"], 26,
        "pending ChangeSet 不因 24 团队上限 / run state 损坏 / succeeded 丢失：{body}"
    );
    for i in [1usize, 2, 13, 24, 25, 26] {
        let team_id = format!("team-cap-{i:02}");
        assert!(
            item_of(&body, &format!("change_set:{team_id}:cs-{team_id}-1:1")).is_some(),
            "{team_id} 的待办缺失"
        );
    }
    assert_eq!(
        body["counts"]["step_retry"], 0,
        "无 run state → 无团队类待办"
    );
}

/// 两个团队使用相同 step_id → 两个独立待办（九期主键含 team_id）。
#[tokio::test]
async fn same_step_id_across_teams_gets_distinct_items() {
    let (state, _temp) = test_state().await;
    seed_all_sources(&state).await; // team-a s-1 Failed attempts=1
    seed_project_space(&state, "ps-c").await;
    seed_team_run(
        &state,
        "team-c",
        "ps-c",
        TeamRunStatus::Running,
        vec![agent_member("m-impl-c", "implementer")],
    )
    .await;
    seed_run_state(
        &state,
        "team-c",
        &[("s-1", "m-impl-c")],
        &[("s-1", StepStatus::Failed, 1, Some("boom-c"))],
    );
    let app = build_router(Arc::clone(&state));

    let (_, body) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert!(
        item_of(&body, "step_retry:team-a:s-1:1").is_some(),
        "{body}"
    );
    assert!(
        item_of(&body, "step_retry:team-c:s-1:1").is_some(),
        "{body}"
    );
    assert_eq!(body["counts"]["step_retry"], 2, "{body}");
}

/// 发生版本推进：重试重置 attempts=0 后再次失败（attempts=1）→ 分配 :2，
/// 不被旧 resolved 项永久吞掉（八期缺陷的回归守卫）。
#[tokio::test]
async fn step_retry_occurrence_advances_after_resolve() {
    let (state, _temp) = test_state().await;
    seed_all_sources(&state).await;
    let app = build_router(Arc::clone(&state));

    // 第一轮失败（attempts=1）→ occurrence 1。
    let (_, list) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert!(
        item_of(&list, "step_retry:team-a:s-1:1").is_some(),
        "{list}"
    );

    // Inbox resolve → 真实 retry 分派（步骤重置，attempts 归零）。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/step_retry:team-a:s-1:1/resolve",
        Some(r#"{ "user": "alice", "note": "重试一次" }"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // 第二轮再次失败：attempts 仍为 1 → 分配 occurrence 2。
    seed_run_state(
        &state,
        "team-a",
        &[("s-1", "m-impl")],
        &[("s-1", StepStatus::Failed, 1, Some("boom again"))],
    );
    let (_, list) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert!(
        item_of(&list, "step_retry:team-a:s-1:2").is_some(),
        "{list}"
    );
    assert!(
        item_of(&list, "step_retry:team-a:s-1:1").is_none(),
        "旧 resolved 项不再出现：{list}"
    );
    assert_eq!(list["counts"]["step_retry"], 1, "{list}");
}

/// resolve change_set 实弹：accept 保留文件现状；reject 恢复该 ChangeSet 修改的
/// 文件（新建文件 → 删除）；重复 resolve 幂等重放；处理完成后待办消失。
#[tokio::test]
async fn resolve_change_set_accept_reject_and_replay() {
    let (state, _temp) = test_state().await;
    let ws = state.workspace.clone();
    std::fs::create_dir_all(ws.join("out")).unwrap();
    std::fs::write(ws.join("out/a.txt"), b"generated-a").unwrap();
    std::fs::write(ws.join("out/b.txt"), b"generated-b").unwrap();
    seed_change_sets(
        &state,
        "team-cs-a",
        &[created_file_change_set(
            "team-cs-a",
            "cs-team-cs-a-1",
            "out/a.txt",
            b"generated-a",
        )],
    );
    seed_change_sets(
        &state,
        "team-cs-b",
        &[created_file_change_set(
            "team-cs-b",
            "cs-team-cs-b-1",
            "out/b.txt",
            b"generated-b",
        )],
    );
    let app = build_router(Arc::clone(&state));

    // accept：保留文件现状 + ChangeSet 落定 accepted。
    let accept_body = r#"{ "user": "alice", "action": "accept" }"#;
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/change_set:team-cs-a:cs-team-cs-a-1:1/resolve",
        Some(accept_body),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["resolved"], true);
    assert_eq!(body["replayed"], false);
    assert_eq!(body["result"]["action"], "change_set_accept");
    assert_eq!(body["result"]["outcome"]["replayed"], false, "{body}");
    assert_eq!(
        std::fs::read(ws.join("out/a.txt")).unwrap(),
        b"generated-a",
        "accept 保留修改"
    );
    let (status, cs) = call(&state, &app, "GET", "/change-sets/cs-team-cs-a-1", None).await;
    assert_eq!(status, 200, "{cs}");
    assert_eq!(cs["change_set"]["status"], "accepted");

    // 同参重放 → inbox 层幂等（缓存结果，不再分派）。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/change_set:team-cs-a:cs-team-cs-a-1:1/resolve",
        Some(accept_body),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["replayed"], true, "{body}");

    // reject：恢复该 ChangeSet 修改的文件（新建文件 → 删除）。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/human/inbox/change_set:team-cs-b:cs-team-cs-b-1:1/resolve",
        Some(r#"{ "user": "bob", "action": "reject" }"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"]["action"], "change_set_reject");
    assert!(
        !ws.join("out/b.txt").exists(),
        "reject 应删除该 ChangeSet 新建的文件"
    );
    let (status, cs) = call(&state, &app, "GET", "/change-sets/cs-team-cs-b-1", None).await;
    assert_eq!(status, 200, "{cs}");
    assert_eq!(cs["change_set"]["status"], "rejected");

    // 处理完成后待办消失。
    let (_, list) = call(&state, &app, "GET", "/human/inbox", None).await;
    assert!(
        item_of(&list, "change_set:team-cs-a:cs-team-cs-a-1:1").is_none(),
        "{list}"
    );
    assert!(
        item_of(&list, "change_set:team-cs-b:cs-team-cs-b-1:1").is_none(),
        "{list}"
    );
    assert_eq!(list["counts"]["change_set"], 0, "{list}");
}
