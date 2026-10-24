//! Build the minimal executable subplan for one WorkSwarm phase.
//!
//! Keep only active work reachable from the claimed frontier plus successful direct
//! dependency anchors. Historical successful nodes are terminal facts; copying the whole
//! completed DAG into every phase is unnecessary and makes later phases increasingly costly.
use super::*;
use std::collections::{HashMap, HashSet, VecDeque};

pub(super) fn filter_pre_fanout_ready_steps(
    meta: &RunMeta,
    state: &GoalRunState,
    ready: &[StepSpec],
) -> Vec<StepSpec> {
    let assignment_pending = meta.parallel
        && !state
            .plan
            .steps
            .iter()
            .any(|step| step.input.get("assigned_task_id").is_some());
    if !assignment_pending {
        return ready.to_vec();
    }
    ready
        .iter()
        .filter(|step| {
            let role = meta
                .roles
                .iter()
                .find(|role| step.worker == format!("m-{}", role.role));
            role.is_none_or(|role| {
                role.is_orchestrator()
                    || (!super::task_graph::is_parallel_task_worker(role) && !role.is_integrator())
            })
        })
        .cloned()
        .collect()
}

pub(super) fn phase_subplan_step_ids(
    state: &GoalRunState,
    meta: &RunMeta,
    initial_ready: &[StepSpec],
    defer_parallel_fanout: bool,
) -> HashSet<String> {
    let mut dependents = HashMap::<String, Vec<String>>::new();
    let mut steps_by_id = HashMap::<&str, &StepSpec>::with_capacity(state.plan.steps.len());
    for step in &state.plan.steps {
        steps_by_id.insert(step.id.as_str(), step);
        for dependency in &step.depends_on {
            dependents
                .entry(dependency.clone())
                .or_default()
                .push(step.id.clone());
        }
    }

    let mut resumable_non_human = HashSet::new();
    let mut pending_dependencies = HashMap::<String, usize>::new();
    for step in &state.plan.steps {
        let is_human = TeamCoordinator::role_spec_of_member(meta, &step.worker)
            .is_ok_and(|role| role.assignee == "human");
        let resumable = state
            .records
            .get(&step.id)
            .is_some_and(|record| record.status.can_resume());
        if !resumable || is_human {
            continue;
        }
        resumable_non_human.insert(step.id.clone());
        let unsatisfied = step
            .depends_on
            .iter()
            .filter(|dependency| {
                state
                    .records
                    .get(dependency.as_str())
                    .is_none_or(|record| record.status != StepStatus::Succeeded)
            })
            .count();
        pending_dependencies.insert(step.id.clone(), unsatisfied);
    }

    let mut active_ids = initial_ready
        .iter()
        .map(|step| step.id.clone())
        .collect::<HashSet<_>>();
    let mut frontier = initial_ready
        .iter()
        .map(|step| step.id.clone())
        .collect::<VecDeque<_>>();

    if !defer_parallel_fanout {
        while let Some(completed_id) = frontier.pop_front() {
            for dependent_id in dependents.get(&completed_id).into_iter().flatten() {
                if active_ids.contains(dependent_id) || !resumable_non_human.contains(dependent_id)
                {
                    continue;
                }
                let Some(remaining) = pending_dependencies.get_mut(dependent_id) else {
                    continue;
                };
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    active_ids.insert(dependent_id.clone());
                    frontier.push_back(dependent_id.clone());
                }
            }
        }
    }

    let mut subplan_ids = active_ids.clone();
    for active_id in &active_ids {
        let Some(step) = steps_by_id.get(active_id.as_str()) else {
            continue;
        };
        for dependency in &step.depends_on {
            if state
                .records
                .get(dependency)
                .is_some_and(|record| record.status == StepStatus::Succeeded)
            {
                subplan_ids.insert(dependency.clone());
            }
        }
    }
    subplan_ids
}

