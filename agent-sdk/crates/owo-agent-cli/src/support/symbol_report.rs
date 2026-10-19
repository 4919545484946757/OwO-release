//! 符号检索结果渲染（CLI `owo-agent symbols` 与 Daemon REPL `/symbols` 共用）。
//!
//! 输入是 `/symbols` 路由/内核的同形 JSON；输出整段文本（含标题与逐条行），
//! 纯函数便于单测——两处界面不会各写一套格式而漂移。

use serde_json::Value;

/// 渲染 `/symbols` 响应（人类可读）。`truncated` 相关字段会如实展示。
pub(crate) fn render_symbol_report(value: &Value) -> String {
    let query = value
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let scanned = value
        .get("scanned_files")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let file_limit = value
        .get("file_limit_reached")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let results_truncated = value
        .get("results_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if results.is_empty() {
        return format!(
            "（没有匹配“{query}”的定义；已扫描 {scanned} 个源文件{}）",
            if file_limit {
                "，且已达文件数上限（结果不完整）"
            } else {
                ""
            }
        );
    }

    let mut lines = vec![format!(
        "符号“{query}”（{} 条，扫描 {scanned} 个源文件{}）：",
        results.len(),
        if results_truncated {
            "，结果被截断，用 --max-results 调整"
        } else {
            ""
        }
    )];
    for row in &results {
        let path = row.get("path").and_then(Value::as_str).unwrap_or_default();
        let line = row.get("line").and_then(Value::as_u64).unwrap_or(0);
        let kind = row.get("kind").and_then(Value::as_str).unwrap_or_default();
        let signature = row
            .get("signature")
            .and_then(Value::as_str)
            .unwrap_or_default();
        lines.push(format!("  {path}:{line}  [{kind}]  {}", signature.trim()));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_entries_with_path_line_kind_and_signature() {
        let value = json!({
            "query": "parse",
            "scanned_files": 12,
            "results": [
                { "path": "src/lib.rs", "line": 3, "kind": "function", "name": "parse", "signature": "pub fn parse() {}" },
                { "path": "web/a.py", "line": 7, "kind": "class", "name": "Parser", "signature": "class Parser:" }
            ],
            "truncated": false,
            "file_limit_reached": false,
            "results_truncated": false
        });
        let text = render_symbol_report(&value);
        assert!(
            text.contains("符号“parse”（2 条，扫描 12 个源文件）"),
            "{text}"
        );
        assert!(
            text.contains("src/lib.rs:3  [function]  pub fn parse() {}"),
            "{text}"
        );
        assert!(
            text.contains("web/a.py:7  [class]  class Parser:"),
            "{text}"
        );
    }

    #[test]
    fn reports_empty_and_truncated_states_honestly() {
        let empty = json!({
            "query": "nope", "scanned_files": 5, "results": [],
            "file_limit_reached": false, "results_truncated": false
        });
        assert_eq!(
            render_symbol_report(&empty),
            "（没有匹配“nope”的定义；已扫描 5 个源文件）"
        );

        let file_limited = json!({
            "query": "nope", "scanned_files": 2000, "results": [],
            "file_limit_reached": true, "results_truncated": false
        });
        assert!(render_symbol_report(&file_limited).contains("已达文件数上限"));

        let truncated = json!({
            "query": "many", "scanned_files": 40,
            "results": [
                { "path": "a.rs", "line": 1, "kind": "function", "name": "many", "signature": "pub fn many() {}" }
            ],
            "file_limit_reached": false, "results_truncated": true
        });
        assert!(render_symbol_report(&truncated).contains("结果被截断"));
    }
}
