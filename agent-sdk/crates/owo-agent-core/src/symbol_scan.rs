//! 符号级检索内核（`find_symbol` 工具、CLI `owo-agent symbols`、服务端 `/symbols` 共用）。
//!
//! 与 `grep`/`search_files` 的区别：按各语言**定义形态**匹配（`fn`/`def`/`class`/`struct`/
//! `interface`/`type`…），返回 `path:line + kind + 签名`。
//!
//! 诚实边界（不做索引、不做解析器）：
//! - 纯行级启发式，只认行首/缩进后的单行定义形态，不处理跨行签名与宏展开；
//! - 有界扫描：跳噪声目录、限文件数/单文件大小/结果数，并把截断如实报出；
//! - 纯读：不写盘、不建缓存，调用方自行决定权限档。

use std::path::{Path, PathBuf};

/// 结果上限（`max_results` 的硬顶）。
pub const MAX_SYMBOL_RESULTS: usize = 200;
/// 默认返回条数。
pub const DEFAULT_SYMBOL_RESULTS: usize = 30;
/// 单文件读取上限（超过则跳过并计入 skipped）。
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// 单次扫描的文件数上限。
const MAX_SCANNED_FILES: usize = 2_000;
/// 目录递归深度上限。
const MAX_DEPTH: usize = 8;
/// 签名展示长度上限（字符）。
const MAX_SIGNATURE_CHARS: usize = 200;

/// 参与扫描的源文件扩展名 → 语言。
const SOURCE_EXTENSIONS: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("py", "python"),
    ("ts", "typescript"),
    ("tsx", "typescript"),
    ("js", "javascript"),
    ("jsx", "javascript"),
    ("mjs", "javascript"),
    ("go", "go"),
    ("java", "java"),
    ("kt", "kotlin"),
    ("cs", "csharp"),
    ("c", "c"),
    ("h", "c"),
    ("cc", "cpp"),
    ("cpp", "cpp"),
    ("hpp", "cpp"),
    ("rb", "ruby"),
    ("php", "php"),
    ("swift", "swift"),
];

/// 一个符号定义。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SymbolDefinition {
    /// 相对扫描基座的路径（`/` 归一）。
    pub path: String,
    /// 1 起的行号。
    pub line: usize,
    /// `function` | `struct` | `enum` | `trait` | `class` | `interface` | `type` | `const` | `module`。
    pub kind: String,
    /// 符号名（已剥离参数表/泛型等尾部）。
    pub name: String,
    /// 定义行原文（截断到 200 字符）。
    pub signature: String,
}

/// 扫描统计（如实报告截断，不用"可能不完整"糊过去）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SymbolScanStats {
    pub scanned_files: usize,
    pub skipped_files: usize,
    /// 因命中文件数上限而停止扫描。
    pub file_limit_reached: bool,
    /// 结果被 `max_results` 截断。
    pub results_truncated: bool,
}

/// 符号检索结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SymbolScan {
    pub definitions: Vec<SymbolDefinition>,
    pub stats: SymbolScanStats,
}

/// 在 `root` 下扫描符号定义；`display_base` 决定返回路径的展示前缀（通常是工作区根）。
///
/// `kind_filter` 为 `None`/`"any"` 表示不过滤；`max_results` 已被调用方限幅到
/// [`MAX_SYMBOL_RESULTS`] 以内时不再二次裁剪。
pub fn find_symbols(
    root: &Path,
    display_base: &Path,
    query: &str,
    kind_filter: Option<&str>,
    max_results: usize,
) -> SymbolScan {
    let needle = query.trim().to_lowercase();
    let kind_filter = kind_filter
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty() && value != "any");
    let max_results = max_results.clamp(1, MAX_SYMBOL_RESULTS);
    let mut scan = SymbolScan {
        definitions: Vec::new(),
        stats: SymbolScanStats::default(),
    };
    if needle.is_empty() {
        return scan;
    }

    let mut queue: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((directory, depth)) = queue.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        let mut children: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            if is_dir {
                if is_noise_dir_name(&name) {
                    continue;
                }
                children.push(path);
                continue;
            }
            let Some(language) = language_of(&path) else {
                continue;
            };
            if scan.stats.scanned_files >= MAX_SCANNED_FILES {
                scan.stats.file_limit_reached = true;
                break;
            }
            if std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) > MAX_FILE_BYTES {
                scan.stats.skipped_files += 1;
                continue;
            }
            scan.stats.scanned_files += 1;
            let Ok(content) = std::fs::read_to_string(&path) else {
                scan.stats.skipped_files += 1;
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                let Some((kind, name)) = parse_definition(line, language) else {
                    continue;
                };
                if kind_filter.as_deref().is_some_and(|filter| filter != kind) {
                    continue;
                }
                if !name.to_lowercase().contains(&needle) {
                    continue;
                }
                scan.definitions.push(SymbolDefinition {
                    path: relative_display(display_base, &path),
                    line: index + 1,
                    kind: kind.to_string(),
                    name,
                    signature: truncate_chars(line.trim(), MAX_SIGNATURE_CHARS),
                });
            }
        }
        if scan.stats.file_limit_reached {
            break;
        }
        for child in children {
            queue.push((child, depth + 1));
        }
    }

    // 精确命中优先，其次按路径/行号，结果稳定可复现。
    scan.definitions.sort_by(|left, right| {
        let left_exact = !left.name.eq_ignore_ascii_case(query.trim());
        let right_exact = !right.name.eq_ignore_ascii_case(query.trim());
        left_exact
            .cmp(&right_exact)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line.cmp(&right.line))
    });
    if scan.definitions.len() > max_results {
        scan.definitions.truncate(max_results);
        scan.stats.results_truncated = true;
    }
    scan
}