pub(super) fn phase_subplan_steps(
    state: &GoalRunState,
    subplan_ids: &HashSet<String>,
    epoch: u64,
) -> Vec<StepSpec> {
    state
        .plan
        .steps
        .iter()
        .filter(|step| subplan_ids.contains(&step.id))
        .cloned()
        .map(|mut step| {
            if state.records[&step.id].status == StepStatus::Succeeded {
                // Successful dependency anchors are immutable phase-local facts. Drop
                // historical edges so the phase need not carry their transitive history.
                step.depends_on.clear();
            } else if let Some(object) = step.input.as_object_mut() {
                let workswarm = object
                    .entry("_workswarm".to_string())
                    .or_insert_with(|| json!({}));
                if let Some(workswarm) = workswarm.as_object_mut() {
                    workswarm.insert("phase_epoch".to_string(), json!(epoch));
                    let attempt_id = state.records[&step.id]
                        .attempt_id
                        .clone()
                        .expect("claimed task must have host attempt identity");
                    workswarm.insert("attempt_id".to_string(), json!(attempt_id));
                }
            }
            step
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Plan, StepSpec};

    fn state_with_steps(steps: Vec<StepSpec>) -> GoalRunState {
        let mut plan = Plan::new("plan-phase", "goal-phase");
        plan.steps = steps;
        GoalRunState::new(Goal::new("goal-phase", "phase closure"), plan)
    }

    fn meta_with_human_role() -> RunMeta {
        let mut human = RoleSpec::agent("human");
        human.assignee = "human".to_string();
        RunMeta {
            team_id: "team-phase".to_string(),
            correlation_id: "corr-phase".to_string(),
            roles: vec![human],
            template_id: None,
            budgets: BTreeMap::new(),
            parallel: false,
        }
    }

    #[test]
    fn subplan_keeps_reachable_work_and_successful_dependency_anchors_only() {
        let mut ancient = StepSpec::new("ancient", "m-writer");
        ancient.depends_on = vec!["outside-history".to_string()];
        let outside_history = StepSpec::new("outside-history", "m-writer");
        let mut old = StepSpec::new("old", "m-writer");
        old.depends_on = vec!["ancient".to_string()];
        let unrelated = StepSpec::new("unrelated-success", "m-writer");
        let mut root = StepSpec::new("root", "m-writer");
        root.depends_on = vec!["old".to_string()];
        let mut next = StepSpec::new("next", "m-writer");
        next.depends_on = vec!["root".to_string()];
        let mut human = StepSpec::new("human", "m-human");
        human.depends_on = vec!["root".to_string()];
        let mut after_human = StepSpec::new("after-human", "m-writer");
        after_human.depends_on = vec!["human".to_string()];
        let mut join = StepSpec::new("join", "m-writer");
        join.depends_on = vec!["next".to_string(), "human".to_string()];
        let mut state = state_with_steps(vec![
            outside_history,
            ancient,
            old,
            unrelated,
            root.clone(),
            next,
            human,
            after_human,
            join,
        ]);
        state.records.get_mut("ancient").unwrap().status = StepStatus::Succeeded;
        state.records.get_mut("old").unwrap().status = StepStatus::Succeeded;
        state.records.get_mut("outside-history").unwrap().status = StepStatus::Succeeded;
        state.records.get_mut("unrelated-success").unwrap().status = StepStatus::Succeeded;

        let ids = phase_subplan_step_ids(&state, &meta_with_human_role(), &[root], false);
        let expected = HashSet::from(["old".to_string(), "root".to_string(), "next".to_string()]);
        assert_eq!(ids, expected);
        let steps = phase_subplan_steps(&state, &ids, 7);
        assert!(steps
            .iter()
            .find(|step| step.id == "old")
            .unwrap()
            .depends_on
            .is_empty());
        let mut plan = Plan::new("phase-only", "goal-phase");
        plan.steps = steps;
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn defer_parallel_fanout_never_runs_unassigned_workers_or_integrators_early() {
        let mut orchestrator = RoleSpec::agent("路线总控");
        orchestrator.capabilities = vec!["orchestrator".to_string()];
        let mut worker = RoleSpec::agent("图像制作");
        worker.capabilities = vec!["task_worker".to_string()];
        let mut integrator = RoleSpec::agent("收尾交付");
        integrator.capabilities = vec!["integrator".to_string()];
        let meta = RunMeta {
            team_id: "team-phase".to_string(),
            correlation_id: "corr-phase".to_string(),
            roles: vec![orchestrator, worker, integrator],
            template_id: None,
            budgets: BTreeMap::new(),
            parallel: true,
        };
        let orchestrator_step = StepSpec::new("orchestrate", "m-路线总控");
        let worker_step = StepSpec::new("worker", "m-图像制作");
        let integrator_step = StepSpec::new("integrate", "m-收尾交付");
        let ready = vec![orchestrator_step, worker_step, integrator_step];
        let state = state_with_steps(ready.clone());

        let filtered = filter_pre_fanout_ready_steps(&meta, &state, &ready);
        assert_eq!(
            filtered
                .iter()
                .map(|step| step.id.as_str())
                .collect::<Vec<_>>(),
            vec!["orchestrate"]
        );

        let mut assigned = StepSpec::new("assigned-worker", "m-图像制作");
        assigned.input = serde_json::json!({"assigned_task_id":"task-a"});
        let assigned_state = state_with_steps(vec![assigned.clone()]);
        assert_eq!(
            filter_pre_fanout_ready_steps(&meta, &assigned_state, &[assigned]).len(),
            1,
            "task workers resume once TaskGraph assignments are installed"
        );
    }

    #[test]
    fn defer_parallel_fanout_keeps_only_claimed_roots_and_successful_anchors() {
        let old = StepSpec::new("old", "m-writer");
        let mut lead = StepSpec::new("lead", "m-lead");
        lead.depends_on = vec!["old".to_string()];
        let mut writer = StepSpec::new("writer", "m-writer");
        writer.depends_on = vec!["lead".to_string()];
        let mut state = state_with_steps(vec![old.clone(), lead.clone(), writer]);
        state.records.get_mut("old").unwrap().status = StepStatus::Succeeded;

        let ids = phase_subplan_step_ids(&state, &meta_with_human_role(), &[lead], true);
        let expected = HashSet::from(["old".to_string(), "lead".to_string()]);
        assert_eq!(ids, expected);
    }

    #[test]
    fn join_waits_for_every_runnable_dependency_and_excludes_human_gated_work() {
        let left = StepSpec::new("left", "m-writer");
        let right = StepSpec::new("right", "m-writer");
        let human = StepSpec::new("human", "m-human");
        let mut join = StepSpec::new("join", "m-writer");
        join.depends_on = vec!["left".to_string(), "right".to_string()];
        let mut after_join = StepSpec::new("after-join", "m-writer");
        after_join.depends_on = vec!["join".to_string()];
        let mut human_gated = StepSpec::new("human-gated", "m-writer");
        human_gated.depends_on = vec!["left".to_string(), "human".to_string()];
        let state = state_with_steps(vec![
            left.clone(),
            right.clone(),
            human,
            join,
            after_join,
            human_gated,
        ]);

        let ids = phase_subplan_step_ids(&state, &meta_with_human_role(), &[left, right], false);
        let expected = HashSet::from([
            "left".to_string(),
            "right".to_string(),
            "join".to_string(),
            "after-join".to_string(),
        ]);
        assert_eq!(ids, expected);
    }

    #[test]
    fn long_linear_frontier_closure_uses_dependency_edges_without_rescanning_waves() {
        let count = 512;
        let mut steps = Vec::with_capacity(count);
        for index in 0..count {
            let mut step = StepSpec::new(format!("step-{index}"), "m-writer");
            if index > 0 {
                step.depends_on = vec![format!("step-{}", index - 1)];
            }
            steps.push(step);
        }
        let root = steps[0].clone();
        let state = state_with_steps(steps);

        let ids = phase_subplan_step_ids(&state, &meta_with_human_role(), &[root], false);
        assert_eq!(ids.len(), count);
    }
}
