use super::registry_builder::build_run_registry;
use super::{project_workspace, workswarm_metrics};
use owo_agent_core::workswarm::{SteerCommand, TeamCoordinator};
use owo_agent_server::AppState;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

async fn fail_pending_rework_tasks(coordinator: &TeamCoordinator, team_id: &str, reason: &str) {
    let Ok(team) = coordinator.get_team_run(team_id).await else {
        return;
    };
    let Some(project_id) = team.project_space_id.as_deref() else {
        return;
    };
    let Ok(mut space) = coordinator.store().get_project_space(project_id).await else {
        return;
    };
    let mut changed = false;
    for task in &mut space.rework_tasks {
        if task.team_id == team_id
            && matches!(
                task.status,
                owo_agent_protocol::ArtifactReworkStatus::Dispatching
                    | owo_agent_protocol::ArtifactReworkStatus::Requested
            )
        {
            task.status = owo_agent_protocol::ArtifactReworkStatus::Failed;
            task.error = reason.to_string();
            changed = true;
        }
    }
    if changed {
        space.version += 1;
        space.updated_at = chrono::Utc::now().to_rfc3339();
        if let Err(error) = coordinator.store().save_project_space(&space).await {
            tracing::error!(team_id = %team_id, %error, "返工终态回写失败");
        }
    }
}
// ---------------------------------------------------------------------------
// 后台运行循环（阶段驱动 + 人节点门闩）
// ---------------------------------------------------------------------------

/// 运行循环存活守卫（R2）：进程内声明「该团队有活动运行循环」——
/// 中断识别据此区分「循环存活（含批次间隙/人节点门闩）」与「重启遗留」。
/// Drop 兜底撤销（覆盖所有 return 路径与 panic 展开栈）。
pub(super) struct LoopAliveGuard {
    coordinator: Arc<TeamCoordinator>,
    team_id: String,
}

impl LoopAliveGuard {
    /// 竞争运行循环所有权：已有循环在世（或并发 spawn 竞争失败）返回 None。
    fn try_acquire(coordinator: &Arc<TeamCoordinator>, team_id: &str) -> Option<Self> {
        if !coordinator.try_claim_loop(team_id) {
            return None;
        }
        Some(Self {
            coordinator: Arc::clone(coordinator),
            team_id: team_id.to_string(),
        })
    }
}

impl Drop for LoopAliveGuard {
    fn drop(&mut self) {
        self.coordinator.set_loop_alive(&self.team_id, false);
    }
}

/// Team 指标预算门的纯判定。无法读取预算属于未知状态，必须按 fail-closed 停止派发。
#[derive(Debug, PartialEq, Eq)]
enum BudgetGuardDecision {
    Continue,
    Exhausted(String),
    Unavailable(String),
}

fn budget_guard_decision(check: Result<Option<String>, String>) -> BudgetGuardDecision {
    match check {
        Ok(Some(reason)) => BudgetGuardDecision::Exhausted(reason),
        Ok(None) => BudgetGuardDecision::Continue,
        Err(error) => BudgetGuardDecision::Unavailable(error),
    }
}

fn next_guard_retry_delay(current: Duration) -> Duration {
    (current * 2).min(Duration::from_secs(5))
}