/// 噪声目录（与 `list_dir`/`search_files` 同口径）。
pub fn is_noise_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".svn" | ".hg" | ".venv"
    )
}

fn language_of(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    SOURCE_EXTENSIONS
        .iter()
        .find(|(candidate, _)| *candidate == extension)
        .map(|(_, language)| *language)
}

/// 行级定义识别（启发式）：返回 `(kind, name)`。
fn parse_definition(line: &str, language: &str) -> Option<(&'static str, String)> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    // 跳过注释行（各语言注释前缀）。
    for prefix in ["//", "#", "*", "/*", "--"] {
        if trimmed.starts_with(prefix) {
            return None;
        }
    }
    let rest = strip_modifiers(trimmed);
    match language {
        "rust" => {
            for (keyword, kind) in [
                ("pub(crate) fn ", "function"),
                ("pub(super) fn ", "function"),
                ("pub async fn ", "function"),
                ("async fn ", "function"),
                ("pub fn ", "function"),
                ("fn ", "function"),
                ("pub struct ", "struct"),
                ("struct ", "struct"),
                ("pub enum ", "enum"),
                ("enum ", "enum"),
                ("pub trait ", "trait"),
                ("trait ", "trait"),
                ("pub type ", "type"),
                ("type ", "type"),
                ("pub const ", "const"),
                ("const ", "const"),
                ("pub static ", "const"),
                ("static ", "const"),
                ("macro_rules! ", "module"),
                ("mod ", "module"),
            ] {
                if let Some(name) = rest.strip_prefix(keyword) {
                    return identifier(name).map(|name| (kind, name));
                }
            }
            None
        }
        "python" => {
            if let Some(name) = rest.strip_prefix("async def ") {
                return identifier(name).map(|name| ("function", name));
            }
            if let Some(name) = rest.strip_prefix("def ") {
                return identifier(name).map(|name| ("function", name));
            }
            if let Some(name) = rest.strip_prefix("class ") {
                return identifier(name).map(|name| ("class", name));
            }
            None
        }
        "typescript" | "javascript" => {
            for (keyword, kind) in [
                ("export default class ", "class"),
                ("export class ", "class"),
                ("export interface ", "interface"),
                ("interface ", "interface"),
                ("export type ", "type"),
                ("type ", "type"),
                ("export enum ", "enum"),
                ("enum ", "enum"),
                ("export async function ", "function"),
                ("export function ", "function"),
                ("async function ", "function"),
                ("function ", "function"),
                ("export const ", "const"),
                ("const ", "const"),
                ("class ", "class"),
            ] {
                if let Some(name) = rest.strip_prefix(keyword) {
                    return identifier(name).map(|name| (kind, name));
                }
            }
            None
        }
        "go" => {
            if let Some(name) = rest.strip_prefix("func ") {
                // 方法形态 `func (r R) Name(...)`：取括号后的名字。
                let after_receiver = match name.strip_prefix('(') {
                    Some(rest) => match rest.split_once(')') {
                        Some((_, tail)) => tail.trim_start(),
                        None => return None,
                    },
                    None => name,
                };
                return identifier(after_receiver).map(|name| ("function", name));
            }
            for (keyword, kind) in [("type ", "type"), ("var ", "const"), ("const ", "const")] {
                if let Some(name) = rest.strip_prefix(keyword) {
                    return identifier(name).map(|name| (kind, name));
                }
            }
            None
        }
        "java" | "kotlin" | "csharp" | "swift" => {
            for (keyword, kind) in [
                ("class ", "class"),
                ("interface ", "interface"),
                ("struct ", "struct"),
                ("enum ", "enum"),
                ("protocol ", "interface"),
                ("object ", "class"),
            ] {
                if let Some(name) = rest.strip_prefix(keyword) {
                    return identifier(name).map(|name| (kind, name));
                }
            }
            for keyword in ["fun ", "func ", "void "] {
                if let Some(name) = rest.strip_prefix(keyword) {
                    return identifier(name).map(|name| ("function", name));
                }
            }
            None
        }
        "c" | "cpp" => {
            for (keyword, kind) in [
                ("class ", "class"),
                ("struct ", "struct"),
                ("enum ", "enum"),
                ("namespace ", "module"),
                ("using ", "type"),
            ] {
                if let Some(name) = rest.strip_prefix(keyword) {
                    return identifier(name).map(|name| (kind, name));
                }
            }
            None
        }
        "ruby" => rest
            .strip_prefix("def ")
            .or_else(|| rest.strip_prefix("class "))
            .and_then(identifier)
            .map(|name| ("function", name)),
        "php" => {
            if let Some(name) = rest.strip_prefix("function ") {
                return identifier(name).map(|name| ("function", name));
            }
            if let Some(name) = rest.strip_prefix("class ") {
                return identifier(name).map(|name| ("class", name));
            }
            None
        }
        _ => None,
    }
}

