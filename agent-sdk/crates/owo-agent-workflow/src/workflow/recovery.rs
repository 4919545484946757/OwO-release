//! Minimal durable recovery: opaque identities and captured predicates, never
//! complete definitions, tool arguments or sampled tool output.
use super::*;

pub const WORKFLOW_RECOVERY_REQUIRED: &str = "WORKFLOW_RECOVERY_REQUIRED";
pub fn recovery_error(detail: impl std::fmt::Display) -> String {
    format!("{WORKFLOW_RECOVERY_REQUIRED}: {detail}；请先核对已有结果，取消旧任务后只为剩余步骤新建任务")
}

/// Stable across JSON map insertion order; no data is written by this helper.
pub fn recovery_fingerprint(value: &serde_json::Value) -> String {
    fn canonical(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
                serde_json::Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.clone(), canonical(value)))
                        .collect(),
                )
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(canonical).collect())
            }
            other => other.clone(),
        }
    }
    format!(
        "sha256:{}",
        owo_agent_kernel::CasStore::hash_of(canonical(value).to_string().as_bytes())
    )
}

pub trait WorkflowRecoveryStore: Send + Sync {
    fn load(&self) -> Result<Option<WorkflowRecoveryState>, String>;
    fn save(&self, state: &WorkflowRecoveryState) -> Result<(), String>;
    fn claim_effect(&self, sequence: usize, step: &serde_json::Value) -> Result<(), String>;
    fn check_tool_budget(&self, _reserved: usize) -> Result<(), String> {
        Ok(())
    }
    fn recovered(&self, _state: &WorkflowRecoveryState) -> Result<(), String> {
        Ok(())
    }
}

