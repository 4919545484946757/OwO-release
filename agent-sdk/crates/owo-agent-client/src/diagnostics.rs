//! 诊断域：健康检查、服务状态、build identity。

use crate::error::Result;
use crate::http::AgentClient;
use owo_agent_protocol::HealthResponse;
use serde_json::Value;

impl AgentClient {
    /// `/health`（公开端点；不要求 token）。
    pub async fn health(&self) -> Result<HealthResponse> {
        self.get_json("/health").await
    }

    /// `/server/status`（受保护）。
    pub async fn server_status(&self) -> Result<Value> {
        self.get_json("/server/status").await
    }

    /// 请求优雅关闭（`daemon stop`；需二次确认，服务端校验 `confirm=true`）。
    pub async fn request_shutdown(&self, confirm: bool) -> Result<Value> {
        self.post_json(
            "/server/shutdown",
            &serde_json::json!({ "confirm": confirm }),
        )
        .await
    }

    /// `/workspace/trust`：工作区信任状态（审查 N1：未信任时不加载 hooks/插件/权限放宽）。
    ///
    /// 返回 `trusted/changed/gated/reasons/fingerprint` 等字段，语义由服务端定义。
    pub async fn workspace_trust(&self) -> Result<Value> {
        self.get_json("/workspace/trust").await
    }

    /// `POST /workspace/trust`：授予（`true`）或撤销（`false`）工作区信任。
    ///
    /// 授予后服务端即时应用工作区 hooks/权限/白名单；插件 MCP 需重启核心。
    pub async fn set_workspace_trust(&self, trusted: bool) -> Result<Value> {
        self.post_json(
            "/workspace/trust",
            &serde_json::json!({ "trusted": trusted }),
        )
        .await
    }
}
