//! 会话变更的逐 hunk 审阅 HTTP API（G13 的服务端面）。
//!
//! - `GET  /session/{id}/hunks`：会话 diff 的逐 hunk 视图（哪些 hunk 可接受/拒绝）；
//! - `POST /session/{id}/hunks/apply`：按序号接受/拒绝 hunk 并把结果**写回磁盘**。
//!
//! 安全边界（与 `/revert` 同档，用户发起的显式操作）：
//! - 路径必须解析到工作区内（拒绝绝对路径/`..`/盘符）；
//! - **磁盘内容必须等于会话快照记录的"写后内容"**，否则 409 冲突——绝不覆盖用户后续编辑；
//! - 原子替换（临时文件 + rename），并写审计。
//!
//! 独立编译约束（沿 session_api 惯例）：不使用 `crate::`/`super::`；引用 server 类型
//! 一律写全限定名 `owo_agent_server::...`。

use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use owo_agent_core::{apply_hunks, resolve_review_path, review_files};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

/// `GET /session/{id}/hunks`：逐 hunk 审阅视图（hunk 序号 0 起，与 apply 一致）。
///
/// `conflicts` 列出"磁盘内容已被改动（与快照记录的写后内容不一致）"的文件：
/// 这些文件可以看，但 apply 会 409，客户端应提示用户先处理其编辑。
pub(super) async fn session_hunks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session = owo_agent_server::session_api::load_session(&state, &id)?;
    let files = review_files(&session.diff());
    let conflicts: Vec<String> = files
        .iter()
        .filter(|file| {
            resolve_review_path(&session.workspace, &file.path)
                .ok()
                .and_then(|absolute| {
                    owo_agent_core::review_snapshot(&session, &absolute)
                        .map(|(_, expected)| (absolute, expected))
                })
                .is_some_and(|(absolute, expected)| {
                    !owo_agent_core::disk_matches_expected_after(&absolute, expected.as_deref())
                })
        })
        .map(|file| file.path.clone())
        .collect();
    Ok(Json(json!({
        "session_id": id,
        "files": files,
        "reviewable": files.iter().filter(|file| file.is_reviewable()).count(),
        "conflicts": conflicts,
    })))
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct HunkApplyRequest {
    /// 会话 diff 里的相对路径（与 `GET /session/{id}/diff` 的 `path` 同口径）。
    pub path: String,
    /// 接受的 hunk 序号（0 起，与 `GET /session/{id}/hunks` 一致）。
    #[serde(default)]
    pub accept: Option<Vec<usize>>,
    /// 拒绝的 hunk 序号（0 起）；与 `accept` 互斥。
    #[serde(default)]
    pub reject: Option<Vec<usize>>,
}

pub(super) async fn session_hunks_apply(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<HunkApplyRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _session_guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let session = owo_agent_server::session_api::load_session(&state, &id)?;
    let workspace = session.workspace.clone();
    let absolute = resolve_review_path(&workspace, &request.path)
        .map_err(|reason| (StatusCode::BAD_REQUEST, reason))?;

    // 写前内容与"写后哈希"都取自写前快照：磁盘内容不匹配摘要 = 用户改过文件。
    let (before, expected_after) = owo_agent_core::review_snapshot(&session, &absolute)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("会话快照里没有该文件：{}", request.path),
            )
        })?;
    let Some(before) = before else {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "该文件没有写前快照（新增文件），无法逐 hunk 应用（请用 /revert）".to_string(),
        ));
    };
    let after = std::fs::read_to_string(&absolute)
        .map_err(|error| {
            (
                StatusCode::CONFLICT,
                format!("读取失败（文件可能已被删除）：{error}"),
            )
        })?
        .to_string();
    if !owo_agent_core::disk_matches_expected_after(&absolute, expected_after.as_deref()) {
        return Err((
            StatusCode::CONFLICT,
            "文件内容已被改动（与快照记录的写后内容不一致）：请先提交/备份或改用 /revert 整体回滚"
                .to_string(),
        ));
    }

    if request.accept.is_some() && request.reject.is_some() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "accept 与 reject 不能同时给出（语义冲突）".to_string(),
        ));
    }
    let hunks = owo_agent_core::diff_hunks(&before, &after);
    let total = hunks.len();
    let (accepted, rejected) = match (&request.accept, &request.reject) {
        (Some(accepted), None) => {
            let accepted = validate_indices(accepted, total)?;
            let rejected = (0..total)
                .filter(|index| !accepted.contains(index))
                .collect();
            (accepted, rejected)
        }
        (None, Some(rejected)) => {
            let rejected = validate_indices(rejected, total)?;
            let accepted = (0..total)
                .filter(|index| !rejected.contains(index))
                .collect();
            (accepted, rejected)
        }
        (None, None) => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                "需要 accept 或 reject（形如 {\"accept\":[0,2]}）".to_string(),
            ))
        }
        (Some(_), Some(_)) => unreachable!("上面已拦截"),
    };

    let next = apply_hunks(&before, &after, &accepted);
    if next == after {
        return Ok(Json(json!({
            "session_id": id,
            "path": request.path.clone(),
            "accepted": accepted,
            "rejected": rejected,
            "total": total,
            "changed": false,
            "note": "所选 hunk 与当前内容一致，未写盘",
        })));
    }
    write_atomically(&absolute, &next)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let remaining = owo_agent_core::diff_hunks(&before, &next).len();
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "session",
            "hunks_apply",
            Some(request.path.clone()),
            Some(true),
            format!(
                "逐 hunk 应用：接受 {:?} / 拒绝 {:?}（共 {total}，剩余 {remaining}）",
                accepted, rejected
            ),
        );
    }
    Ok(Json(json!({
        "session_id": id,
        "path": request.path.clone(),
        "accepted": accepted,
        "rejected": rejected,
        "total": total,
        "remaining": remaining,
        "changed": true,
    })))
}

/// 越界序号直接拒绝（静默忽略会让调用方以为已生效）。
fn validate_indices(indices: &[usize], total: usize) -> Result<Vec<usize>, (StatusCode, String)> {
    let mut unique: Vec<usize> = Vec::new();
    for index in indices {
        if *index >= total {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("hunk 序号越界：{index}（本文件共 {total} 个，序号 0 起）"),
            ));
        }
        if !unique.contains(index) {
            unique.push(*index);
        }
    }
    unique.sort_unstable();
    Ok(unique)
}

/// 原子替换：同目录临时文件 + rename（同卷 rename 不会留下半截文件）。
fn write_atomically(path: &std::path::Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法定位目录：{}", path.display()))?;
    let temp = parent.join(format!(
        ".{}.owo-hunk-tmp",
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string())
    ));
    std::fs::write(&temp, content).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, path).map_err(|error| error.to_string())?;
    Ok(())
}
