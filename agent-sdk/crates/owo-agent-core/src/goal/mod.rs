// R11:goal 质量收尾完成
//! Goal：目标→计划→并行 worker→验证→仲裁→恢复 的编排层（§12 底座 / 续写 §15）。
//!
//! - [`Goal`]：目标对象（objective / 状态机 / 预算 / 验收条件）。
//! - [`Worker`] + [`WorkerRegistry`]：步骤执行抽象（测试注入 MockWorker；
//!   真实接入 `Agent::run_subagent` 由主控后续做，本模块只读引用 agent 语义）。
//! - [`GoalRunner`]：依赖感知关键路径优先调度 + 并行度上限 + 步内重试 + 验证断言 +
//!   replan（只重建未完成子图）+ 预算熔断 + abort + 持久化恢复（已完成步骤不重跑）+ 全程审计。
//! - A2 统一调度适配层：步骤可经显式绑定定向到
//!   `in_process` / `local_process` / `fleet_node`（接口见 [`crate::execution_target`]）；
//!   显式目标不可用时等待/询问/拒绝，绝不静默切换到权限更高的目标；
//!   三类目标共用同一执行/取消/预算语义。HTTP 与节点注册细节不进入本模块。

use crate::audit::AuditLog;
use crate::blackboard::Blackboard;
use crate::critic::CriticConfig;
use crate::execution_target::{select_binding, DispatchCancelRegistry};
use crate::experience_store::{Attribution, ExperienceStore, Outcome};
use crate::plan::{Plan, StepSpec, StepStatus};
use std::sync::{Arc, Mutex};

mod acceptance;
mod execution;
mod persistence;
mod scheduler;
mod types;

#[cfg(test)]
mod tests;

pub use types::*;

use execution::{
    run_step_attempts, step_validation_receipt_matches, ExecutionBudget, StepOutcome, StepResult,
    StepRuntime, StepStop,
};
#[cfg(test)]
use execution::{validate_host_command_validation, verify_step_output};

type StepSkipper = Arc<dyn Fn(&StepSpec) -> Option<String> + Send + Sync>;
type WorkspaceCommandVerifier = Arc<
    dyn Fn(&str, &str, &crate::plan::VerificationRequirementV1) -> HostCommandValidationV1
        + Send
        + Sync,
>;

/// Host-collected result for a registered workspace command check.
/// The callback must only return evidence issued by the trusted tool host.
#[derive(Debug, Clone)]
pub struct HostCommandValidationV1 {
    pub verdict: crate::plan::ValidationVerdictV1,
    pub detail: Option<String>,
    pub subject_sha256: std::collections::BTreeMap<String, String>,
    pub evidence_ref: Option<String>,
}

/// Goal/Plan 调度器：wave 拓扑 + 并行限流 + 重试 + 验证 + replan + 恢复 + 审计。
///
/// 可选编排原语（多 Agent P0）：
/// - [`GoalRunner::attach_critic`]：步骤输出经只读 critic 评审，意见回流 worker（步骤 input 声明 `_critic.rounds`）。
/// - [`GoalRunner::attach_blackboard`]：共享工作区状态（单写主 = 本 runner）；步骤 input 声明 `_bb.read/write`。
pub struct GoalRunner {
    pub state: GoalRunState,
    config: RunnerConfig,
    /// 可选审计（与顶层 events 同步写）。
    audit: Option<Arc<Mutex<AuditLog>>>,
    /// 跨并发任务的 abort 标志（随 state.aborted 初始化）。
    aborted_flag: Arc<std::sync::atomic::AtomicBool>,
    /// 可选 critic 评审配置（步骤 input 声明 `_critic.rounds` 时生效）。
    critic: Option<CriticConfig>,
    /// 可选共享黑板（步骤 input 声明 `_bb.read/write` 时生效；写主为本 runner 的 goal id）。
    blackboard: Option<Blackboard>,
    /// 可选经验库（worker/Goal 结果幂等写入；空闲期由主控调 `aggregate` 蒸馏技能元数据）。
    experience: Option<ExperienceStore>,
    /// 可选步骤进度快照流（按步骤合并，供上层在 DAG 继续时持久化最新状态）。
    step_progress: Option<StepProgressSender>,
    attempt_admission: Option<AttemptAdmissionSender>,
    /// 可选动态 ready 节点跳过判定（用于运行期质量策略）。
    step_skipper: Option<StepSkipper>,
    workspace_verification_root: Option<std::path::PathBuf>,
    workspace_command_verifier: Option<WorkspaceCommandVerifier>,
    defer_goal_acceptance_to_delivery_gate: bool,
    execution_budget_usage: Option<(u32, u32)>,
    persistence_error: Option<String>,
}

