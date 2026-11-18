//! Durable intent/receipt recovery for the existing Computer Use loop.
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use owo_agent_core::computer_task::ComputerTaskRegistry;
use owo_agent_core::computer_use::{
    coded_error, codes as computer_codes, observation_ref, revalidate_window_identity,
    ActionExecutionReceipt, ActionOutcome, ActionReceiptStage, ActionVerification, TaskGoal,
    TaskSurface,
};
use owo_agent_core::task_runtime::{event_type, TaskRunContext, TaskRuntime};
use owo_agent_core::task_surface::{UiaEditIdentity, WindowIdentity};
use serde_json::{json, Value};

pub(super) const RECOVERY_REQUIRED: &str = "ACTION_RECOVERY_REQUIRED";
fn recovery_error(detail: impl std::fmt::Display) -> String {
    coded_error(
        RECOVERY_REQUIRED,
        format!("{detail}；请先核对应用中的结果，取消旧任务后只为剩余步骤新建任务，避免重复输入"),
    )
}

#[derive(Clone)]
pub(super) struct RecoveryJournal {
    runtime: Arc<TaskRuntime>,
    registry: Arc<ComputerTaskRegistry>,
    task_id: String,
    attempt: u32,
    max_tool_calls: u32,
    run_id: String,
    plan_ref: String,
    state: Arc<Mutex<State>>,
}
struct State {
    ids: Vec<String>,
    actions: Vec<String>,
    pending: Vec<usize>,
    cursor: usize,
    active: Option<(usize, bool)>,
    receipts: BTreeMap<usize, ActionExecutionReceipt>,
}
impl RecoveryJournal {
    pub(super) fn plan_ref(context: &TaskRunContext) -> String {
        observation_ref(
            &json!({"user_goal":context.user_goal,"payload":context.payload,
            "workspace_ref":context.workspace_ref,"session_refs":context.session_refs,
            "policy_profile":context.policy_profile}),
        )
    }
    fn key(&self, id: &str) -> String {
        format!("computer-action:{id}")
    }
    pub(super) fn load(
        runtime: Arc<TaskRuntime>,
        registry: Arc<ComputerTaskRegistry>,
        context: &TaskRunContext,
        run_id: &str,
        goals: &[TaskGoal],
    ) -> Result<(Self, Vec<TaskGoal>), String> {
        let task_id = context.task_id.as_str().to_string();
        let plan_ref = Self::plan_ref(context);
        let ids: Vec<_> = (0..goals.len())
            .map(|i| {
                format!(
                    "{task_id}:{}:{}",
                    i + 1,
                    plan_ref.trim_start_matches("sha256:")
                )
            })
            .collect();
        let mut confirmed = BTreeMap::new();
        let mut intended = HashSet::new();
        let mut executed = HashSet::new();
        let mut cursor = 0;
        loop {
            let events = runtime
                .events(&task_id, cursor, 200)
                .map_err(recovery_error)?;
            if events.is_empty() {
                break;
            }
            for event in events {
                cursor = event.sequence;
                if event.event_type == event_type::TASK_COMPUTER_STARTED
                    && event.payload.get("journal_version").and_then(Value::as_u64) != Some(1)
                {
                    return Err(recovery_error(
                        "旧运行缺少持久动作意图，无法确定哪些操作已经完成",
                    ));
                }
                if event.event_type != event_type::TASK_COMPUTER_ACTION_INTENDED
                    && event.event_type != event_type::TASK_COMPUTER_ACTION_RECORDED
                {
                    continue;
                }
                if event.payload.get("plan_ref").and_then(Value::as_str) != Some(plan_ref.as_str())
                {
                    return Err(recovery_error("恢复计划与已记录动作的计划指纹不同或缺失"));
                }
                let index = event
                    .payload
                    .get("goal_index")
                    .and_then(Value::as_u64)
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|i| *i < goals.len())
                    .ok_or_else(|| recovery_error("动作记录的步骤身份无效"))?;
                if event.event_type == event_type::TASK_COMPUTER_ACTION_INTENDED {
                    if event.payload.get("action_id").and_then(Value::as_str)
                        != Some(ids[index].as_str())
                    {
                        return Err(recovery_error("动作意图身份不匹配"));
                    }
                    intended.insert(index);
                    continue;
                }
                let receipt: ActionExecutionReceipt =
                    serde_json::from_value(event.payload["receipt"].clone())
                        .map_err(|e| recovery_error(format!("动作回执无法读取：{e}")))?;
                if receipt.action_id != ids[index] || receipt.action != goals[index].action {
                    return Err(recovery_error("动作回执与当前步骤不匹配"));
                }
                let input_started = event
                    .payload
                    .get("input_started")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| recovery_error("动作回执缺少输入边界记录"))?;
                if input_started
                    && event.payload["stage"] == "execution"
                    && receipt.outcome == ActionOutcome::Executed
                {
                    executed.insert(index);
                }
                if input_started
                    && event.payload["stage"] == "observation"
                    && receipt.outcome == ActionOutcome::Executed
                    && receipt.verification == ActionVerification::Verified
                    && receipt.before_observation_ref.is_some()
                    && receipt.after_observation_ref.is_some()
                    && receipt.before_observation_ref != receipt.after_observation_ref
                    && goals[index]
                        .verify_text
                        .as_ref()
                        .is_some_and(|s| !s.trim().is_empty())
                {
                    confirmed.insert(index, receipt);
                }
            }
        }
        let journal = Self {
            runtime,
            registry,
            task_id,
            attempt: context.attempt,
            max_tool_calls: context.budget.max_tool_calls,
            run_id: run_id.to_string(),
            plan_ref,
            state: Arc::new(Mutex::new(State {
                ids,
                actions: goals.iter().map(|g| g.action.clone()).collect(),
                pending: Vec::new(),
                cursor: 0,
                active: None,
                receipts: confirmed,
            })),
        };
        let mut filtered = Vec::new();
        {
            let mut state = journal
                .state
                .lock()
                .map_err(|_| recovery_error("动作日志锁不可用"))?;
            for (index, goal) in goals.iter().enumerate() {
                let claim = journal
                    .runtime
                    .store()
                    .get_idempotency(&journal.key(&state.ids[index]))
                    .map_err(recovery_error)?;
                if let Some((owner, payload)) = &claim {
                    if owner != &journal.task_id
                        || payload["plan_ref"] != journal.plan_ref
                        || payload["goal_index"].as_u64() != Some(index as u64)
                    {
                        return Err(recovery_error("持久动作意图的范围不匹配"));
                    }
                }
                if let Some(receipt) = state.receipts.get(&index) {
                    if claim.is_none() || !intended.contains(&index) {
                        return Err(recovery_error("验证回执缺少对应的持久动作意图"));
                    }
                    journal.ensure_artifact(receipt)?;
                } else {
                    if claim.is_some() || intended.contains(&index) || executed.contains(&index) {
                        return Err(recovery_error(format!(
                            "第{}步已有动作意图或输入事实，但结果尚未确认",
                            index + 1
                        )));
                    }
                    state.pending.push(index);
                    filtered.push(goal.clone());
                }
            }
        }
        let durable_count =
            u32::try_from(executed.len()).map_err(|_| recovery_error("动作计数无效"))?;
        while journal.registry.actions_taken(&journal.task_id) < durable_count {
            journal.registry.record_action(&journal.task_id);
        }
        if !journal.receipts()?.is_empty() {
            let event = journal.runtime.store().append_event(&journal.task_id, "task.computer.actions.reused",
                journal.attempt,json!({"plan_ref":journal.plan_ref,"action_ids":journal.receipts()?.iter().map(|r| &r.action_id).collect::<Vec<_>>()}),Some(&journal.run_id))
                .map_err(recovery_error)?;
            journal.checkpoint(event.sequence)?;
        }
        Ok((journal, filtered))
    }
    pub(super) fn receipts(&self) -> Result<Vec<ActionExecutionReceipt>, String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| recovery_error("动作日志锁不可用"))?
            .receipts
            .values()
            .cloned()
            .collect())
    }
    fn ensure_artifact(&self, receipt: &ActionExecutionReceipt) -> Result<(), String> {
        let uri = format!("{}/{}", self.task_id, receipt.action_id);
        if !self
            .runtime
            .artifacts(&self.task_id)
            .map_err(|e| e.to_string())?
            .iter()
            .any(|a| a.kind == "desktop_action" && a.uri_or_path == uri)
        {
            self.runtime
                .record_artifact(
                    &self.task_id,
                    "desktop_action",
                    &uri,
                    receipt.after_observation_ref.clone(),
                    None,
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn checkpoint(&self, sequence: u64) -> Result<(), String> {
        let metadata = {
            let state = self
                .state
                .lock()
                .map_err(|_| recovery_error("动作日志锁不可用"))?;
            let confirmed: Vec<_> = state
                .receipts
                .iter()
                .filter(|(_, r)| r.verification == ActionVerification::Verified)
                .map(|(i, _)| *i)
                .collect();
            json!({"journal_version":1,"plan_ref":self.plan_ref,"consumed_action_count":self.registry.actions_taken(&self.task_id),
                "max_tool_calls":self.max_tool_calls,"confirmed_goal_indices":confirmed,
                "remaining_goal_indices":(0..state.ids.len()).filter(|i| !confirmed.contains(i)).collect::<Vec<_>>()})
        };
        let refs = self
            .runtime
            .artifacts(&self.task_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| a.artifact_id)
            .collect();
        self.runtime
            .record_checkpoint(
                &self.task_id,
                "computer_use.action_verified",
                &format!("task-event:{}:{sequence}", self.task_id),
                None,
                refs,
                metadata,
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn check_tool_budget(&self) -> Result<(), String> {
        if self.max_tool_calls > 0
            && self.registry.actions_taken(&self.task_id) >= self.max_tool_calls
        {
            return Err(coded_error(
                owo_agent_core::computer_use::codes::BUDGET_EXCEEDED,
                "已达到任务的工具调用预算，恢复不会清零已消耗动作",
            ));
        }
        Ok(())
    }
    fn begin(
        &self,
        action: &str,
        window: Option<WindowIdentity>,
        before_ref: Option<String>,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| recovery_error("动作日志锁不可用"))?;
        let index = *state
            .pending
            .get(state.cursor)
            .ok_or_else(|| recovery_error("动作超出剩余计划"))?;
        if state.actions[index] != action {
            return Err(recovery_error("动作类型不匹配"));
        }
        state.cursor += 1;
        state.active = Some((index, false));
        self.check_tool_budget()?;
        let intent = json!({"journal_version":1,"action_id":state.ids[index],"plan_ref":self.plan_ref,
            "goal_index":index,"action":action,"observed_window":window,"before_observation_ref":before_ref,
            "created_at":chrono::Utc::now().to_rfc3339()});
        // Unique insertion is the durable intent and excludes concurrent execution.
        // If the process dies before the following event, recovery still sees the claim.
        self.runtime
            .store()
            .put_idempotency(&self.key(&state.ids[index]), &self.task_id, &intent)
            .map_err(|e| recovery_error(format!("无法取得唯一动作意图：{e}")))?;
        self.runtime
            .store()
            .append_event(
                &self.task_id,
                event_type::TASK_COMPUTER_ACTION_INTENDED,
                self.attempt,
                intent,
                Some(&self.run_id),
            )
            .map_err(recovery_error)?;
        Ok(())
    }
    fn input_started(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| recovery_error("动作日志锁不可用"))?;
        state
            .active
            .as_mut()
            .ok_or_else(|| recovery_error("输入缺少动作意图"))?
            .1 = true;
        Ok(())
    }
    pub(super) fn record(
        &self,
        stage: ActionReceiptStage,
        receipt: &ActionExecutionReceipt,
    ) -> Result<(), String> {
        let (sequence, saved, input_started) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| recovery_error("动作日志锁不可用"))?;
            let (index, input_started) = state
                .active
                .ok_or_else(|| recovery_error("回执缺少动作意图"))?;
            let mut saved = receipt.clone();
            saved.action_id = state.ids[index].clone();
            if !input_started {
                saved.outcome = ActionOutcome::Rejected {
                    reason: "输入前的意图或门禁失败，未调用输入接口".into(),
                };
                saved.verification = ActionVerification::Skipped {
                    reason: "没有启动输入".into(),
                };
            }
            let event=self.runtime.store().append_event(&self.task_id,event_type::TASK_COMPUTER_ACTION_RECORDED,self.attempt,
                json!({"journal_version":1,"plan_ref":self.plan_ref,"goal_index":index,"input_started":input_started,
                    "core_action_id":receipt.action_id,"stage":stage,"receipt":saved}),Some(&self.run_id)).map_err(|e| e.to_string())?;
            state.receipts.insert(index, saved.clone());
            (event.sequence, saved, input_started)
        };
        if input_started
            && (stage == ActionReceiptStage::Observation
                || matches!(saved.outcome, ActionOutcome::Failed { .. }))
        {
            self.ensure_artifact(&saved)?;
        }
        if stage == ActionReceiptStage::Observation
            && saved.verification == ActionVerification::Verified
        {
            self.checkpoint(sequence)?;
        }
        Ok(())
    }
}

