use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct ContextAttemptProjection {
    records: BTreeMap<String, ContextAttemptRecord>,
}

#[derive(Deserialize)]
struct ContextAttemptRecord {
    attempt_id: Option<String>,
}

fn parse_context_attempts(
    raw: &str,
) -> Result<BTreeMap<String, Option<String>>, serde_json::Error> {
    let projection: ContextAttemptProjection = serde_json::from_str(raw)?;
    Ok(projection
        .records
        .into_iter()
        .map(|(step_id, record)| (step_id, record.attempt_id))
        .collect())
}

impl TeamCoordinator {
    /// Read only the mutable attempt identities needed by Worker context assembly.
    /// Serde skips the immutable plan, event log, validation receipts and other history.
    pub(super) fn load_context_attempts(
        &self,
        team_id: &str,
    ) -> WorkSwarmResult<BTreeMap<String, Option<String>>> {
        let path = self.run_dir.join(format!("{team_id}.json"));
        let raw = std::fs::read_to_string(&path).map_err(|error| {
            WorkSwarmError::Io(format!("读取 {} 失败：{error}", path.display()))
        })?;
        parse_context_attempts(&raw).map_err(|error| {
            WorkSwarmError::CorruptState(format!(
                "运行状态文件损坏（原文件已保留，未被覆盖）：{}：{error}",
                path.display()
            ))
        })
    }
}

impl TeamCoordinator {
    pub(crate) async fn load_bundle(
        &self,
        team_id: &str,
    ) -> WorkSwarmResult<(TeamRun, ProjectSpace, GoalRunState)> {
        let team = self
            .store
            .get_team_run(team_id)
            .await
            .map_err(|e| match e {
                ProjectSpaceStoreError::NotFound(_) => {
                    WorkSwarmError::NotFound(format!("团队 {team_id} 不存在"))
                }
                other => WorkSwarmError::Store(other),
            })?;
        let project_space_id = team
            .project_space_id
            .clone()
            .ok_or_else(|| WorkSwarmError::Run(format!("团队 {team_id} 缺少 project_space_id")))?;
        let space = self
            .store
            .get_project_space(&project_space_id)
            .await
            .map_err(|e| match e {
                ProjectSpaceStoreError::NotFound(_) => {
                    WorkSwarmError::NotFound(format!("项目空间 {project_space_id} 不存在"))
                }
                other => WorkSwarmError::Store(other),
            })?;
        let state = self.load_goal_state(team_id)?;
        Ok((team, space, state))
    }

    /// 读取运行状态（带损坏分类）：
    /// - 文件缺失 → NotFound；
    /// - 解析失败 → CorruptState（原文件保留；明确失败而非静默重建）；
    /// - 其余 IO 错误 → Io。
    pub(crate) fn load_goal_state(&self, team_id: &str) -> WorkSwarmResult<GoalRunState> {
        let path = self.run_dir.join(format!("{team_id}.json"));
        if !path.exists() {
            return Err(WorkSwarmError::NotFound(format!(
                "运行状态 {team_id} 不存在（{}）",
                path.display()
            )));
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| WorkSwarmError::Io(format!("读取 {} 失败：{e}", path.display())))?;
        serde_json::from_str(&raw).map_err(|e| {
            WorkSwarmError::CorruptState(format!(
                "运行状态文件损坏（原文件已保留，未被覆盖）：{}：{e}",
                path.display()
            ))
        })
    }

    pub(crate) fn load_state(&self, team_id: &str) -> WorkSwarmResult<GoalRunState> {
        self.load_goal_state(team_id)
    }

    /// 读取运行状态（HTTP 任务视图 / 诊断用）。
    pub fn load_run_state(&self, team_id: &str) -> WorkSwarmResult<GoalRunState> {
        self.load_state(team_id)
    }

    /// 读取运行元数据（HTTP 层构建 worker 注册表用）。
    pub fn load_run_meta(&self, team_id: &str) -> WorkSwarmResult<RunMeta> {
        RunMeta::load(&self.run_dir, team_id)
    }

    /// 列出全部团队运行（HTTP 列表视图）。
    pub async fn list_team_runs(&self) -> WorkSwarmResult<Vec<TeamRun>> {
        self.store
            .list_team_runs()
            .await
            .map_err(WorkSwarmError::Store)
    }

