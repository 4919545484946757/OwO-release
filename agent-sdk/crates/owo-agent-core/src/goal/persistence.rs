//! Atomic complete checkpoints for Goal and Team progress and recovery.
//! Readers see the previous complete snapshot or the new one, never truncated JSON.
use super::GoalRunState;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub(super) fn persist_checkpoint(state: &GoalRunState, dir: &Path) -> Result<PathBuf, String> {
    persist_checkpoint_with_tasks(state, dir, state.team_task_events.as_ref())
}
pub(super) fn persist_checkpoint_with_tasks(
    state: &GoalRunState,
    dir: &Path,
    outbox: Option<&crate::project_space_store::TaskEventOutbox>,
) -> Result<PathBuf, String> {
    #[derive(serde::Serialize)]
    struct Checkpoint<'a> {
        #[serde(flatten)]
        state: &'a GoalRunState,
        #[serde(skip_serializing_if = "Option::is_none")]
        team_task_events: Option<&'a crate::project_space_store::TaskEventOutbox>,
    }
    let mut parts = Path::new(&state.run_id).components();
    if !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || state.run_id.contains(['/', '\\'])
    {
        return Err("运行状态 ID 必须是单个文件名".into());
    }
    let bytes = serde_json::to_vec_pretty(&Checkpoint {
        state,
        team_task_events: outbox,
    })
    .map_err(|error| format!("运行状态序列化失败：{error}"))?;
    std::fs::create_dir_all(dir).map_err(|error| format!("创建运行目录失败：{error}"))?;
    let path = dir.join(format!("{}.json", state.run_id));
    let pending = dir.join(format!(
        ".{}.{}.pending",
        state.run_id,
        uuid::Uuid::new_v4()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|error| format!("创建状态检查点失败：{error}"))?;
    let cleanup = PendingCheckpoint(pending);
    let written = file.write_all(&bytes).and_then(|_| file.sync_all());
    // Close on both success and failure before cleanup or replacement on Windows.
    drop(file);
    written.map_err(|error| format!("写入状态检查点失败：{error}"))?;
    std::fs::rename(&cleanup.0, &path)
        .map_err(|error| format!("发布运行状态检查点失败：{error}"))?;
    Ok(path)
}

struct PendingCheckpoint(PathBuf);
impl Drop for PendingCheckpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::Goal;
    use crate::plan::{Plan, StepSpec};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn state() -> GoalRunState {
        let mut plan = Plan::new("plan", "goal");
        let mut step = StepSpec::new("step", "worker");
        step.input = serde_json::json!({"payload":"x".repeat(32 * 1024)});
        plan.add_step(step);
        let mut state = GoalRunState::new(Goal::new("goal", "checkpoint"), plan);
        state.run_id = "checkpoint-test".into();
        state
    }

    #[test]
    fn concurrent_readers_observe_only_complete_monotonic_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state();
        let path = state.persist(dir.path()).unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let reader_flag = running.clone();
        let reader = std::thread::spawn(move || {
            let mut previous = 0;
            let mut reads = 0;
            loop {
                let bytes = std::fs::read(&path).expect("checkpoint never disappears");
                let snapshot: GoalRunState =
                    serde_json::from_slice(&bytes).expect("reader must never see partial JSON");
                assert!(snapshot.steps_taken >= previous);
                previous = snapshot.steps_taken;
                reads += 1;
                if !reader_flag.load(Ordering::SeqCst) {
                    return reads;
                }
                std::thread::yield_now();
            }
        });
        for sequence in 1..=50 {
            state.steps_taken = sequence;
            state.execution_epoch = 3;
            state.persist(dir.path()).unwrap();
        }
        running.store(false, Ordering::SeqCst);
        assert!(reader.join().unwrap() > 0);
        let final_state = GoalRunState::load(dir.path(), &state.run_id).unwrap();
        assert_eq!(final_state.steps_taken, 50);
        assert_eq!(final_state.execution_epoch, 3);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_publication_preserves_destination_and_cleans_pending_file() {
        let dir = tempfile::tempdir().unwrap();
        let state = state();
        let destination = dir.path().join(format!("{}.json", state.run_id));
        std::fs::create_dir(&destination).unwrap();
        let marker = destination.join("keep");
        std::fs::write(&marker, b"original").unwrap();
        assert!(state.persist(dir.path()).is_err());
        assert_eq!(std::fs::read(marker).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn checkpoint_id_cannot_escape_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state();
        for invalid in ["", ".", "..", "../escape", "dir/file", r"dir\file"] {
            state.run_id = invalid.into();
            assert!(state.persist(dir.path()).is_err(), "{invalid}");
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