async fn stop_team_for_guard(
    coordinator: &Arc<TeamCoordinator>,
    team_id: &str,
    event: &str,
    audit_reason: &str,
    rework_reason: &str,
) {
    tracing::warn!(team_id = %team_id, %audit_reason, "workswarm 运行保护门停止后续调度");
    if let Some(log) = coordinator.audit_log() {
        if let Ok(mut audit) = log.lock() {
            audit.record(
                team_id,
                event,
                Some(format!("workswarm/{team_id}")),
                Some(false),
                audit_reason.to_string(),
            );
        }
    }
    let mut retry_delay = Duration::from_millis(100);
    loop {
        match coordinator
            .apply_steer(team_id, &SteerCommand::Cancel)
            .await
        {
            Ok(_) => break,
            Err(owo_agent_core::workswarm::WorkSwarmError::NotFound(_)) => {
                tracing::warn!(team_id = %team_id, "停止保护门时 TeamRun 已不存在");
                break;
            }
            Err(error) => {
                tracing::error!(
                    team_id = %team_id,
                    %error,
                    retry_in_ms = retry_delay.as_millis() as u64,
                    "Team 取消状态尚未持久化；保护门保持关闭并重试",
                );
                tokio::time::sleep(retry_delay).await;
                retry_delay = next_guard_retry_delay(retry_delay);
            }
        }
    }
    fail_pending_rework_tasks(coordinator, team_id, rework_reason).await;
}

/// 指标预算门：累计成本、墙钟或模型调用超过上限时停止下一阶段。
/// 如果预算状态无法核验，也停止调度并审计 team.budget_check_failed；不能把未知状态
/// 当作有预算继续消耗。返回 true 表示运行循环必须立即退出。
pub(super) async fn stop_if_budget_exhausted(
    coordinator: &Arc<TeamCoordinator>,
    team_id: &str,
) -> bool {
    let check = workswarm_metrics::team_budget_exhaustion(coordinator, team_id)
        .await
        .map_err(|error| error.to_string());
    match budget_guard_decision(check) {
        BudgetGuardDecision::Continue => false,
        BudgetGuardDecision::Exhausted(reason) => {
            stop_team_for_guard(
                coordinator,
                team_id,
                "team.budget_exhausted",
                &reason,
                "团队预算耗尽，返工未产出新版本",
            )
            .await;
            true
        }
        BudgetGuardDecision::Unavailable(error) => {
            tracing::error!(team_id = %team_id, %error, "指标预算门无法读取当前预算，fail-closed 停止调度");
            stop_team_for_guard(
                coordinator,
                team_id,
                "team.budget_check_failed",
                "指标预算无法核验，已停止调度并请求取消以保护预算上限",
                "指标预算无法核验，团队已停止调度",
            )
            .await;
            true
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // 测试模块历史位置靠前；移动会打乱同文件阅读顺序
mod budget_guard_tests {
    use super::{budget_guard_decision, BudgetGuardDecision};
    use std::time::Duration;

    #[test]
    fn budget_guard_distinguishes_continue_exhausted_and_unavailable() {
        assert_eq!(
            budget_guard_decision(Ok(None)),
            BudgetGuardDecision::Continue
        );
        assert_eq!(
            budget_guard_decision(Ok(Some("超出成本上限".to_string()))),
            BudgetGuardDecision::Exhausted("超出成本上限".to_string())
        );
        assert_eq!(
            budget_guard_decision(Err("指标读取失败".to_string())),
            BudgetGuardDecision::Unavailable("指标读取失败".to_string())
        );
        assert_eq!(
            super::next_guard_retry_delay(Duration::from_millis(100)),
            Duration::from_millis(200)
        );
        assert_eq!(
            super::next_guard_retry_delay(Duration::from_secs(4)),
            Duration::from_secs(5)
        );
    }
}

/// 团队运行循环：run_phase 阶段推进；人节点等待窗口内轮询（结果落盘即唤醒；cancel 即终止）。
///
/// 每轮外层迭代重建 worker 注册表（steer/replace 修改角色规格后新阶段生效）。
async fn run_bounded_team_phase(
    coordinator: &Arc<TeamCoordinator>,
    team_id: &str,
    registry: &owo_agent_core::goal::WorkerRegistry,
) -> owo_agent_core::workswarm::WorkSwarmResult<owo_agent_core::PhaseOutcome> {
    let (budget_done, mut budget_done_rx) = tokio::sync::watch::channel(false);
    let budget_coordinator = Arc::clone(coordinator);
    let budget_team_id = team_id.to_string();
    let budget_watch = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = budget_done_rx.changed() => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    if stop_if_budget_exhausted(&budget_coordinator, &budget_team_id).await { break; }
                }
            }
        }
    });
    let phase_result = coordinator.run_phase(team_id, registry).await;
    let _ = budget_done.send(true);
    let _ = budget_watch.await;

    phase_result
}

