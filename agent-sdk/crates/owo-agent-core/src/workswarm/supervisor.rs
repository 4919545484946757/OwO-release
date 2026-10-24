//! Deterministic task supervision for live team runs.
//!
//! The supervisor is intentionally a host-side observer, not another model role:
//! it derives a conservative health state from durable step records plus the
//! in-memory phase claim. This keeps progress checking cheap, deterministic and
//! available even while a Worker is blocked in a model/tool call.
//!
//! Escalation is advisory in this layer. A stalled step must remain visible in
//! every `progress` snapshot so a caller can cancel, retry or replace it through
//! the existing steer API; we never silently mark work complete.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::types::{ProgressCounts, ProgressStep};

/// A single Worker attempt can legitimately spend several minutes in a model or
/// command call. The default only flags genuinely long silences; deployments can
/// tune it with `OWO_TEAM_STALL_AFTER_SECS`.
pub const DEFAULT_STALL_AFTER_SECS: u64 = 600;
pub const MIN_STALL_AFTER_SECS: u64 = 30;
pub const MAX_STALL_AFTER_SECS: u64 = 86_400;

/// Host-observed supervision state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgressSupervisor {
    /// `on_track | stalled | blocked | idle | complete | cancelled`.
    pub state: String,
    /// Whether a phase loop currently owns this team.
    pub active: bool,
    /// Whether a persisted interrupted marker exists for this run.
    pub interrupted: bool,
    /// Steps that exceeded the silence threshold without a terminal status.
    pub stalled_steps: Vec<String>,
    /// Threshold used for this snapshot (tunable without changing persisted state).
    pub threshold_secs: u64,
    /// Age of the oldest live step (claimed/running, or an orphaned queue entry).
    pub oldest_step_age_secs: u64,
    /// Short operator-facing next action. Never treated as an instruction to a model.
    pub recommendation: String,
}

impl ProgressSupervisor {
    pub fn is_stalled(&self) -> bool {
        self.state == "stalled"
    }
}

