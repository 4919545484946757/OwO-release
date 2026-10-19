//! 会话变更的逐 hunk 审阅视图（服务端 `/session/{id}/hunks` 与 CLI/TUI/桌面共用）。
//!
//! 输入是会话 diff（`FileDiff{before, after}`，before 来自写前快照、after 来自磁盘），
//! 输出是"可逐 hunk 接受/拒绝"的审阅视图 + 路径解析规则。纯函数，无 IO。

use std::path::{Component, Path, PathBuf};

use owo_agent_protocol::FileDiff;

use crate::diff_hunks::{diff_hunks, DiffHunk};

/// 单个文件的审阅视图。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReviewFile {
    /// 相对工作区的展示路径（与 `FileDiff.path` 同口径）。
    pub path: String,
    /// `modified` | `added` | `removed` | `unavailable`。
    pub kind: String,
    /// 可接受/拒绝的 hunk（`unavailable` 时为空）。
    pub hunks: Vec<DiffHunk>,
    /// 不可逐 hunk 审阅的原因（`kind=unavailable` 时给出）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ReviewFile {
    /// 是否可逐 hunk 审阅（有 hunk 且两侧内容都在）。
    pub fn is_reviewable(&self) -> bool {
        !self.hunks.is_empty()
    }
}

/// 会话 diff → 逐 hunk 审阅视图。
///
/// - 两侧内容都在：切分 hunk（`modified`/`added`/`removed` 按内容形态标注）；
/// - 只有 `after`（新文件没有写前快照）：不可逐 hunk 回退，明确标注原因；
/// - 只有 `before`（文件已删除）：无可写回的内容，标注原因。
pub fn review_files(diffs: &[FileDiff]) -> Vec<ReviewFile> {
    diffs
        .iter()
        .map(
            |diff| match (diff.before.as_deref(), diff.after.as_deref()) {
                (Some(before), Some(after)) if before != after => {
                    let hunks = diff_hunks(before, after);
                    let kind = if before.is_empty() {
                        "added"
                    } else if after.is_empty() {
                        "removed"
                    } else {
                        "modified"
                    };
                    ReviewFile {
                        path: diff.path.clone(),
                        kind: kind.to_string(),
                        hunks,
                        reason: None,
                    }
                }
                (Some(_), Some(_)) => ReviewFile {
                    path: diff.path.clone(),
                    kind: "unavailable".to_string(),
                    hunks: Vec::new(),
                    reason: Some("内容与快照一致，无可审阅改动".to_string()),
                },
                (None, Some(_)) => ReviewFile {
                    path: diff.path.clone(),
                    kind: "unavailable".to_string(),
                    hunks: Vec::new(),
                    reason: Some(
                        "新增文件没有写前快照：请直接接受整份文件或用 /revert 撤销".to_string(),
                    ),
                },
                (Some(_), None) => ReviewFile {
                    path: diff.path.clone(),
                    kind: "unavailable".to_string(),
                    hunks: Vec::new(),
                    reason: Some("文件已被删除：无可写回的现内容，请用 /revert 恢复".to_string()),
                },
                (None, None) => ReviewFile {
                    path: diff.path.clone(),
                    kind: "unavailable".to_string(),
                    hunks: Vec::new(),
                    reason: Some("快照与磁盘两端都缺失内容".to_string()),
                },
            },
        )
        .collect()
}

/// 路径归一（比较用）：canonicalize 尽力而为，Windows 去掉 `\\?\` 前缀、分隔符统一、
/// 大小写不敏感。两侧都用同一归一，避免"存在路径 vs 尚不存在路径"前缀不一致
/// （`canonicalize()` 在 Windows 返回 `\\?\C:\...`，与普通路径直接比较必然不等）。
fn normalize_for_compare(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = canonical.to_string_lossy().replace('\\', "/");
    let text = match text.strip_prefix("//?/UNC/") {
        Some(rest) => format!("//{rest}"),
        None => text.strip_prefix("//?/").unwrap_or(&text).to_string(),
    };
    let trimmed = text.trim_end_matches('/').to_string();
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed
    }
}

/// 把会话 diff 里的相对路径解析为工作区内路径（拒绝绝对路径、`..`、盘符，并校验包含关系）。
pub fn resolve_review_path(workspace: &Path, relative: &str) -> Result<PathBuf, String> {
    let trimmed = relative.trim().trim_start_matches("./");
    if trimmed.is_empty() {
        return Err("路径为空".to_string());
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err(format!("只接受工作区内相对路径：{trimmed}"));
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            other => {
                return Err(format!(
                    "路径不得包含 {other:?}（只能写工作区内文件）：{trimmed}"
                ))
            }
        }
    }
    let joined = workspace.join(candidate);
    let normalized_workspace = normalize_for_compare(workspace);
    let normalized_joined = normalize_for_compare(&joined);
    if normalized_joined != normalized_workspace
        && !normalized_joined.starts_with(&format!("{normalized_workspace}/"))
    {
        return Err(format!("路径越出工作区：{trimmed}"));
    }
    Ok(joined)
}

