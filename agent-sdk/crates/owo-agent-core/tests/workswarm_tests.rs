// R13:WorkSwarm S0 编排层契约测试（§9.0 S0 完成标准）
//! 覆盖：
//! 1. 接力运行全链路（版本化产物经 ref 传递 + 结构化交接 + 交付清单 + 模板提案只提案不启用）；
//! 2. 人节点：等待/唤醒、录入失败校验；
//! 3. 取消：产物保留；取消后 continue 不重跑已完成步骤；
//! 4. steer：只改未完成节点（DecisionRecord 留痕）；运行中 steer → Conflict；
//! 5. 动态团队 Agent 成员 ≤5；
//! 6. 模板采纳后下次组队复用（模板优先）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use owo_agent_core::builtin_team_templates;
use owo_agent_core::goal::{Worker, WorkerRegistry};
use owo_agent_core::project_space_store::{ProjectSpaceStoreBackend, SqliteProjectSpaceStore};
use owo_agent_core::{
    CreateTeamRequest, PhaseOutcome, RoleSpec, RoleWorker, TeamCoordinator, TeamTemplateRegistry,
};
use owo_agent_protocol::{TeamMode, TeamRunStatus};
use serde_json::Value;

const OBJECTIVE: &str = "完成浏览器表单任务并提交报告";

// ---------------------------------------------------------------------------
// 测试内 worker（内层 worker：echo / 慢速 echo）
// ---------------------------------------------------------------------------

struct TaskGraphLeadWorker {
    calls: std::sync::Mutex<Vec<String>>,
    tasks: Vec<Value>,
}

#[async_trait]
impl Worker for TaskGraphLeadWorker {
    fn name(&self) -> &str {
        "task-graph-lead"
    }

    async fn run(&self, input: &Value) -> Result<String, String> {
        let role = input
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.calls.lock().unwrap().push(role.clone());
        if role == "lead" {
            return Ok(serde_json::json!({
                "version": 1,
                "tasks": self.tasks
            })
            .to_string());
        }
        Ok("worker-called".to_string())
    }
}

struct EchoWorker;

#[async_trait]
impl Worker for EchoWorker {
    fn name(&self) -> &str {
        "echo"
    }
    async fn run(&self, input: &Value) -> Result<String, String> {
        let is_reviewer = input
            .get("capabilities")
            .and_then(Value::as_array)
            .is_some_and(|capabilities| capabilities.iter().any(|item| item == "review"))
            || input
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| matches!(role, "critic" | "reviewer" | "content_reviewer"));
        if is_reviewer {
            let team_context = input
                .get("text")
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .unwrap_or_else(|| input.clone());
            let reviewed_requirement_ids = team_context
                .get("upstream")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|artifact| {
                    artifact
                        .get("review_requirements")
                        .and_then(Value::as_array)
                })
                .flatten()
                .filter_map(|requirement| requirement.get("requirement_id").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>();
            return Ok(serde_json::json!({
                "status": "done",
                "summary": "测试评审通过",
                "review_result": {
                    "verdict": "approved",
                    "reviewed_requirement_ids": reviewed_requirement_ids,
                    "findings": []
                },
                "evidence": [],
                "open_issues": []
            })
            .to_string());
        }
        Ok(input
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }
}

struct ReviewRepairWorker {
    review_calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl Worker for ReviewRepairWorker {
    fn name(&self) -> &str {
        "review-repair"
    }

    async fn run(&self, input: &Value) -> Result<String, String> {
        let role = input
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if role == "builder" {
            if let Some(rework) = input.get("rework") {
                let instruction = rework
                    .get("instruction")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !instruction.contains("补齐失败路径的行为说明")
                    || !instruction.contains("补齐取消后的恢复说明")
                {
                    return Err("返修遗漏同一评审的 finding".into());
                }
            }
            let content = if input.get("rework").is_some() {
                "修复后的候选交付"
            } else {
                "初始候选交付"
            };
            return Ok(serde_json::json!({
                "status": "done",
                "summary": "提交候选",
                "artifact": {"kind": "document", "format": "markdown", "content": content},
                "evidence": [],
                "open_issues": []
            })
            .to_string());
        }
        let team_context = input
            .get("text")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or_else(|| input.clone());
        let reviewed_requirement_ids = team_context
            .get("upstream")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|artifact| {
                artifact
                    .get("review_requirements")
                    .and_then(Value::as_array)
            })
            .flatten()
            .filter_map(|requirement| requirement.get("requirement_id").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let first_review = self
            .review_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 0;
        let review_result = if first_review {
            serde_json::json!({
                "verdict": "changes_requested",
                "reviewed_requirement_ids": reviewed_requirement_ids,
                "findings": [{
                    "severity": "blocker",
                    "detail": "补齐失败路径的行为说明",
                    "requirement_id": reviewed_requirement_ids.first(),
                    "evidence_refs": ["artifact-content"],
                    "suggested_owner": "m-builder"
                }, {
                    "severity": "major",
                    "detail": "补齐取消后的恢复说明",
                    "requirement_id": reviewed_requirement_ids.first(),
                    "evidence_refs": ["artifact-content"],
                    "suggested_owner": "m-builder"
                }]
            })
        } else {
            serde_json::json!({
                "verdict": "approved",
                "reviewed_requirement_ids": reviewed_requirement_ids,
                "findings": []
            })
        };
        Ok(serde_json::json!({
            "status": "done",
            "summary": "结构化评审结论",
            "review_result": review_result,
            "evidence": [],
            "open_issues": []
        })
        .to_string())
    }
}

struct FixedWorker {
    output: Value,
}

#[async_trait]
impl Worker for FixedWorker {
    fn name(&self) -> &str {
        "fixed"
    }
    async fn run(&self, _input: &Value) -> Result<String, String> {
        self.output
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "fixed output must be a string".to_string())
    }
}

struct SlowEchoWorker {
    ms: u64,
}