pub(super) struct JournalSurface {
    inner: Box<dyn TaskSurface>,
    journal: RecoveryJournal,
    expected: Option<WindowIdentity>,
    target_window_title: Option<String>,
    observation: Option<String>,
}
impl JournalSurface {
    pub(super) fn new(
        inner: Box<dyn TaskSurface>,
        journal: RecoveryJournal,
        target_window_title: Option<String>,
    ) -> Self {
        let expected = inner.window_identity();
        Self {
            inner,
            journal,
            expected,
            target_window_title,
            observation: None,
        }
    }
    fn checked_window(&self) -> Result<Option<WindowIdentity>, String> {
        let current = self.inner.window_identity();
        if let Some(title) = &self.target_window_title {
            if !current.as_ref().is_some_and(|window| {
                window.foreground
                    && (window.title == *title
                        || window.title.strip_prefix('*') == Some(title.as_str()))
            }) {
                return Err(coded_error(
                    computer_codes::STALE_WINDOW_IDENTITY,
                    "当前前台窗口标题与批准的目标不一致",
                ));
            }
        }
        Ok(current)
    }
    fn before_input(&self, action: &str) -> Result<(), String> {
        self.journal.begin(
            action,
            self.inner.window_identity(),
            self.observation.clone(),
        )?;
        self.journal
            .registry
            .check_can_execute(&self.journal.task_id)?;
        let current = self.checked_window()?;
        if let Some(expected) = &self.expected {
            revalidate_window_identity(expected, current.as_ref())?;
        }
        self.journal.check_tool_budget()?;
        self.journal.input_started()
    }
}
#[async_trait::async_trait]
impl TaskSurface for JournalSurface {
    fn app(&self) -> String {
        self.inner.app()
    }
    fn window_identity(&self) -> Option<WindowIdentity> {
        self.checked_window().ok().flatten()
    }
    async fn accessible_anchors(&mut self, hwnd: i64, name: &str) -> Result<Vec<Value>, String> {
        if self.target_window_title.is_none() || name.is_empty() {
            return Ok(Vec::new());
        }
        if !self
            .checked_window()?
            .is_some_and(|window| window.hwnd == hwnd)
        {
            return Err(coded_error(
                computer_codes::STALE_WINDOW_IDENTITY,
                "无障碍目标窗口已变化",
            ));
        }
        self.inner.accessible_anchors(hwnd, name).await
    }
    async fn accessible_observation(
        &mut self,
        hwnd: i64,
        name: &str,
    ) -> Result<Option<Value>, String> {
        if self.target_window_title.is_none() || name.is_empty() {
            return Ok(None);
        }
        if !self
            .checked_window()?
            .is_some_and(|window| window.hwnd == hwnd)
        {
            return Err(coded_error(
                computer_codes::STALE_WINDOW_IDENTITY,
                "无障碍目标窗口已变化",
            ));
        }
        self.inner.accessible_observation(hwnd, name).await
    }
    async fn ocr(&mut self) -> Result<Value, String> {
        let ocr = if self.target_window_title.is_some() {
            let window = self.checked_window()?.ok_or_else(|| {
                coded_error(computer_codes::STALE_WINDOW_IDENTITY, "目标窗口不可用")
            })?;
            self.inner.ocr_window(window.hwnd).await?
        } else {
            self.inner.ocr().await?
        };
        self.observation = Some(observation_ref(&ocr));
        Ok(ocr)
    }
    async fn click(&mut self, x: i32, y: i32) -> Result<(), String> {
        self.before_input("click")?;
        self.inner.click(x, y).await
    }
    async fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.before_input("type")?;
        self.inner.type_text(text).await
    }
    async fn set_value(
        &mut self,
        identity: &UiaEditIdentity,
        value: &str,
    ) -> Result<String, String> {
        self.before_input("set_value")?;
        self.inner.set_value(identity, value).await
    }
    async fn key(&mut self, key: &str) -> Result<(), String> {
        self.before_input("key")?;
        self.inner.key(key).await
    }
    async fn launch(&mut self, target: &str) -> Result<(), String> {
        self.before_input("launch")?;
        self.inner.launch(target).await
    }
}
