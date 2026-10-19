//! 会话级目标模式（CLI `/goal` 的持久化）客户端调用。
//!
//! 与 `sessions.rs` 分离：目标状态的读写语义（action 取值、何时算完成）
//! 由目标循环负责，这里只做 HTTP 契约映射，便于单独演进与测试。

use crate::error::Result;
use crate::AgentClient;

impl AgentClient {
    /// 会话级目标模式状态：`action ∈ set|advance|done|clear`。
    ///
    /// `set` 需要非空 `objective`；返回体形如
    /// `{"session_id":"…","action":"set","goal":{"objective":…,"iterations":…,"done":…}}`，
    /// 清除后 `goal` 为 `null`。服务端持久化在会话上（重开进程 `/resume` 不丢目标）。
    pub async fn session_goal(
        &self,
        id: &str,
        action: &str,
        objective: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/session/{id}/goal"),
            &serde_json::json!({ "action": action, "objective": objective }),
        )
        .await
    }
}
