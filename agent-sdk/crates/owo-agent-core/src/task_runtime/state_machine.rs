//! 统一任务状态机（Agent V2 §2.3）。
//!
//! 所有状态迁移必须经过本模块校验；HTTP 层与前端不得自行推导并写入状态。
//! 终态（Succeeded / Failed / Cancelled）不可被普通更新覆盖。

use super::model::{TaskError, TaskResult, TaskState};

/// 是否允许从 `from` 迁移到 `to`。
///
/// 同状态视为幂等更新（允许），这样重复的 pause/complete 不会因为重试而失败。
pub fn can_transition(from: TaskState, to: TaskState) -> bool {
    if from == to {
        return true;
    }
    use TaskState::*;
    match from {
        Draft => matches!(to, Queued | Cancelled | Failed),
        Queued => matches!(
            to,
            Clarifying | Planning | Running | Paused | Cancelled | Failed
        ),
        Clarifying => matches!(to, Planning | NeedsUserInput | Cancelled | Failed | Paused),
        Planning => matches!(
            to,
            Running | AwaitingApproval | Paused | Cancelled | Failed | NeedsUserInput
        ),
        Running => matches!(
            to,
            AwaitingApproval
                | Verifying
                | Repairing
                | Paused
                | CancelRequested
                | Failed
                | Interrupted
                | NeedsUserInput
        ),
        AwaitingApproval => matches!(
            to,
            Running | Paused | CancelRequested | Failed | NeedsUserInput
        ),
        Verifying => matches!(
            to,
            Succeeded | Repairing | Failed | Paused | CancelRequested | NeedsUserInput
        ),
        Repairing => matches!(
            to,
            Running | Verifying | Failed | Paused | CancelRequested | NeedsUserInput
        ),
        Paused => matches!(
            to,
            Queued
                | Planning
                | Running
                | Verifying
                | CancelRequested
                | Failed
                | Interrupted
                | NeedsUserInput
        ),
        CancelRequested => matches!(to, Cancelled | Failed),
        Interrupted => matches!(
            to,
            Queued | Planning | Running | NeedsUserInput | Cancelled | Failed
        ),
        NeedsUserInput => matches!(to, Queued | Planning | Running | Cancelled | Failed),
        Succeeded | Failed | Cancelled => false,
    }
}

/// 校验迁移；失败返回稳定错误码 `INVALID_TRANSITION`。
pub fn ensure_transition(from: TaskState, to: TaskState) -> TaskResult<()> {
    if can_transition(from, to) {
        Ok(())
    } else {
        Err(TaskError::invalid_transition(from, to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_requires_verification() {
        assert!(!can_transition(TaskState::Running, TaskState::Succeeded));
        assert!(can_transition(TaskState::Verifying, TaskState::Succeeded));
    }

    #[test]
    fn terminal_states_are_immutable() {
        for state in [
            TaskState::Succeeded,
            TaskState::Failed,
            TaskState::Cancelled,
        ] {
            for next in [
                TaskState::Running,
                TaskState::Queued,
                TaskState::Succeeded,
                TaskState::Cancelled,
            ] {
                // 同状态是幂等更新，不算"覆盖终态"；这里只禁止真正离开终态。
                if next == state {
                    assert!(can_transition(state, next), "{state} -> {next} 应幂等");
                    continue;
                }
                assert!(!can_transition(state, next), "{state} -> {next}");
            }
        }
    }

    #[test]
    fn cancel_path_is_explicit() {
        assert!(can_transition(
            TaskState::Running,
            TaskState::CancelRequested
        ));
        assert!(can_transition(
            TaskState::CancelRequested,
            TaskState::Cancelled
        ));
        assert!(!can_transition(TaskState::Running, TaskState::Cancelled));
    }

    #[test]
    fn same_state_is_idempotent() {
        for state in [
            TaskState::Running,
            TaskState::Paused,
            TaskState::NeedsUserInput,
        ] {
            assert!(can_transition(state, state));
        }
    }

    #[test]
    fn recovery_paths_are_allowed() {
        assert!(can_transition(TaskState::Interrupted, TaskState::Queued));
        assert!(can_transition(
            TaskState::Interrupted,
            TaskState::NeedsUserInput
        ));
        assert!(can_transition(
            TaskState::NeedsUserInput,
            TaskState::Running
        ));
    }
}