#[async_trait]
impl Worker for SlowEchoWorker {
    fn name(&self) -> &str {
        "slow-echo"
    }
    async fn run(&self, input: &Value) -> Result<String, String> {
        tokio::time::sleep(Duration::from_millis(self.ms)).await;
        Ok(input
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }
}

// ---------------------------------------------------------------------------
// 测试脚手架
// ---------------------------------------------------------------------------

struct Harness {
    dir: PathBuf,
    store: Arc<SqliteProjectSpaceStore>,
    coordinator: Arc<TeamCoordinator>,
    audit: Arc<std::sync::Mutex<owo_agent_core::audit::AuditLog>>,
}

fn harness() -> Harness {
    let dir = std::env::temp_dir().join(format!(
        "owo-ws-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(dir.join("runs")).unwrap();
    let store = Arc::new(SqliteProjectSpaceStore::open(&dir.join("space.db")).unwrap());
    let cas = owo_agent_core::cas_store::CasStore::new(dir.join("cas")).unwrap();
    let templates = Arc::new(TeamTemplateRegistry::new(dir.join("templates")));
    let audit = Arc::new(std::sync::Mutex::new(
        owo_agent_core::audit::AuditLog::default(),
    ));
    let mut coordinator = TeamCoordinator::new(
        Arc::clone(&store)
            as Arc<dyn owo_agent_core::project_space_store::ProjectSpaceStoreBackend>,
        templates,
        cas,
        dir.join("runs"),
    );
    coordinator.attach_audit(Arc::clone(&audit));
    Harness {
        dir,
        store,
        coordinator: Arc::new(coordinator),
        audit,
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 按角色表构建运行 worker 注册表（成员名 → RoleWorker；内层 worker 按引用共享）。
async fn register_parallel_lead_artifact(h: &Harness, team_id: &str, step_id: &str, content: &str) {
    let output = owo_agent_core::workswarm_output::WorkerOutputV1 {
        status: owo_agent_core::workswarm_output::WorkerOutputStatus::Done,
        summary: "已提交并行任务拆解".to_string(),
        artifact: Some(owo_agent_core::workswarm_output::WorkerArtifactV1 {
            kind: "plan".to_string(),
            format: "json".to_string(),
            content: content.to_string(),
        }),
        evidence: Vec::new(),
        open_issues: Vec::new(),
        handoff: None,
        review_result: None,
    };
    h.coordinator
        .register_step_output_contract(team_id, "m-lead", "lead", step_id, &output, None)
        .await
        .unwrap();
}

fn build_registry(
    h: &Harness,
    team_id: &str,
    inner: Arc<dyn Worker>,
    roles: &[RoleSpec],
) -> WorkerRegistry {
    let registry = WorkerRegistry::new();
    for r in roles {
        let member_id = format!("m-{}", r.role);
        registry.register(Arc::new(RoleWorker::new(
            Arc::clone(&h.coordinator),
            team_id.to_string(),
            member_id,
            r.role.clone(),
            Arc::clone(&inner),
        )));
    }
    registry
}

/// 驱动到第一个非 MoreReady 结果（MoreReady 内部循环推进）。
async fn drive_next(h: &Harness, team_id: &str, registry: &WorkerRegistry) -> PhaseOutcome {
    let max_rounds = 50;
    let mut rounds = 0;
    loop {
        let outcome = h
            .coordinator
            .run_phase(team_id, registry)
            .await
            .unwrap_or_else(|e| panic!("run_phase({team_id}) 失败：{e}"));
        match outcome {
            PhaseOutcome::MoreReady => {
                rounds += 1;
                assert!(
                    rounds < max_rounds,
                    "MoreReady 循环超过 {max_rounds} 轮（疑似死循环）"
                );
            }
            PhaseOutcome::Done => {
                h.coordinator
                    .finalize_success(team_id)
                    .await
                    .expect("收尾失败");
                return outcome;
            }
            other => return other,
        }
    }
}

/// 接力样例角色（planner → builder → critic → leader），内层 worker 指定。
fn relay_roles(worker: &str) -> Vec<RoleSpec> {
    let mut roles = owo_agent_core::workswarm::default_relay_roles();
    for r in &mut roles {
        r.worker = Some(worker.to_string());
    }
    roles
}

// ---------------------------------------------------------------------------
// 1. 接力全链路（§9.0：产物经 ref 传递 + 交接 + 交付清单 + 模板提案）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn relay_run_completes_with_artifacts_handoffs_and_proposal() {
    let h = harness();
    let roles = relay_roles("echo");
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let team_id = team.team_id.clone();
    let project_id = team.project_space_id.clone().unwrap();
    assert!(team.status == TeamRunStatus::Created);

    let registry = build_registry(&h, &team_id, Arc::new(EchoWorker), &roles);
    let outcome = drive_next(&h, &team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "接力应成功收尾，实际：{outcome:?}；运行状态：{:?}",
        h.coordinator
            .load_run_state(&team_id)
            .map(|state| (state.goal.error, state.events))
    );
    let context_events = h
        .audit
        .lock()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| entry.session_id == team_id && entry.event == "team.context.assembled")
        .map(|entry| entry.detail.clone())
        .collect::<Vec<_>>();
    assert_eq!(context_events.len(), roles.len());
    assert!(
        context_events
            .iter()
            .any(|detail| detail.contains(r#""phase_snapshot_cache_hit":true"#)),
        "阶段内的后续 Worker 应复用快照并记录缓存命中"
    );

    // 团队与项目空间终态。
    let team = h.store.get_team_run(&team_id).await.unwrap();
    assert!(team.status == TeamRunStatus::Succeeded);
    let space = h.store.get_project_space(&project_id).await.unwrap();
    assert!(space.status == owo_agent_protocol::ProjectSpaceStatus::Completed);
    assert!(space.delivery_manifest_ref.is_some(), "交付清单必须落盘");

    // 版本化产物链：4 个角色各 1 个产物；builder 引用 planner 的产物（ref 传递）。
    let artifacts = h
        .store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap();
    assert_eq!(artifacts.len(), 4, "四角色接力应有 4 个产物");
    let manifest_ref = space.delivery_manifest_ref.as_deref().unwrap();
    let manifest_hash = manifest_ref.strip_prefix("cas://sha256:").unwrap();
    let manifest_text = h.coordinator.cas().get_text(manifest_hash).unwrap();
    let manifest: Value = serde_json::from_str(&manifest_text).unwrap();
    let receipts = manifest["acceptance_receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), artifacts.len());
    for receipt in receipts {
        let artifact_id = receipt["artifact_id"].as_str().unwrap();
        let artifact = artifacts
            .iter()
            .find(|artifact| artifact.artifact_id == artifact_id)
            .unwrap();
        assert_eq!(receipt["content_sha256"], artifact.sha256);
        assert_eq!(receipt["format_validator"]["verdict"], "passed");
        let validations = receipt["validation_receipts"].as_array().unwrap();
        assert!(!validations.is_empty());
        assert!(validations.iter().all(|validation| {
            validation["verdict"] == "passed"
                && validation["attempt_id"].as_str().is_some()
                && validation["input_sha256"].as_str().is_some()
                && validation["changeset_sha256"].is_null()
        }));
    }
    let by_kind: HashMap<&str, &owo_agent_protocol::Artifact> =
        artifacts.iter().map(|a| (a.kind.as_str(), a)).collect();
    let planner = by_kind["plan"];
    let builder = by_kind["document"];
    let critic = by_kind["review"];
    let leader = by_kind["final"];
    assert_eq!(builder.source_refs, vec![planner.artifact_id.clone()]);
    assert_eq!(critic.source_refs, vec![builder.artifact_id.clone()]);
    assert_eq!(leader.source_refs, vec![critic.artifact_id.clone()]);
    // CAS 可解引用；最终步骤只接收直接上游评审快照，原始产物仍通过 source_refs 追溯。
    let cas = &h.coordinator;
    let leader_content = cas
        .cas()
        .get_text(leader.content_ref.strip_prefix("cas://sha256:").unwrap())
        .unwrap();
    assert!(leader_content.contains(OBJECTIVE), "上下文切片应含团队目标");
    assert!(leader_content.contains("reviewer_id"));
    assert!(leader_content.contains(&critic.artifact_id));
    assert!(leader_content.contains("reviewed_artifacts"));

    // 结构化交接记录：每步 → 下游（leader 无下游 → "*"）。
    let handoffs = h.store.list_handoffs_by_project(&project_id).await.unwrap();
    assert_eq!(handoffs.len(), 4);
    let ho: HashMap<&str, &owo_agent_protocol::HandoffRecord> = handoffs
        .iter()
        .map(|x| (x.from_member.as_str(), x))
        .collect();
    assert_eq!(ho["m-builder"].to_member, "m-critic");
    assert_eq!(ho["m-critic"].to_member, "m-leader");
    assert_eq!(ho["m-leader"].to_member, "*");
    assert!(!ho["m-planner"].completed_summary.is_empty());

    // 模板提案：只提案，不自动启用。
    let proposals = h.coordinator.templates().list_proposals();
    assert_eq!(proposals.len(), 1);
    let prop = &proposals[0];
    assert_eq!(
        prop.status,
        owo_agent_protocol::TeamTemplateProposalStatus::Proposed
    );
    assert!(
        h.coordinator
            .templates()
            .get_template(&prop.template.template_id)
            .is_none(),
        "提案未采纳前不得进入模板注册表"
    );
    // 审计留痕（关键动作全部落审计）。
    let entries = h.audit.lock().unwrap().entries.clone();
    let events: Vec<&str> = entries.iter().map(|e| e.event.as_str()).collect();
    assert!(events.contains(&"team.created"));
    assert!(events.contains(&"team.handoff"));
    assert!(events.contains(&"team.template_proposed"));
    assert!(events.contains(&"team.succeeded"));
}

// ---------------------------------------------------------------------------
// 2. 人节点：等待 → 录入 → 唤醒
// ---------------------------------------------------------------------------

fn human_relay_roles() -> Vec<RoleSpec> {
    let mut planner = RoleSpec::agent("planner");
    planner.worker = Some("echo".to_string());
    planner.verify = Some("non_empty".to_string());
    let mut approver = RoleSpec {
        role: "approver".to_string(),
        assignee: "human".to_string(),
        worker: Some("alice".to_string()),
        depends_on: vec!["planner".to_string()],
        handoff_contract: Some("人工确认方案可执行后放行".to_string()),
        verify: Some("non_empty".to_string()),
        extra_input: Value::Null,
        model: None,
        write_paths: Vec::new(),
        capabilities: Vec::new(),
        tools: Vec::new(),
    };
    approver.extra_input = Value::Null;
    let mut leader = RoleSpec::agent("leader");
    leader.depends_on = vec!["approver".to_string()];
    leader.worker = Some("echo".to_string());
    leader.verify = Some("non_empty".to_string());
    vec![planner, approver, leader]
}

#[tokio::test]
async fn blocked_reviewer_does_not_register_as_a_successful_review() {
    let h = harness();
    let mut reviewer = RoleSpec::agent("reviewer");
    reviewer.worker = Some("echo".to_string());
    reviewer.verify = Some("non_empty".to_string());
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: vec![reviewer],
        budget: Value::Null,
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let input = state.plan.steps[0].input.clone();
    let blocked = Value::String(
        r#"{"status":"blocked","summary":"缺少可验证的目标文件快照","open_issues":["需要稳定的文件版本引用"]}"#.to_string(),
    );
    let fixed = FixedWorker { output: blocked };
    let worker = RoleWorker::new(
        Arc::clone(&h.coordinator),
        team.team_id.clone(),
        "m-reviewer".to_string(),
        "reviewer".to_string(),
        Arc::new(fixed),
    );
    let error = worker.run(&input).await.unwrap_err();
    assert!(error.starts_with("worker_blocked:"));
    assert!(h
        .store
        .list_artifacts_by_project(&team.project_space_id.clone().unwrap())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn rework_invalidates_old_attempt_artifact_for_downstream_reads() {
    let h = harness();
    let mut request = CreateTeamRequest::new("rework attempt artifact identity", TeamMode::Team);
    request.roles = human_relay_roles();
    let team = h.coordinator.create_team_run(&request).await.unwrap();
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let producer = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-approver")
        .unwrap()
        .id
        .clone();
    let consumer = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-leader")
        .unwrap()
        .id
        .clone();
    let artifact = h
        .coordinator
        .record_human_result(&team.team_id, &producer, "通过")
        .await
        .unwrap();
    assert!(artifact.attempt_id.is_some());

    h.coordinator
        .read_dependency_artifact(
            &team.team_id,
            "m-leader",
            &consumer,
            &artifact.artifact_id,
            128,
        )
        .await
        .expect("当前 attempt 的依赖产物可读取");

    h.coordinator
        .rework_step(&team.team_id, &producer, "更新方案", "测试返工")
        .await
        .unwrap();
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert!(state.records[&producer].attempt_id.is_none());
    assert!(
        h.coordinator
            .read_dependency_artifact(
                &team.team_id,
                "m-leader",
                &consumer,
                &artifact.artifact_id,
                128
            )
            .await
            .is_err(),
        "返工后旧 attempt 产物不能作为下游当前依赖"
    );
}

#[tokio::test]
async fn human_node_waits_then_resumes_downstream() {
    let h = harness();
    let roles = human_relay_roles();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: Some("approve".to_string()),
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let team_id = team.team_id.clone();
    let project_id = team.project_space_id.clone().unwrap();
    let registry = build_registry(&h, &team_id, Arc::new(EchoWorker), &roles);

    // 阶段 1：planner 完成 → 人节点就绪 → AwaitingHuman。
    let outcome = drive_next(&h, &team_id, &registry).await;
    let PhaseOutcome::AwaitingHuman { waits } = outcome else {
        panic!("预期 AwaitingHuman，实际：{outcome:?}");
    };
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].step_id, "s-approver");
    assert_eq!(waits[0].user_id, "alice");
    let team = h.store.get_team_run(&team_id).await.unwrap();
    assert!(team.status == TeamRunStatus::AwaitingHuman);

    // 录错节点 / 非人节点 → 校验失败。
    let err = h
        .coordinator
        .record_human_result(&team_id, "s-planner", "x")
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            owo_agent_core::workswarm::WorkSwarmError::Validation(_)
        ),
        "对非人节点录入人结果必须被拒绝：{err:?}"
    );

    // 录入人节点结果 → 产物登记 + 步骤完成。
    let artifact = h
        .coordinator
        .record_human_result(&team_id, "s-approver", "批准：方案可以执行")
        .await
        .unwrap();
    assert_eq!(artifact.version, 1);
    assert_eq!(artifact.task_id.as_deref(), Some("s-approver"));
    let human_record = h.coordinator.load_run_state(&team_id).unwrap();
    assert_eq!(
        artifact.attempt_id.as_deref(),
        human_record.records["s-approver"].attempt_id.as_deref()
    );

    // 重复录入 → 冲突。
    let err = h
        .coordinator
        .record_human_result(&team_id, "s-approver", "再次批准")
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        owo_agent_core::workswarm::WorkSwarmError::Conflict(_)
    ));

    // 唤醒下游：leader 完成 → 收尾。
    let outcome = drive_next(&h, &team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "人节点唤醒后应完成：{outcome:?}"
    );
    let team = h.store.get_team_run(&team_id).await.unwrap();
    assert!(team.status == TeamRunStatus::Succeeded);
    let artifacts = h
        .store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap();
    assert_eq!(artifacts.len(), 3);
    // 人节点结果进入产物链（leader 内容可见"批准"）。
    let leader = artifacts
        .iter()
        .find(|a| a.kind == "final")
        .expect("leader 产物缺失");
    let content = h
        .coordinator
        .cas()
        .get_text(leader.content_ref.strip_prefix("cas://sha256:").unwrap())
        .unwrap();
    assert!(content.contains("批准：方案可以执行"));
}

// ---------------------------------------------------------------------------
// 3. 取消：产物保留 + continue 不重跑已完成步骤
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_parked_run_retains_artifacts_and_continue_keeps_progress() {
    let h = harness();
    let roles = human_relay_roles();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let team_id = team.team_id.clone();
    let project_id = team.project_space_id.clone().unwrap();
    let registry = build_registry(&h, &team_id, Arc::new(EchoWorker), &roles);
    let _ = drive_next(&h, &team_id, &registry).await; // → AwaitingHuman（planner 已完成）

    // 暂停窗口内取消（steer cancel）。
    let updated = h
        .coordinator
        .apply_steer(&team_id, &owo_agent_core::workswarm::SteerCommand::Cancel)
        .await
        .unwrap();
    assert!(updated.status == TeamRunStatus::Cancelled);
    let team = h.store.get_team_run(&team_id).await.unwrap();
    assert!(team.status.is_terminal());
    // 产物保留：planner 产物仍在。
    let artifacts = h
        .store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap();
    assert_eq!(artifacts.len(), 1, "取消后已完成产物必须保留");
    assert_eq!(artifacts[0].kind, "plan");
    // 运行循环收尾：终态 → Finished。
    let outcome = drive_next(&h, &team_id, &registry).await;
    assert!(matches!(outcome, PhaseOutcome::Finished));

    // continue：重置未完成步骤；planner 不重跑。
    let updated = h
        .coordinator
        .apply_steer(&team_id, &owo_agent_core::workswarm::SteerCommand::Continue)
        .await
        .unwrap();
    assert!(updated.status == TeamRunStatus::Created);
    let _ = h
        .coordinator
        .record_human_result(&team_id, "s-approver", "批准：继续")
        .await
        .unwrap();
    let outcome = drive_next(&h, &team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "continue 后应完成：{outcome:?}"
    );
    let artifacts = h
        .store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap();
    assert_eq!(artifacts.len(), 3, "planner 不应重跑（产物数 3 而非 4）");
    let plan_artifacts: Vec<_> = artifacts.iter().filter(|a| a.kind == "plan").collect();
    assert_eq!(plan_artifacts.len(), 1, "已完成步骤必须不重跑");
    assert_eq!(plan_artifacts[0].version, 1);
}

// ---------------------------------------------------------------------------
// 4. steer：只改未完成节点 + 运行中 steer → Conflict
// ---------------------------------------------------------------------------

#[tokio::test]
async fn steer_changes_only_uncompleted_nodes_with_decision() {
    let h = harness();
    // 两角色接力：planner 先完成；builder 被 steer 改输入后再执行。
    let mut roles = relay_roles("echo");
    roles.truncate(2); // planner + builder
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let team_id = team.team_id.clone();
    let project_id = team.project_space_id.clone().unwrap();
    let registry = build_registry(&h, &team_id, Arc::new(EchoWorker), &roles);

    // 创建后立即 steer 未完成节点（运行未开始 = 未执行）：改盘 + DecisionRecord 留痕；
    // 随后完整运行：新输入生效；已完成节点不可再 steer（结论留痕在 DecisionRecord）。
    let _ = h
        .coordinator
        .apply_steer(
            &team_id,
            &owo_agent_core::workswarm::SteerCommand::Steer {
                step_id: Some("s-builder".to_string()),
                new_input: Some(serde_json::json!({ "text": "STEEERED-INPUT" })),
                note: "聚焦验收要点".to_string(),
            },
        )
        .await
        .unwrap();
    // steer 落盘：DecisionRecord 留痕。
    let decisions = h
        .store
        .list_decisions_by_project(&project_id)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1, "steer 必须留下 DecisionRecord");
    assert!(decisions[0].choice.contains("聚焦验收要点"));
    assert_eq!(decisions[0].affected_refs, vec!["s-builder".to_string()]);

    // 直接运行到底：builder 使用 steer 后的输入；planner 正常。
    let outcome = drive_next(&h, &team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "steer 后应能完成：{outcome:?}"
    );
    let artifacts = h
        .store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap();
    let builder = artifacts.iter().find(|a| a.kind == "document").unwrap();
    let content = h
        .coordinator
        .cas()
        .get_text(builder.content_ref.strip_prefix("cas://sha256:").unwrap())
        .unwrap();
    assert_eq!(
        content, "STEEERED-INPUT",
        "steer 的新输入必须生效（echo 原文返回）"
    );

    // 对已完成节点 steer → 冲突（已完成成果不受影响）。
    let err = h
        .coordinator
        .apply_steer(
            &team_id,
            &owo_agent_core::workswarm::SteerCommand::Steer {
                step_id: Some("s-planner".to_string()),
                new_input: None,
                note: "改已完成节点".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, owo_agent_core::workswarm::WorkSwarmError::Conflict(_)),
        "已完成节点不可 steer：{err:?}"
    );
}

#[tokio::test]
async fn steer_during_active_run_conflicts_and_cancel_propagates() {
    let h = harness();
    let roles = relay_roles("slow-echo"); // planner 耗时 500ms → 运行窗口可观测
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let team_id = team.team_id.clone();
    let registry = build_registry(&h, &team_id, Arc::new(SlowEchoWorker { ms: 500 }), &roles);

    // 后台驱动阶段 1（planner 慢速执行中）。
    let coordinator = Arc::clone(&h.coordinator);
    let spawned_registry = registry.clone();
    let spawned_team_id = team_id.clone();
    let run_phase_task = tokio::spawn(async move {
        coordinator
            .run_phase(&spawned_team_id, &spawned_registry)
            .await
    });

    // 等运行窗口开启（run-active 标志）。
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !h.coordinator.is_run_active(&team_id) {
        assert!(std::time::Instant::now() < deadline, "等待 run-active 超时");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // 运行中 steer → Conflict（409 语义）。
    let err = h
        .coordinator
        .apply_steer(
            &team_id,
            &owo_agent_core::workswarm::SteerCommand::Steer {
                step_id: None,
                new_input: None,
                note: "运行中改方向".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, owo_agent_core::workswarm::WorkSwarmError::Conflict(_)),
        "运行中 steer 必须 Conflict：{err:?}"
    );
    // 运行中 cancel → 立即传播，阶段以 Aborted 结束。
    let _ = h
        .coordinator
        .apply_steer(&team_id, &owo_agent_core::workswarm::SteerCommand::Cancel)
        .await
        .unwrap();
    let outcome = run_phase_task.await.unwrap().unwrap();
    assert!(
        matches!(outcome, PhaseOutcome::Aborted),
        "cancel 必须中止运行：{outcome:?}"
    );
    let team = h.store.get_team_run(&team_id).await.unwrap();
    assert!(team.status == TeamRunStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// 5. 动态团队上限（§6.1：不超过 5 个 Agent）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dynamic_team_agent_members_capped_at_five() {
    let h = harness();
    let roles: Vec<RoleSpec> = (0..6)
        .map(|i| {
            let mut r = RoleSpec::agent(format!("r{i}"));
            r.worker = Some("echo".to_string());
            if i > 0 {
                r.depends_on = vec![format!("r{}", i - 1)];
            }
            r
        })
        .collect();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles,
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let err = h.coordinator.create_team_run(&req).await.unwrap_err();
    assert!(
        matches!(
            err,
            owo_agent_core::workswarm::WorkSwarmError::Validation(_)
        ),
        "6 个 Agent 成员必须被拒绝：{err:?}"
    );
    // 5 个 → 允许。
    let roles: Vec<RoleSpec> = (0..5)
        .map(|i| {
            let mut r = RoleSpec::agent(format!("r{i}"));
            r.worker = Some("echo".to_string());
            if i > 0 {
                r.depends_on = vec![format!("r{}", i - 1)];
            }
            r
        })
        .collect();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles,
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(team.members.len(), 5);
}

// ---------------------------------------------------------------------------
// 6. 模板优先：采纳后下次同形态组队复用（§6.1 / §6.7）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adopted_template_is_reused_for_next_dynamic_run() {
    let h = harness();
    let roles = relay_roles("echo");
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let team_id = team.team_id.clone();
    let registry = build_registry(&h, &team_id, Arc::new(EchoWorker), &roles);
    let _ = drive_next(&h, &team_id, &registry).await; // 成功 → 产生提案

    let proposals = h.coordinator.templates().list_proposals();
    assert_eq!(proposals.len(), 1);
    let proposal_id = proposals[0].proposal_id.clone();
    // 采纳 → 进注册表。
    let template = h
        .coordinator
        .templates()
        .adopt_proposal(&proposal_id)
        .expect("采纳失败");
    assert!(h
        .coordinator
        .templates()
        .get_template(&template.template_id)
        .is_some());

    // 下一次同形态（team + 相同 objective）动态组队 → 命中模板。
    let req2 = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: Vec::new(), // 空 = 走模板优先
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team2 = h.coordinator.create_team_run(&req2).await.unwrap();
    assert_eq!(
        team2.template_id.as_deref(),
        Some(template.template_id.as_str()),
        "同形态组队必须复用已采纳模板"
    );
    assert_eq!(
        team2.members.len(),
        1,
        "自动匹配的模板按 single 策略裁剪为一个执行者"
    );
    assert_eq!(team2.strategy_decision.as_ref().unwrap()["mode"], "single");
}

// ---------------------------------------------------------------------------
// 7. swarmflow 必须基于版本化模板
// ---------------------------------------------------------------------------

#[tokio::test]
async fn swarmflow_requires_versioned_template() {
    let h = harness();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Swarmflow,
        template_id: None,
        roles: Vec::new(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let err = h.coordinator.create_team_run(&req).await.unwrap_err();
    assert!(
        matches!(
            err,
            owo_agent_core::workswarm::WorkSwarmError::Validation(_)
        ),
        "swarmflow 无模板必须被拒绝：{err:?}"
    );

    // 提供模板 → 允许（显式 template_id）。
    let tpl = owo_agent_protocol::TeamTemplate {
        template_id: "tpl-fixed".into(),
        name: "固定流程".into(),
        mode: TeamMode::Swarmflow,
        roles: vec![
            owo_agent_protocol::TeamTemplateRole {
                role: "planner".into(),
                assignee: "agent".into(),
                worker: Some("echo".into()),
                depends_on: Vec::new(),
                handoff_contract: None,
                verify: None,
                model: None,
                write_paths: Vec::new(),
                capabilities: Vec::new(),
                tools: Vec::new(),
            },
            owo_agent_protocol::TeamTemplateRole {
                role: "builder".into(),
                assignee: "agent".into(),
                worker: Some("echo".into()),
                depends_on: vec!["planner".into()],
                handoff_contract: None,
                verify: None,
                model: None,
                write_paths: Vec::new(),
                capabilities: Vec::new(),
                tools: Vec::new(),
            },
        ],
        applicability: OBJECTIVE.into(),
        source_team_id: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        origin: None,
        description: None,
        version: None,
        tags: Vec::new(),
    };
    h.coordinator.templates().save_template(&tpl).unwrap();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Swarmflow,
        template_id: Some("tpl-fixed".to_string()),
        roles: Vec::new(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(team.template_id.as_deref(), Some("tpl-fixed"));
    assert_eq!(team.members.len(), 2);
}

// ---------------------------------------------------------------------------
// 8. 单步运行（single 形态）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn single_mode_ignores_template_and_never_injects_a_reviewer() {
    let h = harness();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "单 Agent 模式完成一项普通工作".to_string(),
        mode: TeamMode::Single,
        template_id: Some("template-id-that-must-be-ignored".to_string()),
        roles: vec![RoleSpec::agent("builder")],
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(team.members.len(), 1, "single 不得暗中增加 Reviewer 成员");
    assert!(team.template_id.is_none(), "single 请求必须忽略模板");
    assert_eq!(team.strategy_decision.as_ref().unwrap()["mode"], "single");
}

#[tokio::test]
async fn single_mode_runs_one_step_and_skips_template_proposal() {
    let h = harness();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Single,
        template_id: None,
        roles: vec![{
            let mut r = RoleSpec::agent("runner");
            r.worker = Some("echo".to_string());
            r.verify = Some("non_empty".to_string());
            r
        }],
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let team_id = team.team_id.clone();
    let registry = build_registry(&h, &team_id, Arc::new(EchoWorker), &req.roles);
    let outcome = drive_next(&h, &team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "single 应直接完成：{outcome:?}"
    );
    // single 不产生模板提案（无团队经验可沉淀）。
    assert!(h.coordinator.templates().list_proposals().is_empty());
    let space = h
        .store
        .get_project_space(team.project_space_id.as_deref().unwrap())
        .await
        .unwrap();
    assert!(space.status == owo_agent_protocol::ProjectSpaceStatus::Completed);
}

// ---------------------------------------------------------------------------
// 五期（第四路接管集成）：组队策略——auto 判定默认裁剪、强制 single、决策暴露。
// ---------------------------------------------------------------------------
#[tokio::test]
async fn strategy_auto_trims_default_relay_to_single_and_exposes_decision() {
    let h = harness();
    // 无显式角色、无模板命中（全新 objective）→ 默认接力多角色；auto 判定 single → 裁剪为 1。
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "五期策略判定：单产物简单任务".to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: Vec::new(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(
        team.members.len(),
        1,
        "single 策略不启动隐藏的 Reviewer Worker"
    );
    let meta = h.coordinator.load_run_meta(&team.team_id).unwrap();
    assert_eq!(
        meta.roles.iter().filter(|role| !role.is_reviewer()).count(),
        1,
        "single 保留一个执行者，独立评审按声明能力分类"
    );
    let decision = team
        .strategy_decision
        .as_ref()
        .expect("创建响应应带 strategy_decision");
    assert_eq!(decision["mode"], "single");
    assert!(
        decision["reasons"]
            .as_array()
            .map(|r| !r.is_empty())
            .unwrap_or(false),
        "auto 判定必须给出可展示理由"
    );
    assert!(
        decision["budget_calls_total"].is_u64(),
        "预算（调用次数）应暴露"
    );
}

#[tokio::test]
async fn strategy_force_single_trims_explicit_multi_roles() {
    let h = harness();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: relay_roles("echo"),
        budget: Value::Null,
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceSingle),
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(team.members.len(), 1, "强制 single 必须裁剪显式多角色");
    assert_eq!(team.strategy_decision.as_ref().unwrap()["mode"], "single");
}

#[tokio::test]
async fn strategy_force_team_keeps_explicit_roles() {
    let h = harness();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: relay_roles("echo"),
        budget: Value::Null,
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(
        team.members.len(),
        relay_roles("echo").len(),
        "强制 team 保留显式角色"
    );
    assert_eq!(team.strategy_decision.as_ref().unwrap()["mode"], "team");
}

// ---------------------------------------------------------------------------
// 8. 八期一路：自适应角色策略（创建期裁剪 + 运行期跳过 + 提前结束）
// ---------------------------------------------------------------------------

/// 安装内置模板到测试注册表并返回其角色（含依赖）。
fn install_template(h: &Harness, template_id: &str) -> Vec<RoleSpec> {
    let d = builtin_team_templates::descriptor(template_id).expect("内置模板应存在");
    h.coordinator
        .templates()
        .save_template(&d.template)
        .expect("模板安装失败");
    d.template
        .roles
        .iter()
        .cloned()
        .map(RoleSpec::from)
        .collect()
}

#[tokio::test]
async fn adaptive_code_template_retains_reviewer_until_runtime() {
    let h = harness();
    let template_roles = install_template(&h, builtin_team_templates::CODE_CHANGE_V1);
    // 角色来自模板（req.roles 为空）→ 创建期裁剪生效。
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "修复 src/calc.rs 的减法符号错误".to_string(),
        mode: TeamMode::Team,
        template_id: Some(builtin_team_templates::CODE_CHANGE_V1.to_string()),
        roles: Vec::new(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    // Reviewer 保留至宿主观察候选变化；无变化时才由运行期跳过。
    assert_eq!(team.members.len(), template_roles.len());
    assert!(team.members.iter().any(|m| m.role == "reviewer"));
    let adaptive = team
        .strategy_decision
        .as_ref()
        .expect("strategy_decision 应存在")["adaptive"]
        .clone();
    assert_eq!(adaptive["saved_budget_calls"], 0);
    assert!(
        adaptive["skipped_roles"].as_array().unwrap().is_empty(),
        "创建时不能在候选版本出现前删除 Reviewer"
    );
    // 全链路可运行（DAG 合法且收尾成功）。
    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &template_roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "保留 reviewer 的团队应正常收尾：{outcome:?}"
    );
}

#[tokio::test]
async fn adaptive_research_template_rewrites_deps_and_keeps_one_summarizer() {
    let h = harness();
    let template_roles = install_template(&h, builtin_team_templates::RESEARCH_BRIEF_V1);
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "对比两种缓存淘汰策略并产出研究简报".to_string(),
        mode: TeamMode::Team,
        template_id: Some(builtin_team_templates::RESEARCH_BRIEF_V1.to_string()),
        roles: Vec::new(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    // 并行研究保留（researcher_a/b），核验被裁 → 只剩一个汇总角色。
    assert_eq!(team.members.len(), template_roles.len() - 1);
    assert!(team.members.iter().any(|m| m.role == "researcher_a"));
    assert!(team.members.iter().any(|m| m.role == "researcher_b"));
    assert!(team.members.iter().any(|m| m.role == "brief_writer"));
    assert!(!team.members.iter().any(|m| m.role == "evidence_verifier"));
    let adaptive = team.strategy_decision.as_ref().unwrap()["adaptive"].clone();
    assert_eq!(adaptive["skipped_roles"][0]["role"], "evidence_verifier");
    assert_eq!(adaptive["saved_budget_calls"], 3);
    // 依赖重定向生效：brief_writer（原依赖 s-evidence_verifier）若未重写到
    // researcher_a/b，create_team_run 内 plan.validate() 会因依赖缺失直接报错；
    // 能走到这里即证明 DAG 合法，再驱动到成功收尾确认无死锁。
    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &template_roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "重定向后研究团队应正常收尾（无双路依赖死锁）：{outcome:?}"
    );
}

#[tokio::test]
async fn adaptive_runtime_skip_ends_dag_early_when_no_changes() {
    let h = harness();
    let template_roles = install_template(&h, builtin_team_templates::CODE_CHANGE_V1);
    // 用户显式编排（含 reviewer）→ 创建期尊重不裁剪；reviewer 由运行期跳过兜底。
    let mut roles = template_roles.clone();
    for r in &mut roles {
        r.worker = Some("echo".to_string());
    }
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "重构登录模块的错误处理".to_string(),
        mode: TeamMode::Team,
        template_id: Some(builtin_team_templates::CODE_CHANGE_V1.to_string()),
        roles,
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(
        team.members.len(),
        template_roles.len(),
        "显式编排保留 reviewer"
    );
    let adaptive_at_creation = team.strategy_decision.as_ref().unwrap()["adaptive"].clone();
    assert!(
        adaptive_at_creation["skipped_roles"]
            .as_array()
            .unwrap()
            .is_empty(),
        "显式编排创建期不裁剪"
    );
    // 运行：echo worker 不产生任何工作区变更 → reviewer 就绪时被运行期跳过，
    // 全部完成条件满足 → DAG 提前结束（Done + early_exit 指标）。
    // 运行前写入有效的空变更记录；缺少/损坏的跟踪文件应视为未知并保留评审。
    std::fs::write(
        h.dir
            .join("runs")
            .join(format!("{}-workspace-changes.json", team.team_id)),
        r#"[{"changed_files":[]}]"#,
    )
    .unwrap();
    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &template_roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "运行期跳过后应提前收尾：{outcome:?}"
    );
    let team = h.store.get_team_run(&team.team_id).await.unwrap();
    assert_eq!(team.status, TeamRunStatus::Succeeded);
    let persisted_state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert!(persisted_state.records["s-reviewer"]
        .skip_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("提前结束")));
    let adaptive = team.strategy_decision.as_ref().unwrap()["adaptive"].clone();
    let runtime_skipped = adaptive["runtime_skipped"].as_array().unwrap();
    assert_eq!(runtime_skipped.len(), 1);
    assert_eq!(runtime_skipped[0]["role"], "reviewer");
    assert!(
        runtime_skipped[0]["reason"]
            .as_str()
            .unwrap()
            .contains("提前结束"),
        "运行期跳过原因应可展示：{}",
        runtime_skipped[0]["reason"]
    );
    // early_exit_reason 平铺字段应存在（四路冻结口径），reason 含提前结束语义。
    assert!(
        adaptive["early_exit_reason"]
            .as_str()
            .unwrap_or("")
            .contains("提前结束"),
        "early_exit_reason 应存在：{}",
        adaptive["early_exit_reason"]
    );
}

// ---------------------------------------------------------------------------
// 十一期 · 一路：角色模型 / 角色写范围（多模型并行开发的编排数据面）
// ---------------------------------------------------------------------------

/// `RoleSpec.model` → 步骤输入 `model`（AgentSubagentWorker 按 input.model 解析）；
/// `RoleSpec.write_paths` → `TeamMember.write_scope`（服务端范围租约/白名单来源）。
#[tokio::test]
async fn role_model_and_write_paths_flow_into_steps_and_members() {
    let h = harness();
    let mut w1 = RoleSpec::agent("w1");
    w1.worker = Some("echo".to_string());
    w1.model = Some("glm-5.3-flash".to_string());
    w1.write_paths = vec!["src/a".to_string()];
    let mut w2 = RoleSpec::agent("w2");
    w2.worker = Some("echo".to_string());
    w2.model = Some("glm-4.6".to_string());
    w2.write_paths = vec!["src/b".to_string(), "src/common".to_string()];
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: vec![w1, w2],
        budget: Value::Null,
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(team.members.len(), 2, "显式编排不得被策略裁剪");
    let member = |role: &str| {
        team.members
            .iter()
            .find(|m| m.role == role)
            .unwrap_or_else(|| panic!("缺少成员 {role}"))
    };
    assert_eq!(member("w1").write_scope, vec!["src/a".to_string()]);
    assert_eq!(
        member("w2").write_scope,
        vec!["src/b".to_string(), "src/common".to_string()]
    );

    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let step = |id: &str| {
        state
            .plan
            .steps
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("缺少步骤 {id}"))
    };
    assert_eq!(step("s-w1").input["model"], "glm-5.3-flash");
    assert_eq!(step("s-w2").input["model"], "glm-4.6");
}