pub(crate) async fn run_team_loop(
    state: Arc<AppState>,
    coordinator: Arc<TeamCoordinator>,
    team_id: String,
) {
    // 原子竞争循环所有权：并发 spawn（如「返工 + 幂等重放」）只能有一个循环运行，
    // 否则第二个循环会把第一个循环 Claimed 的步骤判成死锁并终止团队。
    let Some(_alive) = LoopAliveGuard::try_acquire(&coordinator, &team_id) else {
        tracing::info!(team_id = %team_id, "已有团队运行循环，跳过重复启动");
        return;
    };
    // 七期（第二路）：团队取消令牌 → 运行中 Worker 的即时中断桥。`wait_cancel`
    // 只在真实取消（令牌值变 true）时置位共享 abort 标志（Worker 在回合边界协作
    // 中断）；发送端随团队收尾关闭返回 false，不置位（运行已结束，无需中断）。
    // 桥存活期 = 团队运行循环存活期（令牌由协调器持有，跨注册表重建共用一份标志）。
    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let token = coordinator.cancel_token(&team_id);
        let flag = Arc::clone(&cancel_flag);
        tokio::spawn(async move {
            if owo_agent_core::wait_cancel(&token).await {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                tracing::info!("workswarm 取消桥：运行中 Worker 中断标志已置位");
            }
        });
    }
    let verification_root =
        match project_workspace::load_binding_checked(coordinator.run_dir(), &team_id) {
            Ok(Some(binding)) => binding.scope().root,
            Ok(None) => state.workspace.clone(),
            Err(error) => {
                tracing::error!(team_id = %team_id, %error, "Team 工作区绑定无法校验，拒绝启动");
                stop_team_for_guard(
                    &coordinator,
                    &team_id,
                    "team.workspace_binding_invalid",
                    "工作区绑定无法校验，团队未执行任何任务",
                    "工作区绑定无法校验，返工未产出新版本",
                )
                .await;
                return;
            }
        };
    if let Err(error) = coordinator.bind_verification_workspace(&team_id, &verification_root) {
        tracing::warn!(team_id = %team_id, %error, "workswarm 验证工作区未绑定；WorkspacePaths 要求将在 DeliveryGate 中失败关闭");
    }
    let lifecycle_journal =
        workswarm_metrics::TeamLifecycleMetricsJournal::for_team(coordinator.run_dir(), &team_id);
    let mut lifecycle_sequence = 0u64;
    let mut backoff = Duration::from_secs(1);
    loop {
        lifecycle_sequence = lifecycle_sequence.saturating_add(1);
        let phase_sequence = lifecycle_sequence;
        // 五期（第三路）：指标预算门（外层调度点；门闩内调度点见下方 latch 循环）。
        if stop_if_budget_exhausted(&coordinator, &team_id).await {
            return;
        }
        let registry_timer =
            workswarm_metrics::TeamLifecycleTimer::start(phase_sequence, "registry_build");
        let Some(registry) = build_run_registry(&coordinator, &state, &team_id, &cancel_flag).await
        else {
            registry_timer.finish(&lifecycle_journal, &team_id, "failed");
            tracing::error!(team_id = %team_id, "workswarm 运行循环：worker 注册表构建失败，取消本次运行");
            stop_team_for_guard(
                &coordinator,
                &team_id,
                "team.registry_build_failed",
                "Worker 注册表无法安全构建，团队未继续派发任务",
                "团队运行器初始化失败，返工未产出新版本",
            )
            .await;
            return;
        };
        registry_timer.finish(&lifecycle_journal, &team_id, "succeeded");
        let phase_timer =
            workswarm_metrics::TeamLifecycleTimer::start(phase_sequence, "phase_orchestration");
        let phase_result = run_bounded_team_phase(&coordinator, &team_id, &registry).await;
        let phase_epoch = coordinator.current_execution_epoch(&team_id);
        let phase_outcome = match &phase_result {
            Err(_) => "failed",
            Ok(owo_agent_core::PhaseOutcome::MoreReady) => "more_ready",
            Ok(owo_agent_core::PhaseOutcome::Done) => "done",
            Ok(owo_agent_core::PhaseOutcome::AwaitingHuman { .. }) => "awaiting_human",
            Ok(owo_agent_core::PhaseOutcome::Failed) => "failed",
            Ok(owo_agent_core::PhaseOutcome::Aborted) => "aborted",
            Ok(owo_agent_core::PhaseOutcome::Finished) => "finished",
        };
        phase_timer.finish_with_phase_epoch(
            &lifecycle_journal,
            &team_id,
            phase_outcome,
            Some(phase_epoch),
        );
        match phase_result {
            Err(e) => {
                // 存储/IO 异常：退避重试，避免热循环打爆磁盘。
                tracing::warn!(team_id = %team_id, %e, "run_phase 失败，退避重试");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
            Ok(outcome) => {
                match outcome {
                    owo_agent_core::PhaseOutcome::MoreReady => {
                        backoff = Duration::from_secs(1);
                        continue;
                    }
                    owo_agent_core::PhaseOutcome::Done => {
                        let delivery_timer = workswarm_metrics::TeamLifecycleTimer::start(
                            phase_sequence,
                            "delivery_finalize",
                        );
                        let _delivery_lease =
                            super::acquire_workspace_delivery_lease(&verification_root).await;
                        let result = coordinator.finalize_success(&team_id).await;
                        delivery_timer.finish(
                            &lifecycle_journal,
                            &team_id,
                            if result.is_ok() {
                                "succeeded"
                            } else {
                                "failed"
                            },
                        );
                        if matches!(
                            &result,
                            Err(owo_agent_core::workswarm::WorkSwarmError::DeliveryRepair { .. })
                        ) {
                            tracing::info!(team_id = %team_id, "交付门已派发定向返工，继续调度");
                            continue;
                        }
                        if let Err(e) = result {
                            tracing::error!(team_id = %team_id, %e, "workswarm 收尾失败");
                        }
                        return;
                    }
                    owo_agent_core::PhaseOutcome::Failed
                    | owo_agent_core::PhaseOutcome::Aborted
                    | owo_agent_core::PhaseOutcome::Finished => {
                        fail_pending_rework_tasks(
                            &coordinator,
                            &team_id,
                            "团队执行在返工产出新版本前终止",
                        )
                        .await;
                        return;
                    }
                    owo_agent_core::PhaseOutcome::AwaitingHuman { waits } => {
                        tracing::info!(team_id = %team_id, ?waits, "团队等待人节点，进入事件门闩");
                        // 订阅状态变化后重查一次阶段，关闭“人结果恰好在进入门闩时提交”的竞态。
                        // 后续只有进度、取消或 1s 预算心跳会唤醒；等待期间不重复运行调度器。
                        let mut progress_rx = coordinator.subscribe_progress(&team_id);
                        let cancel = coordinator.cancel_token(&team_id);
                        let mut cancel_rx = cancel.rx();
                        'latch: loop {
                            if cancel.is_cancelled() {
                                if let Err(e) = coordinator
                                    .apply_steer(&team_id, &SteerCommand::Cancel)
                                    .await
                                {
                                    tracing::error!(team_id = %team_id, %e, "取消收尾失败");
                                }
                                fail_pending_rework_tasks(
                                    &coordinator,
                                    &team_id,
                                    "团队已取消，返工未产出新版本",
                                )
                                .await;
                                return;
                            }
                            // 人节点等待期间仍守住墙钟预算；其余时候完全由状态事件唤醒。
                            if stop_if_budget_exhausted(&coordinator, &team_id).await {
                                return;
                            }
                            let observed_progress = *progress_rx.borrow_and_update();
                            lifecycle_sequence = lifecycle_sequence.saturating_add(1);
                            let latch_sequence = lifecycle_sequence;
                            let phase_timer = workswarm_metrics::TeamLifecycleTimer::start(
                                latch_sequence,
                                "phase_orchestration",
                            );
                            let phase_result =
                                run_bounded_team_phase(&coordinator, &team_id, &registry).await;
                            let phase_epoch = coordinator.current_execution_epoch(&team_id);
                            let phase_outcome = match &phase_result {
                                Err(_) => "failed",
                                Ok(owo_agent_core::PhaseOutcome::MoreReady) => "more_ready",
                                Ok(owo_agent_core::PhaseOutcome::Done) => "done",
                                Ok(owo_agent_core::PhaseOutcome::AwaitingHuman { .. }) => {
                                    "awaiting_human"
                                }
                                Ok(owo_agent_core::PhaseOutcome::Failed) => "failed",
                                Ok(owo_agent_core::PhaseOutcome::Aborted) => "aborted",
                                Ok(owo_agent_core::PhaseOutcome::Finished) => "finished",
                            };
                            phase_timer.finish_with_phase_epoch(
                                &lifecycle_journal,
                                &team_id,
                                phase_outcome,
                                Some(phase_epoch),
                            );
                            match phase_result {
                                Ok(owo_agent_core::PhaseOutcome::AwaitingHuman { .. }) => loop {
                                    tokio::select! {
                                        changed = progress_rx.changed() => {
                                            if changed.is_err() || *progress_rx.borrow_and_update() != observed_progress {
                                                continue 'latch;
                                            }
                                        }
                                        changed = cancel_rx.changed() => {
                                            if changed.is_err() || *cancel_rx.borrow() {
                                                if let Err(e) = coordinator
                                                    .apply_steer(&team_id, &SteerCommand::Cancel)
                                                    .await
                                                {
                                                    tracing::error!(team_id = %team_id, %e, "取消收尾失败");
                                                }
                                                fail_pending_rework_tasks(
                                                    &coordinator,
                                                    &team_id,
                                                    "团队已取消，返工未产出新版本",
                                                )
                                                .await;
                                                return;
                                            }
                                        }
                                        _ = tokio::time::sleep(Duration::from_secs(1)) => {
                                            if stop_if_budget_exhausted(&coordinator, &team_id).await {
                                                return;
                                            }
                                        }
                                    }
                                },
                                Ok(owo_agent_core::PhaseOutcome::Done) => {
                                    let delivery_timer =
                                        workswarm_metrics::TeamLifecycleTimer::start(
                                            latch_sequence,
                                            "delivery_finalize",
                                        );
                                    let _delivery_lease =
                                        super::acquire_workspace_delivery_lease(&verification_root)
                                            .await;
                                    let result = coordinator.finalize_success(&team_id).await;
                                    delivery_timer.finish(
                                        &lifecycle_journal,
                                        &team_id,
                                        if result.is_ok() {
                                            "succeeded"
                                        } else {
                                            "failed"
                                        },
                                    );
                                    if matches!(&result, Err(owo_agent_core::workswarm::WorkSwarmError::DeliveryRepair { .. })) {
                                    break 'latch;
                                }
                                    return;
                                }
                                Ok(owo_agent_core::PhaseOutcome::MoreReady) => break, // 外层循环重建注册表继续
                                Ok(_) => {
                                    fail_pending_rework_tasks(
                                        &coordinator,
                                        &team_id,
                                        "团队执行在返工产出新版本前终止",
                                    )
                                    .await;
                                    return;
                                } // 终态
                                Err(e) => {
                                    tracing::warn!(team_id = %team_id, %e, "门闩等待中 run_phase 失败，退避");
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
