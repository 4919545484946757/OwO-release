//! 目标/工作流推进策略：按「进度」决定继续、重试、停滞升级还是结束。
//!
//! ## 为什么单独成模块
//!
//! 「agent 自己推进」不是死循环：每一轮结束后要判断**这一轮有没有让进度前移**，
//! 再决定下一步动作。判定规则如果散落在各客户端的 REPL 里，就会出现
//! 「CLI 会停、桌面不会停」这类不一致。这里把规则收敛成纯函数：
//!
//! | 本轮结果 | 判定 |
//! |---|---|
//! | 宿主接受（Accepted/ResponseComplete + 完成标记） | `Done` |
//! | 回合中止 | `Aborted` |
//! | 回合失败 | 连续失败 ≤ `max_retries` → `Retry`；超过 → `Failed`（交给人） |
//! | 状态与上一轮相同 | 连续同状态 < `stall_window` → `Continue`；达到 → `Stall`（停滞升级） |
//! | 状态变化 | `Continue`（进度在动） |
//!
//! `Stall` 的意义：目标没有失败、但连续多轮宿主状态一模一样（模型在原地打转）。
//! 继续自动推进只会烧预算，所以升级为「需要人/需要重规划」。
//!
//! 纯逻辑、无 IO：调用方负责把 `GoalProgress` 持久化到自己的会话/运行记录里。

use serde::{Deserialize, Serialize};

/// 推进策略（可按工作流/域覆盖）。
///
/// 默认值刻意保守：失败重试 2 次、连续 3 轮同状态判停滞。目标模式本来就要烧
/// 模型预算，早点把"没在动"暴露出来比继续空转更省。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvancePolicy {
    /// 连续失败允许的自动重试上限（0 = 首次失败即停下交给人）。
    pub max_retries: u32,
    /// 连续多少轮宿主状态不变判定为停滞（0 = 不检测停滞）。
    pub stall_window: u32,
}

impl Default for AdvancePolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            stall_window: 3,
        }
    }
}

impl AdvancePolicy {
    pub fn new(max_retries: u32, stall_window: u32) -> Self {
        Self {
            max_retries,
            stall_window,
        }
    }

    /// 从环境变量读取覆盖值：`OWO_GOAL_MAX_RETRIES`、`OWO_GOAL_STALL_WINDOW`。
    pub fn from_env() -> Self {
        let mut policy = Self::default();
        if let Ok(value) = std::env::var("OWO_GOAL_MAX_RETRIES") {
            if let Ok(parsed) = value.parse() {
                policy.max_retries = parsed;
            }
        }
        if let Ok(value) = std::env::var("OWO_GOAL_STALL_WINDOW") {
            if let Ok(parsed) = value.parse() {
                policy.stall_window = parsed;
            }
        }
        policy
    }
}

/// 单次推进的进度账本（跨轮累计）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalProgress {
    /// 已执行轮数。
    pub iterations: u32,
    /// 连续失败轮数（成功一轮即清零）。
    pub consecutive_failures: u32,
    /// 连续「宿主状态不变」轮数（状态变化即重置为 1）。
    pub consecutive_same_status: u32,
    /// 上一轮的宿主状态（Debug 形态，仅用于比较与展示）。
    pub last_status: Option<String>,
    /// 是否被宿主接受（终态）。
    pub accepted: bool,
}

/// 一回合的结果分类（调用方从自己的回合结果里归纳）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnVerdict<'a> {
    /// 宿主接受，目标达成。
    Accepted,
    /// 回合失败（模型/网络/工具链错误）。
    Failed,
    /// 回合被中止（用户取消/超时终态）。
    Aborted,
    /// 回合正常结束但目标未完成，附带宿主状态文本。
    Status(&'a str),
}

/// 推进决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvanceOutcome {
    /// 继续下一轮。
    Continue,
    /// 失败但仍在重试预算内：再给一次机会。
    Retry { consecutive_failures: u32 },
    /// 连续失败超过预算：交给人（附诊断建议）。
    Failed { consecutive_failures: u32 },
    /// 连续多轮状态不变：停滞，交给人或重规划。
    Stall { status: String, rounds: u32 },
    /// 目标达成。
    Done,
    /// 回合中止，停止推进。
    Aborted,
}

impl AdvanceOutcome {
    /// 是否应当结束自动推进循环。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Aborted | Self::Failed { .. } | Self::Stall { .. }
        )
    }
}

impl GoalProgress {
    /// 开始新一轮（调用方在真正跑回合前调用）。
    pub fn begin_iteration(&mut self) {
        self.iterations += 1;
    }