#[tokio::test]
async fn explicit_review_capability_is_read_only_and_requires_bound_upstream_snapshot() {
    let h = harness();
    let mut reviewer = RoleSpec::agent("quality_gate");
    reviewer.capabilities = vec!["review".to_string()];
    reviewer.worker = Some("echo".to_string());
    let mut req = CreateTeamRequest::new(OBJECTIVE, TeamMode::Team);
    req.roles = vec![reviewer];
    req.strategy = Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam);
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let member = team.members.first().expect("review member");
    assert_eq!(member.capabilities, vec!["review"]);
    assert!(member.write_scope.is_empty());
    assert_eq!(member.tool_scope, vec!["read"]);

    let output = owo_agent_core::workswarm_output::WorkerOutputV1 {
        status: owo_agent_core::workswarm_output::WorkerOutputStatus::Done,
        summary: "评审通过".to_string(),
        artifact: None,
        evidence: Vec::new(),
        open_issues: Vec::new(),
        handoff: None,
        review_result: Some(owo_agent_core::workswarm_output::WorkerReviewResultV1 {
            verdict: owo_agent_core::workswarm_output::WorkerReviewVerdict::Approved,
            reviewed_requirement_ids: Vec::new(),
            findings: Vec::new(),
        }),
    };
    let error = h
        .coordinator
        .register_step_output_contract(
            &team.team_id,
            &member.member_id,
            "quality_gate",
            "s-quality_gate",
            &output,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Worker 实际读取的宿主源码快照"));
}