    /// 读取 TeamRun（不存在 → NotFound）。
    pub async fn get_team_run(&self, team_id: &str) -> WorkSwarmResult<TeamRun> {
        self.store.get_team_run(team_id).await.map_err(|e| match e {
            ProjectSpaceStoreError::NotFound(_) => {
                WorkSwarmError::NotFound(format!("团队 {team_id} 不存在"))
            }
            other => WorkSwarmError::Store(other),
        })
    }

    /// 读取 Project Space（不存在 → NotFound）。
    pub async fn get_project_space(
        &self,
        project_id: &str,
    ) -> WorkSwarmResult<owo_agent_protocol::ProjectSpace> {
        self.store
            .get_project_space(project_id)
            .await
            .map_err(|e| match e {
                ProjectSpaceStoreError::NotFound(_) => {
                    WorkSwarmError::NotFound(format!("项目空间 {project_id} 不存在"))
                }
                other => WorkSwarmError::Store(other),
            })
    }

    /// 列出项目空间的版本化产物（按创建时间）。
    pub async fn list_artifacts(
        &self,
        space: &owo_agent_protocol::ProjectSpace,
    ) -> WorkSwarmResult<Vec<Artifact>> {
        self.store
            .list_artifacts_by_project(&space.project_id)
            .await
            .map_err(WorkSwarmError::Store)
    }

    /// Bounded canonical history for interactive observers, without loading goal history.
    pub async fn handoffs_before(
        &self,
        team_id: &str,
        before: Option<&str>,
        limit: usize,
    ) -> WorkSwarmResult<owo_agent_protocol::HandoffPage> {
        let team = self.get_team_run(team_id).await?;
        let project = team
            .project_space_id
            .as_deref()
            .ok_or_else(|| WorkSwarmError::NotFound("team has no project space".into()))?;
        self.store
            .handoffs_before(project, before, limit)
            .map_err(|e| match e {
                ProjectSpaceStoreError::InvalidState(message) => {
                    WorkSwarmError::Validation(message)
                }
                other => WorkSwarmError::Store(other),
            })
    }

    /// 按团队列出全部结构化 Handoff（评测适配器/诊断用）。
    pub async fn list_handoffs(&self, team_id: &str) -> WorkSwarmResult<Vec<HandoffRecord>> {
        let (_team, space, _state) = self.load_bundle(team_id).await?;
        self.store
            .list_handoffs_by_project(&space.project_id)
            .await
            .map_err(WorkSwarmError::Store)
    }

    /// `cas://sha256:{hash}` → 文本内容（CAS 命中时返回 Some）。
    ///
    /// 评测适配器据此把**版本化 Artifact 的内容**复制进评测沙盒，
    /// 保证最终结果来自 ProjectSpace/CAS 而非某次模型回复的内存值。
    pub fn resolve_content_text(&self, content_ref: &str) -> Option<String> {
        let hash = content_ref.strip_prefix("cas://sha256:")?;
        self.cas.get_text(hash)
    }

    /// 审计日志（S0 可见性：team.* 关键动作尾迹）。
    pub fn audit_log(&self) -> Option<Arc<Mutex<crate::audit::AuditLog>>> {
        self.audit.clone()
    }

    pub(crate) fn persist_state(&self, state: &GoalRunState) -> WorkSwarmResult<()> {
        let epoch = Self::durable_execution_epoch(state).max(self.phase_epoch(&state.run_id));
        if epoch == state.execution_epoch {
            return self.persist_observed_state(state);
        }
        let mut checkpoint = state.clone();
        checkpoint.execution_epoch = epoch;
        self.persist_observed_state(&checkpoint)
    }

    pub(crate) fn step_deps_satisfied(state: &GoalRunState, step: &StepSpec) -> bool {
        step.depends_on.iter().all(|d| {
            state
                .records
                .get(d)
                .map(|r| r.status == StepStatus::Succeeded)
                .unwrap_or(false)
        })
    }

    pub(crate) fn ready_steps(state: &GoalRunState) -> Vec<StepSpec> {
        state
            .plan
            .steps
            .iter()
            .filter(|s| {
                let rec = &state.records[&s.id];
                rec.status.can_resume() && Self::step_deps_satisfied(state, s)
            })
            .cloned()
            .collect()
    }

    pub(crate) fn all_succeeded(state: &GoalRunState) -> bool {
        state
            .plan
            .steps
            .iter()
            .all(|s| state.records[&s.id].status == StepStatus::Succeeded)
    }

