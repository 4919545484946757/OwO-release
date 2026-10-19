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
        self.create_team_with_scoped_writes(objective, mode, template_id, acknowledge_risk, false)
            .await
    }

    /// 创建团队运行，并显式声明"允许节点按声明的 write_paths 范围写入工作区"。
    ///
    /// 团队运行没有交互式审批通道：不预授权时，连"落在自己声明范围内"的写也会被
    /// 拒（自定义工作流因此必然失败）。预授权只覆盖范围内的落盘工具，越界写、
    /// 命令、联网与 MCP 仍走原审批/拒绝路径。
    pub async fn create_team_with_scoped_writes(
        &self,
        objective: &str,
        mode: &str,
        template_id: Option<&str>,
        acknowledge_risk: bool,
        allow_scoped_writes: bool,
    ) -> Result<serde_json::Value> {
        self.post_json(
            "/teams",
            &serde_json::json!({
                "objective": objective,
                "mode": mode,
                "template_id": template_id,
                // 社区来源 + 有风险面时服务端要求显式确认（宿主侧兜底）。
                "acknowledge_risk": acknowledge_risk,
                "allow_scoped_writes": allow_scoped_writes,
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