impl GoalRunner {
    pub fn new(goal: Goal, plan: Plan, config: RunnerConfig) -> Self {
        Self {
            state: GoalRunState::new(goal, plan),
            config,
            audit: None,
            aborted_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            critic: None,
            blackboard: None,
            experience: None,
            step_progress: None,
            attempt_admission: None,
            step_skipper: None,
            workspace_verification_root: None,
            workspace_command_verifier: None,
            defer_goal_acceptance_to_delivery_gate: false,
            execution_budget_usage: None,
            persistence_error: None,
        }
    }

    /// 从持久化状态恢复（崩溃重启；已完成步骤不重跑）。
    pub fn from_state(mut state: GoalRunState, config: RunnerConfig) -> Self {
        let aborted = state.aborted;
        if !aborted {
            for record in state.records.values_mut() {
                if record.status == StepStatus::Running {
                    record.status = StepStatus::Pending;
                }
            }
        }
        Self {
            state,
            config,
            audit: None,
            aborted_flag: Arc::new(std::sync::atomic::AtomicBool::new(aborted)),
            critic: None,
            blackboard: None,
            experience: None,
            step_progress: None,
            attempt_admission: None,
            step_skipper: None,
            workspace_verification_root: None,
            workspace_command_verifier: None,
            defer_goal_acceptance_to_delivery_gate: false,
            execution_budget_usage: None,
            persistence_error: None,
        }
    }

    /// Defer aggregate goal acceptance to the outer Team DeliveryGate.
    ///
    /// Team runs execute partial DAG phases here. A successful phase only means its
    /// claimed steps completed; it must not create a delivery completion record.
    pub(crate) fn defer_goal_acceptance_to_delivery_gate(&mut self) {
        self.defer_goal_acceptance_to_delivery_gate = true;
    }

    /// Seed a phase-local runner with usage committed by prior Team phases.
    pub(crate) fn seed_execution_budget_usage(&mut self, steps_taken: u32, total_retries: u32) {
        self.execution_budget_usage = Some((steps_taken, total_retries));
    }

    /// 把每步终态推送给上层；WorkSwarm 用它逐任务写入完整运行状态。
    pub fn attach_step_progress(&mut self, sender: StepProgressSender) {
        self.step_progress = Some(sender);
    }

