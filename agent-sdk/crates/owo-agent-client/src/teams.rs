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
        self.create_team_with_scoped_writes_and_members(
            objective,
            mode,
            template_id,
            acknowledge_risk,
            allow_scoped_writes,
            None,
        )
        .await
    }

    /// 与 [`Self::create_team_with_scoped_writes`] 相同，但可显式放开 Agent 成员数。
    ///
    /// 自定义工作流的角色数可能超过服务端默认 5 个；调用方按**已审阅的模板角色数**
    /// 传入上限，而不是让建队在建到一半时因 7>5 失败。`None` 保持服务端默认。
    pub async fn create_team_with_scoped_writes_and_members(
        &self,
        objective: &str,
        mode: &str,
        template_id: Option<&str>,
        acknowledge_risk: bool,
        allow_scoped_writes: bool,
        max_agent_members: Option<usize>,
    ) -> Result<serde_json::Value> {
        let mut body = serde_json::json!({
            "objective": objective,
            "mode": mode,
            "template_id": template_id,
            // 社区来源 + 有风险面时服务端要求显式确认（宿主侧兜底）。
            "acknowledge_risk": acknowledge_risk,
            "allow_scoped_writes": allow_scoped_writes,
        });
        if let Some(limit) = max_agent_members {
            body["max_agent_members"] = serde_json::json!(limit);
        }
        self.post_json("/teams", &body).await
    }

    /// 自动模式：不指定模板，由 Daemon 按目标文本匹配已安装工作流。
    ///
    /// `strategy=team` 明确要求组队；模板匹配仍由服务端 `find_match_revision`
    /// 负责，调用方不需要知道模板 ID。没有匹配模板时服务端回退到默认接力角色。
    pub async fn create_auto_team(
        &self,
        objective: &str,
        acknowledge_risk: bool,
        allow_scoped_writes: bool,
    ) -> Result<serde_json::Value> {
        self.post_json(
            "/teams",
            &serde_json::json!({
                "objective": objective,
                "mode": "team",
                "strategy": "team",
                // 自动匹配的模板可能超过默认 5 个 Agent（游戏流水线需要
                // systems/gameplay/presentation/assembler/tester/playtester/review）。
                // 显式模板仍可自行覆盖；这里只放开自动组队的成员预算。
                "max_agent_members": 12,
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
    /// List learned template proposals (proposal only; never auto-enabled).
    pub async fn list_template_proposals(&self) -> Result<serde_json::Value> {
        self.get_json("/teams/templates/proposals").await
    }

    /// Adopt a proposal into the user workflow registry.
    pub async fn adopt_template_proposal(&self, proposal_id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/teams/templates/proposals/{proposal_id}/adopt"),
            &serde_json::json!({}),
        )
        .await
    }

    /// Reject a proposal while keeping an auditable record.
    pub async fn reject_template_proposal(&self, proposal_id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/teams/templates/proposals/{proposal_id}/reject"),
            &serde_json::json!({}),
        )
        .await
    }
}