/// 去掉常见修饰关键字（`public`/`static`/`final`/`export`…），便于统一匹配定义形态。
fn strip_modifiers(mut line: &str) -> &str {
    const MODIFIERS: &[&str] = &[
        "public ",
        "private ",
        "protected ",
        "internal ",
        "static ",
        "final ",
        "sealed ",
        "abstract ",
        "open ",
        "inline ",
        "export ",
    ];
    loop {
        let mut changed = false;
        for modifier in MODIFIERS {
            if let Some(rest) = line.strip_prefix(modifier) {
                line = rest.trim_start();
                changed = true;
            }
        }
        if !changed {
            return line;
        }
    }
}

/// 取标识符（到参数表/泛型/赋值/冒号/分号/花括号为止）。
fn identifier(input: &str) -> Option<String> {
    let name: String = input
        .chars()
        .take_while(|character| {
            character.is_alphanumeric() || matches!(character, '_' | '$' | '.' | ':')
        })
        .collect();
    let cleaned = name
        .trim_end_matches([':', '.'])
        .rsplit([':', '.'])
        .next()
        .unwrap_or_default()
        .trim();
    if cleaned.is_empty() || cleaned.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(cleaned.to_string())
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut kept: String = value.chars().take(max).collect();
    kept.push('…');
    kept
}

fn relative_display(base: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(base).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn workspace(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("owo-scan-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn scans_definitions_across_languages_and_ranks_exact_matches_first() {
        let root = workspace("multi");
        write(
            &root.join("src/lib.rs"),
            "pub fn parse_config() {}\npub fn parse_config_extended() {}\n",
        );
        write(
            &root.join("web/app.py"),
            "class ParseConfig:\n    def parse_config(self):\n",
        );
        let scan = find_symbols(&root, &root, "parse_config", None, 30);
        let labelled: Vec<String> = scan
            .definitions
            .iter()
            .map(|definition| format!("{}:{}", definition.path, definition.name))
            .collect();
        assert_eq!(
            labelled,
            vec![
                "src/lib.rs:parse_config".to_string(),
                "web/app.py:parse_config".to_string(),
                "src/lib.rs:parse_config_extended".to_string(),
            ],
            "精确命中按路径排序在前，片段命中在后"
        );
        assert!(!scan.stats.results_truncated);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kind_filter_and_empty_query_behave_explicitly() {
        let root = workspace("kind");
        write(&root.join("a.rs"), "pub struct Alpha;\npub fn alpha() {}\n");
        let structs = find_symbols(&root, &root, "alpha", Some("struct"), 30);
        assert_eq!(structs.definitions.len(), 1);
        assert_eq!(structs.definitions[0].kind, "struct");
        // 空查询不返回任何结果（也不误报截断）。
        let empty = find_symbols(&root, &root, "   ", None, 30);
        assert!(empty.definitions.is_empty());
        assert!(!empty.stats.results_truncated);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn noise_directories_are_not_scanned_and_truncation_is_reported() {
        let root = workspace("noise");
        write(
            &root.join("target/generated.rs"),
            "pub fn hidden_symbol() {}\n",
        );
        let skipped = find_symbols(&root, &root, "hidden", None, 30);
        assert!(skipped.definitions.is_empty());
        assert_eq!(skipped.stats.scanned_files, 0, "噪声目录不得被扫描");

        for index in 0..40 {
            write(
                &root.join(format!("src/mod_{index}.rs")),
                "pub fn repeated_symbol() {}\n",
            );
        }
        let limited = find_symbols(&root, &root, "repeated", None, 5);
        assert_eq!(limited.definitions.len(), 5);
        assert!(limited.stats.results_truncated);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn max_results_is_clamped_and_signatures_are_bounded() {
        let root = workspace("clamp");
        write(
            &root.join("a.rs"),
            "pub fn render_widget_tree(very_long_argument_list: impl IntoIterator<Item = String>) -> Result<(), String> {}\n",
        );
        let clamped = find_symbols(&root, &root, "render_widget", None, usize::MAX);
        assert_eq!(clamped.definitions.len(), 1);
        assert!(clamped.definitions[0].signature.chars().count() <= MAX_SIGNATURE_CHARS + 1);
        // 展示前缀来自 display_base（工作区根），而不是扫描根。
        let nested = find_symbols(&root.join(""), &root, "render_widget", None, 5);
        assert_eq!(nested.definitions[0].path, "a.rs");
        let _ = std::fs::remove_dir_all(&root);
    }
}
