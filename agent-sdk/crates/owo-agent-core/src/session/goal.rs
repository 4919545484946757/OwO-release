//! 会话级目标模式状态（CLI `/goal` 的持久化落点）。
//!
//! 与 Codex 的目标/计划语义同级：目标、已推进轮次与完成标记**随会话落库**，
//! 重开进程 `/resume` 后目标与进度不丢。本模块只承载状态与合法状态迁移；
//! 谁推进、何时算完成仍由调用方（CLI 目标循环 + 宿主完成裁决）决定。

use serde::{Deserialize, Serialize};

use super::Session;

/// 会话级目标模式状态：目标、已推进轮次与完成标记。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionGoal {
    /// 用户设定的目标描述。
    pub objective: String,
    /// 已执行的推进轮次。
    #[serde(default)]
    pub iterations: usize,
    /// 模型宣告完成且宿主裁决允许交付后的终态。
    #[serde(default)]
    pub done: bool,
    /// 最近一次状态变更时间（RFC3339）。
    #[serde(default)]
    pub updated_at: String,
    /// 目标来源：`user`（用户 `/goal` 设定，已由宿主循环驱动）或 `model`
    /// （模型经 `goal_request` 工具登记，宿主在本回合结束后决定是否启动循环）。
    #[serde(default = "default_goal_origin")]
    pub origin: String,
}

/// 旧数据没有 `origin` 列 → 视为用户设定。
fn default_goal_origin() -> String {
    GOAL_ORIGIN_USER.to_string()
}

/// 用户显式设定（宿主循环负责启动）。
pub const GOAL_ORIGIN_USER: &str = "user";
/// 模型登记（宿主决定是否启动，不能自行开跑）。
pub const GOAL_ORIGIN_MODEL: &str = "model";

impl SessionGoal {
    /// 由目标描述新建（轮次与完成标记复位）。
    pub fn new(objective: impl Into<String>) -> Self {
        Self {
            objective: objective.into(),
            iterations: 0,
            done: false,
            updated_at: now(),
            origin: GOAL_ORIGIN_USER.to_string(),
        }
    }

    /// 是否由模型登记（尚未由宿主启动）。
    pub fn is_model_requested(&self) -> bool {
        self.origin == GOAL_ORIGIN_MODEL
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl Session {
    /// 设定/替换目标（轮次与完成标记复位）。
    pub fn set_goal(&mut self, objective: impl Into<String>) {
        self.goal = Some(SessionGoal::new(objective));
        self.updated_at = now();
    }

    /// 模型登记目标（origin=model）：只记录"要做什么"，不启动循环——宿主在回合
    /// 结束后决定是否接管（回合内启动会与会话/回合锁冲突）。
    pub fn set_model_goal(&mut self, objective: impl Into<String>) {
        let mut goal = SessionGoal::new(objective);
        goal.origin = GOAL_ORIGIN_MODEL.to_string();
        self.goal = Some(goal);
        self.updated_at = now();
    }

    /// 推进一轮（无目标时不产生状态）。
    pub fn advance_goal(&mut self) -> Option<&SessionGoal> {
        if let Some(goal) = self.goal.as_mut() {
            goal.iterations = goal.iterations.saturating_add(1);
            goal.updated_at = now();
            self.updated_at = goal.updated_at.clone();
        }
        self.goal.as_ref()
    }

    /// 标记目标完成（模型宣告完成且宿主裁决允许交付后）。
    pub fn complete_goal(&mut self) -> Option<&SessionGoal> {
        if let Some(goal) = self.goal.as_mut() {
            goal.done = true;
            goal.updated_at = now();
            self.updated_at = goal.updated_at.clone();
        }
        self.goal.as_ref()
    }

    /// 清除目标（无论是否已完成）。
    pub fn clear_goal(&mut self) {
        if self.goal.take().is_some() {
            self.updated_at = now();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn session() -> Session {
        Session::new(Path::new("."), "mock", None)
    }

    #[test]
    fn goal_lifecycle_set_advance_complete_clear() {
        let mut session = session();
        assert!(session.goal.is_none());

        session.set_goal("把 README 更新到最新");
        let goal = session.goal.as_ref().unwrap();
        assert_eq!(goal.objective, "把 README 更新到最新");
        assert_eq!(goal.iterations, 0);
        assert!(!goal.done);
        assert!(!goal.updated_at.is_empty());

        session.advance_goal();
        session.advance_goal();
        assert_eq!(session.goal.as_ref().unwrap().iterations, 2);
        assert!(!session.goal.as_ref().unwrap().done);

        session.complete_goal();
        assert!(session.goal.as_ref().unwrap().done);

        session.clear_goal();
        assert!(session.goal.is_none());
    }

    #[test]
    fn re_setting_goal_resets_progress() {
        let mut session = session();
        session.set_goal("旧目标");
        session.advance_goal();
        session.complete_goal();

        session.set_goal("新目标");
        let goal = session.goal.as_ref().unwrap();
        assert_eq!(goal.objective, "新目标");
        assert_eq!(goal.iterations, 0, "重设目标必须复位轮次");
        assert!(!goal.done, "重设目标必须复位完成标记");
    }

    #[test]
    fn advance_and_complete_without_goal_are_noops() {
        let mut session = session();
        let before = session.updated_at.clone();
        assert!(session.advance_goal().is_none());
        assert!(session.complete_goal().is_none());
        session.clear_goal();
        assert!(session.goal.is_none());
        assert_eq!(session.updated_at, before, "无目标时不得推进 updated_at");
    }

    #[test]
    fn goal_origin_distinguishes_user_and_model_requests() {
        let mut session = session();
        session.set_goal("用户目标");
        assert_eq!(session.goal.as_ref().unwrap().origin, GOAL_ORIGIN_USER);
        assert!(!session.goal.as_ref().unwrap().is_model_requested());

        session.set_model_goal("模型登记的目标");
        let goal = session.goal.as_ref().unwrap();
        assert!(goal.is_model_requested());
        assert_eq!(goal.iterations, 0, "模型登记也要复位轮次");
        assert!(!goal.done);

        // 推进/完成不改变来源，宿主据此判断"是否已由循环接管"。
        session.advance_goal();
        session.complete_goal();
        assert!(session.goal.as_ref().unwrap().is_model_requested());
    }

    #[test]
    fn legacy_rows_without_origin_are_treated_as_user_goals() {
        let mut session = session();
        session.set_model_goal("模型目标");
        let json = serde_json::to_string(&session).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["goal"].as_object_mut().unwrap().remove("origin");
        let loaded: Session = serde_json::from_value(value).unwrap();
        assert_eq!(
            loaded.goal.as_ref().unwrap().origin,
            GOAL_ORIGIN_USER,
            "旧数据没有 origin → 按用户设定处理（不会自动触发宿主循环）"
        );
    }

    #[test]
    fn goal_survives_serde_round_trip_and_legacy_sessions() {
        let mut session = session();
        session.set_goal("持久化目标");
        session.advance_goal();
        let json = serde_json::to_string(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.goal, session.goal);

        // 旧会话 JSON 没有 goal 字段：按 None 加载，不报错。
        let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
        legacy.as_object_mut().unwrap().remove("goal");
        let loaded: Session = serde_json::from_value(legacy).unwrap();
        assert!(loaded.goal.is_none());
    }
}
