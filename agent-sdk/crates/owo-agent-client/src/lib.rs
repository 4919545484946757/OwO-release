//! `owo-agent-client`：CLI / TUI / 桌面壳共用的**唯一** Daemon 客户端门面（指南 §7.1）。
//!
//! 依赖边界（指南 §3.2）：`clients ─► owo-agent-client ─► owo-agent-protocol`。
//! 本 crate **不得**依赖 `owo-agent-core` / `owo-agent-server` / SQLite / MCP /
//! Perception / Executor / Policy——由 `tests/dependency_guard_tests.rs` 强制。
//!
//! 提供：
//!   * `discovery`：读取 `<data_root>/runtime/daemon.json`，校验 pid 与 API 版本；
//!   * `auth`：从 `auth/token` 读取或经 `/auth/token` 引导；
//!   * `http`：`AgentClient`（认证头、JSON 往返、错误结构化）；
//!   * `sse`：turn 事件流解析（跨 chunk UTF-8 安全）；
//!   * `sessions` / `turns` / `approvals` / `diagnostics`：各域便捷方法。

pub mod approvals;
pub mod auth;
pub mod diagnostics;
pub mod discovery;
pub mod events;
pub mod http;
pub mod mcp;
pub mod session_goal;
pub mod sessions;
pub mod sse;
pub mod teams;
pub mod turns;
pub mod workflows;

mod error;

pub use error::{ClientError, Result};
pub use http::{AgentClient, ClientConfig};
pub use owo_agent_protocol::DaemonDescriptor;
pub use sse::{JsonEventStream, JsonSseBuffer, SseBuffer, SseFrame, TurnStream};

/// 发现并连接当前数据根下的 Daemon（不存在/进程已退出 → `ClientError::NotFound`）。
///
/// `expected_api_version` 为 `None` 时只要求发现文件非空且进程存活；
/// 传入版本（如 `owo_build_info::API_VERSION`）时不兼容即拒绝连接（§2.3 规则 1/7）。
pub async fn connect(
    data_root: &std::path::Path,
    expected_api_version: Option<&str>,
) -> Result<AgentClient> {
    let discovery = discovery::DaemonDiscovery::read(data_root)?;
    if let Some(expected) = expected_api_version {
        discovery.validate_api_version(expected)?;
    }
    let token = auth::read_token(data_root).ok();
    let config = ClientConfig::new(discovery.base_url(), token);
    AgentClient::new(config).map(|client| client.with_descriptor(discovery.descriptor))
}