#[tokio::test]
async fn review_finding_dispatches_bounded_repair_to_original_owner() {
    let h = harness();
    let mut builder = RoleSpec::agent("builder");
    builder.worker = Some("review-repair".to_string());
    builder.verify = Some("non_empty".to_string());
    let mut reviewer = RoleSpec::agent("quality_gate");
    reviewer.capabilities = vec!["review".to_string()];
    reviewer.worker = Some("review-repair".to_string());
    reviewer.verify = Some("non_empty".to_string());
    reviewer.depends_on = vec!["builder".to_string()];
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: vec![builder, reviewer],
        budget: Value::Null,
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let project_id = team.project_space_id.clone().unwrap();
    let registry = build_registry(
        &h,
        &team.team_id,
        Arc::new(ReviewRepairWorker {
            review_calls: std::sync::atomic::AtomicUsize::new(0),
        }),
        &[RoleSpec::agent("builder"), {
            let mut role = RoleSpec::agent("quality_gate");
            role.capabilities = vec!["review".to_string()];
            role
        }],
    );
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    let audit = h.audit.lock().unwrap().entries.clone();
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "返修复验应收敛：{outcome:?}；audit={audit:#?}"
    );

    let artifacts = h
        .store
        .list_artifacts_by_project(&project_id)
        .await
        .unwrap();
    let builder_versions = artifacts
        .iter()
        .filter(|artifact| artifact.producer == "m-builder")
        .collect::<Vec<_>>();
    assert_eq!(builder_versions.len(), 2, "finding 应让原 owner 产生新版本");
    assert_eq!(
        builder_versions[1].supersedes_artifact_id.as_deref(),
        Some(builder_versions[0].artifact_id.as_str())
    );
    let reviews = artifacts
        .iter()
        .filter(|artifact| artifact.kind == "review")
        .collect::<Vec<_>>();
    assert_eq!(reviews.len(), 2, "修复后必须重新评审新快照");
    let latest_review_hash = reviews[1]
        .content_ref
        .strip_prefix("cas://sha256:")
        .unwrap();
    let review: Value =
        serde_json::from_slice(&h.coordinator.cas().get(latest_review_hash).unwrap()).unwrap();
    assert_eq!(review["result"]["verdict"], "approved");
    assert_eq!(
        review["reviewed_artifacts"][0]["sha256"],
        builder_versions[1].sha256
    );

    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert_eq!(
        state.delivery_issues.len(),
        2,
        "全部 finding 必须登记到交付问题账本"
    );
    assert!(state
        .delivery_issues
        .iter()
        .all(|issue| issue.status == owo_agent_core::goal::DeliveryIssueStatusV1::Resolved));
    let owner_step = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-builder")
        .unwrap();
    assert_eq!(
        owner_step.input["rework"]["attempt"], 1,
        "多个 finding 只消耗一次 owner 返修"
    );

    let space = h.store.get_project_space(&project_id).await.unwrap();
    let manifest_ref = space.delivery_manifest_ref.as_deref().unwrap();
    let manifest_hash = manifest_ref.strip_prefix("cas://sha256:").unwrap();
    let manifest: Value =
        serde_json::from_slice(&h.coordinator.cas().get(manifest_hash).unwrap()).unwrap();
    let deliverables = manifest["artifacts"].as_array().unwrap();
    assert!(
        deliverables
            .iter()
            .all(|artifact| artifact["kind"] != "review"),
        "review artifacts remain evidence and must not be published as user deliverables: {deliverables:#?}"
    );
    assert!(
        manifest["acceptance_receipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|receipt| receipt["artifact_kind"] == "review"),
        "review artifact must remain represented in acceptance evidence"
    );
}

/// 写路径必须是相对路径（权限默认 deny：非法声明在组队期直接拒绝）。
#[tokio::test]
async fn role_write_paths_must_be_relative() {
    let h = harness();
    let mut writer = RoleSpec::agent("writer");
    writer.worker = Some("echo".to_string());
    writer.write_paths = vec!["../escape".to_string()];
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: vec![writer],
        budget: Value::Null,
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let err = h.coordinator.create_team_run(&req).await.unwrap_err();
    assert!(
        err.to_string().contains("写路径"),
        "非法写路径必须报错：{err:?}"
    );
}

/// 十一期 additive wire：`RoleSpec` / `TeamTemplateRole` 新字段可解析，旧体缺省。
#[test]
fn role_spec_and_template_role_deserialize_new_fields_additively() {
    let spec: RoleSpec = serde_json::from_value(serde_json::json!({
        "role": "w1",
        "assignee": "agent",
        "depends_on": [],
        "model": "glm-5.3-flash",
        "write_paths": ["src/a", "src/common"],
    }))
    .unwrap();
    assert_eq!(spec.model.as_deref(), Some("glm-5.3-flash"));
    assert_eq!(spec.write_paths, vec!["src/a", "src/common"]);

    let legacy: RoleSpec = serde_json::from_value(serde_json::json!({"role": "builder"})).unwrap();
    assert!(legacy.model.is_none(), "旧 wire 缺省 model");
    assert!(legacy.write_paths.is_empty(), "旧 wire 缺省 write_paths");

    let template_role: owo_agent_protocol::TeamTemplateRole =
        serde_json::from_value(serde_json::json!({
            "role": "w1",
            "assignee": "agent",
            "model": "glm-4.6",
            "write_paths": ["src/b"],
        }))
        .unwrap();
    assert_eq!(template_role.model.as_deref(), Some("glm-4.6"));
    assert_eq!(template_role.write_paths, vec!["src/b"]);
}

// ---------------------------------------------------------------------------
// 十一期 · 并行开发：统一模型 / 预算并行度 / lead 拆解的任务主动分配
// ---------------------------------------------------------------------------

/// 团队统一模型注入全部 agent 步骤；角色显式 model 覆盖统一值。
#[tokio::test]
async fn team_unified_model_injects_into_agent_steps_with_role_override() {
    let h = harness();
    let mut w1 = RoleSpec::agent("w1");
    w1.worker = Some("echo".to_string());
    let mut w2 = RoleSpec::agent("w2");
    w2.worker = Some("echo".to_string());
    w2.model = Some("glm-4.6".to_string());
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: vec![w1, w2],
        budget: serde_json::json!({ "max_parallel": 3 }),
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: Some("glm-5.3-flashx".to_string()),
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let step = |id: &str| {
        state
            .plan
            .steps
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("缺少步骤 {id}"))
    };
    assert_eq!(step("s-w1").input["model"], "glm-5.3-flashx");
    assert_eq!(
        step("s-w2").input["model"],
        "glm-4.6",
        "角色显式覆盖统一模型"
    );
    assert_eq!(
        state.goal.budget.max_parallel, 3,
        "团队预算 max_parallel 应进 GoalBudget（缺省 4）"
    );
}

/// 空白 TeamRun 的首个阶段只应运行 lead；TaskGraph 持久化后再派发动态任务。
#[tokio::test]
async fn parallel_team_runs_only_lead_before_dynamic_task_assignment() {
    let h = harness();
    let mut roles = owo_agent_core::workswarm::parallel_roles(2);
    for role in &mut roles {
        role.worker = Some("task-graph-lead".to_string());
    }
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "先拆分再执行动态任务".to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: serde_json::json!({ "max_parallel": 2 }),
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: Some("glm-5.3-flashx".to_string()),
        parallel: true,
        max_agent_members: Some(4),
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let inner = Arc::new(TaskGraphLeadWorker {
        calls: std::sync::Mutex::new(Vec::new()),
        tasks: vec![serde_json::json!({
            "task_id": "only", "worker": "w1", "task": "实现唯一任务", "depends_on": [],
            "requirement_quotes": ["执行动态任务"],
            "read_refs": [], "write_paths": [], "contract_refs": [],
            "required_capabilities": [], "estimated_effort": 1, "verification": "non_empty",
            "risk": "low", "priority": 1, "acceptance": "执行动态任务：交付任务结果"
        })],
    });
    let registry = build_registry(&h, &team.team_id, inner.clone(), &roles);

    let outcome = h
        .coordinator
        .run_phase(&team.team_id, &registry)
        .await
        .unwrap();
    assert!(matches!(outcome, PhaseOutcome::MoreReady), "{outcome:?}");
    assert_eq!(*inner.calls.lock().unwrap(), vec!["lead"]);

    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert_eq!(
        state
            .plan
            .steps
            .iter()
            .filter_map(|step| { step.input.get("assigned_task_id").and_then(Value::as_str) })
            .collect::<Vec<_>>(),
        vec!["only"]
    );
}

/// 独立任务由宿主清单汇总；不存在跨任务集成需求时不启动固定 leader 模型调用。
#[tokio::test]
async fn parallel_independent_tasks_skip_leader_and_publish_host_manifest() {
    let h = harness();
    let mut roles = owo_agent_core::workswarm::parallel_roles(2);
    for role in &mut roles {
        role.worker = Some("task-graph-lead".to_string());
    }
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "独立交付两个互不冲突的模块".to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: serde_json::json!({ "max_parallel": 2 }),
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: Some("glm-5.3-flashx".to_string()),
        parallel: true,
        max_agent_members: Some(4),
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let inner = Arc::new(TaskGraphLeadWorker {
        calls: std::sync::Mutex::new(Vec::new()),
        tasks: vec![
            serde_json::json!({
                "task_id": "module-a", "worker": "w1", "task": "交付模块 A", "depends_on": [],
                "read_refs": [], "write_paths": ["src/a"], "contract_refs": [],
                "required_capabilities": ["write_file"], "estimated_effort": 2,
                "verification": "non_empty", "risk": "low", "priority": 80,
                "requirement_quotes": ["互不冲突的模块"],
                "acceptance": "互不冲突的模块：模块 A 结果已提交"
            }),
            serde_json::json!({
                "task_id": "module-b", "worker": "w2", "task": "交付模块 B", "depends_on": [],
                "read_refs": [], "write_paths": ["src/b"], "contract_refs": [],
                "required_capabilities": ["write_file"], "estimated_effort": 2,
                "verification": "non_empty", "risk": "low", "priority": 80,
                "requirement_quotes": ["独立交付两个互不冲突的模块"],
                "acceptance": "独立交付两个互不冲突的模块：模块 B 结果已提交"
            }),
        ],
    });
    let registry = build_registry(&h, &team.team_id, inner.clone(), &roles);

    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "{outcome:?}; error={:?}",
        h.coordinator
            .load_run_state(&team.team_id)
            .ok()
            .and_then(|state| state.goal.error)
    );
    let calls = inner.calls.lock().unwrap().clone();
    assert_eq!(
        calls.iter().filter(|role| role.as_str() == "lead").count(),
        1
    );
    assert_eq!(calls.iter().filter(|role| role.as_str() == "w1").count(), 1);
    assert_eq!(calls.iter().filter(|role| role.as_str() == "w2").count(), 1);
    assert!(
        !calls.iter().any(|role| role == "leader"),
        "calls={calls:?}"
    );

    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let leader = state
        .plan
        .steps
        .iter()
        .find(|step| step.id == "s-leader")
        .unwrap();
    assert_eq!(
        state.records[&leader.id].status,
        owo_agent_core::plan::StepStatus::Succeeded
    );
    assert_eq!(
        state.records[&leader.id].skip_reason.as_deref(),
        Some("host_manifest:independent_task_graph")
    );
    let project_space = h
        .store
        .get_project_space(team.project_space_id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(
        project_space.status,
        owo_agent_protocol::ProjectSpaceStatus::Completed
    );
    let manifest_ref = project_space.delivery_manifest_ref.as_deref().unwrap();
    let manifest_hash = manifest_ref.strip_prefix("cas://sha256:").unwrap();
    let manifest_bytes = owo_agent_core::cas_store::CasStore::new(h.dir.join("cas"))
        .unwrap()
        .get(manifest_hash)
        .unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let manifest_artifacts = manifest["artifacts"].as_array().unwrap();
    let producers = manifest_artifacts
        .iter()
        .filter_map(|artifact| artifact["producer"].as_str())
        .collect::<Vec<_>>();
    assert!(producers.contains(&"m-w1"), "producers={producers:?}");
    assert!(producers.contains(&"m-w2"), "producers={producers:?}");
    assert_eq!(
        manifest["acceptance_receipts"].as_array().unwrap().len(),
        manifest_artifacts.len()
    );
}

