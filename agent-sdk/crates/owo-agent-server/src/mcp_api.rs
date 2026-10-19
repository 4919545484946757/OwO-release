//! MCP HTTP API（§12 第一刀：从 lib.rs 机械外移的 MCP 管理域）。
//!
//! 路由面（`/mcp`、`/mcp/health`、`/mcp/add`、`/mcp/remove`）与 /openapi.json
//! 登记保持不变，本模块只承载领域处理函数与配置持久化 helper，零行为变化。
//!
//! 独立编译约束（沿 notes_api 惯例）：不使用 `crate::`/`super::`；引用 server
//! 类型一律写全限定名 `owo_agent_server::AppState`。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

use owo_agent_server::AppState;

#[derive(Deserialize)]
pub(super) struct McpAddRequest {
    name: String,
    /// "stdio" 或 "http"
    #[serde(default = "default_mcp_transport")]
    transport: String,
    /// stdio 传输时的启动命令
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    /// http 传输时的端点 URL
    #[serde(default)]
    url: Option<String>,
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

#[derive(Deserialize)]
pub(super) struct McpRemoveRequest {
    name: String,
}

#[derive(Deserialize)]
pub(super) struct McpReconnectRequest {
    name: String,
}

#[derive(Deserialize)]
pub(super) struct McpEnabledRequest {
    name: String,
    enabled: bool,
}

fn load_mcp_configs(root: &Path) -> Vec<owo_agent_core::McpServerConfig> {
    std::fs::read_to_string(root.join("mcp-servers.json"))
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_mcp_configs(root: &Path, configs: &[owo_agent_core::McpServerConfig]) {
    if let Ok(content) = serde_json::to_string_pretty(configs) {
        let _ = std::fs::write(root.join("mcp-servers.json"), content);
    }
}

pub(super) async fn mcp_list(State(state): State<Arc<AppState>>) -> Json<Value> {
    let configs = load_mcp_configs(&state.data_root);
    let settings = owo_agent_core::Settings::load(&state.workspace);
    let mut merged = configs.clone();
    for server in settings.mcp_servers {
        if !merged.iter().any(|config| config.name == server.name) {
            merged.push(server);
        }
    }
    Json(json!({
        "count": merged.len(),
        "servers": merged,
    }))
}

// ==== mcp_health_snapshot（§10 可观测面：per-server 状态机/熔断/失败计数） ====
pub(super) async fn mcp_health_snapshot(State(state): State<Arc<AppState>>) -> Json<Value> {
    let snapshot = state.agent.mcp_health().snapshot();
    Json(json!({
        "count": snapshot.len(),
        "servers": snapshot,
    }))
}

pub(super) async fn mcp_add(
    State(state): State<Arc<AppState>>,
    Json(request): Json<McpAddRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // 名字规则与 CLI `owo-agent mcp` 共用唯一实现：`__` 会破坏
    // `mcp__{server}__{tool}` 命名空间反解，空白/路径字符会污染配置与审计。
    let name = owo_agent_core::validate_mcp_server_name(&request.name)
        .map_err(|reason| (StatusCode::BAD_REQUEST, reason))?;
    let config = owo_agent_core::McpServerConfig {
        name: name.clone(),
        transport: request.transport.clone(),
        command: request.command.clone(),
        args: request.args.clone(),
        url: request.url.clone(),
        timeout_ms: None,
        network_allowlist: Vec::new(),
        trusted_readonly: Vec::new(),
    };
    let mut configs = load_mcp_configs(&state.data_root);
    if configs.iter().any(|existing| existing.name == name) {
        return Err((StatusCode::CONFLICT, format!("MCP 服务器 {name} 已存在")));
    }
    // 热连接（经 Agent 注册表：注册工具 + 记入进程注册表，可被 /mcp/remove 进程级卸载）。
    let tool_count = state
        .agent
        .connect_mcp_server(&config)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("连接失败：{error}")))?;
    let connected = true;
    configs.push(config);
    save_mcp_configs(&state.data_root, &configs);
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "mcp",
            "add",
            Some(name.clone()),
            Some(true),
            format!(
                "新增 MCP 服务器 {name}（{}，工具 {tool_count} 个）",
                request.transport
            ),
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Mcp);
    Ok(Json(json!({
        "ok": true,
        "name": name,
        "connected": connected,
        "tools": tool_count,
    })))
}

