//! 逐 hunk 变更审阅（对标 G13）：把"原内容 → 现内容"切分成可独立接受/拒绝的 hunk。
//!
//! 纯函数、无 IO：审阅列表与部分应用共用同一套分段（列表里看到的 hunk 序号，
//! 与 `apply_hunks` 接受/拒绝的序号严格一致）。
//!
//! 算法与边界：
//! - 先裁掉公共前后缀（真实改动通常只占文件一小段）；
//! - 中间部分做行级 LCS 分段；LCS 规模超过 [`MAX_LCS_CELLS`] 时退化为**单个粗 hunk**
//!   （宁可给"整段替换"，也不做无界内存/时间开销，且不丢内容）；
//! - 连续变化的行为一个 hunk；行尾 `\r\n` 原样保留，不做换行归一。

/// LCS 动态规划的最大单元数；超过则退化为单个粗 hunk。
pub const MAX_LCS_CELLS: usize = 250_000;

/// 一个可独立接受/拒绝的变更段。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiffHunk {
    /// 在本次审阅里的序号（从 0 起），接受/拒绝都用它。
    pub index: usize,
    /// 原内容中的起始行（0 起，含）。
    pub old_start: usize,
    /// 原内容中该段的行数。
    pub old_lines: usize,
    /// 现内容中的起始行（0 起，含）。
    pub new_start: usize,
    /// 现内容中该段的行数。
    pub new_lines: usize,
}

impl DiffHunk {
    /// 该 hunk 是否为纯新增/纯删除/替换（供展示用）。
    pub fn kind(&self) -> &'static str {
        match (self.old_lines, self.new_lines) {
            (0, _) => "added",
            (_, 0) => "removed",
            _ => "modified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Segment {
    Equal {
        old_start: usize,
        new_start: usize,
        len: usize,
    },
    Change {
        old_start: usize,
        old_len: usize,
        new_start: usize,
        new_len: usize,
    },
}

/// 按行切分（保留行尾；`split_inclusive('\n')` 保证 `a\nb` 与 `a\nb\n` 分段稳定）。
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').collect()
}

fn join_lines(lines: &[&str]) -> String {
    lines.concat()
}

fn segments(original: &str, modified: &str) -> Vec<Segment> {
    let old = split_lines(original);
    let new = split_lines(modified);
    // 公共前后缀裁剪：让 LCS 只作用在真正变化的区间。
    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];

    let mut result = Vec::new();
    if prefix > 0 {
        result.push(Segment::Equal {
            old_start: 0,
            new_start: 0,
            len: prefix,
        });
    }
    if old_mid.is_empty() && new_mid.is_empty() {
        // 完全相同：只有前缀段。
    } else if old_mid.len().saturating_mul(new_mid.len()) > MAX_LCS_CELLS {
        result.push(Segment::Change {
            old_start: prefix,
            old_len: old_mid.len(),
            new_start: prefix,
            new_len: new_mid.len(),
        });
    } else {
        result.extend(lcs_segments(old_mid, new_mid, prefix));
    }
    if suffix > 0 {
        result.push(Segment::Equal {
            old_start: old.len() - suffix,
            new_start: new.len() - suffix,
            len: suffix,
        });
    }
    result
}

