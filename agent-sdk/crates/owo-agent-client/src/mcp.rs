//! MCP 服务器管理（CLI `owo-agent mcp` 的在线路径）。
//!
//! 与 `sessions.rs`/`session_goal.rs` 分离：MCP 管理只做 HTTP 契约映射，
//! 「写配置文件还是热连接」由 CLI 侧决定（见 `commands/mcp.rs`）。

use crate::error::Result;
use crate::AgentClient;

impl AgentClient {
    /// 已配置的 MCP 服务器与连接状态。
    pub async fn mcp_list(&self) -> Result<serde_json::Value> {
        self.get_json("/mcp").await
    }

    /// 新增 MCP 服务器（服务端热连接 + 落 `<data>/mcp-servers.json`）。
    ///
    /// 服务端在连接失败时返回 502 且不写配置——在线路径不会留下连不上的死配置。
    pub async fn mcp_add(
        &self,
        name: &str,
        transport: &str,
        command: &str,
        args: &[String],
        url: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.post_json(
            "/mcp/add",
            &serde_json::json!({
                "name": name,
                "transport": transport,
                "command": command,
                "args": args,
                "url": url,
            }),
        )
        .await
    }

    /// 移除 MCP 服务器（进程级卸载 + 落配置）。
    pub async fn mcp_remove(&self, name: &str) -> Result<serde_json::Value> {
        self.post_json("/mcp/remove", &serde_json::json!({ "name": name }))
            .await
    }
}
