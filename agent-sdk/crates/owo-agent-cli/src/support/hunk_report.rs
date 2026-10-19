//! 逐 hunk 审阅结果渲染（CLI `owo-agent review --session` 与 Daemon REPL `/hunks` 共用）。
//!
//! 输入是 `/session/{id}/hunks` 与 `/hunks/apply` 的同形 JSON；输出整段文本，
//! 纯函数便于单测——两个入口不会各写一套显示。

use serde_json::Value;

/// 渲染会话 hunk 视图（列表）。
pub(crate) fn render_hunks_view(session_id: &str, value: &Value) -> String {
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let conflicts: Vec<String> = value
        .get("conflicts")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        return format!("（会话 {session_id} 相对写前快照没有改动）");
    }

    let mut lines = vec![format!("会话 {session_id} 的改动（写前快照 → 当前内容）：")];
    for file in &files {
        let path = file.get("path").and_then(Value::as_str).unwrap_or_default();
        let conflict = conflicts.iter().any(|item| item == path);
        let hunks = file
            .get("hunks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if hunks.is_empty() {
            let reason = file
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("不可逐 hunk 审阅");
            lines.push(format!("  {path}（{reason}）"));
            continue;
        }
        lines.push(format!(
            "  {path} {} 个 hunk{}",
            hunks.len(),
            if conflict {
                " · ⚠ 文件已被改动，应用会被拒绝（409）"
            } else {
                ""
            }
        ));
        for hunk in hunks {
            let index = hunk.get("index").and_then(Value::as_u64).unwrap_or(0);
            let kind = hunk
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("modified");
            let old_start = hunk.get("old_start").and_then(Value::as_u64).unwrap_or(0);
            let new_start = hunk.get("new_start").and_then(Value::as_u64).unwrap_or(0);
            let old_lines = hunk.get("old_lines").and_then(Value::as_u64).unwrap_or(0);
            let new_lines = hunk.get("new_lines").and_then(Value::as_u64).unwrap_or(0);
            lines.push(format!(
                "    #{} {} @{} → {}（-{old_lines} +{new_lines}）",
                index + 1,
                kind,
                old_start + 1,
                new_start + 1
            ));
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "应用示例：/hunks --path <文件> --accept 1（序号从 1 起）；CLI 等价 `owo-agent review --session {session_id} --path <文件> --accept 1`"
    ));
    lines.join("\n")
}

/// 渲染 `/hunks/apply` 响应。
pub(crate) fn render_hunks_apply(value: &Value) -> String {
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let accepted = indices(value, "accepted");
    let rejected = indices(value, "rejected");
    let remaining = value.get("remaining").and_then(Value::as_u64).unwrap_or(0);
    let changed = value
        .get("changed")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    format!(
        "已应用 {path}：接受 [{}] / 拒绝 [{}]；剩余改动 {remaining} 处{}",
        numbers(&accepted),
        numbers(&rejected),
        if changed {
            ""
        } else {
            "（内容已一致，未写盘）"
        }
    )
}

/// 解析 1 起的 hunk 序号列表（`"1,3"` / `"1 3"`）为 0 起集合（去重 + 排序）。
///
/// `all` 需要调用方先取到 hunk 总数再展开；这里明确拒绝，避免"静默当空列表"。
pub(crate) fn parse_hunk_indices(spec: &str) -> Result<Vec<usize>, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("序号为空：请给 1 起的序号（如 --accept 1,3）或 all".to_string());
    }
    if trimmed.eq_ignore_ascii_case("all") {
        return Err("all 需要 hunk 总数：调用方展开（本函数只解析具体序号）".to_string());
    }
    let mut indices = Vec::new();
    for token in trimmed
        .split([',', ' ', ';'])
        .filter(|token| !token.is_empty())
    {
        let number: usize = token
            .parse()
            .map_err(|_| format!("无法解析 hunk 序号：{token}（应为 1 起的整数或 all）"))?;
        if number == 0 {
            return Err("hunk 序号从 1 起（0 非法）".to_string());
        }
        let index = number - 1;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    indices.sort_unstable();
    if indices.is_empty() {
        return Err("没有有效序号".to_string());
    }
    Ok(indices)
}

/// 从 hunk 视图里取某文件的 hunk 数（`all` 展开用）；文件不可审阅或不存在时为 `None`。
pub(crate) fn hunk_count_for_path(view: &Value, path: &str) -> Option<usize> {
    view.get("files")
        .and_then(Value::as_array)?
        .iter()
        .find(|file| file.get("path").and_then(Value::as_str) == Some(path))
        .map(|file| {
            file.get("hunks")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        })
}

fn indices(value: &Value, key: &str) -> Vec<usize> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .map(|index| index as usize)
                .collect()
        })
        .unwrap_or_default()
}

fn numbers(indices: &[usize]) -> String {
    indices
        .iter()
        .map(|index| (index + 1).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_hunk_list_with_conflicts_and_reasons() {
        let value = json!({
            "files": [
                { "path": "a.txt", "kind": "modified", "hunks": [
                    { "index": 0, "old_start": 2, "old_lines": 1, "new_start": 2, "new_lines": 1 },
                    { "index": 1, "old_start": 6, "old_lines": 1, "new_start": 6, "new_lines": 2 }
                ]},
                { "path": "new.txt", "kind": "unavailable", "hunks": [], "reason": "新增文件没有写前快照" }
            ],
            "conflicts": ["a.txt"]
        });
        let text = render_hunks_view("sess-1", &value);
        assert!(text.contains("a.txt 2 个 hunk · ⚠ 文件已被改动"), "{text}");
        assert!(text.contains("#1 modified @3 → 3（-1 +1）"), "{text}");
        assert!(text.contains("#2 modified @7 → 7（-1 +2）"), "{text}");
        assert!(text.contains("new.txt（新增文件没有写前快照）"), "{text}");
        assert!(text.contains("应用示例"), "{text}");
    }

    #[test]
    fn renders_empty_view_and_apply_result() {
        let empty = json!({ "files": [], "conflicts": [] });
        assert_eq!(
            render_hunks_view("sess-9", &empty),
            "（会话 sess-9 相对写前快照没有改动）"
        );

        let applied = json!({
            "path": "a.txt", "accepted": [1], "rejected": [0], "remaining": 1, "changed": true
        });
        assert_eq!(
            render_hunks_apply(&applied),
            "已应用 a.txt：接受 [2] / 拒绝 [1]；剩余改动 1 处"
        );

        let noop = json!({
            "path": "a.txt", "accepted": [0, 1], "rejected": [], "remaining": 2, "changed": false
        });
        assert!(render_hunks_apply(&noop).contains("内容已一致，未写盘"));
    }
}