    /// Bind host cancellation directly to every in-flight step and admission wait.
    pub(crate) fn attach_abort_signal(&mut self, abort_signal: Arc<std::sync::atomic::AtomicBool>) {
        if self.aborted_flag.load(std::sync::atomic::Ordering::SeqCst) {
            abort_signal.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.aborted_flag = abort_signal;
    }

    pub(crate) fn attach_attempt_admission(&mut self, sender: AttemptAdmissionSender) {
        self.attempt_admission = Some(sender);
    }

    /// 为显式 WorkspacePaths 计划绑定宿主提供的验证根目录。
    /// 没有绑定时验证结果保持 Unverified，必需要求会阻止下游依赖解锁。
    pub fn attach_workspace_verification_root(&mut self, root: impl Into<std::path::PathBuf>) {
        self.workspace_verification_root = Some(root.into());
    }

    /// Attach a host-owned receipt resolver for registered workspace behavior commands.
    /// GoalRunner never runs the command itself; without this trusted resolver, the
    /// command requirement remains Unsupported and cannot unlock dependent steps.
    pub fn attach_workspace_command_verifier<F>(&mut self, verifier: F)
    where
        F: Fn(&str, &str, &crate::plan::VerificationRequirementV1) -> HostCommandValidationV1
            + Send
            + Sync
            + 'static,
    {
        self.workspace_command_verifier = Some(Arc::new(verifier));
    }

    /// 对刚就绪步骤执行确定性跳过判定；返回原因时该节点以成功跳过方式收敛。
    pub fn attach_step_skipper<F>(&mut self, skipper: F)
    where
        F: Fn(&StepSpec) -> Option<String> + Send + Sync + 'static,
    {
        self.step_skipper = Some(Arc::new(skipper));
    }

    fn notify_step_progress(&self, step_id: &str) {
        self.notify_step_progress_with_skip(step_id, None);
    }

    fn notify_step_progress_with_skip(&self, step_id: &str, skip_reason: Option<String>) {
        let Some(sender) = &self.step_progress else {
            return;
        };
        if let Some(record) = self.state.records.get(step_id) {
            let worker = self
                .state
                .plan
                .step(step_id)
                .map(|step| step.worker.clone())
                .unwrap_or_default();
            sender.send(StepProgressUpdate {
                step_id: step_id.to_string(),
                worker,
                record: record.clone(),
                steps_taken: self.state.steps_taken,
                total_retries: self.state.total_retries,
                skip_reason,
            });
        }
    }

    fn notify_all_step_progress(&self) {
        for step_id in self.state.records.keys() {
            self.notify_step_progress(step_id);
        }
    }

    pub fn attach_audit(&mut self, log: Arc<Mutex<AuditLog>>) {
        self.audit = Some(log);
    }

    /// 注入经验库：步骤/Goal 结果以 correlation_id 幂等写入（崩溃恢复/空闲聚合的数据源）。
    pub fn attach_experience(&mut self, store: ExperienceStore) {
        self.experience = Some(store);
    }

    /// 注入 critic 评审配置：步骤声明 `_critic.rounds` 时输出经只读评审后回流 worker。
    pub fn attach_critic(&mut self, config: CriticConfig) {
        self.critic = Some(config);
    }

    /// 注入共享黑板：步骤声明 `_bb.read` 读取共享中间结果、`_bb.write` 写回。
    /// 黑板写主应为该 goal 的 id（`Blackboard::new(goal.id, policy)`）。
    pub async fn attach_blackboard(&mut self, blackboard: Blackboard) {
        self.blackboard = Some(blackboard);
    }

    fn log(&mut self, event: &str, detail: impl Into<String>) {
        let detail = detail.into();
        self.state.events.push(format!("{event}: {detail}"));
        if let Some(log) = &self.audit {
            if let Ok(mut log) = log.lock() {
                log.record(
                    &self.state.goal.id,
                    event,
                    Some(format!("goal/{}", self.state.plan.goal_id)),
                    None,
                    detail,
                );
            }
        }
    }

    fn record_mut(&mut self, step_id: &str) -> &mut StepRecord {
        self.state
            .records
            .get_mut(step_id)
            .expect("步骤记录必须存在")
    }

    /// abort 已请求？（state.aborted 与标志任意一个为真——十期·四路 R4：
    /// 协调器置位标志后，运行循环必须在回合边界协作退出，而非被外部丢弃。）
    fn is_aborting(&self) -> bool {
        self.state.aborted || self.aborted_flag.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn record_aborted_completion(&mut self) {
        let evidence_receipt_ids = self
            .state
            .validation_receipts
            .iter()
            .chain(
                self.state
                    .records
                    .values()
                    .flat_map(|record| record.validation_receipts.iter()),
            )
            .map(|receipt| receipt.receipt_id.clone())
            .collect::<Vec<_>>();
        let candidate_version_sha256 = self.candidate_version_sha256(true, false).ok().flatten();
        self.state.completion_record = Some(crate::completion::build_completion_record(
            &self.state.goal.id,
            &self.state.run_id,
            crate::completion::decide_completion(crate::completion::CompletionEvidence {
                aborted: true,
                ..crate::completion::CompletionEvidence::default()
            }),
            evidence_receipt_ids,
            candidate_version_sha256,
        ));
    }

    /// 进入 Aborted 终态（状态 + 未完成步骤 + 落盘），幂等。
    fn enter_aborted(&mut self) {
        if !self.state.aborted {
            self.state.aborted = true;
        }
        self.mark_remaining(StepStatus::Aborted);
        self.state.goal.transition(GoalStatus::Aborted);
        self.record_aborted_completion();
        self.persist_if_needed();
        self.notify_all_step_progress();
        self.log("goal.abort", "协调器取消：协作退出并保留已完成产物");
    }

    /// 取消执行：abort 标志置位，未完成步骤标记 Aborted 保留现场。
    pub fn abort(&mut self) {
        self.state.aborted = true;
        self.aborted_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.mark_remaining(StepStatus::Aborted);
        self.log("goal.abort", "调度器收到 abort 请求");
        self.state.goal.transition(GoalStatus::Aborted);
        self.record_aborted_completion();
        self.persist_if_needed();
        self.notify_all_step_progress();
    }

    /// 暴露只读 abort 标志（供协调器取消链置位，不必强占运行 Future 的 &mut 借用）。
    pub fn abort_signal(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.aborted_flag)
    }

    /// 执行计划（恢复时已完成步骤自动跳过）。返回目标终态。
    /// 结束后把 Goal 结果幂等写入经验库（若有）。
    pub async fn run(&mut self, workers: &WorkerRegistry) -> Result<GoalStatus, String> {
        if self.persistence_error.is_some() {
            self.persist_if_needed();
        }
        let run_result = self.run_inner(workers).await;
        let status = match (run_result, self.persistence_error.clone()) {
            (Ok(status), None) => status,
            (Err(run_error), Some(persist_error)) => {
                return Err(format!("{run_error}; {persist_error}"));
            }
            (_, Some(persist_error)) => return Err(persist_error),
            (Err(run_error), None) => return Err(run_error),
        };
        if let Some(exp) = &self.experience {
            let outcome = match status {
                GoalStatus::Succeeded => Outcome::Success,
                GoalStatus::Aborted => Outcome::Aborted,
                _ => Outcome::Failure,
            };
            let attribution = Attribution {
                goal_id: Some(self.state.goal.id.clone()),
                plan_id: Some(self.state.plan.id.clone()),
                step_id: None,
                input_keys: Vec::new(),
                error: self.state.goal.error.clone(),
            };
            let _ = exp.record_goal_outcome(
                self.state.run_id.clone(),
                self.state.goal.id.clone(),
                outcome,
                attribution,
            );
        }
        Ok(status)
    }

    /// 执行主体（run 的收尾经验写入拆在 [`Self::run`]）。
    async fn run_inner(&mut self, workers: &WorkerRegistry) -> Result<GoalStatus, String> {
        if self.state.goal.status.is_terminal() {
            return Ok(self.state.goal.status);
        }
        if self.is_aborting() {
            self.enter_aborted();
            return Ok(GoalStatus::Aborted);
        }
        if let Err(reason) = self.state.plan.validate() {
            return self.fail_goal(format!("执行计划非法：{reason}"));
        }
        self.state.goal.transition(GoalStatus::Running);
        self.log("goal.start", format!("目标 {}", self.state.goal.objective));

        let started = std::time::Instant::now();
        let budget = self.state.goal.budget;
        let max_parallel = self.config.max_parallel.max(1);
        let (used_steps, used_retries) = self
            .execution_budget_usage
            .unwrap_or((self.state.steps_taken, self.state.total_retries));
        let execution_budget = Arc::new(ExecutionBudget::new(used_steps, used_retries, budget));
        let abort_flag = Arc::clone(&self.aborted_flag);
        let workers = workers.clone();
        let bb_writer = if let Some(bb) = &self.blackboard {
            Some(bb.writer().await)
        } else {
            None
        };
        let rt = StepRuntime {
            execution_budget: Arc::clone(&execution_budget),
            aborted: abort_flag,
            critic: self.critic.clone(),
            blackboard: self.blackboard.clone(),
            bb_writer,
            use_worker_pool: self.config.use_worker_pool,
            worker_pool: self.config.worker_pool.clone(),
            capabilities: self.config.capability_registry.clone(),
            capability_requirement: self.config.capability_requirement.clone(),
            transport: self.config.transport.clone(),
            leases: self.config.leases.clone(),
            bindings: self.config.bindings.clone(),
            run_id: self.state.run_id.clone(),
            cancels: DispatchCancelRegistry::default(),
            workspace_verification_root: self.workspace_verification_root.clone(),
            workspace_command_verifier: self.workspace_command_verifier.clone(),
            attempt_admission: self.attempt_admission.clone(),
        };

        loop {
            // 十期·四路 R2/R4：协调器置位 abort 标志或状态标记为 aborted 时，
            // 在回合边界协作退出——不在此处丢弃在飞步骤 Future（有界清理见
            // 主循环早退路径与 [`run_worker_cancellable`]）。
            if self.is_aborting() {
                self.enter_aborted();
                return Ok(GoalStatus::Aborted);
            }
            // 时长预算熔断。
            if budget.max_duration_secs > 0
                && started.elapsed().as_secs() >= budget.max_duration_secs
            {
                return self.fail_goal(format!(
                    "预算熔断：执行时长超过 {}s",
                    budget.max_duration_secs
                ));
            }
            // 计算当前就绪步骤：依赖全部 Succeeded 且自身未完成。
            let mut frontier = scheduler::ReadyFrontier::new(&self.state.plan.steps);
            frontier.enqueue_ready_steps(&self.state.plan.steps, |step| {
                self.state.records[&step.id].status.can_resume() && self.deps_succeeded(step)
            });
            if frontier.is_empty() {
                // 检查是否全部成功 → 目标验收。
                if self
                    .state
                    .plan
                    .steps
                    .iter()
                    .all(|s| self.state.records[&s.id].status == StepStatus::Succeeded)
                {
                    if self.defer_goal_acceptance_to_delivery_gate {
                        self.state.goal.transition(GoalStatus::Succeeded);
                        self.state.goal.error = None;
                        self.log(
                            "goal.phase_succeeded",
                            "阶段步骤完成，目标验收交由外层 Team DeliveryGate",
                        );
                        self.persist_if_needed();
                        return Ok(GoalStatus::Succeeded);
                    }
                    return self.verify_goal();
                }
                if self.state.replan_count >= budget.max_replans {
                    return self.fail_goal("replan 次数超限，未完成步骤无法恢复".to_string());
                }
                return self.fail_goal("死锁：存在未完成步骤但无就绪步骤".to_string());
            }

            // 就绪即派发：关键路径优先队列在完成事件到达时只接纳直接解锁的后继，
            // 关键链可越过无关积压，且不等待同一批次里的慢步骤结束。
            let mut set = tokio::task::JoinSet::new();
            let mut failed: Vec<StepSpec> = Vec::new();
            let mut stop_error: Option<StepStop> = None;
            loop {
                while set.len() < max_parallel && stop_error.is_none() {
                    let Some(step_index) = frontier.pop() else {
                        break;
                    };
                    let step = self.state.plan.steps[step_index].clone();
                    if let Some(reason) = self
                        .step_skipper
                        .as_ref()
                        .and_then(|skipper| skipper(&step))
                    {
                        if let Some(record) = self.state.records.get_mut(&step.id) {
                            record.status = StepStatus::Succeeded;
                            record.attempts = 0;
                            record.output = None;
                            record.error = None;
                            record.skip_reason = Some(reason.clone());
                        }
                        self.log(
                            "goal.step.skipped",
                            format!("步骤 {} 按运行期策略跳过", step.id),
                        );
                        self.persist_if_needed();
                        self.notify_step_progress_with_skip(&step.id, Some(reason));
                        frontier.enqueue_ready_successors(
                            &step.id,
                            &self.state.plan.steps,
                            |candidate| {
                                self.state.records[&candidate.id].status.can_resume()
                                    && self.deps_succeeded(candidate)
                            },
                        );
                        continue;
                    }
                    let target_note = select_binding(&self.config.bindings, &step.id, &step.worker)
                        .map(|b| {
                            format!(
                                "，target={} node={}",
                                b.target.kind(),
                                b.target.node_id().unwrap_or("-")
                            )
                        })
                        .unwrap_or_default();
                    self.log(
                        "goal.step.start",
                        format!("步骤 {}（worker {}）{}", step.id, step.worker, target_note),
                    );
                    let epoch = self
                        .state
                        .records
                        .get(&step.id)
                        .and_then(|record| record.phase_epoch)
                        .unwrap_or_else(|| self.state.replan_count.saturating_add(1) as u64);
                    let attempt_id = self
                        .state
                        .records
                        .get(&step.id)
                        .and_then(|record| record.attempt_id.clone())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    if let Some(record) = self.state.records.get_mut(&step.id) {
                        record.status = StepStatus::Running;
                        record.attempt_id = Some(attempt_id.clone());
                        record.phase_epoch = Some(epoch);
                    }
                    self.notify_step_progress(&step.id);
                    set.spawn(run_step_attempts(
                        workers.clone(),
                        step,
                        rt.clone(),
                        attempt_id,
                        epoch,
                    ));
                }
                if set.is_empty() || stop_error.is_some() {
                    break;
                }
                let joined = if budget.max_duration_secs > 0 {
                    let remaining = std::time::Duration::from_secs(budget.max_duration_secs)
                        .saturating_sub(started.elapsed());
                    match tokio::time::timeout(remaining, set.join_next()).await {
                        Ok(joined) => joined,
                        Err(_) => {
                            self.aborted_flag
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                            stop_error = Some(StepStop::Budget(format!(
                                "执行时长超过 {}s，已取消在飞任务",
                                budget.max_duration_secs
                            )));
                            break;
                        }
                    }
                } else {
                    set.join_next().await
                };
                let completed_step_id = joined
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map(|outcome| outcome.step_id.clone());
                if let Err(stop) = self.merge_step_outcome(joined, &mut failed) {
                    stop_error = Some(stop);
                } else if let Some(completed_step_id) = completed_step_id {
                    frontier.enqueue_ready_successors(
                        &completed_step_id,
                        &self.state.plan.steps,
                        |candidate| {
                            self.state.records[&candidate.id].status.can_resume()
                                && self.deps_succeeded(candidate)
                        },
                    );
                }
            }
            // 早退路径（abort/预算熔断/目标拒绝）：先置位 abort 标志（通知在飞
            // 步骤在其回合边界协作退出并完成变更收尾），再做**有界清理**——绝不
            // 直接 `abort_all` 丢弃 Future 跳过变更收尾。清理窗口内 join 完的在飞
            // 步骤正常合并（其内部 TrackedRoleWorker 已完成后快照/变更登记）；
            // 超过清理时限才强制终止剩余任务（进程树由沙箱 Job kill-on-close
            // 兜底），并把在飞的远端派发任务统一 cancel（防 transport 残留 pending）。
            if !set.is_empty() {
                self.aborted_flag
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let deadline = tokio::time::Instant::now() + CANCELLATION_CLEANUP_GRACE;
                while !set.is_empty() && tokio::time::Instant::now() < deadline {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    let joined = match tokio::time::timeout(remaining, set.join_next()).await {
                        Ok(Some(joined)) => joined,
                        Ok(None) | Err(_) => break,
                    };
                    // join 到已送达的结果照常合并（Retried/Ok 均进入状态；
                    // 步骤内部已在 Future 中完成自己的收尾）。
                    let _ = self.merge_step_outcome(Some(joined), &mut failed);
                }
            }
            set.abort_all();
            while set.join_next().await.is_some() {}
            rt.cancels.cancel_all().await;
            match stop_error {
                Some(StepStop::Budget(reason)) => {
                    return self.fail_goal(format!("预算熔断：{reason}"));
                }
                Some(StepStop::Fatal(reason)) => {
                    return self.fail_goal(reason);
                }
                None => {}
            }

            if self.is_aborting() {
                self.enter_aborted();
                return Ok(GoalStatus::Aborted);
            }

            if !failed.is_empty() {
                let completion_status = self
                    .failed_step_verification_status(&failed)
                    .unwrap_or(owo_agent_protocol::CompletionStatusV1::Blocked);
                if !self.config.allow_replan {
                    return self.fail_goal_with_status(
                        format!(
                            "步骤失败且 replan 未启用：{:?}",
                            failed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()
                        ),
                        completion_status,
                    );
                }
                if self.state.replan_count >= budget.max_replans {
                    return self.fail_goal_with_status(
                        format!(
                            "步骤失败且 replan 次数超限（{}）：{:?}",
                            budget.max_replans,
                            failed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()
                        ),
                        completion_status,
                    );
                }
                self.replan(&failed);
            }
        }
    }

    /// 合并一个步骤的并发执行结果到运行状态。失败步骤加入 failed；
    /// 预算熔断返回 [`StepStop::Budget`]，确定性失败返回 [`StepStop::Fatal`]。
    fn merge_step_outcome(
        &mut self,
        joined: Option<Result<StepOutcome, tokio::task::JoinError>>,
        failed: &mut Vec<StepSpec>,
    ) -> Result<(), StepStop> {
        let outcome = match joined {
            Some(Ok(outcome)) => outcome,
            // 既有语义保持：panic/丢失按「预算熔断」表述收尾。
            Some(Err(e)) => return Err(StepStop::Budget(format!("步骤任务 panic：{e}"))),
            None => return Err(StepStop::Budget("步骤任务丢失".to_string())),
        };
        self.state.steps_taken = self.state.steps_taken.saturating_add(outcome.attempts);
        // The first execution is a step attempt, not a retry; later worker/critic
        // executions consume the global retry budget.
        self.state.total_retries = self
            .state
            .total_retries
            .saturating_add(outcome.attempts.saturating_sub(1));
        match outcome.result {
            StepResult::Ok {
                ref output,
                ref attempt_id,
                ref validation_receipts,
                epoch,
            } => {
                let output_hash = crate::cas_store::CasStore::hash_of(output.as_bytes());
                let record = self.record_mut(&outcome.step_id);
                for previous in &mut record.validation_receipts {
                    if previous.verdict == crate::plan::ValidationVerdictV1::Passed
                        && previous.subject_sha256.get("step-output") != Some(&output_hash)
                    {
                        previous.verdict = crate::plan::ValidationVerdictV1::Stale;
                        previous.detail = Some("步骤输出版本已变化，旧收据失效".to_string());
                    }
                }
                for receipt in validation_receipts {
                    let mut receipt = receipt.clone();
                    if receipt.verdict == crate::plan::ValidationVerdictV1::Passed
                        && receipt.subject_sha256.get("step-output") != Some(&output_hash)
                    {
                        receipt.verdict = crate::plan::ValidationVerdictV1::Stale;
                        receipt.detail = Some("验收收据绑定的输出不是最终接受版本".to_string());
                    }
                    if !record
                        .validation_receipts
                        .iter()
                        .any(|previous| previous.receipt_id == receipt.receipt_id)
                    {
                        record.validation_receipts.push(receipt);
                    }
                }
                record.status = StepStatus::Succeeded;
                record.attempts = outcome.attempts;
                record.attempt_id = Some(attempt_id.clone());
                record.phase_epoch = Some(epoch);
                record.output = Some(output.clone());
                record.error = None;
                record.skip_reason = None;
                self.record_worker_experience(&outcome, true, None);
                self.mark_step_health(&outcome, true);
                self.log(
                    "goal.step.succeeded",
                    format!(
                        "步骤 {} 通过（{} 次尝试）",
                        outcome.step_id, outcome.attempts
                    ),
                );
                self.persist_if_needed();
                self.notify_step_progress(&outcome.step_id);
            }
            StepResult::FailedWithReceipts {
                ref error,
                ref attempt_id,
                ref validation_receipts,
                epoch,
            } => {
                let record = self.record_mut(&outcome.step_id);
                for previous in &mut record.validation_receipts {
                    if previous.verdict == crate::plan::ValidationVerdictV1::Passed
                        && previous.attempt_id != *attempt_id
                    {
                        previous.verdict = crate::plan::ValidationVerdictV1::Stale;
                        previous.detail = Some("步骤进入新尝试，旧收据失效".to_string());
                    }
                }
                for receipt in validation_receipts {
                    let mut receipt = receipt.clone();
                    if receipt.verdict == crate::plan::ValidationVerdictV1::Passed
                        && receipt.attempt_id != *attempt_id
                    {
                        receipt.verdict = crate::plan::ValidationVerdictV1::Stale;
                        receipt.detail = Some("验收收据不属于最终失败的尝试".to_string());
                    }
                    if !record
                        .validation_receipts
                        .iter()
                        .any(|previous| previous.receipt_id == receipt.receipt_id)
                    {
                        record.validation_receipts.push(receipt);
                    }
                }
                record.status = StepStatus::Failed;
                record.attempts = outcome.attempts;
                record.attempt_id = Some(attempt_id.clone());
                record.phase_epoch = Some(epoch);
                record.error = Some(error.clone());
                record.skip_reason = None;
                self.record_worker_experience(&outcome, false, Some(error));
                self.mark_step_health(&outcome, false);
                self.log(
                    "goal.step.failed",
                    format!("步骤 {} 验证失败：{error}", outcome.step_id),
                );
                if let Some(step) = self.state.plan.step(&outcome.step_id) {
                    failed.push(step.clone());
                }
                self.persist_if_needed();
                self.notify_step_progress(&outcome.step_id);
            }
            StepResult::Retried { ref error } => {
                let record = self.record_mut(&outcome.step_id);
                record.status = StepStatus::Failed;
                record.attempts = outcome.attempts;
                record.error = Some(error.clone());
                record.skip_reason = None;
                self.record_worker_experience(&outcome, false, Some(error));
                self.mark_step_health(&outcome, false);
                self.log(
                    "goal.step.failed",
                    format!("步骤 {} 失败：{error}", outcome.step_id),
                );
                if let Some(step) = self.state.plan.step(&outcome.step_id) {
                    failed.push(step.clone());
                }
                self.persist_if_needed();
                self.notify_step_progress(&outcome.step_id);
            }
            StepResult::Budget { reason } => {
                return Err(StepStop::Budget(reason));
            }
            StepResult::Fatal { ref reason } => {
                // 确定性失败：只记录现场，不写 worker 健康/经验（执行体根本未运行），
                // 不参与 replan；原因直达目标终态。
                let record = self.record_mut(&outcome.step_id);
                record.status = StepStatus::Failed;
                record.attempts = outcome.attempts;
                record.error = Some(reason.clone());
                record.skip_reason = None;
                self.log(
                    "goal.step.rejected",
                    format!("步骤 {} 目标不可用终止：{reason}", outcome.step_id),
                );
                self.persist_if_needed();
                self.notify_step_progress(&outcome.step_id);
                return Err(StepStop::Fatal(reason.clone()));
            }
        }
        Ok(())
    }

    /// 步骤结果反馈能力注册表健康度（worker 生命周期事件接线；路由会跳过失败过多的 worker）。
    fn mark_step_health(&self, outcome: &StepOutcome, ok: bool) {
        let Some(reg) = &self.config.capability_registry else {
            return;
        };
        if let Some(step) = self.state.plan.step(&outcome.step_id) {
            reg.mark_health(&step.worker, ok);
        }
    }

    /// 把单步结果幂等写入经验库（correlation_id = `run_id:step_id`；重放/重跑不重复）。
    fn record_worker_experience(&self, outcome: &StepOutcome, ok: bool, error: Option<&str>) {
        let Some(exp) = &self.experience else {
            return;
        };
        let Some(step) = self.state.plan.step(&outcome.step_id) else {
            return;
        };
        let attribution = Attribution {
            goal_id: Some(self.state.goal.id.clone()),
            plan_id: Some(self.state.plan.id.clone()),
            step_id: Some(step.id.clone()),
            input_keys: step
                .input
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default(),
            error: error.map(|e| e.to_string()),
        };
        let result = if ok {
            Outcome::Success
        } else {
            Outcome::Failure
        };
        let _ = exp.record_worker_outcome(
            format!("{}:{}", self.state.run_id, step.id),
            step.worker.clone(),
            result,
            attribution,
        );
    }

    /// replan：重置失败步骤及其未完成的后代子图（已 Succeeded 步骤保留），重新调度。
    fn replan(&mut self, failed: &[StepSpec]) {
        self.state.replan_count += 1;
        self.log(
            "goal.replan",
            format!(
                "第 {} 次 replan：重置 {}",
                self.state.replan_count,
                failed
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
        // 收集受影响的未完成后代（依赖链中包含 failed 步骤的）。
        let mut to_reset: Vec<String> = Vec::new();
        for step in &self.state.plan.steps {
            if self.state.records[&step.id].status == StepStatus::Succeeded {
                continue;
            }
            if self.depends_on_any(
                step,
                &failed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ) {
                to_reset.push(step.id.clone());
            }
        }
        for step_id in &to_reset {
            let record = self.record_mut(step_id);
            record.status = StepStatus::Pending;
            record.error = None;
            record.attempts = 0;
            record.attempt_id = None;
            record.phase_epoch = None;
        }
        // 失败的步骤本身也要重置（含在 to_reset 中，因为 depends_on_any 对自身成立时）——
        // 显式重置避免依赖判断遗漏。
        for step in failed {
            if let Some(record) = self.state.records.get_mut(&step.id) {
                if !to_reset.contains(&step.id) {
                    record.status = StepStatus::Pending;
                    record.error = None;
                    record.attempts = 0;
                    record.attempt_id = None;
                    record.phase_epoch = None;
                }
            }
        }
        self.persist_if_needed();
        self.notify_all_step_progress();
    }

    fn depends_on_any(&self, step: &StepSpec, targets: &[&str]) -> bool {
        let mut stack: Vec<&str> = step.depends_on.iter().map(|s| s.as_str()).collect();
        let mut visited = std::collections::HashSet::new();
        while let Some(dep) = stack.pop() {
            if targets.contains(&dep) {
                return true;
            }
            if !visited.insert(dep) {
                continue;
            }
            if let Some(dep_step) = self.state.plan.step(dep) {
                stack.extend(dep_step.depends_on.iter().map(|s| s.as_str()));
            }
        }
        false
    }

    fn deps_succeeded(&self, step: &StepSpec) -> bool {
        step.depends_on.iter().all(|dep| {
            self.state
                .records
                .get(dep)
                .map(|r| r.status == StepStatus::Succeeded)
                .unwrap_or(false)
        })
    }
}