/// 对裁剪后的区间做行级 LCS，合并为 Equal / Change 段（偏移 `base` 为裁剪掉的公共前缀行数）。
fn lcs_segments(old: &[&str], new: &[&str], base: usize) -> Vec<Segment> {
    let rows = old.len() + 1;
    let cols = new.len() + 1;
    let mut table = vec![0usize; rows * cols];
    let at = |i: usize, j: usize| i * cols + j;
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[at(i, j)] = if old[i] == new[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }

    let mut segments = Vec::new();
    let mut pending_change: Option<Segment> = None;
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() || j < new.len() {
        let equal = i < old.len() && j < new.len() && old[i] == new[j];
        if equal {
            if let Some(change) = pending_change.take() {
                segments.push(change);
            }
            let start_old = base + i;
            let start_new = base + j;
            let mut len = 0usize;
            while i < old.len() && j < new.len() && old[i] == new[j] {
                i += 1;
                j += 1;
                len += 1;
            }
            segments.push(Segment::Equal {
                old_start: start_old,
                new_start: start_new,
                len,
            });
            continue;
        }
        // 不在 LCS 上：向前推进一条变化行（优先消耗 old，再消耗 new）。
        let consume_old = if j >= new.len() {
            true
        } else if i >= old.len() {
            false
        } else {
            table[at(i + 1, j)] >= table[at(i, j + 1)]
        };
        let change = pending_change.get_or_insert(Segment::Change {
            old_start: base + i,
            old_len: 0,
            new_start: base + j,
            new_len: 0,
        });
        if let Segment::Change {
            old_len, new_len, ..
        } = change
        {
            if consume_old {
                *old_len += 1;
                i += 1;
            } else {
                *new_len += 1;
                j += 1;
            }
        }
    }
    if let Some(change) = pending_change.take() {
        segments.push(change);
    }
    segments
}

/// 列出可接受/拒绝的 hunk（序号即 [`DiffHunk::index`]）。
pub fn diff_hunks(original: &str, modified: &str) -> Vec<DiffHunk> {
    segments(original, modified)
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Change {
                old_start,
                old_len,
                new_start,
                new_len,
            } => Some(DiffHunk {
                index: 0, // 占位，下面统一编号
                old_start,
                old_lines: old_len,
                new_start,
                new_lines: new_len,
            }),
            Segment::Equal { .. } => None,
        })
        .enumerate()
        .map(|(index, mut hunk)| {
            hunk.index = index;
            hunk
        })
        .collect()
}

/// 应用部分 hunk：`accepted` 中的 hunk 取新内容，其余取原内容。
///
/// `accepted` 里的越界序号忽略（调用方应先按 [`diff_hunks`] 校验）；
/// `accepted` 为空 = 完全回退到 `original`，全选 = 等于 `modified`。
pub fn apply_hunks(original: &str, modified: &str, accepted: &[usize]) -> String {
    let old_lines = split_lines(original);
    let new_lines = split_lines(modified);
    let mut out: Vec<&str> = Vec::new();
    let mut hunk_index = 0usize;
    for segment in segments(original, modified) {
        match segment {
            Segment::Equal {
                old_start,
                new_start,
                len,
            } => {
                // 相等段在两侧都有；任取一侧内容（保持一致）。
                let _ = new_start;
                out.extend_from_slice(&old_lines[old_start..old_start + len]);
            }
            Segment::Change {
                old_start,
                old_len,
                new_start,
                new_len,
            } => {
                if accepted.contains(&hunk_index) {
                    out.extend_from_slice(&new_lines[new_start..new_start + new_len]);
                } else {
                    out.extend_from_slice(&old_lines[old_start..old_start + old_len]);
                }
                hunk_index += 1;
            }
        }
    }
    join_lines(&out)
}

/// 原内容里某个 hunk 覆盖的行（用于列表展示"将被还原的内容"）。
pub fn hunk_original_lines<'a>(original: &'a str, hunk: &DiffHunk) -> Vec<&'a str> {
    let lines = split_lines(original);
    let end = (hunk.old_start + hunk.old_lines).min(lines.len());
    lines.get(hunk.old_start..end).unwrap_or(&[]).to_vec()
}