/// 并行开发：`parallel_roles(N)` + parallel 标记 + 成员上限覆盖可建队；
/// lead 产物就绪后，运行期把 subtasks（子任务 + 写范围）动态应用到 writer。
#[tokio::test]
async fn parallel_lead_assignment_applies_writer_scopes_and_tasks() {
    let h = harness();
    let mut roles = owo_agent_core::workswarm::parallel_roles(2);
    let mut reviewer = RoleSpec::agent("reviewer");
    reviewer.depends_on = vec!["lead".to_string()];
    reviewer.verify = Some("non_empty".to_string());
    roles.push(reviewer);
    for role in &mut roles {
        role.worker = Some("echo".to_string());
    }
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: serde_json::json!({ "max_parallel": 2 }),
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: Some("glm-5.3-flashx".to_string()),
        parallel: true,
        max_agent_members: Some(5),
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(team.members.len(), 5, "lead + w1 + w2 + reviewer + leader");
    assert!(
        !owo_agent_core::worker_profile::WorkerProfile::for_role("lead", 0).is_writer(),
        "lead 必须只读（拆解不落盘）"
    );

    // 模拟 lead 已经成功并产出可解析的拆解 JSON（真实运行由模型产出同构实体）。
    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let lead_id = state
        .plan
        .steps
        .iter()
        .find(|s| s.id == "s-lead")
        .map(|s| s.id.clone())
        .expect("lead 步骤");
    state.records.get_mut(&lead_id).unwrap().status = owo_agent_core::plan::StepStatus::Succeeded;
    state.records.get_mut(&lead_id).unwrap().output = Some(
        serde_json::json!({
            "version": 1,
            "tasks": [
                {"task_id":"a", "worker": "w1", "task": "实现模块 A", "depends_on":[], "requirement_quotes":["完成浏览器表单任务"], "read_refs":["src/lib.rs"], "write_paths": ["src/a"], "contract_refs":["API-A"], "required_capabilities":["write_file"], "estimated_effort":3, "verification":"contains:done", "risk":"normal", "priority":80, "acceptance": "完成浏览器表单任务：A 通过"},
                {"task_id":"b", "worker": "w2", "task": "实现模块 B", "depends_on":[], "requirement_quotes":["提交报告"], "read_refs":[], "write_paths": ["src/b"], "contract_refs":[], "required_capabilities":["write_file"], "estimated_effort":5, "verification":"non_empty", "risk":"normal", "priority":80, "acceptance": "提交报告：B 通过"},
                {"task_id":"c", "worker": "w1", "task": "实现模块 C", "depends_on":["a"], "requirement_quotes":["浏览器表单任务"], "read_refs":[], "write_paths": ["src/c"], "contract_refs":[], "required_capabilities":["apply_patch"], "estimated_effort":2, "verification":"non_empty", "risk":"critical", "priority":100, "acceptance": "浏览器表单任务：C 通过"}
            ]
        })
        .to_string(),
    );
    state
        .goal
        .transition(owo_agent_core::goal::GoalStatus::Running);
    state.persist(&h.dir.join("runs")).unwrap();
    let lead_output = state.records[&lead_id].output.clone().unwrap();
    register_parallel_lead_artifact(&h, &team.team_id, &lead_id, &lead_output).await;

    // 驱动：w1/w2 同一 wave 并行（echo），随后 leader 汇总，最终 Done。
    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "{outcome:?}; state={:?}",
        h.coordinator
            .load_run_state(&team.team_id)
            .ok()
            .and_then(|state| state.goal.error)
    );

    // RunMeta：写范围与子任务说明已动态应用。
    let meta_raw = std::fs::read_to_string(
        h.dir
            .join("runs")
            .join(format!("{}-meta.json", team.team_id)),
    )
    .unwrap();
    let meta: Value = serde_json::from_str(&meta_raw).unwrap();
    let meta_role = |name: &str| {
        meta["roles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|role| role["role"] == name)
            .unwrap_or_else(|| panic!("meta 缺少角色 {name}"))
            .clone()
    };
    assert_eq!(
        meta_role("w1")["write_paths"],
        serde_json::json!(["src/a", "src/c"])
    );
    assert_eq!(meta_role("w2")["write_paths"], serde_json::json!(["src/b"]));
    assert!(meta_role("w1")["handoff_contract"]
        .as_str()
        .unwrap()
        .contains("assigned_task"));

    // 步骤输入：assigned_task 同步 + 团队统一模型已注入。
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let dispatched_task_order: Vec<_> = state
        .plan
        .steps
        .iter()
        .filter_map(|step| step.input.get("assigned_task_id").and_then(Value::as_str))
        .collect();
    assert_eq!(dispatched_task_order, vec!["c", "b", "a"]);
    let step = |id: &str| {
        state
            .plan
            .steps
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("缺少步骤 {id}"))
    };
    assert_eq!(step("s-w1").input["assigned_task"], "实现模块 A");
    assert_eq!(step("s-w1").input["assigned_task_id"], "a");
    assert_eq!(
        step("s-w1").input["assigned_acceptance"],
        "完成浏览器表单任务：A 通过"
    );
    assert_eq!(step("s-w1").input["assigned_verification"], "contains:done");
    assert_eq!(
        step("s-w1").input["assigned_read_refs"],
        serde_json::json!(["src/lib.rs"])
    );
    assert_eq!(
        step("s-w1").input["assigned_contract_refs"],
        serde_json::json!(["API-A"])
    );
    assert_eq!(step("s-w1").input["model"], "glm-5.3-flashx");
    assert_eq!(step("s-w2").input["assigned_task"], "实现模块 B");
    assert_eq!(step("s-task-c").input["assigned_task"], "实现模块 C");
    assert_eq!(step("s-task-c").depends_on, vec!["s-w1"]);
    assert_eq!(step("s-task-c").input["assigned_write_paths"][0], "src/c");
    assert_eq!(
        step("s-task-c").input["required_capabilities"],
        serde_json::json!(["apply_patch"])
    );
    assert_eq!(
        step("s-reviewer").depends_on,
        vec![
            "s-lead".to_string(),
            "s-leader".to_string(),
            "s-task-c".to_string(),
            "s-w1".to_string(),
            "s-w2".to_string(),
        ]
    );
    assert!(
        !step("s-leader")
            .depends_on
            .contains(&"s-reviewer".to_string()),
        "integrator must run before the reviewer to bind the review to the final integrated snapshot"
    );
    assert_eq!(
        state.records["s-reviewer"].status,
        owo_agent_core::plan::StepStatus::Succeeded
    );
    assert_eq!(
        state.records["s-leader"].skip_reason.as_deref(),
        Some("host_manifest:independent_task_graph")
    );
}

#[tokio::test]
async fn invalid_parallel_lead_plan_fails_team_before_reporting_success() {
    let h = harness();
    let mut roles = owo_agent_core::workswarm::parallel_roles(2);
    for role in &mut roles {
        role.worker = Some("echo".to_string());
    }
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "拒绝不完整的并行计划".to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: serde_json::json!({ "max_parallel": 2 }),
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: None,
        parallel: true,
        max_agent_members: Some(4),
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let lead_id = state
        .plan
        .steps
        .iter()
        .find(|step| step.id == "s-lead")
        .map(|step| step.id.clone())
        .expect("lead step");
    state.records.get_mut(&lead_id).unwrap().status = owo_agent_core::plan::StepStatus::Succeeded;
    state.records.get_mut(&lead_id).unwrap().output = Some(
        serde_json::json!({
            "subtasks": [
                {"worker": "w1", "task": "task one", "acceptance": "", "write_paths": ["src/a"]}
            ]
        })
        .to_string(),
    );
    state
        .goal
        .transition(owo_agent_core::goal::GoalStatus::Running);
    state.persist(&h.dir.join("runs")).unwrap();

    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(matches!(outcome, PhaseOutcome::Failed), "{outcome:?}");
    let failed = h.coordinator.get_team_run(&team.team_id).await.unwrap();
    assert_eq!(format!("{:?}", failed.status), "Failed");
}

/// 用户显式预声明的写范围是硬约束：lead 的动态分配不得改写它（只更新子任务说明）。
#[tokio::test]
async fn parallel_assignment_respects_preexisting_write_scope() {
    let h = harness();
    let mut roles = owo_agent_core::workswarm::parallel_roles(2);
    for role in &mut roles {
        role.worker = Some("echo".to_string());
    }
    // w2 预声明 src/user（索引：lead=0,w1=1,w2=2,leader=3）。
    roles[2].write_paths = vec!["src/user".to_string()];
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "并行实现并保留用户声明的写范围".to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: serde_json::json!({ "max_parallel": 2 }),
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: None,
        parallel: true,
        max_agent_members: Some(4),
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let lead_id = state
        .plan
        .steps
        .iter()
        .find(|s| s.id == "s-lead")
        .map(|s| s.id.clone())
        .expect("lead 步骤");
    state.records.get_mut(&lead_id).unwrap().status = owo_agent_core::plan::StepStatus::Succeeded;
    state.records.get_mut(&lead_id).unwrap().output = Some(
        serde_json::json!({
            "subtasks": [
                {"worker": "w1", "task": "任务一", "write_paths": ["src/a"], "acceptance": "A 通过"},
                {"worker": "w2", "task": "任务二", "write_paths": ["src/user"], "acceptance": "B 通过"}
            ]
        })
        .to_string(),
    );
    state
        .goal
        .transition(owo_agent_core::goal::GoalStatus::Running);
    state.persist(&h.dir.join("runs")).unwrap();
    let lead_output = state.records[&lead_id].output.clone().unwrap();
    register_parallel_lead_artifact(&h, &team.team_id, &lead_id, &lead_output).await;

    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "{outcome:?}; error={:?}",
        h.coordinator
            .load_run_state(&team.team_id)
            .ok()
            .and_then(|state| state.goal.error)
    );

    let meta_raw = std::fs::read_to_string(
        h.dir
            .join("runs")
            .join(format!("{}-meta.json", team.team_id)),
    )
    .unwrap();
    let meta: Value = serde_json::from_str(&meta_raw).unwrap();
    let meta_role = |name: &str| {
        meta["roles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|role| role["role"] == name)
            .unwrap_or_else(|| panic!("meta 缺少角色 {name}"))
            .clone()
    };
    assert_eq!(
        meta_role("w2")["write_paths"],
        serde_json::json!(["src/user"]),
        "用户声明的写范围不得被 lead 分配覆盖"
    );
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let task_step = state
        .plan
        .steps
        .iter()
        .find(|step| step.id == "s-w2")
        .unwrap();
    assert_eq!(
        task_step.input["assigned_write_paths"],
        serde_json::json!(["src/user"])
    );
    assert_eq!(
        meta_role("w1")["write_paths"],
        serde_json::json!(["src/a"]),
        "未预声明的 writer 采用 lead 分配"
    );
}

#[tokio::test]
async fn adaptive_context_event_enters_team_audit_without_context_contents() {
    let h = harness();
    let team = h
        .coordinator
        .create_team_run(&CreateTeamRequest::new(OBJECTIVE, TeamMode::Single))
        .await
        .unwrap();
    h.coordinator
        .note_adaptive_event(
            &team.team_id,
            serde_json::json!({
                "kind": "context",
                "role": "w1",
                "step_id": "s-w1",
                "context_bytes": 42,
                "context_revision": 7,
                "body": "private context must not reach the audit",
            }),
        )
        .await;
    let audit = h.audit.lock().unwrap();
    let entry = audit
        .entries
        .iter()
        .find(|entry| entry.event == "team.context.revision")
        .expect("context event should be visible to the team SSE audit feed");
    assert!(entry.detail.contains("role=w1"));
    assert!(entry.detail.contains("step_id=s-w1"));
    assert!(entry.detail.contains("context_bytes=42"));
    assert!(!entry.detail.contains("private context"));
}

#[tokio::test]
async fn source_session_core_spec_is_saved_in_cas_and_restored_for_workers() {
    let h = harness();
    let mut req = CreateTeamRequest::new("根据源会话约束完成任务", TeamMode::Single);
    req.parent_context_snapshot = Some(
        serde_json::json!({
            "kind": "source_session_context_v1",
            "source_session_id": "session-source",
            "source_updated_at": "2026-10-02T00:00:00Z",
            "core_spec": {
                "system_constraints": "保留现有 API，不要扩大写入范围。",
                "recent_user_requirements": ["优先完成功能闭环。"]
            }
        })
        .to_string(),
    );
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    assert_eq!(team.shared_context_refs.len(), 1);
    let step_id = h
        .coordinator
        .load_run_state(&team.team_id)
        .unwrap()
        .plan
        .steps[0]
        .id
        .clone();
    let slice = h
        .coordinator
        .assemble_context_slice(&team.team_id, "m-runner", &step_id)
        .await
        .unwrap();
    assert!(slice["core_spec"].to_string().contains("保留现有 API"));
    assert!(slice["core_spec"].to_string().contains("优先完成功能闭环"));

    let hash = team.shared_context_refs[0]
        .strip_prefix("cas://sha256:")
        .unwrap();
    std::fs::write(h.dir.join("cas").join(hash), b"tampered").unwrap();
    assert!(
        h.coordinator
            .assemble_context_slice(&team.team_id, "m-runner", &step_id)
            .await
            .is_err(),
        "损坏的父会话约束必须阻断组装，不能静默当成无上下文"
    );
}

#[tokio::test]
async fn parent_context_cas_uses_bounded_single_pass_and_enforces_size_limit() {
    let h = harness();
    let long_constraint = "约束".repeat(40_000);
    let snapshot = serde_json::json!({
        "kind": "source_session_context_v1",
        "core_spec": { "system_constraints": long_constraint }
    })
    .to_string();
    let mut request = CreateTeamRequest::new("保留完整父会话约束", TeamMode::Single);
    request.parent_context_snapshot = Some(snapshot);
    let team = h.coordinator.create_team_run(&request).await.unwrap();
    let step_id = h
        .coordinator
        .load_run_state(&team.team_id)
        .unwrap()
        .plan
        .steps[0]
        .id
        .clone();
    let slice = h
        .coordinator
        .assemble_context_slice(&team.team_id, "m-runner", &step_id)
        .await
        .unwrap();
    assert_eq!(
        slice["core_spec"][0]["system_constraints"].as_str(),
        Some(long_constraint.as_str()),
        "跨 64 KiB 分页后的上下文必须完整还原"
    );

    let oversized = format!(
        "{{\"kind\":\"source_session_context_v1\",\"core_spec\":{{\"text\":\"{}\"}}}}",
        "x".repeat(256 * 1024)
    );
    let mut invalid = CreateTeamRequest::new("拒绝超大父会话上下文", TeamMode::Single);
    invalid.parent_context_snapshot = Some(oversized);
    assert!(matches!(
        h.coordinator.create_team_run(&invalid).await,
        Err(owo_agent_core::WorkSwarmError::Validation(_))
    ));
}

#[tokio::test]
async fn dependency_artifact_read_is_scoped_to_step_and_bounded() {
    let h = harness();
    let mut producer = RoleSpec::agent("producer");
    producer.worker = Some("echo".to_string());
    let mut consumer = RoleSpec::agent("consumer");
    consumer.worker = Some("echo".to_string());
    consumer.depends_on = vec!["producer".to_string()];
    let mut request = CreateTeamRequest::new("test scoped artifact read", TeamMode::Team);
    request.roles = vec![producer, consumer];
    let team = h.coordinator.create_team_run(&request).await.unwrap();
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let producer_step = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-producer")
        .unwrap();
    let consumer_step = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-consumer")
        .unwrap();
    let artifact = h
        .coordinator
        .register_step_output(
            &team.team_id,
            "m-producer",
            "producer",
            &producer_step.id,
            "ABCDEFGHI",
        )
        .await
        .unwrap();

    let read = h
        .coordinator
        .read_dependency_artifact(
            &team.team_id,
            "m-consumer",
            &consumer_step.id,
            &artifact.artifact_id,
            5,
        )
        .await
        .unwrap();
    assert_eq!(read["content"], "ABCDE");
    assert_eq!(read["truncated"], true);
    assert_eq!(read["artifact_id"], artifact.artifact_id);
    assert_eq!(artifact.task_id.as_deref(), Some(producer_step.id.as_str()));
    assert!(artifact.attempt_id.as_deref().is_some_and(|attempt| {
        attempt.starts_with(&format!("{}:{}:", team.team_id, producer_step.id))
    }));

    let denied = h
        .coordinator
        .read_dependency_artifact(
            &team.team_id,
            "m-producer",
            &producer_step.id,
            &artifact.artifact_id,
            64,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        denied,
        owo_agent_core::WorkSwarmError::Validation(_)
    ));
}