/// Values stay in the current executor only. A resumed executor receives the
/// predicates needed by this immutable program, not the original sampled data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CapturedValue {
    value_ref: String,
    predicates: BTreeMap<String, Option<bool>>,
}
pub(super) type CapturedContext = BTreeMap<String, CapturedValue>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRecoveryState {
    version: u32,
    definition_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision_header_ref: Option<String>,
    entries: Vec<CompletedInvocation>,
    pending: Option<PendingInvocation>,
    terminal: Option<String>,
    reserved_tool_calls: usize,
}
impl WorkflowRecoveryState {
    pub fn completed_invocations(&self) -> usize {
        self.entries.len()
    }
    pub fn completed_effects(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.effect_started)
            .count()
    }
    pub fn has_pending_effect(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.effect_started)
    }
    pub fn reserved_tool_calls(&self) -> usize {
        self.reserved_tool_calls
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingInvocation {
    step_ref: String,
    step_kind: String,
    ctx_before: CapturedContext,
    effect_started: bool,
    tool_reserved: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    tool_retried: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompletedInvocation {
    step_ref: String,
    ctx_before: CapturedContext,
    ctx_after: CapturedContext,
    steps_before: u32,
    records: Vec<StepRecord>,
    checkpoints: Vec<CheckpointRef>,
    effect_started: bool,
    tool_call: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    tool_retried: bool,
}
pub(super) struct RecoverySession {
    store: Arc<dyn WorkflowRecoveryStore>,
    state: WorkflowRecoveryState,
    cursor: usize,
    expressions: BTreeMap<String, Vec<String>>,
}
fn expression_input(expr: &str) -> Option<&str> {
    let trimmed = expr.trim();
    if let Some(inner) = trimmed
        .strip_prefix("exists(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return Some(inner.trim());
    }
    for op in ["==", "!=", ">=", "<=", ">", "<"] {
        if let Some((left, _)) = trimmed.split_once(op) {
            return Some(left.trim());
        }
    }
    None
}
fn expressions_for(
    flow: &WorkflowDefinition,
    flows: &HashMap<String, WorkflowDefinition>,
) -> BTreeMap<String, Vec<String>> {
    fn add(expr: &str, out: &mut BTreeMap<String, Vec<String>>) {
        if let Some(key) = expression_input(expr) {
            let values = out.entry(key.to_string()).or_default();
            if !values.iter().any(|value| value == expr) {
                values.push(expr.to_string());
            }
        }
    }
    fn walk(steps: &[WorkflowStep], out: &mut BTreeMap<String, Vec<String>>) {
        for step in steps {
            match step {
                WorkflowStep::Assert { expr, .. } => add(expr, out),
                WorkflowStep::Cond {
                    expr,
                    then,
                    otherwise,
                    ..
                } => {
                    add(expr, out);
                    walk(then, out);
                    walk(otherwise, out);
                }
                WorkflowStep::Loop { cond, body, .. } => {
                    if let Some(expr) = cond {
                        add(expr, out);
                    }
                    walk(body, out);
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    for definition in std::iter::once(flow).chain(flows.values()) {
        for expr in &definition.preconditions {
            add(expr, &mut out);
        }
        walk(&definition.steps, &mut out);
    }
    out
}
fn step_kind(step: &WorkflowStep) -> &'static str {
    match step {
        WorkflowStep::Sense { .. } => "sense",
        WorkflowStep::Locate { .. } => "locate",
        WorkflowStep::Act { .. } => "act",
        WorkflowStep::Assert { .. } => "assert",
        WorkflowStep::InvokeSkill { .. } => "invoke_skill",
        WorkflowStep::InvokeMcp { .. } => "invoke_mcp",
        WorkflowStep::HumanApprove { .. } => "human_approve",
        WorkflowStep::Notify { .. } => "notify",
        WorkflowStep::Subflow { .. } => "subflow",
        WorkflowStep::Loop { .. } => "loop",
        WorkflowStep::Cond { .. } => "cond",
        WorkflowStep::RollbackPoint { .. } => "rollback_point",
    }
}
impl WorkflowEngine {
    pub fn with_recovery(self, store: Arc<dyn WorkflowRecoveryStore>) -> Result<Self, String> {
        self.with_recovery_revision(store, None)
    }

    /// The caller supplies the exact definition approved for this resume request.
    /// Changes to completed steps, control flow or permissions require reconciliation.
    pub fn with_recovery_revision(
        mut self,
        store: Arc<dyn WorkflowRecoveryStore>,
        approved: Option<&serde_json::Value>,
    ) -> Result<Self, String> {
        let definition = serde_json::to_value(&self.flow).map_err(recovery_error)?;
        if approved.is_some_and(|value| value != &definition) {
            return Err(recovery_error(
                "approved definition does not match current workflow",
            ));
        }
        let definition_ref = recovery_fingerprint(
            &serde_json::json!({"flow":definition,"subflows":self.flows,"work_root":self.work_root,
                "expression_semantics":EXPRESSION_SEMANTICS_VERSION}),
        );
        let expressions = expressions_for(&self.flow, &self.flows);
        let revision_header_ref = if self.flows.is_empty()
            && self.flow.steps.iter().all(|step| {
                !matches!(
                    step,
                    WorkflowStep::Loop { .. }
                        | WorkflowStep::Cond { .. }
                        | WorkflowStep::Subflow { .. }
                )
            }) {
            let mut header = definition.clone();
            header
                .as_object_mut()
                .expect("workflow definition is an object")
                .remove("steps");
            Some(recovery_fingerprint(&serde_json::json!({
                "header": header, "work_root": self.work_root, "expressions": expressions,
                "expression_semantics": EXPRESSION_SEMANTICS_VERSION
            })))
        } else {
            None
        };
        let state = match store.load().map_err(recovery_error)? {
            Some(mut state) => {
                if state.version != 2 {
                    return Err(recovery_error("工作流计划、恢复格式或判断规则已经变化"));
                }
                if state.terminal.as_deref() == Some("failed") {
                    return Err(recovery_error("previous execution failed or rolled back"));
                }
                if state.has_pending_effect() {
                    return Err(recovery_error(
                        "effect started without a durable completion",
                    ));
                }
                if state.definition_ref != definition_ref {
                    if approved.is_none()
                        || state.terminal.is_some()
                        || state.pending.is_some()
                        || revision_header_ref.is_none()
                        || state.revision_header_ref != revision_header_ref
                        || state.entries.len() >= self.flow.steps.len()
                    {
                        return Err(recovery_error(
                            "workflow revision is not compatible with its checkpoint",
                        ));
                    }
                    for (entry, step) in state.entries.iter().zip(&self.flow.steps) {
                        let step = serde_json::to_value(step).map_err(recovery_error)?;
                        if entry.step_ref != recovery_fingerprint(&step) {
                            return Err(recovery_error(
                                "workflow revision changes a completed step",
                            ));
                        }
                    }
                    state.definition_ref = definition_ref;
                    store.save(&state).map_err(recovery_error)?;
                }
                store.recovered(&state).map_err(recovery_error)?;
                state
            }
            None => {
                if approved.is_some() {
                    return Err(recovery_error("workflow revision requires a checkpoint"));
                }
                let state = WorkflowRecoveryState {
                    version: 2,
                    definition_ref,
                    revision_header_ref,
                    entries: Vec::new(),
                    pending: None,
                    terminal: None,
                    reserved_tool_calls: 0,
                };
                store.save(&state).map_err(recovery_error)?;
                state
            }
        };
        self.recovery = Some(RecoverySession {
            store,
            state,
            cursor: 0,
            expressions,
        });
        Ok(self)
    }
    fn captured_context(&self) -> Result<CapturedContext, String> {
        let session = self
            .recovery
            .as_ref()
            .ok_or_else(|| recovery_error("recovery session missing"))?;
        let mut captured = BTreeMap::new();
        for (key, value) in &self.ctx {
            if let Some(previous) = self.restored_context.get(key) {
                captured.insert(key.clone(), previous.clone());
                continue;
            }
            let mut predicates = BTreeMap::new();
            if let Some(expressions) = session.expressions.get(key) {
                let context = BTreeMap::from([(key.clone(), value.clone())]);
                for expr in expressions {
                    predicates.insert(
                        recovery_fingerprint(&serde_json::json!(expr)),
                        eval_expr(expr, &context).ok(),
                    );
                }
            }
            captured.insert(
                key.clone(),
                CapturedValue {
                    value_ref: recovery_fingerprint(value),
                    predicates,
                },
            );
        }
        Ok(captured)
    }
    fn restore_context(&mut self, captured: CapturedContext) {
        self.ctx = captured
            .keys()
            .map(|key| (key.clone(), serde_json::Value::Null))
            .collect();
        self.restored_context = captured;
    }
    pub(super) fn set_context_value(&mut self, key: String, value: serde_json::Value) {
        self.restored_context.remove(&key);
        self.ctx.insert(key, value);
    }
    pub(super) fn evaluate_expression(&self, expr: &str) -> Result<bool, String> {
        if let Some(key) = expression_input(expr) {
            if let Some(captured) = self.restored_context.get(key) {
                return match captured
                    .predicates
                    .get(&recovery_fingerprint(&serde_json::json!(expr)))
                {
                    Some(Some(value)) => Ok(*value),
                    Some(None) => {
                        Err("WORKFLOW_EXPRESSION_ERROR: expression is not valid".to_string())
                    }
                    None => Err(recovery_error(
                        "restored predicate unavailable for this expression",
                    )),
                };
            }
        }
        let result = eval_expr(expr, &self.ctx);
        if self.recovery.is_some() {
            result.map_err(|_| {
                format!(
                    "WORKFLOW_EXPRESSION_ERROR: {}",
                    recovery_fingerprint(&serde_json::json!(expr))
                )
            })
        } else {
            result
        }
    }
    pub fn metrics(&self) -> WorkflowRunMetrics {
        let mut metrics = self.metrics.clone();
        metrics.reserved_task_tool_calls = self
            .recovery
            .as_ref()
            .map(|session| session.state.reserved_tool_calls)
            .unwrap_or(metrics.tool_calls);
        metrics
    }
    pub(super) fn persist_recovery(&self) -> Result<(), String> {
        if let Some(session) = &self.recovery {
            session.store.save(&session.state).map_err(recovery_error)?;
        }
        Ok(())
    }
    pub(super) fn finish_recovery(&mut self, succeeded: bool) -> Result<(), String> {
        if let Some(session) = &mut self.recovery {
            if succeeded && session.cursor != session.state.entries.len() {
                return Err(recovery_error("not all recorded invocations consumed"));
            }
            session.state.terminal =
                Some(if succeeded { "succeeded" } else { "failed" }.to_string());
        }
        self.persist_recovery()
    }
    pub(super) fn begin_effect(&mut self) -> Result<(), String> {
        if self.cancelled() || self.state == WorkflowState::Aborted {
            return Err("workflow aborted before effect".to_string());
        }
        if let Some(session) = &mut self.recovery {
            session
                .store
                .check_tool_budget(session.state.reserved_tool_calls)
                .map_err(recovery_error)?;
            let pending = session
                .state
                .pending
                .as_mut()
                .ok_or_else(|| recovery_error("invocation intent missing"))?;
            session
                .store
                .claim_effect(
                    session.cursor,
                    &serde_json::json!({"kind":pending.step_kind}),
                )
                .map_err(recovery_error)?;
            pending.effect_started = true;
        }
        self.persist_recovery()?;
        if self.cancelled() {
            return Err(recovery_error(
                "cancelled after intent, no external call entered",
            ));
        }
        Ok(())
    }
    /// Reserve before entering a backend or snapshot operation. A cancelled
    /// reservation remains consumed; it is never silently reset by resume.
    pub(super) fn begin_tool_call(&mut self) -> Result<(), String> {
        if self.cancelled() || self.state == WorkflowState::Aborted {
            return Err("workflow aborted before tool call".to_string());
        }
        if let Some(session) = &mut self.recovery {
            session
                .store
                .check_tool_budget(session.state.reserved_tool_calls)
                .map_err(recovery_error)?;
            session.state.reserved_tool_calls += 1;
            let pending = session
                .state
                .pending
                .as_mut()
                .ok_or_else(|| recovery_error("tool intent missing"))?;
            pending.tool_retried |= pending.tool_reserved;
            pending.tool_reserved = true;
        }
        self.persist_recovery()?;
        if self.cancelled() {
            return Err(recovery_error("cancelled after tool reservation"));
        }
        self.metrics.tool_calls += 1;
        Ok(())
    }
    pub(super) async fn execute_recoverable_step(
        &mut self,
        step: &WorkflowStep,
    ) -> Result<(), String> {
        let container = matches!(
            step,
            WorkflowStep::Loop { .. } | WorkflowStep::Cond { .. } | WorkflowStep::Subflow { .. }
        );
        if self.recovery.is_none() || container {
            return self.execute_step_inner(step).await;
        }
        let step_ref = recovery_fingerprint(&serde_json::to_value(step).map_err(recovery_error)?);
        let before = self.captured_context()?;
        let completed = self
            .recovery
            .as_ref()
            .and_then(|session| session.state.entries.get(session.cursor))
            .cloned();
        if let Some(mut entry) = completed {
            if entry.step_ref != step_ref
                || entry.ctx_before != before
                || entry.steps_before != self.executed_steps
            {
                return Err(recovery_error("recorded control flow/context mismatch"));
            }
            if entry
                .checkpoints
                .iter()
                .any(|checkpoint| !checkpoint.snapshot_dir.is_dir())
            {
                return Err(recovery_error("rollback snapshot missing"));
            }
            self.restore_context(entry.ctx_after);
            self.checkpoints = entry.checkpoints;
            self.executed_steps += entry.records.len() as u32;
            for record in &mut entry.records {
                record.detail = "Recovered completion; original output not copied".to_string();
                record.reused = true;
            }
            self.step_log.extend(entry.records);
            self.metrics.reused_invocations += 1;
            self.metrics.reused_tool_calls +=
                usize::from(entry.tool_call) + usize::from(entry.tool_retried);
            self.recovery.as_mut().unwrap().cursor += 1;
            return Ok(());
        }
        if self.executed_steps >= self.flow.max_steps {
            return Err("workflow step budget exhausted".to_string());
        }
        let ctx_before = self.ctx.clone();
        let restored_before = self.restored_context.clone();
        let steps_before = self.executed_steps;
        let log_before = self.step_log.len();
        let checkpoints_before = self.checkpoints.clone();
        {
            let session = self.recovery.as_mut().unwrap();
            if let Some(pending) = &session.state.pending {
                if pending.step_ref != step_ref || pending.ctx_before != before {
                    return Err(recovery_error("pending invocation/context mismatch"));
                }
            }
            session.state.pending = Some(PendingInvocation {
                step_ref: step_ref.clone(),
                step_kind: step_kind(step).to_string(),
                ctx_before: before.clone(),
                effect_started: false,
                tool_reserved: false,
                tool_retried: false,
            });
        }
        self.persist_recovery()?;
        let result = self.execute_step_inner(step).await;
        let pending = self
            .recovery
            .as_ref()
            .unwrap()
            .state
            .pending
            .as_ref()
            .unwrap();
        let effect_started = pending.effect_started;
        let tool_call = pending.tool_reserved;
        let tool_retried = pending.tool_retried;
        if let Err(error) = result {
            if self.cancelled() && !effect_started {
                self.ctx = ctx_before;
                self.restored_context = restored_before;
                self.executed_steps = steps_before;
                self.step_log.truncate(log_before);
                self.checkpoints = checkpoints_before;
                self.recovery.as_mut().unwrap().state.pending = None;
                self.persist_recovery()?;
                return Err("workflow approval/read cancelled".to_string());
            }
            let diagnostic = recovery_fingerprint(&serde_json::json!(error));
            if effect_started || error.contains(WORKFLOW_RECOVERY_REQUIRED) {
                return Err(recovery_error(format!(
                    "operation requires reconciliation ({diagnostic})"
                )));
            }
            return Err(format!(
                "WORKFLOW_STEP_FAILED: {} ({diagnostic})",
                step_kind(step)
            ));
        }
        let after = self.captured_context()?;
        let mut records = self.step_log[log_before..].to_vec();
        for record in &mut records {
            record.detail.clear();
        }
        let session = self.recovery.as_mut().unwrap();
        session.state.entries.push(CompletedInvocation {
            step_ref,
            ctx_before: before,
            ctx_after: after,
            steps_before,
            records,
            checkpoints: self.checkpoints.clone(),
            effect_started,
            tool_call,
            tool_retried,
        });
        session.state.pending = None;
        session.cursor += 1;
        self.persist_recovery()
    }
}
