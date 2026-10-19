//! `owo-agent sessions`：列出会话（对标 Codex `codex resume` 的选择器；脚本可 `--json`）。
//!
//! 只经 Daemon 读取（CLI 不直连 SQLite）：复用 `ensure_daemon_client` 的单实例协议。
//! 默认只列**当前工作区**的未归档会话，并标出 `owo-agent turn --continue` 会选中的那个
//! ——标记与 `turn` 的选择共用 `support::latest_session_for_workspace`，不会各说各话。

use std::path::PathBuf;

use clap::Args;
use owo_agent_protocol::SessionInfo;

use crate::support::{
    ensure_daemon_client, ensure_data_root, latest_session_for_workspace, session_matches_workspace,
};

/// 默认展示条数（`--limit`/`--all` 可调）。
const DEFAULT_LIMIT: usize = 20;

#[derive(Debug, Args)]
pub struct SessionsArgs {
    /// 工作区（默认当前目录；只列该工作区的会话）
    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,
    /// 数据根（缺省用户级数据目录）；用于发现/启动共享 Daemon
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// 机器可读输出（JSON）
    #[arg(long)]
    pub json: bool,
    /// 最多列出多少条（默认 20）
    #[arg(long)]
    pub limit: Option<usize>,
    /// 列出所有工作区的会话（含已归档）
    #[arg(long)]
    pub all: bool,
}

pub(crate) fn run_sessions_cmd(args: SessionsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = args
        .workspace
        .canonicalize()
        .map_err(|error| format!("工作区不可用：{}（{error}）", args.workspace.display()))?;
    let root = ensure_data_root(args.data_dir.clone(), &workspace);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let client = ensure_daemon_client(&root, &workspace).await?;
        let sessions = client.list_sessions().await?;
        let continue_target =
            latest_session_for_workspace(&sessions, &workspace).map(|session| session.id.clone());
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        let rows = session_rows(
            &sessions,
            &workspace,
            args.all,
            if args.all { usize::MAX } else { limit },
            continue_target.as_deref(),
        );
        if args.json {
            println!("{}", serde_json::to_string_pretty(&rows)?);
            return Ok(());
        }
        if rows.is_empty() {
            println!(
                "（{} 没有会话；用 `owo-agent turn --prompt <任务>` 发起第一个）",
                if args.all {
                    "数据根".to_string()
                } else {
                    format!("工作区 {}", workspace.display())
                }
            );
            return Ok(());
        }
        let hidden = total_in_scope(&sessions, &workspace, args.all).saturating_sub(rows.len());
        println!(
            "会话（{} 条{}）：",
            rows.len(),
            if hidden > 0 {
                format!("，另有 {hidden} 条未显示（--limit/--all）")
            } else {
                String::new()
            }
        );
        for row in &rows {
            println!("{}", format_row(row));
        }
        if continue_target.is_some() {
            println!("（* = `owo-agent turn --continue` 会续接这个）");
        }
        Ok(())
    })
}

/// 一行会话（JSON 与人类输出共用同一结构，字段即契约）。
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub(crate) struct SessionRow {
    pub id: String,
    pub title: String,
    pub model: String,
    pub updated_at: String,
    pub archived: bool,
    pub pinned: bool,
    /// 目标摘要（objective/iterations/done/origin）；无目标为 null。
    pub goal: Option<GoalSummary>,
    /// 是否为 `turn --continue` 会选中的会话。
    pub continue_target: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub(crate) struct GoalSummary {
    pub objective: String,
    pub iterations: usize,
    pub done: bool,
    pub origin: String,
}

fn total_in_scope(sessions: &[SessionInfo], workspace: &std::path::Path, all: bool) -> usize {
    sessions
        .iter()
        .filter(|session| {
            all || (!session.archived && session_matches_workspace(session, workspace))
        })
        .count()
}

/// 过滤 + 排序 + 截断（排序与 Daemon 一致：置顶优先、再按更新时间的倒序）。
pub(crate) fn session_rows(
    sessions: &[SessionInfo],
    workspace: &std::path::Path,
    all: bool,
    limit: usize,
    continue_target: Option<&str>,
) -> Vec<SessionRow> {
    let mut selected: Vec<&SessionInfo> = sessions
        .iter()
        .filter(|session| {
            all || (!session.archived && session_matches_workspace(session, workspace))
        })
        .collect();
    selected.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
    });
    selected
        .into_iter()
        .take(limit)
        .map(|session| SessionRow {
            id: session.id.clone(),
            title: session
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| "（未命名）".to_string()),
            model: session.model.clone(),
            updated_at: session.updated_at.clone(),
            archived: session.archived,
            pinned: session.pinned,
            goal: session.goal.as_ref().map(|goal| GoalSummary {
                objective: goal.objective.clone(),
                iterations: goal.iterations,
                done: goal.done,
                origin: goal.origin.clone(),
            }),
            continue_target: continue_target == Some(session.id.as_str()),
        })
        .collect()
}

