//! TaskGraph scheduling and integration policy, separate from untrusted graph validation.
use super::task_graph::{is_parallel_writer_name, write_paths_overlap};
use super::util::worker_role;
use super::RoleSpec;
use crate::plan::StepSpec;
use serde_json::Value;

/// Validate a bounded DAG separately from worker capacity, then bind omitted workers.
pub(super) fn is_independent_reviewer_role(role: &RoleSpec) -> bool {
    // 并行写角色（w1/w2…，或声明 task_worker 能力）即使带 review 能力也不能充当
    // 独立评审：它自己就在改动被评审的产物，独立性不成立。
    role.is_reviewer()
        && !role.is_orchestrator()
        && role.role != "leader"
        && !is_parallel_writer_name(&role.role)
}

pub(super) const PARALLEL_LEADER_HOST_MANIFEST_SKIP_REASON: &str =
    "host_manifest:independent_task_graph";

pub(super) fn host_manifest_can_replace_integration(
    parallel_enabled: bool,
    role: &str,
    integration_required: bool,
) -> bool {
    parallel_enabled && !integration_required && matches!(role, "leader" | "project_integrator")
}

pub(super) fn should_enable_parallel_assignment(explicit: bool, roles: &[RoleSpec]) -> bool {
    // 显式请求优先；否则只有"有编排者 + 有可派发的任务工人"才自动启用，
    // 避免纯评审/单角色团队被误判为并行。
    explicit
        || (roles.iter().any(RoleSpec::is_orchestrator)
            && roles.iter().any(super::task_graph::is_parallel_task_worker))
}

/// Rebind configured integrator and reviewer steps to every dynamic task, including additional tasks
/// assigned to a worker whose first task reuses the legacy s-wN step ID.
pub(super) fn bind_dynamic_follow_up_dependencies(
    steps: &mut [StepSpec],
    assignment_step_ids: &[String],
    scheduled_reviewer_step_ids: &[String],
) {
    let available_step_ids = steps
        .iter()
        .map(|step| step.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let is_integrator = |step: &StepSpec| {
        worker_role(&step.worker).is_some_and(|role| {
            role == "leader"
                || role == "project_integrator"
                || step
                    .input
                    .get("_workswarm")
                    .and_then(|meta| meta.get("role_is_integrator"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
    };
    let integration_step_ids = steps
        .iter()
        .filter(|step| is_integrator(step))
        .map(|step| step.id.clone())
        .collect::<Vec<_>>();
    for step in steps {
        let is_scheduled_reviewer = scheduled_reviewer_step_ids.contains(&step.id);
        let step_is_integrator = is_integrator(step);
        if is_scheduled_reviewer || step_is_integrator {
            // Review the integrated source snapshot: validators wait for every dynamic
            // task and every integration step. Integrators consume candidate artifacts
            // without waiting for review, so the final review cannot precede later edits.
            let mut dependencies = step
                .depends_on
                .iter()
                .filter(|dependency| {
                    available_step_ids.contains(*dependency)
                        && !(step_is_integrator
                            && scheduled_reviewer_step_ids.contains(*dependency))
                })
                .cloned()
                .collect::<Vec<_>>();
            dependencies.extend(assignment_step_ids.iter().cloned());
            if is_scheduled_reviewer {
                dependencies.extend(integration_step_ids.iter().cloned());
            }
            dependencies.retain(|dependency| dependency != &step.id);
            dependencies.sort();
            dependencies.dedup();
            step.depends_on = dependencies;
        }
    }
}

/// Dynamic tasks with explicit, disjoint write scopes need no extra model-based
/// integration pass solely because one consumes another task's artifact: the DAG
/// carries that dependency and DeliveryGate validates the final workspace. Shared
/// write surfaces or contract references still require explicit integration; an
/// unscoped dependency stays conservative because the host cannot prove isolation.
pub(super) fn parallel_tasks_require_integration(steps: &[StepSpec]) -> bool {
    let tasks = steps
        .iter()
        .filter(|step| {
            step.input
                .get("assigned_task_id")
                .and_then(Value::as_str)
                .is_some()
        })
        .collect::<Vec<_>>();
    if tasks.is_empty() {
        return true;
    }
    for (index, task) in tasks.iter().enumerate() {
        let paths = task
            .input
            .get("assigned_write_paths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .collect::<Vec<_>>();
        for dependency in &task.depends_on {
            let Some(upstream) = tasks.iter().find(|candidate| candidate.id == *dependency) else {
                continue;
            };
            let upstream_paths = upstream
                .input
                .get("assigned_write_paths")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|path| !path.trim().is_empty())
                .collect::<Vec<_>>();
            if paths.is_empty() || upstream_paths.is_empty() {
                return true;
            }
        }
        let contracts = task
            .input
            .get("assigned_contract_refs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<std::collections::HashSet<_>>();
        for other in tasks.iter().skip(index + 1) {
            let other_paths = other
                .input
                .get("assigned_write_paths")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|path| !path.trim().is_empty());
            if paths.iter().any(|left| {
                other_paths
                    .clone()
                    .any(|right| write_paths_overlap(left, right))
            }) {
                return true;
            }
            let other_contracts = other
                .input
                .get("assigned_contract_refs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            if other_contracts
                .into_iter()
                .any(|item| contracts.contains(item))
            {
                return true;
            }
        }
    }
    false
}
