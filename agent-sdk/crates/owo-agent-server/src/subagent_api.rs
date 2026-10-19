//! 子代理 HTTP API（§12：从 lib.rs 机械外移的 subagent 域）。
//!
//! 路由面（`POST /subagent/run`）与 /openapi.json 登记保持不变，零行为变化。
//! 只读探索（@explore 对齐）与通用子代理（@subagent 对齐）语义原样保留。
//!
//! 独立编译约束（沿 notes_api 惯例）：不使用 `crate::`/`super::`；引用 server
//! 类型一律写全限定名 `owo_agent_server::AppState`。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

#[derive(Deserialize)]
pub(super) struct SubagentRunRequest {
    prompt: String,
    /// true 为只读探索模式（对齐 CLI `@explore`）；false 为通用子代理（对齐 `@subagent`）。
    ///
    /// 缺省从"通用"改为"只读"：调用方忘传字段时 fail-closed，避免非只读子代理
    /// 在无审批通道的情况下执行命令（审查 N3）。
    #[serde(default = "default_read_only")]
    read_only: bool,
    /// `read_only=false` 时是否放行工作区内命令执行；缺省 false（必须显式 opt-in）。
    #[serde(default)]
    allow_commands: bool,
    #[serde(default)]
    model: Option<String>,
}

fn default_read_only() -> bool {
    true
}

/// M4.2 子代理模型解析：显式请求 > fast 档（`OWO_MODEL_FAST`）> 空串（自动 =
/// Provider 解析链，OPENAI_MODEL 热切换 → 启动配置 → 内置默认）。
/// 返回 `(wire, display)`：wire 为契约执行器固定模型（空 = 不固定）；
/// display 仅供响应/审计展示，绝不参与路由。
fn resolve_subagent_model(explicit: Option<String>) -> (String, String) {
    if let Some(value) = explicit
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v != owo_agent_core::gateway::MODEL_DEFAULT_SENTINEL)
    {
        return (value.clone(), value);
    }
    if let Some(fast) =
        owo_agent_core::gateway::resolve_tier_model(owo_agent_core::gateway::ModelTier::Fast)
    {
        return (fast.clone(), fast);
    }
    let display = std::env::var("OPENAI_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| owo_agent_core::gateway::DEFAULT_MODEL_ID.to_string());
    (String::new(), display)
}

pub(super) async fn subagent_run(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SubagentRunRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if request.prompt.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "缺少 prompt".to_string()));
    }
    let started = std::time::Instant::now();
    let workspace = state.workspace.clone();
    let (wire_model, display_model) = resolve_subagent_model(request.model);
    let agent = Arc::clone(&state.agent);
    let text = agent
        .run_subagent_with_options(
            &workspace,
            &wire_model,
            &request.prompt,
            request.read_only,
            request.allow_commands,
        )
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let duration_ms = started.elapsed().as_millis() as u64;
    let mode = if request.read_only {
        "只读探索"
    } else if request.allow_commands {
        "通用（含命令）"
    } else {
        "通用"
    };
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "subagent",
            if request.read_only { "explore" } else { "run" },
            None,
            Some(true),
            format!(
                "{}子代理完成（{}ms）：{}",
                mode,
                duration_ms,
                request.prompt.chars().take(120).collect::<String>()
            ),
        );
    }
    Ok(Json(json!({
        "ok": true,
        "read_only": request.read_only,
        "allow_commands": request.allow_commands && !request.read_only,
        "model": display_model,
        "duration_ms": duration_ms,
        "text": text,
    })))
}

#[cfg(test)]
mod tests {
    use super::resolve_subagent_model;

    /// 本文件按独立编译约束不使用 crate::/super:: 路径（生产代码），
    /// 测试仅在本模块内引用私有函数，不破坏该约束。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn explicit_beats_fast_tier_beats_auto_with_display_fallback() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        for name in ["OWO_MODEL_FAST", "OPENAI_MODEL"] {
            std::env::remove_var(name);
        }
        // 显式优先（含 trim）。
        assert_eq!(
            resolve_subagent_model(Some(" pick-me ".to_string())),
            ("pick-me".to_string(), "pick-me".to_string())
        );
        // 无显式 → fast 档进请求体（wire=display=fast-m）。
        std::env::set_var("OWO_MODEL_FAST", "fast-m");
        assert_eq!(
            resolve_subagent_model(None),
            ("fast-m".to_string(), "fast-m".to_string())
        );
        // 无显式、无 fast 档 → wire 空（自动，Provider 解析链）；display 仅展示。
        std::env::remove_var("OWO_MODEL_FAST");
        std::env::set_var("OPENAI_MODEL", "env-m");
        assert_eq!(
            resolve_subagent_model(Some("  ".to_string())),
            (String::new(), "env-m".to_string())
        );
        // "default" 哨兵 = 自动（与空同义，不得固定到请求体）。
        assert_eq!(
            resolve_subagent_model(Some("default".to_string())),
            (String::new(), "env-m".to_string())
        );
        std::env::remove_var("OPENAI_MODEL");
        let default_display = owo_agent_core::gateway::DEFAULT_MODEL_ID.to_string();
        assert_eq!(
            resolve_subagent_model(None),
            (String::new(), default_display)
        );
    }
}
