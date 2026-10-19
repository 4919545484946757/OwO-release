//! 非交互 `turn` 的会话选择（对标 Codex `codex exec resume --last`）。
//!
//! 选择规则集中在纯函数里，便于单测与复用（daemon REPL/TUI 若需要"续接上一个会话"
//! 也走同一规则）：
//! - 显式 `--session <id>` 优先，直接取该会话；
//! - `--continue`/`--last` 取**当前工作区**里最近更新的未归档会话；
//! - 工作区不匹配（大小写/分隔符无关）或已归档都不算候选，避免把别的项目的历史接进来。

use std::path::Path;

use owo_agent_protocol::SessionInfo;

/// 归一工作区字符串：Windows 下大小写不敏感、分隔符统一，并去掉 `\\?\` 规范化前缀
/// （CLI 侧 `canonicalize()` 会产出 `\\?\C:\x`，而桌面/HTTP 建的会话存的是 `C:\x`——
/// 两者是同一目录，不归一就会把"自己的会话"判成别的项目）。
fn normalize_workspace(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/");
    if let Some(rest) = normalized.strip_prefix("//?/UNC/") {
        normalized = format!("//{rest}");
    } else if let Some(rest) = normalized.strip_prefix("//?/") {
        normalized = rest.to_string();
    }
    let trimmed = normalized.trim_end_matches('/').to_string();
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed
    }
}

/// 会话是否属于该工作区。
pub(crate) fn session_matches_workspace(session: &SessionInfo, workspace: &Path) -> bool {
    let workspace = workspace.to_string_lossy();
    normalize_workspace(&session.workspace) == normalize_workspace(&workspace)
}

/// 该工作区里最近更新的未归档会话（按 `updated_at` 降序，同刻按 id 稳定排序）。
pub(crate) fn latest_session_for_workspace<'a>(
    sessions: &'a [SessionInfo],
    workspace: &Path,
) -> Option<&'a SessionInfo> {
    sessions
        .iter()
        .filter(|session| !session.archived && session_matches_workspace(session, workspace))
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn session(id: &str, workspace: &str, updated_at: &str, archived: bool) -> SessionInfo {
        SessionInfo {
            id: id.to_string(),
            workspace: workspace.to_string(),
            model: "mock".to_string(),
            created_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
            title: None,
            archived,
            pinned: false,
            parent_id: None,
            fork_point: None,
            goal: None,
        }
    }

    #[test]
    fn picks_the_most_recently_updated_session_in_the_workspace() {
        let workspace = PathBuf::from(r"T:\创新创业\OwO-master");
        let sessions = vec![
            session(
                "old",
                r"T:\创新创业\OwO-master",
                "2026-10-17T00:00:00Z",
                false,
            ),
            session(
                "new",
                r"T:\创新创业\OwO-master",
                "2026-10-18T00:00:00Z",
                false,
            ),
            session("other", r"T:\别的项目", "2026-10-19T00:00:00Z", false),
        ];
        let picked = latest_session_for_workspace(&sessions, &workspace).unwrap();
        assert_eq!(picked.id, "new", "必须按 updated_at 取最新且限定工作区");
    }

    #[test]
    fn archived_sessions_are_never_resumed() {
        let workspace = PathBuf::from("/ws");
        let sessions = vec![
            session("live", "/ws", "2026-10-17T00:00:00Z", false),
            session("gone", "/ws", "2026-10-18T00:00:00Z", true),
        ];
        assert_eq!(
            latest_session_for_workspace(&sessions, &workspace)
                .unwrap()
                .id,
            "live"
        );
        let only_archived = vec![session("gone", "/ws", "2026-10-18T00:00:00Z", true)];
        assert!(latest_session_for_workspace(&only_archived, &workspace).is_none());
    }

    #[test]
    fn workspace_matching_ignores_case_and_separator_style() {
        let sessions = vec![session(
            "a",
            "T:/OwO-master/",
            "2026-10-18T00:00:00Z",
            false,
        )];
        let workspace = PathBuf::from(r"t:\owo-master");
        assert!(session_matches_workspace(&sessions[0], &workspace));
        assert!(latest_session_for_workspace(&sessions, &workspace).is_some());
    }

    #[test]
    fn verbatim_prefix_from_canonicalize_still_matches_stored_workspace() {
        // CLI 的 --workspace 经 canonicalize 得到 `\\?\C:\...`；HTTP/桌面建的会话存普通路径。
        let stored = session("s", r"C:\work\proj", "2026-10-18T00:00:00Z", false);
        let canonical = PathBuf::from(r"\\?\C:\work\proj");
        assert!(
            session_matches_workspace(&stored, &canonical),
            "去掉 \\\\?\\ 前缀后应视为同一工作区"
        );
        // UNC 形态同样归一。
        let unc = session("u", r"\\server\share\proj", "2026-10-18T00:00:00Z", false);
        assert!(session_matches_workspace(
            &unc,
            Path::new(r"\\?\UNC\server\share\proj")
        ));
    }

    #[test]
    fn no_candidate_returns_none() {
        assert!(latest_session_for_workspace(&[], Path::new("/ws")).is_none());
        let other = vec![session("x", "/other", "2026-10-18T00:00:00Z", false)];
        assert!(latest_session_for_workspace(&other, Path::new("/ws")).is_none());
    }

    #[test]
    fn ties_are_broken_stably_by_id() {
        let workspace = Path::new("/ws");
        let sessions = vec![
            session("b", "/ws", "2026-10-18T00:00:00Z", false),
            session("a", "/ws", "2026-10-18T00:00:00Z", false),
        ];
        // 同一时刻重复调用必须得到同一结果（max_by 的次序依赖不能泄漏到行为）。
        let first = latest_session_for_workspace(&sessions, workspace)
            .unwrap()
            .id
            .clone();
        let second = latest_session_for_workspace(&sessions, workspace)
            .unwrap()
            .id
            .clone();
        assert_eq!(first, second);
        assert_eq!(first, "a");
    }
}
