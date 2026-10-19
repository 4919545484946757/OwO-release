//! `owo-agent symbols <名字>`：符号级检索（G14 实用切片；离线，不需要 Daemon）。
//!
//! 直接调用 `owo_agent_core::find_symbols` 纯函数：不建会话、不开数据库、不构造 Agent，
//! 因此**只读**且无副作用（与 `review` 的 git 路径同一定位）。模型侧对应 `find_symbol` 工具。

use std::path::PathBuf;

use clap::Args;
use owo_agent_core::{find_symbols, DEFAULT_SYMBOL_RESULTS, MAX_SYMBOL_RESULTS};

#[derive(Debug, Args)]
pub struct SymbolsArgs {
    /// 符号名或名字片段（大小写不敏感）
    pub query: String,
    /// 工作区（默认当前目录）
    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,
    /// 只在工作区的这个相对子目录里扫描
    #[arg(long)]
    pub path: Option<String>,
    /// 只看某种定义（function|struct|enum|trait|class|interface|type|const|module）
    #[arg(long)]
    pub kind: Option<String>,
    /// 返回条数（默认 30，上限 200）
    #[arg(long)]
    pub max_results: Option<usize>,
    /// 机器可读输出（JSON）
    #[arg(long)]
    pub json: bool,
}

pub(crate) fn run_symbols_cmd(args: SymbolsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = args
        .workspace
        .canonicalize()
        .map_err(|error| format!("工作区不可用：{}（{error}）", args.workspace.display()))?;
    let root = match args.path.as_deref() {
        Some(relative) => {
            let trimmed = relative.trim().trim_start_matches("./");
            let candidate = std::path::Path::new(trimmed);
            if candidate.is_absolute()
                || candidate.components().any(|component| {
                    !matches!(
                        component,
                        std::path::Component::Normal(_) | std::path::Component::CurDir
                    )
                })
            {
                return Err(format!("--path 只接受工作区内相对路径：{relative}").into());
            }
            workspace.join(candidate)
        }
        None => workspace.clone(),
    };
    if !root.is_dir() {
        return Err(format!("扫描目录不存在：{}", root.display()).into());
    }
    let max_results = args
        .max_results
        .unwrap_or(DEFAULT_SYMBOL_RESULTS)
        .clamp(1, MAX_SYMBOL_RESULTS);
    let scan = find_symbols(
        &root,
        &workspace,
        &args.query,
        args.kind.as_deref(),
        max_results,
    );

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "query": args.query,
                "kind": args.kind,
                "root": root.strip_prefix(&workspace).map(|relative| relative.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| ".".to_string()),
                "results": scan.definitions,
                "scanned_files": scan.stats.scanned_files,
                "skipped_files": scan.stats.skipped_files,
                "truncated": scan.stats.results_truncated || scan.stats.file_limit_reached,
                "file_limit_reached": scan.stats.file_limit_reached,
                "results_truncated": scan.stats.results_truncated,
            }))?
        );
        return Ok(());
    }

    // 人类输出走共享渲染器（Daemon REPL `/symbols` 用同一份，不各写一套格式）。
    println!(
        "{}",
        crate::support::render_symbol_report(&serde_json::json!({
            "query": args.query,
            "results": scan.definitions,
            "scanned_files": scan.stats.scanned_files,
            "file_limit_reached": scan.stats.file_limit_reached,
            "results_truncated": scan.stats.results_truncated,
        }))
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_relative_display_and_scan_are_wired() {
        let root = std::env::temp_dir().join(format!("owo-symbols-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn parse_config() {}\npub struct Parser;\n",
        )
        .unwrap();
        let scan = find_symbols(&root, &root, "parse_config", None, DEFAULT_SYMBOL_RESULTS);
        assert_eq!(scan.definitions.len(), 1, "片段查询应只命中 parse_config");
        assert_eq!(scan.definitions[0].path, "src/lib.rs");
        assert_eq!(scan.definitions[0].kind, "function");
        assert_eq!(scan.definitions[0].line, 1);

        // 片段查询是子串匹配：`parse` 会同时命中 `parse_config` 与 `Parser`（大小写不敏感）。
        let fragment = find_symbols(&root, &root, "parse", None, DEFAULT_SYMBOL_RESULTS);
        let names: Vec<&str> = fragment
            .definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert_eq!(names, vec!["parse_config", "Parser"], "{names:?}");

        let only_structs = find_symbols(
            &root,
            &root,
            "parser",
            Some("struct"),
            DEFAULT_SYMBOL_RESULTS,
        );
        assert_eq!(only_structs.definitions.len(), 1);
        assert_eq!(only_structs.definitions[0].name, "Parser");
        let _ = std::fs::remove_dir_all(&root);
    }
}
