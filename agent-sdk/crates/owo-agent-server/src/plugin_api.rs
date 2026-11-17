//! 插件 HTTP API（§12：从 lib.rs 机械外移的 plugins 域）。
//!
//! 路由面（`GET /plugins`、`POST /plugins/{id}/enabled`）与 /openapi.json 登记
//! 保持不变，零行为变化。启停的 MCP 进程级热卸载/重连语义原样保留。
//!
//! 独立编译约束（沿 notes_api 惯例）：不使用 `crate::`/`super::`；引用 server
//! 类型一律写全限定名 `owo_agent_server::AppState`。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

#[derive(Deserialize)]
pub(super) struct PluginEnabledRequest {
    enabled: bool,
}

pub(super) async fn plugins_list(State(state): State<Arc<AppState>>) -> Json<Value> {
    let plugins = owo_agent_core::discover_plugins(&state.workspace, &state.data_root);
    let plugin_state = state
        .plugin_state
        .lock()
        .map(|guard| guard.disabled_ids())
        .unwrap_or_default();
    let items: Vec<Value> = plugins
        .into_iter()
        .map(|(path, manifest)| {
            let enabled = !plugin_state.contains(&manifest.id);
            let tools_hidden = state
                .agent
                .tool_disabled(&owo_agent_core::tools::mcp_tool_prefix(&manifest.id));
            // 签名状态：历史格式看清单字段；规范包看包根是否有 plugin.sig.json
            // （只报告"有无"，不在此处重算摘要——验签发生在安装/更新/check）。
            let signed = manifest.signature.is_some()
                || (path.file_name().and_then(|name| name.to_str())
                    == Some(owo_agent_core::agent_plugins::AGENT_PLUGINS_MANIFEST_FILE)
                    && path
                        .parent()
                        .map(|dir| {
                            dir.join(owo_agent_core::agent_plugins::AGENT_PLUGIN_SIGNATURE_FILE)
                                .exists()
                        })
                        .unwrap_or(false));
            json!({
                "id": manifest.id,
                "name": manifest.name,
                "version": manifest.version,
                "description": manifest.description,
                "enabled": enabled,
                "signed": signed,
                "tools_hidden": tools_hidden,
                "permissions": manifest.permissions,
                // 历史单服务器字段（保留兼容）；Agent Plugins 包见 mcp_servers。
                "mcp": manifest.mcp.as_ref().map(|mcp| json!({
                    "name": mcp.name,
                    "transport": mcp.transport,
                    "command": mcp.command,
                    "args": mcp.args,
                })),
                // 只暴露可展示的连接信息；env/headers 可能含凭据，绝不出网。
                "mcp_servers": manifest.mcp_servers.iter().map(|mcp| json!({
                    "name": mcp.name,
                    "transport": mcp.transport,
                    "command": mcp.command,
                    "args": mcp.args,
                    "url": mcp.url,
                })).collect::<Vec<_>>(),
                // 插件注入的 hooks：事件/匹配器可见（透明性），命令路径不外泄。
                "hooks": manifest.hooks.iter().map(|hook| json!({
                    "event": hook.event,
                    "matcher": hook.matcher,
                    "fail_mode": hook.fail_mode,
                })).collect::<Vec<_>>(),
                "format": if path.file_name().and_then(|name| name.to_str())
                    == Some(owo_agent_core::agent_plugins::AGENT_PLUGINS_MANIFEST_FILE) {
                    "agent-plugins-1.0.0"
                } else {
                    "owo-manifest"
                },
                "manifest_path": path.to_string_lossy(),
            })
        })
        .collect();
    Json(json!({ "count": items.len(), "plugins": items }))
}

pub(super) async fn plugin_enabled(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<PluginEnabledRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    {
        let mut plugin_state = state.plugin_state.lock().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "插件状态锁中毒".to_string(),
            )
        })?;
        plugin_state
            .set_enabled(&id, request.enabled)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    let prefix = owo_agent_core::tools::mcp_tool_prefix(&id);
    let mut process_killed = false;
    let mut tools = 0usize;
    if request.enabled {
        // 启用：重新连接插件 MCP 服务器并注册工具（幂等：先清理旧连接再连接）。
        let _ = state.agent.shutdown_mcp_server(&id).await;
        let discovered = owo_agent_core::discover_plugins(&state.workspace, &state.data_root);
        let Some((manifest_path, manifest)) = discovered.into_iter().find(|(_, m)| m.id == id)
        else {
            return Err((
                StatusCode::NOT_FOUND,
                format!("插件 {id} 不存在（已从工作区移除？）"),
            ));
        };
        match owo_agent_core::plugin_mcp_config(&manifest_path, &manifest) {
            Some(config) => match state.agent.connect_mcp_server(&config).await {
                Ok(count) => {
                    tools = count;
                    state.agent.set_tool_prefix_enabled(&prefix, true);
                }
                Err(error) => {
                    // 连接失败仍标记启用（状态持久化），工具不可用由 UI 提示。
                    return Err((
                        StatusCode::BAD_GATEWAY,
                        format!("插件 MCP 服务器连接失败：{error}"),
                    ));
                }
            },
            None => {
                // 无 MCP 声明（纯视图插件）：仅恢复前缀。
                state.agent.set_tool_prefix_enabled(&prefix, true);
            }
        }
    } else {
        // 禁用：进程级热卸载（kill 子进程 + 撤销工具）。
        process_killed = state
            .agent
            .shutdown_mcp_server(&id)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "plugin",
            "set-enabled",
            Some(id.clone()),
            Some(true),
            format!(
                "插件 {} 已{}（{}）",
                id,
                if request.enabled { "启用" } else { "禁用" },
                if request.enabled {
                    format!("重新连接 MCP，工具 {tools} 个")
                } else if process_killed {
                    "进程级热卸载（子进程已终止）".to_string()
                } else {
                    "无 MCP 子进程".to_string()
                }
            ),
        );
    }
    // §3.2：插件启停成功后发布 plugins 域失效（失败路径提前 return，不发布）。
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Plugins);
    Ok(Json(json!({
        "ok": true,
        "id": id,
        "enabled": request.enabled,
        "process_killed": process_killed,
        "tools": tools,
    })))
}
