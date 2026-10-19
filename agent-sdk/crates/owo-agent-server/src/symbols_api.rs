//! `GET /symbols`：符号级检索的服务端面（与 `find_symbol` 工具、CLI `owo-agent symbols`
//! 共用 `owo_agent_core::find_symbols` 内核）。
//!
//! 只读、有界、无索引：查询串走 query params，返回与工具一致的字段
//! （`results/scanned_files/skipped_files/truncated/...`），便于桌面与 TUI 直接渲染。
//!
//! 独立编译约束（沿 session_api 惯例）：不使用 `crate::`/`super::`。

use axum::extract::{Query, State};
use axum::http::StatusCode;
use owo_agent_core::{find_symbols, DEFAULT_SYMBOL_RESULTS, MAX_SYMBOL_RESULTS};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

#[derive(Debug, Default, Deserialize)]
pub(super) struct SymbolsQuery {
    /// 符号名或片段（大小写不敏感）；必填。
    pub query: Option<String>,
    /// 定义种类过滤（function|struct|enum|trait|class|interface|type|const|module|any）。
    pub kind: Option<String>,
    /// 限定在工作区内某个相对子目录。
    pub path: Option<String>,
    /// 返回条数（默认 30，硬顶 200）。
    pub max_results: Option<usize>,
    /// 用该会话的工作区作为扫描基座（桌面多项目场景：Daemon 的工作区未必是被打开的项目）。
    pub session_id: Option<String>,
}

pub(super) async fn symbols(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SymbolsQuery>,
) -> Result<axum::Json<Value>, (StatusCode, String)> {
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if query.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "缺少查询参数 query（符号名或名字片段）".to_string(),
        ));
    }
    // 基座工作区：给了 session_id 就以该会话的工作区为准（多项目桌面端必须如此），
    // 否则退回 Daemon 自己的工作区。
    let (workspace, session_id) = match params.session_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => {
            let session = owo_agent_server::session_api::load_session(&state, id)?;
            (session.workspace.clone(), Some(id.to_string()))
        }
        _ => (state.workspace.clone(), None),
    };
    let root = match params.path.as_deref().map(str::trim) {
        Some(relative) if !relative.is_empty() => {
            owo_agent_core::resolve_review_path(&workspace, relative)
                .map_err(|reason| (StatusCode::BAD_REQUEST, reason))?
        }
        _ => workspace.clone(),
    };
    if !root.is_dir() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("扫描目录不存在：{}", root.display()),
        ));
    }
    let max_results = params
        .max_results
        .unwrap_or(DEFAULT_SYMBOL_RESULTS)
        .clamp(1, MAX_SYMBOL_RESULTS);
    let scan = find_symbols(
        &root,
        &workspace,
        &query,
        params.kind.as_deref(),
        max_results,
    );
    let root_display = root
        .strip_prefix(&workspace)
        .map(|relative| {
            let text = relative.to_string_lossy().replace('\\', "/");
            if text.is_empty() {
                ".".to_string()
            } else {
                text
            }
        })
        .unwrap_or_else(|_| root.display().to_string());
    Ok(axum::Json(json!({
        "query": query,
        "kind": params.kind,
        "session_id": session_id,
        "root": root_display,
        "results": scan.definitions,
        "scanned_files": scan.stats.scanned_files,
        "skipped_files": scan.stats.skipped_files,
        "truncated": scan.stats.results_truncated || scan.stats.file_limit_reached,
        "file_limit_reached": scan.stats.file_limit_reached,
        "results_truncated": scan.stats.results_truncated,
    })))
}