pub fn stall_after_secs() -> u64 {
    std::env::var("OWO_TEAM_STALL_AFTER_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_STALL_AFTER_SECS)
        .clamp(MIN_STALL_AFTER_SECS, MAX_STALL_AFTER_SECS)
}

fn step_age_secs(step: &ProgressStep, now: DateTime<Utc>) -> Option<u64> {
    let raw = if step.started_at.trim().is_empty() {
        step.claimed_at.trim()
    } else {
        step.started_at.trim()
    };
    if raw.is_empty() {
        return None;
    }
    let started = DateTime::parse_from_rfc3339(raw).ok()?.with_timezone(&Utc);
    let secs = now.signed_duration_since(started).num_seconds();
    u64::try_from(secs).ok()
}

fn live(step: &ProgressStep) -> bool {
    matches!(step.status.as_str(), "Claimed" | "Running")
}

/// Derive supervision from one snapshot. Pure so it can be tested without a
/// coordinator, clock or filesystem.
pub fn supervise_progress(
    status: &str,
    active: bool,
    interrupted: bool,
    counts: &ProgressCounts,
    steps: &[ProgressStep],
    now: DateTime<Utc>,
    threshold_secs: u64,
) -> ProgressSupervisor {
    let status = status.trim().to_ascii_lowercase();
    let mut stalled_steps = Vec::new();
    let mut oldest_step_age_secs = 0u64;
    // A queue is healthy while any claimed/running step owns an execution slot.
    // When nothing is live, however, an old Queued entry means the scheduler or
    // worker admission path stopped moving; report it instead of calling the run
    // on_track forever.
    let queue_can_stall = counts.claimed == 0 && counts.running == 0;
    for step in steps {
        let queued_without_live_work = queue_can_stall && step.status == "Queued";
        if !live(step) && !queued_without_live_work {
            continue;
        }
        let Some(age_secs) = step_age_secs(step, now) else {
            continue;
        };
        oldest_step_age_secs = oldest_step_age_secs.max(age_secs);
        if age_secs >= threshold_secs {
            stalled_steps.push(step.step_id.clone());
        }
    }
    stalled_steps.sort();
    stalled_steps.dedup();

    let terminal_completed = matches!(status.as_str(), "completed" | "done" | "succeeded");
    let terminal_cancelled = matches!(status.as_str(), "cancelled" | "aborted");
    let terminal_failed = status == "failed";
    let has_failures = counts.failed > 0 || counts.aborted > 0;

    let (state, recommendation) = if terminal_completed {
        (
            "complete",
            "团队已进入完成态；继续核对交付门禁与证据，不要把进程结束等同于验收通过。".to_string(),
        )
    } else if terminal_cancelled {
        (
            "cancelled",
            "团队已取消；已完成产物仍保留，可从检查点重试未完成节点。".to_string(),
        )
    } else if terminal_failed && has_failures {
        (
            "blocked",
            "存在失败/中止步骤；先查看失败原因，再用 steer retry 或 replace 只会重跑未完成节点。"
                .to_string(),
        )
    } else if terminal_failed {
        (
            "blocked",
            "团队已失败但步骤均已结束；检查交付门禁、评审证据和最终化错误，不要用重跑步骤掩盖门禁失败。"
                .to_string(),
        )
    } else if !active && has_failures {
        (
            "blocked",
            "存在失败/中止步骤；先查看失败原因，再用 steer retry 或 replace 只会重跑未完成节点。"
                .to_string(),
        )
    } else if interrupted && !active {
        (
            "blocked",
            "检测到未完成运行被中断；已完成产物保留，请通过团队恢复入口继续未完成节点，不要直接重放已成功步骤。"
                .to_string(),
        )
    } else if !stalled_steps.is_empty() {
        (
            "stalled",
            format!(
                "步骤 {} 超过 {threshold_secs}s 无新进展；检查审批、命令和 Worker 输出，再决定继续、取消或重试。",
                stalled_steps.join(", ")
            ),
        )
    } else if active {
        (
            "on_track",
            "当前阶段仍在推进；监督器会在下一次状态转移或超时后重新评估。".to_string(),
        )
    } else if counts.pending == 0
        && counts.claimed == 0
        && counts.running == 0
        && counts.succeeded > 0
        && !has_failures
    {
        (
            "complete",
            "全部步骤已完成；等待收尾与交付门禁。".to_string(),
        )
    } else {
        (
            "idle",
            "当前没有运行中的阶段；可能等待人工节点或下一次调度，请核对状态。".to_string(),
        )
    };

    ProgressSupervisor {
        state: state.to_string(),
        active,
        interrupted,
        stalled_steps,
        threshold_secs,
        oldest_step_age_secs,
        recommendation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, status: &str, started_at: &str) -> ProgressStep {
        ProgressStep {
            step_id: id.to_string(),
            worker: "worker".to_string(),
            status: status.to_string(),
            attempts: 1,
            claimed_at: started_at.to_string(),
            started_at: started_at.to_string(),
        }
    }

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn recent_running_step_is_on_track() {
        let snapshot = supervise_progress(
            "Running",
            true,
            false,
            &ProgressCounts {
                running: 1,
                ..Default::default()
            },
            &[step("a", "Running", "2026-01-01T00:00:00Z")],
            at("2026-01-01T00:01:00Z"),
            600,
        );
        assert_eq!(snapshot.state, "on_track");
        assert_eq!(snapshot.oldest_step_age_secs, 60);
        assert!(snapshot.stalled_steps.is_empty());
    }

    #[test]
    fn old_live_step_is_stalled_and_named() {
        let snapshot = supervise_progress(
            "Running",
            true,
            false,
            &ProgressCounts {
                running: 1,
                ..Default::default()
            },
            &[step("worker-a", "Running", "2026-01-01T00:00:00Z")],
            at("2026-01-01T00:20:00Z"),
            600,
        );
        assert_eq!(snapshot.state, "stalled");
        assert_eq!(snapshot.stalled_steps, vec!["worker-a"]);
        assert!(snapshot.recommendation.contains("worker-a"));
    }

    #[test]
    fn failed_step_blocks_even_when_loop_is_idle() {
        let snapshot = supervise_progress(
            "Failed",
            false,
            false,
            &ProgressCounts {
                failed: 1,
                ..Default::default()
            },
            &[],
            at("2026-01-01T00:20:00Z"),
            600,
        );
        assert_eq!(snapshot.state, "blocked");
        assert!(snapshot.recommendation.contains("retry"));
    }

    #[test]
    fn failed_team_with_successful_steps_points_at_delivery_gate() {
        let snapshot = supervise_progress(
            "Failed",
            false,
            false,
            &ProgressCounts {
                succeeded: 3,
                ..Default::default()
            },
            &[],
            at("2026-01-01T00:20:00Z"),
            600,
        );
        assert_eq!(snapshot.state, "blocked");
        assert!(!snapshot.recommendation.contains("存在失败/中止步骤"));
        assert!(snapshot.recommendation.contains("交付门禁"));
    }

    #[test]
    fn completed_and_cancelled_are_terminal_not_stalled() {
        let now = at("2026-01-01T01:00:00Z");
        let completed = supervise_progress(
            "Completed",
            false,
            false,
            &ProgressCounts {
                succeeded: 2,
                ..Default::default()
            },
            &[],
            now,
            600,
        );
        assert_eq!(completed.state, "complete");
        let cancelled = supervise_progress(
            "Cancelled",
            false,
            false,
            &ProgressCounts::default(),
            &[],
            now,
            600,
        );
        assert_eq!(cancelled.state, "cancelled");
    }

    #[test]
    fn orphaned_queue_entry_stalls_instead_of_reporting_on_track() {
        let snapshot = supervise_progress(
            "Running",
            true,
            false,
            &ProgressCounts::default(),
            &[step("queued-a", "Queued", "2026-01-01T00:00:00Z")],
            at("2026-01-01T00:20:00Z"),
            600,
        );
        assert_eq!(snapshot.state, "stalled");
        assert_eq!(snapshot.stalled_steps, vec!["queued-a"]);
    }

    #[test]
    fn queued_backlog_waits_while_a_worker_is_running() {
        let snapshot = supervise_progress(
            "Running",
            true,
            false,
            &ProgressCounts {
                running: 1,
                ..Default::default()
            },
            &[
                step("running-a", "Running", "2026-01-01T00:19:30Z"),
                step("queued-b", "Queued", "2026-01-01T00:00:00Z"),
            ],
            at("2026-01-01T00:20:00Z"),
            600,
        );
        assert_eq!(snapshot.state, "on_track");
        assert!(snapshot.stalled_steps.is_empty());
    }

    #[test]
    fn interrupted_run_is_blocked_and_points_at_recovery() {
        let snapshot = supervise_progress(
            "Running",
            false,
            true,
            &ProgressCounts {
                pending: 2,
                ..Default::default()
            },
            &[],
            at("2026-01-01T00:20:00Z"),
            600,
        );
        assert_eq!(snapshot.state, "blocked");
        assert!(snapshot.interrupted);
        assert!(snapshot.recommendation.contains("恢复"));
        assert!(snapshot.recommendation.contains("不要直接重放"));
    }

    #[test]
    fn malformed_timestamps_do_not_create_false_stalls() {
        let snapshot = supervise_progress(
            "Running",
            true,
            false,
            &ProgressCounts {
                claimed: 1,
                ..Default::default()
            },
            &[step("a", "Claimed", "not-a-time")],
            at("2026-01-01T00:20:00Z"),
            600,
        );
        assert_eq!(snapshot.state, "on_track");
        assert_eq!(snapshot.oldest_step_age_secs, 0);
    }
}