#[tokio::test]
async fn published_shared_context_is_versioned_and_reaches_worker_slice() {
    let h = harness();
    let team = h
        .coordinator
        .create_team_run(&CreateTeamRequest::new(
            "实现接口并共享事实",
            TeamMode::Single,
        ))
        .await
        .unwrap();
    let fact = h
        .coordinator
        .publish_team_context_fact(
            &team.team_id,
            0,
            owo_agent_core::workswarm::SharedContextFactDraft {
                key: "api.contract".to_string(),
                value: "GET /v1/items returns ItemList; verified by route test.".to_string(),
                producer: "m-runner".to_string(),
                task_id: None,
                source_refs: vec!["route_contract_tests.rs".to_string()],
                file_hash: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(fact.revision, 1);
    let revised_fact = h
        .coordinator
        .publish_team_context_fact(
            &team.team_id,
            1,
            owo_agent_core::workswarm::SharedContextFactDraft {
                key: "api.contract".to_string(),
                value: "GET /items returns ItemList v2".to_string(),
                producer: "m-runner".to_string(),
                task_id: None,
                source_refs: vec!["updated-test".to_string()],
                file_hash: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(revised_fact.revision, 2);
    let stale = h
        .coordinator
        .publish_team_context_fact(
            &team.team_id,
            0,
            owo_agent_core::workswarm::SharedContextFactDraft {
                key: "stale".to_string(),
                value: "must be rejected".to_string(),
                producer: "m-runner".to_string(),
                task_id: None,
                source_refs: vec![],
                file_hash: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(stale, owo_agent_core::WorkSwarmError::Conflict(_)));
    let step = h
        .coordinator
        .load_run_state(&team.team_id)
        .unwrap()
        .plan
        .steps[0]
        .id
        .clone();
    let slice = h
        .coordinator
        .assemble_context_slice(&team.team_id, "m-runner", &step)
        .await
        .unwrap();
    assert_eq!(slice["shared_context_revision"], 2);
    assert!(slice["shared_facts"]
        .to_string()
        .contains("GET /items returns ItemList v2"));
    assert!(!slice["shared_facts"]
        .to_string()
        .contains("verified by route test"));
    assert!(slice["shared_facts"].to_string().contains("unverified"));

    let hash = revised_fact
        .value_ref
        .strip_prefix("cas://sha256:")
        .unwrap();
    std::fs::write(h.dir.join("cas").join(hash), b"tampered").unwrap();
    assert!(
        h.coordinator
            .assemble_context_slice(&team.team_id, "m-runner", &step)
            .await
            .is_err(),
        "损坏的共享约束必须阻断组装，不能静默跳过"
    );
}

#[tokio::test]
async fn shared_fact_context_parallel_reads_keep_newest_first_and_exact_byte_budget() {
    let h = harness();
    let team = h
        .coordinator
        .create_team_run(&CreateTeamRequest::new(
            "assemble bounded shared facts",
            TeamMode::Single,
        ))
        .await
        .unwrap();
    for revision in 0..5_u64 {
        h.coordinator
            .publish_team_context_fact(
                &team.team_id,
                revision,
                owo_agent_core::workswarm::SharedContextFactDraft {
                    key: format!("fact-{revision}"),
                    value: format!("fact-{revision} {}", "x".repeat(3000)),
                    producer: "m-runner".to_string(),
                    task_id: None,
                    source_refs: Vec::new(),
                    file_hash: None,
                },
            )
            .await
            .unwrap();
    }
    let step = h
        .coordinator
        .load_run_state(&team.team_id)
        .unwrap()
        .plan
        .steps[0]
        .id
        .clone();
    let slice = h
        .coordinator
        .assemble_context_slice(&team.team_id, "m-runner", &step)
        .await
        .unwrap();
    let facts = slice["shared_facts"].as_array().unwrap();

    assert_eq!(facts.len(), 4);
    assert_eq!(facts[0]["key"], "fact-4");
    assert_eq!(facts[1]["key"], "fact-3");
    assert_eq!(facts[2]["key"], "fact-2");
    assert_eq!(facts[3]["key"], "fact-1");
    let lengths = facts
        .iter()
        .map(|fact| fact["value"].as_str().unwrap().len())
        .collect::<Vec<_>>();
    assert_eq!(lengths, vec![2400, 2400, 2400, 992]);
    assert_eq!(lengths.iter().sum::<usize>(), 8 * 1024);
    assert!(facts.iter().all(|fact| fact["truncated"] == true));
}

#[tokio::test]
async fn hash_bound_context_fact_is_not_injected_before_workspace_validation() {
    let h = harness();
    let team = h
        .coordinator
        .create_team_run(&CreateTeamRequest::new(
            "defer hash-bound facts until validation",
            TeamMode::Single,
        ))
        .await
        .unwrap();
    h.coordinator
        .publish_team_context_fact(
            &team.team_id,
            0,
            owo_agent_core::workswarm::SharedContextFactDraft {
                key: "api.contract".into(),
                value: "hash-bound private contract".into(),
                producer: "m-runner".into(),
                task_id: None,
                source_refs: vec!["src/api.rs".into()],
                file_hash: Some(format!("sha256:{}", "b".repeat(64))),
            },
        )
        .await
        .unwrap();
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let step = state.plan.steps[0].id.clone();
    let slice = h
        .coordinator
        .assemble_context_slice(&team.team_id, "m-runner", &step)
        .await
        .unwrap();
    assert!(!slice["shared_facts"]
        .to_string()
        .contains("hash-bound private contract"));
}

#[tokio::test]
async fn stale_context_fact_is_appended_and_hides_the_previous_candidate() {
    let h = harness();
    let team = h
        .coordinator
        .create_team_run(&CreateTeamRequest::new(
            "invalidate stale project fact",
            TeamMode::Single,
        ))
        .await
        .unwrap();
    let published = h
        .coordinator
        .publish_team_context_fact(
            &team.team_id,
            0,
            owo_agent_core::workswarm::SharedContextFactDraft {
                key: "api.contract".into(),
                value: "old contract".into(),
                producer: "m-runner".into(),
                task_id: None,
                source_refs: vec!["src/api.rs".into()],
                file_hash: Some(format!("sha256:{}", "a".repeat(64))),
            },
        )
        .await
        .unwrap();
    let stale = h
        .coordinator
        .mark_team_context_fact_stale(&team.team_id, &published.key, published.revision, 1)
        .await
        .unwrap();
    assert_eq!(stale.revision, 2);
    assert_eq!(stale.status, "stale");
    let snapshot = h
        .coordinator
        .read_team_context(&team.team_id)
        .await
        .unwrap();
    assert_eq!(snapshot.revision, 2);
    assert_eq!(snapshot.facts.last().unwrap().status, "stale");
    let conflict = h
        .coordinator
        .mark_team_context_fact_stale(
            &team.team_id,
            &published.key,
            published.revision,
            snapshot.revision,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        conflict,
        owo_agent_core::WorkSwarmError::Conflict(_)
    ));
}

#[tokio::test]
async fn concurrent_team_context_publishers_cannot_both_commit_same_revision() {
    let h = harness();
    let second = Arc::new(SqliteProjectSpaceStore::open(&h.dir.join("space.db")).unwrap());
    let fact = |key: &str| owo_agent_protocol::SharedContextFact {
        key: key.to_string(),
        value_ref: format!("cas://sha256:{key}"),
        revision: 1,
        producer: "user".to_string(),
        task_id: None,
        source_refs: vec![],
        file_hash: None,
        confidence: "unverified".to_string(),
        status: "candidate".to_string(),
        created_at: "2026-10-02T00:00:00Z".to_string(),
    };
    let store_a = Arc::clone(&h.store);
    let store_b = second;
    let fact_a = fact("a");
    let fact_b = fact("b");
    let (a, b) = tokio::join!(
        store_a.compare_and_swap_team_context("team-race", 0, &fact_a),
        store_b.compare_and_swap_team_context("team-race", 0, &fact_b),
    );
    let outcomes = [a.unwrap(), b.unwrap()];
    assert_eq!(outcomes.iter().filter(|committed| **committed).count(), 1);
    let snapshot = h.store.get_team_context("team-race").await.unwrap();
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.facts.len(), 1);
}

#[tokio::test]
async fn finalize_success_rejects_unaccepted_changeset_for_current_attempt() {
    let h = harness();
    let roles = relay_roles("echo");
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &roles);
    let mut completed = false;
    for _ in 0..50 {
        match h
            .coordinator
            .run_phase(&team.team_id, &registry)
            .await
            .unwrap()
        {
            PhaseOutcome::MoreReady => {}
            PhaseOutcome::Done => {
                completed = true;
                break;
            }
            other => panic!("接力步骤未完成：{other:?}"),
        }
    }
    assert!(completed, "接力步骤未在限制轮数内完成");

    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let (step, attempt_id) = state
        .plan
        .steps
        .iter()
        .find_map(|step| {
            state
                .records
                .get(&step.id)
                .and_then(|record| record.attempt_id.as_deref())
                .map(|attempt_id| (step, attempt_id))
        })
        .expect("已完成步骤应绑定 attempt_id");
    let role = step.worker.strip_prefix("m-").expect("worker member id");
    let change_set = owo_agent_protocol::ChangeSet {
        change_set_id: format!("{}:{}:attempt-pending-old", team.team_id, step.id),
        team_id: team.team_id.clone(),
        step_id: step.id.clone(),
        attempt_id: Some(attempt_id.to_string()),
        role: role.to_string(),
        base_hashes: Vec::new(),
        result_hashes: Vec::new(),
        changed_files: vec!["docs/report.md".to_string()],
        diff_ref: Some("test.patch".to_string()),
        status: owo_agent_protocol::ChangeSetStatus::PendingReview,
        created_at: "2026-10-02T00:00:00Z".to_string(),
        decision: None,
        conflicts: Vec::new(),
    };
    let changes = owo_agent_core::change_set_store::ChangeSetStore::new(h.coordinator.run_dir());
    changes.save_upsert(&change_set).unwrap();
    let mut newer_accepted = change_set.clone();
    newer_accepted.change_set_id = format!("{}:{}:attempt-accepted-new", team.team_id, step.id);
    newer_accepted.status = owo_agent_protocol::ChangeSetStatus::Accepted;
    newer_accepted.created_at = "2026-10-03T00:00:00Z".to_string();
    newer_accepted.decision = Some(owo_agent_protocol::ChangeSetDecision {
        action: "accept".to_string(),
        idempotency_key: "accepted-newer".to_string(),
        decided_at: newer_accepted.created_at.clone(),
        note: None,
    });
    changes.save_upsert(&newer_accepted).unwrap();

    let error = h
        .coordinator
        .finalize_success(&team.team_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("仍待人工接受"), "{error}");
    let waiting_team = h.store.get_team_run(&team.team_id).await.unwrap();
    assert_eq!(waiting_team.status, TeamRunStatus::AwaitingHuman);
    let waiting_state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert_eq!(
        waiting_state.goal.status,
        owo_agent_core::goal::GoalStatus::Verifying
    );
    assert!(waiting_state
        .goal
        .error
        .as_deref()
        .is_some_and(|reason| reason.starts_with("delivery_pending:changeset:")));

    let mut accepted_old = change_set;
    accepted_old.status = owo_agent_protocol::ChangeSetStatus::Accepted;
    accepted_old.decision = Some(owo_agent_protocol::ChangeSetDecision {
        action: "accept".to_string(),
        idempotency_key: "accept-older-pending".to_string(),
        decided_at: "2026-10-04T00:00:00Z".to_string(),
        note: None,
    });
    changes.save_upsert(&accepted_old).unwrap();
    let finalized = h.coordinator.finalize_success(&team.team_id).await.unwrap();
    assert_eq!(finalized.status, TeamRunStatus::Succeeded);
    let project_id = team.project_space_id.as_deref().unwrap();
    let space = h.store.get_project_space(project_id).await.unwrap();
    let manifest_ref = space.delivery_manifest_ref.as_deref().unwrap();
    let manifest_hash = manifest_ref.strip_prefix("cas://sha256:").unwrap();
    let manifest_bytes = h.coordinator.cas().get(manifest_hash).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let accepted = manifest["acceptance_receipts"].as_array().unwrap();
    let step_receipt = accepted
        .iter()
        .find(|receipt| receipt["step_id"] == step.id)
        .unwrap();
    let refs = step_receipt["validation_receipts"][0]["evidence_refs"]
        .as_array()
        .unwrap();
    assert!(refs
        .iter()
        .any(|item| item == &format!("changeset://{}", accepted_old.change_set_id)));
    assert!(refs
        .iter()
        .any(|item| item == &format!("changeset://{}", newer_accepted.change_set_id)));
}

#[tokio::test]
async fn finalize_success_rejects_missing_manifest_artifact_before_success() {
    let h = harness();
    let roles = relay_roles("echo");
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &roles);
    let mut completed = false;
    for _ in 0..50 {
        match h
            .coordinator
            .run_phase(&team.team_id, &registry)
            .await
            .unwrap()
        {
            PhaseOutcome::MoreReady => {}
            PhaseOutcome::Done => {
                completed = true;
                break;
            }
            other => panic!("接力步骤未完成：{other:?}"),
        }
    }
    assert!(completed, "接力步骤未在限制轮数内完成");

    let project_id = team.project_space_id.as_deref().unwrap();
    let mut space = h.store.get_project_space(project_id).await.unwrap();
    space.artifacts.push("missing-artifact-ref".to_string());
    h.store.save_project_space(&space).await.unwrap();

    assert!(h.coordinator.finalize_success(&team.team_id).await.is_err());
    let saved_team = h.store.get_team_run(&team.team_id).await.unwrap();
    assert_ne!(saved_team.status, TeamRunStatus::Succeeded);
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert_ne!(
        state.goal.status,
        owo_agent_core::goal::GoalStatus::Succeeded
    );
}

#[tokio::test]
async fn finalize_success_records_workspace_validation_bound_to_file_hash() {
    let h = harness();
    let mut builder = RoleSpec::agent("builder");
    builder.worker = Some("fixed".to_string());
    let roles = vec![builder];
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let workspace = h.dir.join("workspace");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    let file_content = "export const result = 'READY';";
    std::fs::write(workspace.join("src/result.js"), file_content).unwrap();
    h.coordinator
        .bind_verification_workspace(&team.team_id, &workspace)
        .unwrap();

    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let step = state.plan.steps.first().unwrap().clone();
    state.plan.steps[0].verification_plan = Some(owo_agent_core::plan::VerificationPlanV1 {
        plan_id: "verify-builder".to_string(),
        requirements: vec![owo_agent_core::plan::VerificationRequirementV1 {
            requirement_id: "req-workspace".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-file-contains-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: owo_agent_core::plan::VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["src/result.js".to_string()],
            },
            arguments: serde_json::json!({"text":"READY"}),
            required: true,
            resources: owo_agent_core::plan::VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 16,
                exclusive_workspace: false,
                timeout_ms: 3_000,
            },
        }],
    });
    let record = state.records.get_mut(&step.id).unwrap();
    record.status = owo_agent_core::plan::StepStatus::Succeeded;
    record.phase_epoch = Some(0);
    record.attempt_id = Some("attempt-workspace-validation".to_string());
    record.output = Some("candidate artifact".to_string());
    state.persist(&h.dir.join("runs")).unwrap();

    let output = owo_agent_core::workswarm_output::WorkerOutputV1 {
        status: owo_agent_core::workswarm_output::WorkerOutputStatus::Done,
        summary: "提交候选交付".to_string(),
        artifact: Some(owo_agent_core::workswarm_output::WorkerArtifactV1 {
            kind: "document".to_string(),
            format: "markdown".to_string(),
            content: "candidate artifact".to_string(),
        }),
        evidence: Vec::new(),
        open_issues: Vec::new(),
        handoff: None,
        review_result: None,
    };
    h.coordinator
        .register_step_output_contract(
            &team.team_id,
            &step.worker,
            "builder",
            &step.id,
            &output,
            None,
        )
        .await
        .unwrap();

    let finalized = h.coordinator.finalize_success(&team.team_id).await.unwrap();
    assert_eq!(finalized.status, TeamRunStatus::Succeeded);
    let accepted_state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let receipt = accepted_state.records[&step.id]
        .validation_receipts
        .iter()
        .find(|receipt| receipt.requirement_id == "req-workspace")
        .expect("DeliveryGate must persist the host workspace receipt");
    let digest = owo_agent_core::cas_store::CasStore::hash_of(file_content.as_bytes());
    assert_eq!(
        receipt.verdict,
        owo_agent_core::plan::ValidationVerdictV1::Passed
    );
    assert_eq!(
        receipt.subject_sha256.get("workspace-path:src/result.js"),
        Some(&digest)
    );
    assert!(receipt
        .evidence_refs
        .iter()
        .any(|reference| reference == &format!("workspace-path:src/result.js@sha256:{digest}")));
}