    /// 记录一回合的结果并给出推进决策。
    pub fn record(&mut self, verdict: TurnVerdict<'_>, policy: &AdvancePolicy) -> AdvanceOutcome {
        match verdict {
            TurnVerdict::Accepted => {
                self.accepted = true;
                AdvanceOutcome::Done
            }
            TurnVerdict::Aborted => AdvanceOutcome::Aborted,
            TurnVerdict::Failed => {
                self.consecutive_failures += 1;
                // 失败轮不参与"同状态"计数：失败本身已经是进度信号。
                self.consecutive_same_status = 0;
                if self.consecutive_failures > policy.max_retries {
                    AdvanceOutcome::Failed {
                        consecutive_failures: self.consecutive_failures,
                    }
                } else {
                    AdvanceOutcome::Retry {
                        consecutive_failures: self.consecutive_failures,
                    }
                }
            }
            TurnVerdict::Status(status) => {
                self.consecutive_failures = 0;
                if self.last_status.as_deref() == Some(status) {
                    self.consecutive_same_status += 1;
                } else {
                    self.last_status = Some(status.to_string());
                    self.consecutive_same_status = 1;
                }
                if policy.stall_window > 0 && self.consecutive_same_status >= policy.stall_window {
                    AdvanceOutcome::Stall {
                        status: status.to_string(),
                        rounds: self.consecutive_same_status,
                    }
                } else {
                    AdvanceOutcome::Continue
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> AdvancePolicy {
        AdvancePolicy::new(2, 3)
    }

    #[test]
    fn accepted_and_aborted_are_terminal() {
        let mut progress = GoalProgress::default();
        assert_eq!(
            progress.record(TurnVerdict::Accepted, &policy()),
            AdvanceOutcome::Done
        );
        assert!(progress.accepted);
        assert!(AdvanceOutcome::Done.is_terminal());

        let mut progress = GoalProgress::default();
        assert_eq!(
            progress.record(TurnVerdict::Aborted, &policy()),
            AdvanceOutcome::Aborted
        );
    }

    #[test]
    fn failures_retry_then_escalate() {
        let mut progress = GoalProgress::default();
        // max_retries=2 → 允许两次重试，第三次连续失败升级。
        assert_eq!(
            progress.record(TurnVerdict::Failed, &policy()),
            AdvanceOutcome::Retry {
                consecutive_failures: 1
            }
        );
        assert_eq!(
            progress.record(TurnVerdict::Failed, &policy()),
            AdvanceOutcome::Retry {
                consecutive_failures: 2
            }
        );
        let outcome = progress.record(TurnVerdict::Failed, &policy());
        assert_eq!(
            outcome,
            AdvanceOutcome::Failed {
                consecutive_failures: 3
            }
        );
        assert!(outcome.is_terminal());
    }

    #[test]
    fn a_successful_status_turn_clears_failure_streak() {
        let mut progress = GoalProgress::default();
        progress.record(TurnVerdict::Failed, &policy());
        progress.record(TurnVerdict::Failed, &policy());
        assert_eq!(
            progress.record(TurnVerdict::Status("Candidate"), &policy()),
            AdvanceOutcome::Continue
        );
        assert_eq!(progress.consecutive_failures, 0);
        // 失败预算重新可用。
        assert_eq!(
            progress.record(TurnVerdict::Failed, &policy()),
            AdvanceOutcome::Retry {
                consecutive_failures: 1
            }
        );
    }

    #[test]
    fn unchanged_status_escalates_as_stall() {
        let mut policy = policy();
        policy.stall_window = 3;
        let mut progress = GoalProgress::default();
        // 第 1 次出现：计数 1；第 2 次：2；第 3 次：达到窗口 → Stall。
        assert_eq!(
            progress.record(TurnVerdict::Status("Blocked"), &policy),
            AdvanceOutcome::Continue
        );
        assert_eq!(
            progress.record(TurnVerdict::Status("Blocked"), &policy),
            AdvanceOutcome::Continue
        );
        let outcome = progress.record(TurnVerdict::Status("Blocked"), &policy);
        assert_eq!(
            outcome,
            AdvanceOutcome::Stall {
                status: "Blocked".to_string(),
                rounds: 3
            }
        );
        assert!(outcome.is_terminal());
    }

    #[test]
    fn changing_status_resets_the_stall_counter() {
        let mut progress = GoalProgress::default();
        progress.record(TurnVerdict::Status("Unverified"), &policy());
        progress.record(TurnVerdict::Status("Unverified"), &policy());
        // 状态变化 = 仍在推进（例如 Unverified → Candidate）。
        assert_eq!(
            progress.record(TurnVerdict::Status("Candidate"), &policy()),
            AdvanceOutcome::Continue
        );
        assert_eq!(progress.consecutive_same_status, 1);
    }

    #[test]
    fn stall_detection_can_be_disabled() {
        let policy = AdvancePolicy::new(2, 0);
        let mut progress = GoalProgress::default();
        for _ in 0..10 {
            assert_eq!(
                progress.record(TurnVerdict::Status("Blocked"), &policy),
                AdvanceOutcome::Continue
            );
        }
    }

    #[test]
    fn iteration_counter_advances() {
        let mut progress = GoalProgress::default();
        progress.begin_iteration();
        progress.begin_iteration();
        assert_eq!(progress.iterations, 2);
    }
}