/// 会话里某路径的写前内容与"写后哈希"（逐 hunk 应用的安全护栏）。
///
/// 返回 `(before, expected_after_sha256)`：`before` 来自写前快照（base64 解码后的文本），
/// `expected_after_sha256` 是 Agent 最近一次成功写入后的摘要——磁盘内容与它不一致
/// 说明用户改过文件，绝不能逐 hunk 写回（否则覆盖用户编辑）。
pub fn review_snapshot(
    session: &crate::session::Session,
    target: &Path,
) -> Option<(Option<String>, Option<String>)> {
    use base64::Engine;
    let canonical = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    for (path, snapshot) in &session.snapshots {
        let stored = Path::new(path);
        let stored_canonical = stored
            .canonicalize()
            .unwrap_or_else(|_| stored.to_path_buf());
        if stored_canonical != canonical && stored != target {
            continue;
        }
        let before = snapshot.original_b64.as_ref().and_then(|encoded| {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        });
        return Some((before, snapshot.expected_after_sha256.clone()));
    }
    None
}

/// 磁盘内容是否仍是 Agent 记录的"写后内容"（未记录摘要时按未改动处理）。
pub fn disk_matches_expected_after(path: &Path, expected_after_sha256: Option<&str>) -> bool {
    let Some(expected) = expected_after_sha256 else {
        return true;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    crate::CasStore::hash_of(&bytes) == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(path: &str, before: Option<&str>, after: Option<&str>) -> FileDiff {
        FileDiff {
            path: path.to_string(),
            before: before.map(str::to_string),
            after: after.map(str::to_string),
        }
    }

    #[test]
    fn snapshot_lookup_exposes_before_content_and_expected_after_hash() {
        use base64::Engine;
        let workspace = std::env::temp_dir().join(format!("owo-review-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let file = workspace.join("a.txt");
        std::fs::write(&file, "after\n").unwrap();

        let mut session = crate::session::Session::new(&workspace, "mock", None);
        session.snapshots.insert(
            file.to_string_lossy().to_string(),
            crate::session::SnapshotEntry {
                original_b64: Some(base64::engine::general_purpose::STANDARD.encode(b"before\n")),
                expected_after_sha256: Some(crate::CasStore::hash_of(b"after\n")),
                turn: 0,
            },
        );

        let (before, expected) = review_snapshot(&session, &file).expect("应找到快照");
        assert_eq!(before.as_deref(), Some("before\n"));
        assert!(disk_matches_expected_after(&file, expected.as_deref()));

        // 用户改过文件 → 摘要不匹配（逐 hunk 应用必须拒绝）。
        std::fs::write(&file, "user edit\n").unwrap();
        assert!(!disk_matches_expected_after(&file, expected.as_deref()));
        // 没有记录摘要时按未改动处理（老数据兼容）。
        assert!(disk_matches_expected_after(&file, None));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn modified_file_exposes_hunks_and_kind() {
        let files = review_files(&[diff("src/a.rs", Some("1\n2\n3\n"), Some("1\nX\n3\n"))]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, "modified");
        assert_eq!(files[0].hunks.len(), 1);
        assert!(files[0].is_reviewable());
    }

    #[test]
    fn added_and_removed_files_are_marked_but_not_reviewable() {
        let added = review_files(&[diff("new.txt", Some(""), Some("hello\n"))]);
        assert_eq!(added[0].kind, "added");
        assert_eq!(added[0].hunks.len(), 1, "整份新增可作为单个 hunk 展示");

        let created_without_snapshot = review_files(&[diff("new.txt", None, Some("hello\n"))]);
        assert_eq!(created_without_snapshot[0].kind, "unavailable");
        assert!(created_without_snapshot[0].hunks.is_empty());
        assert!(created_without_snapshot[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("没有写前快照"));

        let deleted = review_files(&[diff("gone.txt", Some("bye\n"), None)]);
        assert_eq!(deleted[0].kind, "unavailable");
        assert!(
            deleted[0].reason.as_deref().unwrap().contains("删除"),
            "{:?}",
            deleted[0].reason
        );
    }

    #[test]
    fn identical_content_is_not_reported_as_reviewable() {
        let files = review_files(&[diff("same.txt", Some("x\n"), Some("x\n"))]);
        assert_eq!(files[0].kind, "unavailable");
        assert!(!files[0].is_reviewable());
    }

    #[test]
    fn path_resolution_rejects_escapes_and_accepts_workspace_relative_paths() {
        let workspace = std::env::temp_dir().canonicalize().unwrap();
        let nested = workspace.join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("a.txt");
        std::fs::write(&file, "x\n").unwrap();

        assert!(resolve_review_path(&workspace, "sub/a.txt").is_ok());
        assert!(resolve_review_path(&workspace, "./sub/a.txt").is_ok());
        // 尚不存在的目标也必须落在工作区内（canonicalize 失败时按归一前缀判断）。
        assert!(resolve_review_path(&workspace, "sub/not-created-yet.txt").is_ok());
        assert!(resolve_review_path(&workspace, "not-created-yet.txt").is_ok());
        // 相邻目录不算工作区内（前缀必须按路径分隔符对齐）。
        let sibling = workspace.join("..").join("sibling-dir");
        std::fs::create_dir_all(&sibling).unwrap();
        assert!(resolve_review_path(&workspace, "../sibling-dir/x.txt").is_err());
        assert!(resolve_review_path(&workspace, "../escape.txt").is_err());
        assert!(resolve_review_path(&workspace, r"sub\..\..\escape.txt").is_err());
        assert!(resolve_review_path(&workspace, r"C:\Windows\system.ini").is_err());
        assert!(resolve_review_path(&workspace, "").is_err());
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&nested);
    }
}
