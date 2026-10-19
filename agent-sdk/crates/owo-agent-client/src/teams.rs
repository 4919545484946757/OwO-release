//! 团队运行的最小客户端面（创建/查询/取消）。
//!
//! CLI `owo-agent workflow run` 用它把"自定义工作流"落成一次真实团队运行；
//! 详细跟踪仍走桌面「团队协作」面板或 HTTP/SSE（这里不复制那套 UI）。

use crate::error::Result;
use crate::AgentClient;

impl AgentClient {
    /// 创建团队运行（服务端 202 + 后台运行循环）。
    ///
    /// `mode`：`single` | `team` | `swarmflow`；`template_id` 指向已保存的用户/社区工作流。
    pub async fn create_team(
        &self,
        objective: &str,
        mode: &str,
        template_id: Option<&str>,
        acknowledge_risk: bool,
    ) -> Result<serde_json::Value> {
        self.post_json(
            "/teams",
            &serde_json::json!({
                "objective": objective,
                "mode": mode,
                "template_id": template_id,
                // 社区来源 + 有风险面时服务端要求显式确认（宿主侧兜底）。
                "acknowledge_risk": acknowledge_risk,
            }),
        )
        .await
    }

    /// 团队当前状态。
    pub async fn get_team(&self, id: &str) -> Result<serde_json::Value> {
        self.get_json(&format!("/teams/{id}")).await
    }

    /// 取消团队（等价于 `POST /teams/{id}/steer {"command":"cancel"}`）。
    pub async fn cancel_team(&self, id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/teams/{id}/steer"),
            &serde_json::json!({ "command": "cancel" }),
        )
        .await
    }
}