    pub(crate) fn role_spec_of_member<'a>(
        meta: &'a RunMeta,
        member_id: &'a str,
    ) -> WorkSwarmResult<&'a RoleSpec> {
        meta.roles
            .iter()
            .find(|r| format!("m-{}", r.role) == member_id)
            .ok_or_else(|| {
                WorkSwarmError::Run(format!("成员 {member_id} 无对应角色规格（元数据不一致）"))
            })
    }

    // -- 组队（§6.1：模板优先 + 动态组队 ≤5 Agent） --
}

// The journal remains in the existing ProjectSpace store; this exposes no write authority.
impl TeamCoordinator {
    pub fn run_events_before(
        &self,
        team_id: &str,
        before: u64,
        limit: usize,
    ) -> WorkSwarmResult<owo_agent_protocol::RunEventHistoryPage> {
        self.store
            .run_events_before(team_id, before, limit)
            .map_err(|error| match error {
                ProjectSpaceStoreError::NotFound(message) => WorkSwarmError::NotFound(message),
                ProjectSpaceStoreError::InvalidState(message) => {
                    WorkSwarmError::Validation(message)
                }
                other => WorkSwarmError::Store(other),
            })
    }

    pub fn run_events_after(
        &self,
        team_id: &str,
        after: u64,
        limit: usize,
    ) -> WorkSwarmResult<owo_agent_protocol::RunEventPage> {
        self.store
            .run_events_after(team_id, after, limit)
            .map_err(|error| match error {
                ProjectSpaceStoreError::NotFound(message) => WorkSwarmError::NotFound(message),
                ProjectSpaceStoreError::InvalidState(message) => {
                    WorkSwarmError::Validation(message)
                }
                other => WorkSwarmError::Store(other),
            })
    }
}

impl TeamCoordinator {
    /// Project one durable canonical message reference. Safe to replay before acknowledging its outbox row.
    pub fn publish_team_turn_delivery(
        &self,
        delivery: &crate::session::TeamTurnDelivery,
    ) -> WorkSwarmResult<owo_agent_protocol::RunEvent> {
        use crate::project_space_store::NewRunEvent;
        let run = &delivery.binding.run_id;
        let key = |seq| format!("turn:{}:{seq}", delivery.session_id);
        let mut event = NewRunEvent::new(
            run,
            key(delivery.session_seq),
            &delivery.event_type,
            serde_json::json!({"record_ref":{"session_id":delivery.session_id,
                "seq":delivery.session_seq,"turn_id":delivery.turn_id},
                "source_timestamp":delivery.created_at,
                "artifact_refs_complete":delivery.artifact_refs_complete}),
        );
        event.node_id = delivery.binding.node_id.clone();
        event.task_id = Some(delivery.binding.task_id.clone());
        event.attempt = delivery.binding.attempt.clone();
        event.tool_call_id = delivery.tool_call_id.clone();
        event.artifact_refs = delivery.artifact_refs.clone();
        if let Some(seq) = delivery.cause_seq {
            let cause = self
                .store
                .run_event_by_source(run, &key(seq))?
                .ok_or_else(|| {
                    WorkSwarmError::Run("tool start reference has not been delivered".into())
                })?;
            event.causation_id = Some(cause.event_id);
        }
        Ok(self.store.append_run_event(&event)?)
    }
}

#[cfg(test)]
mod context_attempt_projection_tests {
    use super::parse_context_attempts;

    #[test]
    fn attempt_projection_skips_immutable_plan_events_and_receipt_history() {
        let raw = serde_json::json!({
            "goal": {"objective": "large goal"},
            "plan": {"steps": [{"id": "large", "input": {"prompt": "x".repeat(64 * 1024)}}]},
            "events": ["old event".repeat(1024)],
            "validation_receipts": [{"detail": "old receipt".repeat(1024)}],
            "records": {
                "step-a": {
                    "attempt_id": "attempt-a",
                    "output": "old output".repeat(1024),
                    "validation_receipts": [{"detail": "step receipt".repeat(1024)}]
                },
                "step-b": {"attempt_id": null, "output": "unused"}
            }
        })
        .to_string();

        let attempts = parse_context_attempts(&raw).unwrap();
        assert_eq!(attempts.get("step-a"), Some(&Some("attempt-a".to_string())));
        assert_eq!(attempts.get("step-b"), Some(&None));
    }

    #[test]
    fn malformed_attempt_projection_fails_closed() {
        assert!(parse_context_attempts(r#"{"records":{"step-a":{"attempt_id":7}}}"#).is_err());
    }
}