#[tokio::test]
async fn finalize_success_rechecks_workspace_after_a_previously_passed_receipt() {
    let h = harness();
    let mut builder = RoleSpec::agent("builder");
    builder.worker = Some("fixed".to_string());
    let roles = vec![builder];
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let workspace = h.dir.join("workspace-final-version");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    let file_path = workspace.join("src/result.js");
    let previously_checked = "export const result = 'READY';";
    std::fs::write(&file_path, previously_checked).unwrap();
    h.coordinator
        .bind_verification_workspace(&team.team_id, &workspace)
        .unwrap();

    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let step = state.plan.steps.first().unwrap().clone();
    state.plan.steps[0].verification_plan = Some(owo_agent_core::plan::VerificationPlanV1 {
        plan_id: "verify-builder-final-version".to_string(),
        requirements: vec![owo_agent_core::plan::VerificationRequirementV1 {
            requirement_id: "req-workspace-final-version".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-file-contains-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: owo_agent_core::plan::VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["src/result.js".to_string()],
            },
            arguments: serde_json::json!({"text":"READY"}),
            required: true,
            resources: owo_agent_core::plan::VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 16,
                exclusive_workspace: false,
                timeout_ms: 3_000,
            },
        }],
    });
    let previous_hash = owo_agent_core::cas_store::CasStore::hash_of(previously_checked.as_bytes());
    let record = state.records.get_mut(&step.id).unwrap();
    record.status = owo_agent_core::plan::StepStatus::Succeeded;
    record.phase_epoch = Some(0);
    record.attempt_id = Some("attempt-final-version".to_string());
    record.output = Some("candidate artifact".to_string());
    record
        .validation_receipts
        .push(owo_agent_core::plan::ValidationReceiptV1 {
            receipt_id: "stale-passed-receipt".to_string(),
            task_id: step.id.clone(),
            attempt_id: "attempt-final-version".to_string(),
            epoch: 0,
            requirement_id: "req-workspace-final-version".to_string(),
            validator_id: "workspace-file-contains-v1".to_string(),
            validator_version: "1".to_string(),
            arguments_sha256: owo_agent_core::cas_store::CasStore::hash_of(
                serde_json::json!({"text":"READY"}).to_string().as_bytes(),
            ),
            input_sha256: owo_agent_core::cas_store::CasStore::hash_of(b"input"),
            environment_id: "test-environment".to_string(),
            changeset_sha256: None,
            detail: None,
            subject_sha256: std::collections::HashMap::from([(
                "workspace-path:src/result.js".to_string(),
                previous_hash,
            )]),
            verdict: owo_agent_core::plan::ValidationVerdictV1::Passed,
            evidence_refs: vec!["workspace-path:src/result.js@old-version".to_string()],
            review_result: None,
            started_at: "2026-10-03T00:00:00Z".to_string(),
            completed_at: "2026-10-03T00:00:01Z".to_string(),
        });
    state.persist(&h.dir.join("runs")).unwrap();

    let output = owo_agent_core::workswarm_output::WorkerOutputV1 {
        status: owo_agent_core::workswarm_output::WorkerOutputStatus::Done,
        summary: "提交候选交付".to_string(),
        artifact: Some(owo_agent_core::workswarm_output::WorkerArtifactV1 {
            kind: "document".to_string(),
            format: "markdown".to_string(),
            content: "candidate artifact".to_string(),
        }),
        evidence: Vec::new(),
        open_issues: Vec::new(),
        handoff: None,
        review_result: None,
    };
    h.coordinator
        .register_step_output_contract(
            &team.team_id,
            &step.worker,
            "builder",
            &step.id,
            &output,
            None,
        )
        .await
        .unwrap();

    let final_content = "export const result = 'TAMPERED';";
    std::fs::write(&file_path, final_content).unwrap();
    let error = h
        .coordinator
        .finalize_success(&team.team_id)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("req-workspace-final-version"),
        "{error}"
    );
    assert_ne!(
        h.store.get_team_run(&team.team_id).await.unwrap().status,
        TeamRunStatus::Succeeded,
        "a prior passing receipt cannot authorize a changed workspace version"
    );

    let final_state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let receipts = &final_state.records[&step.id].validation_receipts;
    assert!(receipts.iter().any(|receipt| {
        receipt.receipt_id == "stale-passed-receipt"
            && receipt.verdict == owo_agent_core::plan::ValidationVerdictV1::Passed
    }));
    let fresh_failure = receipts
        .iter()
        .find(|receipt| {
            receipt.requirement_id == "req-workspace-final-version"
                && receipt.subject_sha256.get("workspace-path:src/result.js")
                    == Some(&owo_agent_core::cas_store::CasStore::hash_of(
                        final_content.as_bytes(),
                    ))
        })
        .expect("DeliveryGate must persist a fresh receipt for the final file hash");
    assert_eq!(
        fresh_failure.verdict,
        owo_agent_core::plan::ValidationVerdictV1::Failed
    );
}

#[tokio::test]
async fn finalize_success_checks_assertion_against_artifact_not_worker_report() {
    let h = harness();
    let mut builder = RoleSpec::agent("builder");
    builder.worker = Some("fixed".to_string());
    builder.verify = Some("contains:ACCEPTED".to_string());
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: vec![builder],
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let step = state.plan.steps.first().unwrap().clone();
    let record = state.records.get_mut(&step.id).unwrap();
    record.status = owo_agent_core::plan::StepStatus::Succeeded;
    record.phase_epoch = Some(0);
    record.attempt_id = Some("attempt-manual-test".to_string());
    record.output = Some("ACCEPTED in worker report".to_string());
    state.persist(&h.dir.join("runs")).unwrap();

    let output = owo_agent_core::workswarm_output::WorkerOutputV1 {
        status: owo_agent_core::workswarm_output::WorkerOutputStatus::Done,
        summary: "ACCEPTED in worker report".to_string(),
        artifact: Some(owo_agent_core::workswarm_output::WorkerArtifactV1 {
            kind: "document".to_string(),
            format: "markdown".to_string(),
            content: "candidate body without the required marker".to_string(),
        }),
        evidence: Vec::new(),
        open_issues: Vec::new(),
        handoff: None,
        review_result: None,
    };
    h.coordinator
        .register_step_output_contract(
            &team.team_id,
            &step.worker,
            "builder",
            &step.id,
            &output,
            None,
        )
        .await
        .unwrap();

    let error = h
        .coordinator
        .finalize_success(&team.team_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("验收要求") && error.to_string().contains("未通过"));
    let saved_team = h.store.get_team_run(&team.team_id).await.unwrap();
    assert_ne!(saved_team.status, TeamRunStatus::Succeeded);
    let saved_state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let receipts = &saved_state.records[&step.id].validation_receipts;
    assert!(receipts.iter().any(|receipt| {
        receipt.verdict == owo_agent_core::plan::ValidationVerdictV1::Failed
            && receipt.changeset_sha256.is_none()
            && receipt.input_sha256.len() == 64
    }));

    h.coordinator
        .rework_step(
            &team.team_id,
            &step.id,
            "update the artifact to satisfy the required marker",
            "validation repair",
        )
        .await
        .unwrap();
    let reworked_state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert!(reworked_state.records[&step.id]
        .validation_receipts
        .iter()
        .any(|receipt| receipt.verdict == owo_agent_core::plan::ValidationVerdictV1::Stale));
}