fn format_row(row: &SessionRow) -> String {
    let marker = if row.continue_target { "*" } else { " " };
    let flags = format!(
        "{}{}",
        if row.pinned { " 📌" } else { "" },
        if row.archived { "（已归档）" } else { "" }
    );
    let mut line = format!(
        "  {marker} {}  {}  {}{}  {}",
        short_id(&row.id),
        compact_time(&row.updated_at),
        row.model,
        flags,
        row.title
    );
    if let Some(goal) = &row.goal {
        line.push_str(&format!(
            "\n      目标：{}（{} 轮，{}{}）",
            goal.objective,
            goal.iterations,
            if goal.done { "已完成" } else { "推进中" },
            if goal.origin == "model" {
                "，模型登记"
            } else {
                ""
            }
        ));
    }
    line
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// RFC3339 → `MM-DD HH:MM`（时区部分照实截断展示，不臆造本地时间）。
fn compact_time(updated_at: &str) -> String {
    if updated_at.len() < 16 {
        return updated_at.to_string();
    }
    let date = &updated_at[5..10];
    let time = &updated_at[11..16];
    format!("{date} {time}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn session(
        id: &str,
        workspace: &str,
        updated_at: &str,
        archived: bool,
        pinned: bool,
    ) -> SessionInfo {
        SessionInfo {
            id: id.to_string(),
            workspace: workspace.to_string(),
            model: "mock".to_string(),
            created_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
            title: None,
            archived,
            pinned,
            parent_id: None,
            fork_point: None,
            goal: None,
        }
    }

    #[test]
    fn rows_filter_by_workspace_and_default_hide_archived() {
        let sessions = vec![
            session("s1", "/ws", "2026-10-18T07:00:00Z", false, false),
            session("s2", "/ws", "2026-10-18T08:00:00Z", true, false),
            session("s3", "/other", "2026-10-18T09:00:00Z", false, false),
        ];
        let rows = session_rows(&sessions, Path::new("/ws"), false, 20, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "s1");

        let all = session_rows(&sessions, Path::new("/ws"), true, 20, None);
        assert_eq!(all.len(), 3, "--all 应包含归档与其它工作区");
    }

    #[test]
    fn rows_sort_pinned_first_then_by_update_time_and_apply_limit() {
        let sessions = vec![
            session("old", "/ws", "2026-10-17T00:00:00Z", false, false),
            session("new", "/ws", "2026-10-18T00:00:00Z", false, false),
            session("pin", "/ws", "2026-10-01T00:00:00Z", false, true),
        ];
        let rows = session_rows(&sessions, Path::new("/ws"), false, 20, None);
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["pin", "new", "old"]
        );
        let limited = session_rows(&sessions, Path::new("/ws"), false, 2, None);
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].id, "pin");
    }

    #[test]
    fn continue_marker_matches_the_same_helper_turn_uses() {
        let sessions = vec![
            session("old", "/ws", "2026-10-17T00:00:00Z", false, false),
            session("new", "/ws", "2026-10-18T00:00:00Z", false, false),
        ];
        let target = latest_session_for_workspace(&sessions, Path::new("/ws"))
            .map(|session| session.id.clone());
        let rows = session_rows(&sessions, Path::new("/ws"), false, 20, target.as_deref());
        let marked: Vec<&str> = rows
            .iter()
            .filter(|row| row.continue_target)
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(marked, vec!["new"], "标记必须与 turn --continue 的选择一致");
    }

    #[test]
    fn unknown_or_missing_titles_and_goal_summaries_are_rendered_truthfully() {
        let mut with_goal = session("g1", "/ws", "2026-10-18T08:00:00Z", false, false);
        with_goal.goal = Some(owo_agent_protocol::SessionGoalV1 {
            objective: "把文档补全".to_string(),
            iterations: 2,
            done: false,
            updated_at: "2026-10-18T08:00:00Z".to_string(),
            origin: "model".to_string(),
        });
        let sessions = vec![
            with_goal,
            session("g2", "/ws", "2026-10-18T07:00:00Z", false, false),
        ];
        let rows = session_rows(&sessions, Path::new("/ws"), false, 20, Some("g1"));
        let rendered = rows.iter().map(format_row).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("（未命名）"), "{rendered}");
        assert!(rendered.contains("* g1"), "{rendered}");
        assert!(
            rendered.contains("目标：把文档补全（2 轮，推进中，模型登记）"),
            "{rendered}"
        );
        assert!(
            rendered.contains("10-18 08:00"),
            "时间应压缩展示：{rendered}"
        );
    }

    #[test]
    fn compact_time_keeps_short_values_verbatim() {
        assert_eq!(compact_time("2026-10-18T08:00:00+00:00"), "10-18 08:00");
        assert_eq!(compact_time("未知"), "未知");
    }
}
