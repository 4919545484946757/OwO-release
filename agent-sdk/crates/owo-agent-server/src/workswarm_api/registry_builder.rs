//! Team worker registry assembly: role profiles, scoped tools, budgets, leases and metering.
//!
//! This module owns construction of the executable worker surface. The runtime loop
//! consumes the completed registry and remains responsible for lifecycle transitions.
use super::workers::*;
use super::write_lease::{manager_for_workspace, WriteLease, WriteScope};
use super::{project_workspace, workspace_change_tracker, workswarm_metrics};
use owo_agent_core::goal::{Worker, WorkerRegistry};
use owo_agent_core::worker_profile::{intersect_paths, WorkerProfile};
use owo_agent_core::workswarm::{RoleWorker, TeamCoordinator};
use owo_agent_server::AppState;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

fn resolved_worker_budget(
    resolved: &std::collections::BTreeMap<String, usize>,
    template: &[owo_agent_core::builtin_team_templates::RoleBudget],
    role: &str,
) -> usize {
    resolved.get(role).copied().unwrap_or_else(|| {
        template
            .iter()
            .find(|budget| budget.role == role)
            .map(|budget| budget.budget_calls)
            .unwrap_or(0)
    })
}

/// 声明写范围与团队绑定无交集时的不可达写白名单哨兵（工具/审批层据此拒绝一切写入）。
const NO_WRITE_SCOPE_MARKER: &str = ".owo-no-write-scope";

/// 按角色 worker 名解析内层 worker（"agent"/缺省 = 模型驱动）。
///
/// 五期（第三路）：agent 角色接收 `model_calls` 计数器（MeasuredProvider 注入；
/// 仅指标用途，不影响执行行为）。
/// 六期（第二路）：`scope` 为团队工作区绑定（None = 全局工作区，行为不变）。
/// 七期（第二路）：agent 角色带角色画像（工具面/只读/回合上限）+ critic 代理 +
/// 团队取消桥标志 + 「角色 ∩ 绑定」写白名单交集（内置 echo/sleep/fail 不受影响）。
#[allow(clippy::too_many_arguments)]
fn inner_worker_for(
    state: &AppState,
    worker_name: Option<&str>,
    model_calls: Option<&Arc<AtomicU64>>,
    request_usage: Option<&Arc<workswarm_metrics::RequestUsageCollector>>,
    team_request_budget: Option<&Arc<workswarm_metrics::TeamModelRequestBudget>>,
    scope: Option<&project_workspace::WorkspaceScope>,
    profile: &WorkerProfile,
    is_critic: bool,
    cancel_flag: &Arc<AtomicBool>,
    write_allowed: Vec<PathBuf>,
    coordinator: &Arc<TeamCoordinator>,
    team_id: &str,
    parent_session_id: Option<&str>,
    role: &str,
) -> Option<Arc<dyn Worker>> {
    match worker_name.map(str::trim).filter(|w| !w.is_empty()) {
        Some("echo") => Some(Arc::new(EchoWorker)),
        Some("sleep") => Some(Arc::new(SleepWorker)),
        Some("fail") => Some(Arc::new(FailWorker)),
        None | Some("agent") => {
            // 绑定后：Worker 实际运行目录 = 项目绑定目录。
            let workspace = scope
                .map(|s| s.root.clone())
                .unwrap_or_else(|| state.workspace.clone());
            let workspace_scope = scope.cloned();
            Some(Arc::new(AgentSubagentWorker {
                agent: Arc::clone(&state.agent),
                workspace,
                model_calls: model_calls.cloned(),
                request_usage: request_usage.cloned(),
                team_request_budget: team_request_budget.cloned(),
                workspace_scope,
                profile: Some(profile.clone()),
                is_critic,
                cancel_flag: Some(Arc::clone(cancel_flag)),
                write_allowed,
                coordinator: Arc::clone(coordinator),
                session_store: Arc::clone(&state.store),
                parent_session_id: parent_session_id.map(str::to_string),
                team_id: team_id.to_string(),
                role: role.to_string(),
            }))
        }
        Some(_) => None,
    }
}

fn worker_kind_uses_agent_provider(worker_kind: &str) -> bool {
    !matches!(worker_kind, "echo" | "sleep" | "fail")
}