pub(super) async fn mcp_remove(
    State(state): State<Arc<AppState>>,
    Json(request): Json<McpRemoveRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let name = request.name.trim().to_string();
    let mut configs = load_mcp_configs(&state.data_root);
    let before = configs.len();
    configs.retain(|config| config.name != name);
    if configs.len() == before {
        return Err((StatusCode::NOT_FOUND, format!("MCP 服务器 {name} 不存在")));
    }
    save_mcp_configs(&state.data_root, &configs);
    //
    // 进程级卸载：kill stdio 子进程 + 撤销工具（前缀移除且禁用）。
    let process_killed = state
        .agent
        .shutdown_mcp_server(&name)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "mcp",
            "remove",
            Some(name.clone()),
            Some(true),
            format!(
                "移除 MCP 服务器 {name}（{}）",
                if process_killed {
                    "子进程已终止"
                } else {
                    "未连接"
                }
            ),
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Mcp);
    Ok(Json(
        json!({ "ok": true, "name": name, "process_killed": process_killed }),
    ))
}

/// 任务 10 尾：MCP 重连——从已保存配置热重建连接（进程级卸载 + connect_mcp_server）。
/// 配置来源与 /mcp 列表一致（mcp-servers.json ∪ settings.mcp_servers 去重合并）。
pub(super) async fn mcp_reconnect(
    State(state): State<Arc<AppState>>,
    Json(request): Json<McpReconnectRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let name = request.name.trim().to_string();
    let mut configs = load_mcp_configs(&state.data_root);
    let settings = owo_agent_core::Settings::load(&state.workspace);
    for server in settings.mcp_servers {
        if !configs.iter().any(|config| config.name == server.name) {
            configs.push(server);
        }
    }
    let config = configs
        .iter()
        .find(|config| config.name == name)
        .cloned()
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("MCP 服务器 {name} 不存在")))?;
    // 幂等：未连接时 shutdown 返回 false，不视为错误；随后按配置热连接。
    let _ = state.agent.shutdown_mcp_server(&name).await;
    let tool_count = state
        .agent
        .connect_mcp_server(&config)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("重连失败：{error}")))?;
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "mcp",
            "reconnect",
            Some(name.clone()),
            Some(true),
            format!("重连 MCP 服务器 {name}（工具 {tool_count} 个）"),
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Mcp);
    Ok(Json(json!({
        "ok": true,
        "name": name,
        "tools": tool_count,
    })))
}

/// 任务 10 尾：MCP 工具前缀启停——热卸载语义（模型不可见 + 直接调用被拒，无需重建）。
/// 注意：进程级开关，不持久化；服务重启后恢复启用（审计留痕）。
pub(super) async fn mcp_enabled(
    State(state): State<Arc<AppState>>,
    Json(request): Json<McpEnabledRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let name = request.name.trim().to_string();
    let known = {
        let mut configs = load_mcp_configs(&state.data_root);
        let settings = owo_agent_core::Settings::load(&state.workspace);
        for server in settings.mcp_servers {
            if !configs.iter().any(|config| config.name == server.name) {
                configs.push(server);
            }
        }
        configs.iter().any(|config| config.name == name)
    };
    if !known {
        return Err((StatusCode::NOT_FOUND, format!("MCP 服务器 {name} 不存在")));
    }
    let prefix = owo_agent_core::tools::mcp_tool_prefix(&name);
    state
        .agent
        .set_tool_prefix_enabled(&prefix, request.enabled);
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "mcp",
            if request.enabled { "enable" } else { "disable" },
            Some(name.clone()),
            Some(true),
            format!(
                "{} MCP 服务器 {name} 的工具（前缀 {prefix}，进程级不持久化）",
                if request.enabled { "启用" } else { "禁用" }
            ),
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Mcp);
    Ok(Json(json!({
        "ok": true,
        "name": name,
        "enabled": request.enabled,
        "note": "进程级开关，服务重启后恢复启用",
    })))
}