#[tokio::test]
async fn adaptive_runtime_keeps_review_when_change_tracker_is_missing() {
    let h = harness();
    let template_roles = install_template(&h, builtin_team_templates::CODE_CHANGE_V1);
    let mut roles = template_roles.clone();
    for role in &mut roles {
        role.worker = Some("echo".to_string());
    }
    let req = CreateTeamRequest {
        goal_id: None,
        objective: "重构登录模块的错误处理".to_string(),
        mode: TeamMode::Team,
        template_id: Some(builtin_team_templates::CODE_CHANGE_V1.to_string()),
        roles,
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &template_roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(matches!(outcome, PhaseOutcome::Done));
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert_eq!(
        state.records["s-reviewer"].status,
        owo_agent_core::plan::StepStatus::Succeeded
    );
    assert!(state.records["s-reviewer"].skip_reason.is_none());
}

#[tokio::test]
async fn finalize_success_rejects_non_review_runtime_skip() {
    let h = harness();
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles: vec![RoleSpec {
            worker: Some("fixed".to_string()),
            ..RoleSpec::agent("builder")
        }],
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let record = state.records.values_mut().next().unwrap();
    record.status = owo_agent_core::plan::StepStatus::Succeeded;
    record.skip_reason = Some("forged skip disposition".to_string());
    state.persist(&h.dir.join("runs")).unwrap();

    let error = h
        .coordinator
        .finalize_success(&team.team_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("不能作为已验收交付"));
    let failed_team = h.coordinator.get_team_run(&team.team_id).await.unwrap();
    assert_eq!(failed_team.status, TeamRunStatus::Failed);
    let failed_state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert_eq!(
        failed_state.goal.status,
        owo_agent_core::goal::GoalStatus::Failed
    );
    assert!(failed_state
        .goal
        .error
        .as_deref()
        .unwrap()
        .contains("交付验收未通过"));
}

#[tokio::test]
async fn finalize_success_rejects_succeeded_step_without_output() {
    let h = harness();
    let roles = relay_roles("echo");
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles,
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    for record in state.records.values_mut() {
        record.status = owo_agent_core::plan::StepStatus::Succeeded;
        record.output = None;
    }
    state.persist(&h.dir.join("runs")).unwrap();

    let error = h
        .coordinator
        .finalize_success(&team.team_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("没有提交候选输出"));
}

#[tokio::test]
async fn finalize_success_rejects_latest_artifact_with_open_issues() {
    let h = harness();
    let roles = vec![RoleSpec::agent("builder")];
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.to_string(),
        mode: TeamMode::Team,
        template_id: None,
        roles,
        budget: Value::Null,
        human_policy: None,
        strategy: None,
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let mut state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let step = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-builder")
        .unwrap()
        .clone();
    let record = state.records.get_mut(&step.id).unwrap();
    record.status = owo_agent_core::plan::StepStatus::Succeeded;
    record.output = Some("候选交付".to_string());
    for other in state.records.values_mut() {
        other.status = owo_agent_core::plan::StepStatus::Succeeded;
    }
    state.persist(&h.dir.join("runs")).unwrap();

    let output = owo_agent_core::workswarm_output::WorkerOutputV1 {
        status: owo_agent_core::workswarm_output::WorkerOutputStatus::Done,
        summary: "已提交候选结果".to_string(),
        artifact: Some(owo_agent_core::workswarm_output::WorkerArtifactV1 {
            kind: "document".to_string(),
            format: "markdown".to_string(),
            content: "候选内容".to_string(),
        }),
        evidence: Vec::new(),
        open_issues: vec!["关键行为未验证".to_string()],
        handoff: None,
        review_result: None,
    };
    h.coordinator
        .register_step_output_contract(
            &team.team_id,
            "m-builder",
            "builder",
            &step.id,
            &output,
            None,
        )
        .await
        .unwrap();

    let error = h
        .coordinator
        .finalize_success(&team.team_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("未解决问题"));
}

struct BatchReviewRepairWorker {
    calls: std::sync::Mutex<HashMap<String, usize>>,
}

#[async_trait]
impl Worker for BatchReviewRepairWorker {
    fn name(&self) -> &str {
        "batch-review-repair"
    }

    async fn run(&self, input: &Value) -> Result<String, String> {
        let role = input
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let call = {
            let mut calls = self.calls.lock().unwrap();
            let count = calls.entry(role.to_string()).or_default();
            *count += 1;
            *count
        };
        if role != "quality_gate" {
            if call == 2 {
                let instruction = input
                    .pointer("/rework/instruction")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !instruction.contains(&format!("repair {role}")) {
                    return Err(format!("{role} 未收到自身的修复问题"));
                }
            }
            return Ok(serde_json::json!({"status":"done","summary":"candidate",
                "artifact":{"kind":"document","format":"markdown","content":format!("完整文档 {role} 版本 {call}")},
                "evidence":[],"open_issues":[]}).to_string());
        }
        let context = input
            .get("text")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or_else(|| input.clone());
        let ids = context
            .get("upstream")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|artifact| {
                artifact
                    .get("review_requirements")
                    .and_then(Value::as_array)
            })
            .flatten()
            .filter_map(|requirement| requirement.get("requirement_id").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let findings = if call == 1 {
            vec![
                serde_json::json!({"detail":"repair a boundary","target_task_id":"s-a","severity":"major","evidence_refs":["artifact"]}),
                serde_json::json!({"detail":"repair a cancellation","target_task_id":"s-a","severity":"major","evidence_refs":["artifact"]}),
                serde_json::json!({"detail":"repair b integration","target_task_id":"s-b","severity":"major","evidence_refs":["artifact"]}),
            ]
        } else {
            Vec::new()
        };
        Ok(serde_json::json!({"status":"done","summary":"review",
            "review_result":{"verdict":if call==1 {"changes_requested"} else {"approved"},
                "reviewed_requirement_ids":ids,"findings":findings},
            "evidence":[],"open_issues":[]})
        .to_string())
    }
}

#[tokio::test]
async fn review_batch_repairs_dependent_owners_once_and_closes_every_issue() {
    let h = harness();
    let mut a = RoleSpec::agent("a");
    a.worker = Some("batch-review-repair".into());
    a.verify = Some("non_empty".into());
    let mut b = RoleSpec::agent("b");
    b.worker = a.worker.clone();
    b.verify = a.verify.clone();
    b.depends_on = vec!["a".into()];
    let mut review = RoleSpec::agent("quality_gate");
    review.worker = a.worker.clone();
    review.verify = a.verify.clone();
    review.capabilities = vec!["review".into()];
    review.depends_on = vec!["a".into(), "b".into()];
    let roles = vec![a, b, review];
    let req = CreateTeamRequest {
        goal_id: None,
        objective: OBJECTIVE.into(),
        mode: TeamMode::Team,
        template_id: None,
        roles: roles.clone(),
        budget: Value::Null,
        human_policy: None,
        strategy: Some(owo_agent_core::team_strategy::TeamSelectionMode::ForceTeam),
        model: None,
        parallel: false,
        max_agent_members: None,
        parent_context_snapshot: None,
    };
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let worker = Arc::new(BatchReviewRepairWorker {
        calls: std::sync::Mutex::new(HashMap::new()),
    });
    let registry = build_registry(&h, &team.team_id, worker.clone(), &roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(
        matches!(outcome, PhaseOutcome::Done),
        "batch repair must converge: {outcome:?}"
    );
    let calls = worker.calls.lock().unwrap();
    for role in ["a", "b", "quality_gate"] {
        assert_eq!(calls.get(role), Some(&2), "{role} must run exactly twice");
    }
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert_eq!(state.delivery_issues.len(), 3);
    assert!(state
        .delivery_issues
        .iter()
        .all(|issue| issue.status == owo_agent_core::goal::DeliveryIssueStatusV1::Resolved));
    for step in state
        .plan
        .steps
        .iter()
        .filter(|step| step.worker == "m-a" || step.worker == "m-b")
    {
        assert_eq!(step.input["rework"]["attempt"], 1);
    }
}

#[tokio::test]
async fn dependency_context_bounds_preview_keeps_full_hash_and_rejects_member_spoofing() {
    let h = harness();
    let mut producer = RoleSpec::agent("producer");
    producer.worker = Some("echo".into());
    let mut consumer = RoleSpec::agent("consumer");
    consumer.worker = Some("echo".into());
    consumer.depends_on = vec!["producer".into()];
    let mut req = CreateTeamRequest::new("verify current context", TeamMode::Team);
    req.roles = vec![producer, consumer];
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let producer_step = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-producer")
        .unwrap();
    let consumer_step = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-consumer")
        .unwrap();
    let full = "中文全文".repeat(2000);
    let artifact = h
        .coordinator
        .register_step_output(
            &team.team_id,
            "m-producer",
            "producer",
            &producer_step.id,
            &full,
        )
        .await
        .unwrap();
    let context = h
        .coordinator
        .assemble_context_slice(&team.team_id, "m-consumer", &consumer_step.id)
        .await
        .unwrap();
    let upstream = context["upstream"].as_array().unwrap();
    assert_eq!(upstream.len(), 1);
    let preview = upstream[0]["content"].as_str().unwrap();
    assert!(preview.len() <= owo_agent_core::team_prompt::DEFAULT_PER_ARTIFACT_BYTES);
    assert!(full.starts_with(preview));
    assert_eq!(upstream[0]["truncated"], true);
    assert_eq!(upstream[0]["content_bytes"], full.len());
    assert_eq!(upstream[0]["sha256"], artifact.sha256);
    assert_ne!(
        owo_agent_core::cas_store::CasStore::hash_of(preview.as_bytes()),
        artifact.sha256
    );
    assert!(h
        .coordinator
        .assemble_context_slice(&team.team_id, "m-producer", &consumer_step.id)
        .await
        .is_err());
    assert!(h
        .coordinator
        .read_dependency_artifact(
            &team.team_id,
            "m-producer",
            &consumer_step.id,
            &artifact.artifact_id,
            100
        )
        .await
        .is_err());
    std::fs::write(h.dir.join("cas").join(&artifact.sha256), b"tampered").unwrap();
    assert!(h
        .coordinator
        .assemble_context_slice(&team.team_id, "m-consumer", &consumer_step.id)
        .await
        .is_err());
}

#[tokio::test]
async fn artifact_pages_reach_tail_and_pin_current_task_attempt_and_hash() {
    use owo_agent_core::workswarm::DependencyArtifactRead;
    let h = harness();
    let mut producer = RoleSpec::agent("producer");
    producer.worker = Some("echo".into());
    let mut consumer = RoleSpec::agent("consumer");
    consumer.worker = Some("echo".into());
    consumer.depends_on = vec!["producer".into()];
    let mut req = CreateTeamRequest::new("read full large dependency", TeamMode::Team);
    req.roles = vec![producer, consumer];
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    let producer_step = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-producer")
        .unwrap();
    let consumer_step = state
        .plan
        .steps
        .iter()
        .find(|step| step.worker == "m-consumer")
        .unwrap();
    let full = format!("{}尾部验收要求", "正文😀".repeat(12000));
    let artifact = h
        .coordinator
        .register_step_output(
            &team.team_id,
            "m-producer",
            "producer",
            &producer_step.id,
            &full,
        )
        .await
        .unwrap();
    let mut read = DependencyArtifactRead {
        max_bytes: 32 * 1024,
        ..Default::default()
    };
    let mut assembled = String::new();
    loop {
        let page = h
            .coordinator
            .read_dependency_artifact_page(
                &team.team_id,
                "m-consumer",
                &consumer_step.id,
                &artifact.artifact_id,
                &read,
            )
            .await
            .unwrap();
        assembled.push_str(page["content"].as_str().unwrap());
        assert_eq!(page["sha256"], artifact.sha256);
        if page["eof"] == true {
            break;
        }
        let next = page["next_offset_bytes"].as_u64().unwrap();
        assert!(next > read.offset_bytes);
        read.offset_bytes = next;
        read.expected_sha256 = Some(artifact.sha256.clone());
    }
    assert_eq!(assembled, full);
    assert!(assembled.ends_with("尾部验收要求"));
    let wrong = DependencyArtifactRead {
        offset_bytes: 3,
        max_bytes: 100,
        expected_sha256: Some("a".repeat(64)),
    };
    assert!(h
        .coordinator
        .read_dependency_artifact_page(
            &team.team_id,
            "m-consumer",
            &consumer_step.id,
            &artifact.artifact_id,
            &wrong
        )
        .await
        .is_err());
    let unpinned = DependencyArtifactRead {
        offset_bytes: 3,
        ..Default::default()
    };
    assert!(h
        .coordinator
        .read_dependency_artifact_page(
            &team.team_id,
            "m-consumer",
            &consumer_step.id,
            &artifact.artifact_id,
            &unpinned
        )
        .await
        .is_err());
    let mut restarted = h.coordinator.load_run_state(&team.team_id).unwrap();
    restarted
        .records
        .get_mut(&producer_step.id)
        .unwrap()
        .attempt_id = Some("different-attempt".into());
    restarted.persist(&h.dir.join("runs")).unwrap();
    assert!(h
        .coordinator
        .read_dependency_artifact_page(
            &team.team_id,
            "m-consumer",
            &consumer_step.id,
            &artifact.artifact_id,
            &read
        )
        .await
        .is_err());
}

struct HtmlCandidateWorker;
#[async_trait]
impl Worker for HtmlCandidateWorker {
    fn name(&self) -> &str {
        "html-candidate"
    }
    async fn run(&self, input: &Value) -> Result<String, String> {
        if input.get("role").and_then(Value::as_str) == Some("reviewer") {
            return EchoWorker.run(input).await;
        }
        Ok(serde_json::json!({"status":"done","summary":"HTML candidate","artifact":{"kind":"document","format":"text","content":"<!doctype html><html><body><button>预约试玩</button></body></html>"},"evidence":[],"open_issues":[]}).to_string())
    }
}
async fn drive_candidate_phase(
    h: &Harness,
    team_id: &str,
    registry: &WorkerRegistry,
) -> PhaseOutcome {
    for _ in 0..50 {
        let outcome = h.coordinator.run_phase(team_id, registry).await.unwrap();
        if !matches!(outcome, PhaseOutcome::MoreReady) {
            return outcome;
        }
    }
    panic!("candidate phase did not converge");
}

#[tokio::test]
async fn pure_html_candidate_dispatches_bounded_host_repair_and_rechecks_review() {
    let h = harness();
    let mut producer = RoleSpec::agent("implementer");
    producer.verify = Some("non_empty".into());
    let mut reviewer = RoleSpec::agent("reviewer");
    reviewer.depends_on = vec!["implementer".into()];
    reviewer.verify = Some("non_empty".into());
    let roles = vec![producer, reviewer];
    let mut req = CreateTeamRequest::new("create a playable page", TeamMode::Team);
    req.roles = roles.clone();
    req.budget = serde_json::json!({"max_retries_per_step":1,"max_total_retries":1});
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let registry = build_registry(&h, &team.team_id, Arc::new(HtmlCandidateWorker), &roles);
    let initial = drive_candidate_phase(&h, &team.team_id, &registry).await;
    assert!(
        matches!(initial, PhaseOutcome::Done),
        "initial={initial:?}, state={:?}",
        h.coordinator
            .load_run_state(&team.team_id)
            .map(|state| (state.goal.error, state.events))
    );
    let error = h
        .coordinator
        .finalize_success(&team.team_id)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        owo_agent_core::workswarm::WorkSwarmError::DeliveryRepair { .. }
    ));
    let state = h.coordinator.load_run_state(&team.team_id).unwrap();
    assert_eq!(
        state.records["s-implementer"].status,
        owo_agent_core::plan::StepStatus::Pending
    );
    assert_eq!(
        state.records["s-reviewer"].status,
        owo_agent_core::plan::StepStatus::Pending
    );
    assert!(matches!(
        drive_candidate_phase(&h, &team.team_id, &registry).await,
        PhaseOutcome::Done
    ));
    assert!(h.coordinator.finalize_success(&team.team_id).await.is_err());
    assert_eq!(
        h.coordinator
            .get_team_run(&team.team_id)
            .await
            .unwrap()
            .status,
        TeamRunStatus::Failed
    );
}

#[tokio::test]
async fn cancelled_candidate_cannot_publish_a_success_manifest() {
    let h = harness();
    let roles = relay_roles("echo");
    let mut req = CreateTeamRequest::new(OBJECTIVE, TeamMode::Team);
    req.roles = roles.clone();
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let registry = build_registry(&h, &team.team_id, Arc::new(EchoWorker), &roles);
    assert!(matches!(
        drive_candidate_phase(&h, &team.team_id, &registry).await,
        PhaseOutcome::Done
    ));
    h.coordinator
        .apply_steer(
            &team.team_id,
            &owo_agent_core::workswarm::SteerCommand::Cancel,
        )
        .await
        .unwrap();
    assert!(h.coordinator.finalize_success(&team.team_id).await.is_err());
    assert_eq!(
        h.coordinator
            .get_team_run(&team.team_id)
            .await
            .unwrap()
            .status,
        TeamRunStatus::Cancelled
    );
    let space = h
        .store
        .get_project_space(team.project_space_id.as_deref().unwrap())
        .await
        .unwrap();
    assert!(space.delivery_manifest_ref.is_none());
}

struct CorrectingCommanderWorker {
    lead_calls: std::sync::atomic::AtomicUsize,
    writer_calls: std::sync::atomic::AtomicUsize,
    remains_invalid: bool,
}
#[async_trait]
impl Worker for CorrectingCommanderWorker {
    fn name(&self) -> &str {
        "agent"
    }
    async fn run(&self, input: &Value) -> Result<String, String> {
        match input["role"].as_str().unwrap_or("") {
            "lead" => {
                let call = self
                    .lead_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if call > 0 {
                    assert!(input["task_graph_repair"]["reason"].as_str().is_some());
                    assert!(input["team_prompt_context"]["handoff_contract"]
                        .as_str()
                        .unwrap()
                        .contains("禁止扩大权限"));
                }
                let verification = if call == 0 || self.remains_invalid {
                    serde_json::json!({"plan_id":"bad","requirements":[{"requirement_id":"r","validator_id":"non_empty","validator_version":"1","scope":{"kind":"workspace_paths","relative_paths":[]},"arguments":{},"required":true}]})
                } else {
                    serde_json::json!("non_empty")
                };
                let plan = serde_json::json!({"version":1,"brief":{"goal":"整理文案"},"tasks":[{
                    "task_id":"t1","worker":"w1","task":"整理文案","acceptance":"整理文案",
                    "depends_on":[],"requirement_quotes":["整理文案"],"write_paths":[],"read_refs":[],"contract_refs":[],"required_capabilities":[],"estimated_effort":1,"verification":verification,"risk":"low","priority":50
                }]});
                Ok(serde_json::json!({"status":"done","summary":"指挥计划","artifact":{"kind":"plan","format":"json","content":plan.to_string()},"evidence":[],"open_issues":[]}).to_string())
            }
            "reviewer" => {
                let context = input["team_prompt_context"].clone();
                EchoWorker.run(&serde_json::json!({"role":"reviewer","capabilities":["review"],"text":context.to_string()})).await
            }
            _ => {
                self.writer_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(serde_json::json!({"status":"done","summary":"文案完成","artifact":{"kind":"document","format":"markdown","content":"整理后的文案"},"evidence":[],"open_issues":[]}).to_string())
            }
        }
    }
}
fn commander_test_roles() -> Vec<RoleSpec> {
    let lead = RoleSpec::agent("lead");
    let mut writer = RoleSpec::agent("w1");
    writer.depends_on = vec!["lead".into()];
    let mut reviewer = RoleSpec::agent("reviewer");
    reviewer.depends_on = vec!["w1".into()];
    reviewer.capabilities = vec!["review".into()];
    let mut roles = vec![lead, writer, reviewer];
    for role in &mut roles {
        role.verify = Some("non_empty".into());
    }
    roles
}
#[tokio::test]
async fn commander_corrects_plan_before_any_executor_is_dispatched() {
    let h = harness();
    let roles = commander_test_roles();
    let mut req = CreateTeamRequest::new("整理文案", TeamMode::Team);
    req.roles = roles.clone();
    req.parallel = true;
    req.budget = serde_json::json!({"max_retries_per_step":0,"max_total_retries":0});
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let worker = Arc::new(CorrectingCommanderWorker {
        lead_calls: 0.into(),
        writer_calls: 0.into(),
        remains_invalid: false,
    });
    let registry = build_registry(&h, &team.team_id, worker.clone(), &roles);
    let outcome = drive_next(&h, &team.team_id, &registry).await;
    assert!(matches!(outcome, PhaseOutcome::Done));
    assert_eq!(
        worker.lead_calls.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(
        worker
            .writer_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let space = h
        .store
        .get_project_space(team.project_space_id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(
        space
            .artifacts
            .iter()
            .filter(|id| id.contains(":lead:"))
            .count(),
        1,
        "invalid plan must not be published"
    );
}
#[tokio::test]
async fn commander_correction_is_bounded_and_never_dispatches_invalid_tasks() {
    let h = harness();
    let roles = commander_test_roles();
    let mut req = CreateTeamRequest::new("整理文案", TeamMode::Team);
    req.roles = roles.clone();
    req.parallel = true;
    req.budget = serde_json::json!({"max_retries_per_step":0,"max_total_retries":0});
    let team = h.coordinator.create_team_run(&req).await.unwrap();
    let worker = Arc::new(CorrectingCommanderWorker {
        lead_calls: 0.into(),
        writer_calls: 0.into(),
        remains_invalid: true,
    });
    let registry = build_registry(&h, &team.team_id, worker.clone(), &roles);
    assert!(matches!(
        drive_next(&h, &team.team_id, &registry).await,
        PhaseOutcome::Failed
    ));
    assert_eq!(
        worker.lead_calls.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(
        worker
            .writer_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        h.coordinator
            .get_team_run(&team.team_id)
            .await
            .unwrap()
            .status,
        TeamRunStatus::Failed
    );
}