/// 构建团队运行 worker 注册表（成员名 → MeasuredRoleWorker(RoleWorker(Tracked(inner)))）。
///
/// 五期（第三路）：每个角色 worker 外层包一层 [`workswarm_metrics::MeasuredRoleWorker`]——
/// span 级起止/墙钟/终态/失败原因/尝试序数/输出 Artifact + model_calls/token/费用，
/// 指标 JSONL 落盘 TeamRun 数据目录（`<run_dir>/<team_id>-metrics.jsonl`，重启可读）。
/// 包装在 RoleWorker 之外：span 覆盖「上下文切片组装 → 执行 → 产物登记」全窗口。
/// 失败返回 None（运行任务记录后退出）。
///
/// 七期（第二路）：
/// - 角色画像：模板 `budget_calls_per_role` → 真实 `max_turns`；`WorkerProfile::for_role`
///   决定每个角色实际可见工具面（注册表面即权限边界，不靠审批事后拒绝）；
/// - 范围写租约 + 变更追踪：写角色包 `TrackedRoleWorker`（未声明写范围 = 工作区级
///   全局互斥；声明 `write_paths` 且互不重叠 = 并发落盘；执行前后 git 快照 →
///   变更摘要/diff ref 落盘 → 白名单越界 `scope_violation`）；
/// - 取消桥：`cancel_flag` 由 run_team_loop 的令牌监听任务置位，Worker 协作中断。
pub(super) async fn build_run_registry(
    coordinator: &Arc<TeamCoordinator>,
    state: &AppState,
    team_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Option<WorkerRegistry> {
    let meta = coordinator.load_run_meta(team_id).ok()?;
    // 六期（第二路）：团队工作区绑定（cancel/retry/resume 后循环按迭代重读——
    // 绑定生命周期独立于运行状态，恢复后继续生效）。
    let scope = match project_workspace::load_binding_checked(coordinator.run_dir(), team_id) {
        Ok(binding) => binding.map(|binding| binding.scope()),
        Err(error) => {
            tracing::error!(team_id = %team_id, %error, "工作区绑定无法校验，拒绝构建 Team Worker 注册表");
            return None;
        }
    };
    // 七期（第二路）：运行元数据是注册表和权限/预算装配的必需输入；读取失败不得
    // 降级成「无模板、无全局模型调用上限」继续派发。
    let team_run = coordinator.get_team_run(team_id).await.ok()?;
    let parent_session_id = team_run.shared_context_refs.iter().find_map(|reference| {
        let hash = reference.strip_prefix("cas://sha256:")?;
        let snapshot = coordinator.cas().get_text(hash)?;
        serde_json::from_str::<serde_json::Value>(&snapshot)
            .ok()?
            .get("source_session_id")?
            .as_str()
            .map(str::to_string)
    });
    let budgets: Vec<owo_agent_core::builtin_team_templates::RoleBudget> = team_run
        .template_id
        .as_deref()
        .and_then(owo_agent_core::builtin_team_templates::descriptor)
        .map(|descriptor| descriptor.budget_calls_per_role)
        .unwrap_or_default();
    let journal = workswarm_metrics::MetricsJournal::for_team(coordinator.run_dir(), team_id);
    let request_reservation_journal =
        workswarm_metrics::RequestReservationJournal::for_team(coordinator.run_dir(), team_id);
    let team_request_limit = team_run.budget.get("max_model_calls");
    let team_request_budget = Some(Arc::new(match team_request_limit {
        Some(value) => workswarm_metrics::TeamModelRequestBudget::new(
            value.as_u64()?,
            request_reservation_journal,
        )
        .ok()?,
        None => {
            workswarm_metrics::TeamModelRequestBudget::new_scoped_only(request_reservation_journal)
                .ok()?
        }
    }));
    // 范围写租约按实际工作区共享：跨调度阶段和不同 TeamRun 仍能互斥重叠写面。
    // 未声明范围的写角色全局互斥；声明写范围且互不重叠的写角色可并发落盘。
    let tracking_root = scope
        .as_ref()
        .map(|s| s.root.as_path())
        .unwrap_or(state.workspace.as_path());
    let write_lease_manager = manager_for_workspace(tracking_root);
    let usage_tracker = Arc::new(workswarm_metrics::UsageAttributionTracker::default());
    let registry = WorkerRegistry::new();
    for r in &meta.roles {
        // Human nodes are satisfied by submit_human_result, never by a worker registry.
        if r.assignee == "human" {
            continue;
        }
        let member_id = format!("m-{}", r.role);
        let worker_kind = r
            .worker
            .as_deref()
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .unwrap_or("agent")
            .to_string();
        let model_calls =
            worker_kind_uses_agent_provider(&worker_kind).then(|| Arc::new(AtomicU64::new(0)));
        let request_usage = worker_kind_uses_agent_provider(&worker_kind)
            .then(|| Arc::new(workswarm_metrics::RequestUsageCollector::default()));
        // 角色画像：显式 write_paths 是写能力声明；并行 TaskGraph writer 槽位
        // 以可写工具面启动，再由每个任务的 host-validated capability scope 收窄。
        let budget_calls = resolved_worker_budget(&meta.budgets, &budgets, &r.role);
        let profile = WorkerProfile::for_team_role_spec(
            r,
            budget_calls,
            meta.parallel,
            meta.template_id.as_deref(),
        );
        let is_critic = r.is_reviewer();
        let is_writer = profile.is_writer();
        // 最终写面 = 角色白名单 ∩ 团队绑定白名单（角色白名单空 = 交由绑定决定；
        // 两侧都空 = 工作区内可写，仍受审批约束）。
        let tracking_root = scope
            .as_ref()
            .map(|s| s.root.clone())
            .unwrap_or_else(|| state.workspace.clone());
        let scope_allowed: Vec<PathBuf> = scope
            .as_ref()
            .map(|s| s.allowed.clone())
            .unwrap_or_default();
        // 十一期（二路）：角色级写范围优先（RoleSpec.write_paths，相对工作区根）；
        // 未声明沿用画像白名单（当前内置画像恒为空 = 工作区级）。
        let role_allowed =
            WorkerProfile::team_role_write_allowed_paths(r, &tracking_root, is_writer).ok()?;
        // 角色 ∩ 绑定：两侧都非空且无交集 = 该角色在此绑定下不可写任何文件
        // （哨兵路径保证工具/审批/租约三层一致拒绝；空白的「未约束」语义不被复用）。
        let write_allowed = if role_allowed.is_empty() {
            scope_allowed.clone()
        } else if scope_allowed.is_empty() {
            role_allowed
        } else {
            let intersection = intersect_paths(&role_allowed, &scope_allowed);
            if intersection.is_empty() {
                vec![tracking_root.join(NO_WRITE_SCOPE_MARKER)]
            } else {
                intersection
            }
        };
        let lease_waits =
            is_writer.then(|| Arc::new(workswarm_metrics::LeaseWaitTracker::default()));
        let lease = is_writer.then(|| {
            // 空写面 = 工作区级（全局互斥）；声明写面 = 范围租约（不重叠可并发）。
            let scope = if write_allowed.is_empty() {
                WriteScope::global()
            } else {
                WriteScope::from_paths(&write_allowed)
            };
            WriteLease::new(Arc::clone(&write_lease_manager), scope)
        });
        let inner = inner_worker_for(
            state,
            r.worker.as_deref(),
            model_calls.as_ref(),
            request_usage.as_ref(),
            team_request_budget.as_ref(),
            scope.as_ref(),
            &profile,
            is_critic,
            cancel_flag,
            write_allowed.clone(),
            coordinator,
            team_id,
            parent_session_id.as_deref(),
            &r.role,
        )?;
        // 追踪 + 租约只作用于写角色（读角色没有写工具不会改文件；并行读角色的
        // 快照窗口会误捕写角色的变更）。
        let tracking = is_writer.then(|| workspace_change_tracker::Tracker {
            root: tracking_root,
            run_dir: coordinator.run_dir().to_path_buf(),
            team_id: team_id.to_string(),
            role: r.role.clone(),
            allowed: write_allowed.clone(),
            // 八期（二路）：ChangeSet 基线快照进团队 CAS + 生成留痕审计。
            cas: coordinator.cas().clone(),
            audit: coordinator.audit_log(),
        });
        let inner: Arc<dyn Worker> = Arc::new(TrackedRoleWorker {
            inner,
            lease,
            lease_waits: lease_waits.clone(),
            tracking,
        });
        let role_worker = Arc::new(RoleWorker::new_with_capabilities(
            Arc::clone(coordinator),
            team_id.to_string(),
            member_id.clone(),
            r.role.clone(),
            r.capabilities.clone(),
            inner,
        ));
        let provider =
            worker_kind_uses_agent_provider(&worker_kind).then(|| state.agent.provider());
        registry.register(Arc::new(
            workswarm_metrics::MeasuredRoleWorker::new_with_usage_tracker(
                role_worker,
                Arc::clone(coordinator),
                journal.clone(),
                team_id.to_string(),
                member_id,
                r.role.clone(),
                worker_kind,
                provider,
                model_calls,
                Arc::clone(&usage_tracker),
                request_usage,
                lease_waits,
            ),
        ));
    }
    Some(registry)
}

// ---------------------------------------------------------------------------
// 请求模型

#[cfg(test)]
mod worker_kind_budget_tests {
    use super::{resolved_worker_budget, worker_kind_uses_agent_provider};
    use owo_agent_core::builtin_team_templates::RoleBudget;
    use std::collections::BTreeMap;

    #[test]
    fn resolved_role_budget_wins_and_template_budget_is_a_compatibility_fallback() {
        let template = vec![RoleBudget {
            role: "builder".to_string(),
            budget_calls: 9,
        }];
        let resolved = BTreeMap::from([("builder".to_string(), 4)]);
        assert_eq!(resolved_worker_budget(&resolved, &template, "builder"), 4);
        assert_eq!(
            resolved_worker_budget(&BTreeMap::new(), &template, "builder"),
            9
        );
        assert_eq!(
            resolved_worker_budget(&BTreeMap::new(), &template, "critic"),
            0
        );
    }

    #[test]
    fn every_fallback_agent_worker_is_metered_and_budgeted() {
        assert!(worker_kind_uses_agent_provider("agent"));
        assert!(worker_kind_uses_agent_provider("custom-agent"));
        assert!(!worker_kind_uses_agent_provider("echo"));
        assert!(!worker_kind_uses_agent_provider("sleep"));
        assert!(!worker_kind_uses_agent_provider("fail"));
    }
}