/// 新内容里某个 hunk 覆盖的行。
pub fn hunk_modified_lines<'a>(modified: &'a str, hunk: &DiffHunk) -> Vec<&'a str> {
    let lines = split_lines(modified);
    let end = (hunk.new_start + hunk.new_lines).min(lines.len());
    lines.get(hunk.new_start..end).unwrap_or(&[]).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_has_no_hunks() {
        assert!(diff_hunks("a\nb\n", "a\nb\n").is_empty());
        assert!(diff_hunks("", "").is_empty());
        assert_eq!(apply_hunks("a\nb\n", "a\nb\n", &[]), "a\nb\n");
    }

    #[test]
    fn separate_changes_produce_separate_hunks() {
        let original = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let modified = "1\n2\nX\n4\n5\n6\nY\n8\n";
        let hunks = diff_hunks(original, modified);
        assert_eq!(hunks.len(), 2, "{hunks:?}");
        assert_eq!(hunks[0].old_start, 2);
        assert_eq!(hunks[0].old_lines, 1);
        assert_eq!(hunks[0].new_lines, 1);
        assert_eq!(hunks[1].old_start, 6);
        assert_eq!(hunks[1].kind(), "modified");

        // 只接受第二个 hunk：第一个位置回到原文。
        let applied = apply_hunks(original, modified, &[1]);
        assert_eq!(applied, "1\n2\n3\n4\n5\n6\nY\n8\n");
        // 只接受第一个：第二个回到原文。
        assert_eq!(
            apply_hunks(original, modified, &[0]),
            "1\n2\nX\n4\n5\n6\n7\n8\n"
        );
        // 全接受 = 新内容；全拒绝 = 原内容。
        assert_eq!(apply_hunks(original, modified, &[0, 1]), modified);
        assert_eq!(apply_hunks(original, modified, &[]), original);
    }

    #[test]
    fn pure_additions_and_deletions_are_single_hunks() {
        let original = "a\nb\n";
        let added = "a\nnew\nb\n";
        let hunks = diff_hunks(original, added);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind(), "added");
        assert_eq!(hunks[0].old_lines, 0);
        assert_eq!(hunks[0].new_lines, 1);
        assert_eq!(apply_hunks(original, added, &[0]), added);
        assert_eq!(apply_hunks(original, added, &[]), original);

        let removed = "a\n";
        let hunks = diff_hunks(original, removed);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind(), "removed");
        assert_eq!(apply_hunks(original, removed, &[0]), removed);
    }

    #[test]
    fn line_endings_and_missing_trailing_newline_are_preserved() {
        let original = "a\r\nb\r\n";
        let modified = "a\r\nB\r\n";
        assert_eq!(apply_hunks(original, modified, &[0]), modified);
        assert_eq!(apply_hunks(original, modified, &[]), original);

        let original = "a\nb";
        let modified = "a\nc";
        assert_eq!(apply_hunks(original, modified, &[0]), "a\nc");
        assert_eq!(apply_hunks(original, modified, &[]), "a\nb");
    }

    #[test]
    fn all_changed_file_is_one_hunk_and_still_round_trips() {
        let original = "one\ntwo\nthree\n";
        let modified = "uno\ndos\ntres\n";
        let hunks = diff_hunks(original, modified);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 0);
        assert_eq!(hunks[0].old_lines, 3);
        assert_eq!(hunks[0].new_lines, 3);
        assert_eq!(apply_hunks(original, modified, &[0]), modified);
        assert_eq!(apply_hunks(original, modified, &[]), original);
    }

    #[test]
    fn huge_lcs_falls_back_to_one_coarse_hunk_without_losing_content() {
        // 超过 MAX_LCS_CELLS：退化为单个粗 hunk（不丢内容、不无界开销）。
        let original: String = (0..1_000).map(|i| format!("line-{i}\n")).collect();
        let modified: String = (0..1_000).map(|i| format!("LINE-{i}\n")).collect();
        let hunks = diff_hunks(&original, &modified);
        assert_eq!(hunks.len(), 1);
        assert_eq!(apply_hunks(&original, &modified, &[0]), modified);
        assert_eq!(apply_hunks(&original, &modified, &[]), original);
    }

    #[test]
    fn hunk_line_views_expose_both_sides() {
        let original = "1\n2\n3\n";
        let modified = "1\nX\nY\n3\n";
        let hunks = diff_hunks(original, modified);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunk_original_lines(original, &hunks[0]), vec!["2\n"]);
        assert_eq!(hunk_modified_lines(modified, &hunks[0]), vec!["X\n", "Y\n"]);
    }

    #[test]
    fn out_of_range_selections_are_ignored_not_panicking() {
        let original = "a\nb\n";
        let modified = "a\nB\n";
        assert_eq!(apply_hunks(original, modified, &[9]), original);
        assert_eq!(apply_hunks(original, modified, &[0, 9]), modified);
    }
}
