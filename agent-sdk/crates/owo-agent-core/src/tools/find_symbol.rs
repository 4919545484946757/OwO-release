//! `find_symbol` 工具：把符号检索内核暴露给模型（G14 的实用切片）。
//!
//! 扫描与匹配逻辑在 `crate::symbol_scan`（CLI `owo-agent symbols` 与服务端 `/symbols`
//! 共用同一内核）；本文件只做参数校验、工作区路径解析与结果包装。

use std::path::Path;

use super::{Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::symbol_scan::{find_symbols, DEFAULT_SYMBOL_RESULTS, MAX_SYMBOL_RESULTS};

pub(super) struct FindSymbolTool;

#[async_trait]
impl Tool for FindSymbolTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "find_symbol".into(),
            description: "按名字定位符号定义（函数/结构体/类/接口/类型/常量），返回 path:line + kind + 签名。\
                          行级启发式、无持久索引；跳噪声目录并有界扫描，截断会如实报告。\
                          kind 可选 function|struct|enum|trait|class|interface|type|const|module|any"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "符号名或名字片段（大小写不敏感）" },
                    "kind": { "type": "string", "description": "按定义种类过滤；缺省 any" },
                    "path": { "type": "string", "description": "限定在工作区某个相对子目录内扫描" },
                    "max_results": { "type": "integer", "description": "返回条数（默认 30，上限 200）" }
                },
                "required": ["query"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if query.is_empty() {
            return Err("find_symbol.query 不能为空".to_string());
        }
        let kind_filter = args.get("kind").and_then(Value::as_str).map(str::to_string);
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_SYMBOL_RESULTS)
            .min(MAX_SYMBOL_RESULTS);

        let root = match args.get("path").and_then(Value::as_str) {
            Some(relative) => {
                let trimmed = relative.trim();
                super::registry_support::resolve_session_path(ctx, trimmed)
                    .map_err(|reason| format!("find_symbol.path 非法：{reason}"))?
            }
            None => ctx.workspace.to_path_buf(),
        };

        let scan = find_symbols(
            &root,
            ctx.workspace,
            &query,
            kind_filter.as_deref(),
            max_results,
        );
        Ok(json!({
            "query": query,
            "kind": kind_filter,
            "root": relative_display(ctx.workspace, &root),
            "results": scan.definitions,
            "scanned_files": scan.stats.scanned_files,
            "skipped_files": scan.stats.skipped_files,
            "truncated": scan.stats.results_truncated || scan.stats.file_limit_reached,
            "file_limit_reached": scan.stats.file_limit_reached,
            "results_truncated": scan.stats.results_truncated,
        }))
    }
}

fn relative_display(workspace: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(workspace).unwrap_or(path);
    let text = relative.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        ".".to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn workspace(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("owo-symbol-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn with_context<T>(root: &Path, f: impl FnOnce(&mut ToolContext<'_>) -> T) -> T {
        let policy = crate::Policy::new(root);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = crate::SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = crate::session::Session::new(root, "mock", None);
        let mut context = ToolContext {
            workspace: root,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };
        f(&mut context)
    }

    fn run_tool(root: &Path, args: Value) -> Value {
        with_context(root, |context| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(FindSymbolTool.run(context, args))
                .unwrap()
        })
    }

    fn run_tool_error(root: &Path, args: Value) -> String {
        with_context(root, |context| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(FindSymbolTool.run(context, args))
                .unwrap_err()
        })
    }

    #[test]
    fn exposes_symbol_scan_result_shape() {
        let root = workspace("shape");
        write(&root.join("src/lib.rs"), "pub fn parse_config() {}\n");
        let result = run_tool(&root, json!({ "query": "parse_config" }));
        assert_eq!(result["query"], json!("parse_config"));
        assert_eq!(result["kind"], json!(null));
        assert_eq!(result["results"][0]["path"], json!("src/lib.rs"));
        assert_eq!(result["results"][0]["kind"], json!("function"));
        assert_eq!(result["results"][0]["line"], json!(1));
        assert_eq!(result["truncated"], json!(false));
        assert!(result["scanned_files"].as_u64().unwrap() >= 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn subdir_kind_and_max_results_are_forwarded_to_the_core_scan() {
        let root = workspace("args");
        write(
            &root.join("a/one.rs"),
            "pub fn alpha() {}\npub struct Alpha;\n",
        );
        write(&root.join("b/two.rs"), "pub fn alpha_two() {}\n");

        let subdir = run_tool(&root, json!({ "query": "alpha", "path": "b" }));
        assert_eq!(subdir["root"], json!("b"));
        assert_eq!(subdir["results"].as_array().unwrap().len(), 1);
        assert_eq!(subdir["results"][0]["path"], json!("b/two.rs"));

        let kind = run_tool(&root, json!({ "query": "alpha", "kind": "struct" }));
        assert_eq!(kind["kind"], json!("struct"));
        assert_eq!(kind["results"].as_array().unwrap().len(), 1);

        let limited = run_tool(&root, json!({ "query": "alpha", "max_results": 1 }));
        assert_eq!(limited["results"].as_array().unwrap().len(), 1);
        assert_eq!(limited["results_truncated"], json!(true));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_query_and_escaping_path_are_rejected() {
        let root = workspace("guard");
        assert!(run_tool_error(&root, json!({})).contains("不能为空"));
        let escaped = run_tool_error(&root, json!({ "query": "x", "path": "../outside" }));
        assert!(escaped.contains("非法"), "{escaped}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn noise_directories_are_skipped_by_the_tool_path_too() {
        let root = workspace("noise");
        write(
            &root.join("target/generated.rs"),
            "pub fn hidden_symbol() {}\n",
        );
        let result = run_tool(&root, json!({ "query": "hidden" }));
        assert_eq!(result["results"].as_array().unwrap().len(), 0);
        assert_eq!(result["scanned_files"], json!(0));
        let _ = std::fs::remove_dir_all(&root);
    }
}
