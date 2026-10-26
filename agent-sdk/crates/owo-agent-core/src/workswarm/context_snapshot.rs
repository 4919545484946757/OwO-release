//! Bounded, phase-scoped immutable context reused by Team workers.
//!
//! The snapshot contains only inputs that remain stable for one scheduler phase:
//! ProjectSpace identity, role metadata, and immutable parent-session CAS refs.
//! GoalRunState, TeamContext facts, and ArtifactCatalog are deliberately reloaded
//! per step so newly completed dependencies and published facts remain visible.
use super::*;
use std::collections::{HashMap, VecDeque};
use tokio::sync::OnceCell;

const MAX_PHASE_CONTEXT_SNAPSHOTS: usize = 32;

pub(super) struct PhaseContextSnapshot {
    pub(super) project_id: String,
    pub(super) meta: RunMeta,
    pub(super) goal_objective: String,
    pub(super) steps_by_id: Arc<HashMap<String, StepSpec>>,
    shared_context_refs: Vec<String>,
    core_specs: OnceCell<Arc<Vec<Value>>>,
}

impl PhaseContextSnapshot {
    pub(super) fn new(
        space: ProjectSpace,
        meta: RunMeta,
        goal_objective: String,
        steps: Vec<StepSpec>,
        shared_context_refs: Vec<String>,
    ) -> Self {
        Self {
            project_id: space.project_id,
            meta,
            goal_objective,
            steps_by_id: Arc::new(
                steps
                    .into_iter()
                    .map(|step| (step.id.clone(), step))
                    .collect(),
            ),
            shared_context_refs,
            core_specs: OnceCell::new(),
        }
    }

    pub(super) async fn load_core_specs(&self, cas: &CasStore) -> WorkSwarmResult<Arc<Vec<Value>>> {
        let cas = cas.clone();
        let references = self.shared_context_refs.clone();
        let loaded = self
            .core_specs
            .get_or_try_init(|| async move {
                super::coord_context_slice::load_parent_core_specs(cas, references)
                    .await
                    .map(Arc::new)
            })
            .await?;
        Ok(Arc::clone(loaded))
    }
}

#[derive(Default)]
pub(super) struct PhaseContextSnapshotCache {
    entries: HashMap<String, (u64, Arc<PhaseContextSnapshot>)>,
    insertion_order: VecDeque<String>,
}

impl PhaseContextSnapshotCache {
    pub(super) fn install(&mut self, team_id: &str, epoch: u64, snapshot: PhaseContextSnapshot) {
        self.entries.remove(team_id);
        self.insertion_order.retain(|entry| entry != team_id);
        self.entries
            .insert(team_id.to_string(), (epoch, Arc::new(snapshot)));
        self.insertion_order.push_back(team_id.to_string());
        while self.entries.len() > MAX_PHASE_CONTEXT_SNAPSHOTS {
            let Some(expired) = self.insertion_order.pop_front() else {
                break;
            };
            self.entries.remove(&expired);
        }
    }

    pub(super) fn get(&self, team_id: &str, epoch: u64) -> Option<Arc<PhaseContextSnapshot>> {
        self.entries
            .get(team_id)
            .filter(|(cached_epoch, _)| *cached_epoch == epoch)
            .map(|(_, snapshot)| Arc::clone(snapshot))
    }

    pub(super) fn remove(&mut self, team_id: &str, epoch: u64) {
        if self
            .entries
            .get(team_id)
            .is_some_and(|(cached_epoch, _)| *cached_epoch == epoch)
        {
            self.entries.remove(team_id);
            self.insertion_order.retain(|entry| entry != team_id);
        }
    }
}

impl TeamCoordinator {
    #[allow(clippy::too_many_arguments)] // 快照安装是单点内部接口，字段来源彼此独立
    pub(super) fn install_phase_context_snapshot(
        &self,
        team_id: &str,
        epoch: u64,
        space: ProjectSpace,
        meta: RunMeta,
        goal_objective: String,
        steps: Vec<StepSpec>,
        shared_context_refs: Vec<String>,
    ) {
        self.phase_context_snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .install(
                team_id,
                epoch,
                PhaseContextSnapshot::new(space, meta, goal_objective, steps, shared_context_refs),
            );
    }

    pub(super) fn phase_context_snapshot(
        &self,
        team_id: &str,
        epoch: u64,
    ) -> Option<Arc<PhaseContextSnapshot>> {
        self.phase_context_snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(team_id, epoch)
    }

    pub(super) fn clear_phase_context_snapshot(&self, team_id: &str, epoch: u64) {
        self.phase_context_snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(team_id, epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(cas_ref: String) -> PhaseContextSnapshot {
        let space = serde_json::from_value(serde_json::json!({
            "project_id": "project-a",
            "version": 1,
            "created_at": "created",
            "updated_at": "updated"
        }))
        .unwrap();
        PhaseContextSnapshot::new(
            space,
            RunMeta {
                team_id: "team-a".to_string(),
                correlation_id: "correlation-a".to_string(),
                roles: Vec::new(),
                template_id: None,
                template_source: None,
                budgets: Default::default(),
                parallel: false,
            },
            "objective".to_string(),
            vec![StepSpec::new("step-a", "member-a")],
            vec![cas_ref],
        )
    }

    #[tokio::test]
    async fn parent_core_specs_are_shared_by_concurrent_context_assemblies() {
        let dir = tempfile::tempdir().unwrap();
        let cas = CasStore::new(dir.path().to_path_buf()).unwrap();
        let hash = cas
            .put(
                serde_json::json!({
                    "kind": "source_session_context_v1",
                    "core_spec": {"constraint": "parent source"}
                })
                .to_string()
                .as_bytes(),
            )
            .unwrap();
        let snapshot = snapshot(format!("cas://sha256:{hash}"));

        let (first, second) = tokio::join!(
            snapshot.load_core_specs(&cas),
            snapshot.load_core_specs(&cas),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            first.as_slice(),
            &[serde_json::json!({"constraint": "parent source"})]
        );
    }

    #[test]
    fn phase_snapshot_cache_fences_epoch_and_replaces_previous_phase() {
        let mut cache = PhaseContextSnapshotCache::default();
        cache.install("team-a", 7, snapshot(String::new()));
        assert!(cache.get("team-a", 6).is_none());
        let current = cache.get("team-a", 7).unwrap();
        assert_eq!(current.project_id, "project-a");

        cache.install("team-a", 8, snapshot(String::new()));
        assert!(cache.get("team-a", 7).is_none());
        assert!(cache.get("team-a", 8).is_some());
        cache.remove("team-a", 7);
        assert!(cache.get("team-a", 8).is_some());
    }
}
