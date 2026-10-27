use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
pub(crate) const STATUS_LINE_CAP: usize = 500;

/// diff 补丁限定路径数上限：超过则**不回退为全仓 `git diff HEAD`**（十期·四路硬性
/// 规则：禁止超过 100 个文件就退回全仓 diff），而是交给调用方落退化差异摘要——
/// 该摘要逐文件、以执行前 CAS 内容为基线、行级截断，天然有界且不混入用户改动。
/// 本常量只用于「限定路径的 git 补丁」这一路径数的上限判定。
pub(crate) const DIFF_PATH_ARG_CAP: usize = 100;

/// 工作区 git 快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSnapshot {
    /// 是否取得有效 git 快照（false = 非 git 目录 / 无 git，变更不可检测）。
    pub git: bool,
    /// `git status --porcelain` 原始行（截断 [`STATUS_LINE_CAP`] 行防膨胀）。
    pub status: Vec<String>,
    /// `git diff --stat` 摘要文本（untracked 不入 diff，以 status 行为准）。
    pub diff_stat: String,
    /// 快照时刻（Unix 毫秒）。
    pub at: u64,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 运行一次 git 子命令（current_dir = root）；失败（非 git 目录 / 无 git）→ None。
pub(crate) async fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// porcelain 状态行 → 相对路径（`XY PATH`；重命名/拷贝取 `old -> new` 的 new；
/// 带引号路径去引号）。解析失败 → None（忽略该行）。
pub fn parse_porcelain_path(line: &str) -> Option<String> {
    let rest = line.get(3..)?; // "XY " 前缀：X、Y 各 1 字符 + 1 空格
    let rest = rest.trim();
    let target = match rest.find(" -> ") {
        Some(index) => rest[index + 4..].trim(),
        None => rest,
    };
    let unquoted = target.trim_matches('"');
    if unquoted.is_empty() {
        None
    } else {
        Some(unquoted.to_string())
    }
}

/// porcelain 重命名/拷贝行 → `(old, new)` 路径对（非重命名行 → None）。
///
/// 九期（一路）：重命名的 old 侧在 `parse_porcelain_path` 里被丢弃，恢复时无法
/// 还原被移走的源文件——合并检测需要两侧。
pub fn parse_porcelain_rename(line: &str) -> Option<(String, String)> {
    let rest = line.get(3..)?.trim();
    let index = rest.find(" -> ")?;
    let old = rest[..index].trim().trim_matches('"');
    let new = rest[index + 4..].trim().trim_matches('"');
    if old.is_empty() || new.is_empty() {
        None
    } else {
        Some((old.to_string(), new.to_string()))
    }
}

/// 把 `git status` 行中的仓库根相对路径改写成 workspace 根相对路径。
///
/// 保留 porcelain 的 `XY ` 前缀；重命名行同时改写 old/new 两侧。`prefix` 为空
/// （workspace 就是仓库根）时原样返回。
fn normalize_status_line(line: &str, prefix: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return line.trim_end().to_string();
    }
    let code = line.get(..3).unwrap_or("");
    let relative = |path: &str| {
        let normalized = path.replace('\\', "/");
        normalized
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('/'))
            .map(str::to_string)
            .unwrap_or_else(|| path.to_string())
    };
    if let Some((old, new)) = parse_porcelain_rename(line) {
        return format!("{code}{} -> {}", relative(&old), relative(&new));
    }
    if let Some(path) = parse_porcelain_path(line) {
        return format!("{code}{}", relative(&path));
    }
    line.trim_end().to_string()
}

impl GitSnapshot {
    /// 采集快照（git 不可用 → `git:false` 空快照，不视为错误）。
    ///
    /// `-uall`：未跟踪目录展开为具体文件（否则 porcelain 只给 `?? dir/`，
    /// 变更文件列表与白名单校验都拿不到文件级路径）。
    ///
    /// `git status` 即使在 workspace 子目录执行也返回**仓库根相对路径**；团队以
    /// 子目录为 workspace 时，白名单/写租约却是子目录绝对路径，直接比对会把所有
    /// 合法写入判成越界。这里用 `git rev-parse --show-prefix` 把状态路径改写成
    /// 当前 workspace 根相对路径，并限制 pathspec 为当前目录。
    pub async fn snapshot(root: &Path) -> Self {
        let at = now_ms();
        let prefix = git_output(root, &["rev-parse", "--show-prefix"])
            .await
            .unwrap_or_default()
            .trim()
            .to_string();
        match git_output(root, &["status", "--porcelain", "-uall", "--", "."]).await {
            Some(text) => {
                let status: Vec<String> = text
                    .lines()
                    .map(str::trim_end)
                    .filter(|line| !line.is_empty())
                    .take(STATUS_LINE_CAP)
                    .map(|line| normalize_status_line(line, &prefix))
                    .collect();
                let diff_stat = git_output(root, &["diff", "--stat", "--relative", "--", "."])
                    .await
                    .unwrap_or_default();
                Self {
                    git: true,
                    status,
                    diff_stat,
                    at,
                }
            }
            None => Self {
                git: false,
                status: Vec::new(),
                diff_stat: String::new(),
                at,
            },
        }
    }

    /// 本次执行窗口新增的变更文件（porcelain 状态行差集 → 相对路径，去重保序）。
    ///
    /// 注意：这是**纯路径集合差**——执行前已脏的文件不在结果里（九期一路的合并
    /// 检测见 [`merge_changed_files`]；本方法保留为合并算法的第一层）。
    pub fn changed_files(&self, before: &GitSnapshot) -> Vec<String> {
        if !self.git || !before.git {
            return Vec::new();
        }
        let before_paths: HashSet<String> = before
            .status
            .iter()
            .filter_map(|line| parse_porcelain_path(line))
            .collect();
        let mut seen = HashSet::new();
        let mut changed = Vec::new();
        for path in self
            .status
            .iter()
            .filter_map(|line| parse_porcelain_path(line))
        {
            if !before_paths.contains(&path) && seen.insert(path.clone()) {
                changed.push(path);
            }
        }
        changed
    }

    /// 快照中的脏路径（去重保序；`git=false` → 空）。
    pub fn dirty_paths(&self) -> Vec<String> {
        if !self.git {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        self.status
            .iter()
            .filter_map(|line| parse_porcelain_path(line))
            .filter(|path| seen.insert(path.clone()))
            .collect()
    }
}

/// 单文件当前内容哈希（不存在/不可读 → None）。父模块（TrackedRoleWorker 基线
/// 采集）与本模块共用；与 core `change_set::file_hash` 同口径。
pub(crate) fn content_hash(root: &Path, relative: &str) -> Option<String> {
    let bytes = std::fs::read(root.join(relative)).ok()?;
    Some(owo_agent_core::cas_store::CasStore::hash_of(&bytes))
}

/// 九期（一路）：合并变更检测——`changed_files` = **窗口差集 ∪ 重命名 old 侧 ∪
/// 内容哈希差集**。
///
/// - `pre_dirty_hashes`：执行前已脏文件的内容哈希（执行前采集；`Some(Some(hash))` =
///   有基线，`Some(None)` = 执行前不存在/不可读，键缺失 = 未登记——后两者只有
///   「内容消失（删除）」可证明变更，保守不计修改）；
/// - 窗口差集捕获：新建/暂存新增/干净文件被修改或删除/重命名 new 侧；
/// - 内容哈希差集捕获：执行前已脏 → 执行后仍脏但内容变化（核心修复点）、执行前
///   已脏文件被删除、执行前已脏文件被恢复到 HEAD（内容 ≠ 执行前脏内容，同样计入
///   ——ChangeSet 恢复目标是「执行前状态」，回退到 HEAD 对它而言也是一次修改）；
/// - 重命名 old 侧捕获：post 有 `R old -> new` 且 pre 没有同一重命名 → old 被移走，
///   计入 changed（恢复时才能还原源文件）。
pub fn merge_changed_files(
    pre: &GitSnapshot,
    post: &GitSnapshot,
    pre_dirty_hashes: &HashMap<String, Option<String>>,
    root: &Path,
) -> Vec<String> {
    if !pre.git || !post.git {
        return Vec::new();
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut changed: Vec<String> = Vec::new();
    // 1) 窗口差集（新建/删除干净文件/重命名 new 侧/首次修改）。
    for path in post.changed_files(pre) {
        if seen.insert(path.clone()) {
            changed.push(path);
        }
    }
    // 2) 重命名 old 侧（pre 不存在同一重命名 → old 在窗口内被移走）。
    let pre_renames: HashSet<(String, String)> = pre
        .status
        .iter()
        .filter_map(|line| parse_porcelain_rename(line))
        .collect();
    for line in &post.status {
        if let Some((old, new)) = parse_porcelain_rename(line) {
            if !pre_renames.contains(&(old.clone(), new.clone())) && seen.insert(old.clone()) {
                changed.push(old);
            }
        }
    }
    // 3) 内容哈希差集（执行前已脏路径 × 执行后内容）。
    for path in pre.dirty_paths() {
        if seen.contains(&path) {
            continue;
        }
        let current = content_hash(root, &path);
        let changed_now = match pre_dirty_hashes.get(&path) {
            Some(Some(pre_hash)) => current.as_deref() != Some(pre_hash.as_str()),
            // 执行前不存在/不可读/未登记：只有删除可证明变更（保守，不误报修改）。
            _ => current.is_none(),
        };
        if changed_now && seen.insert(path.clone()) {
            changed.push(path);
        }
    }
    changed
}

/// 去掉 Windows verbatim 前缀（`\\?\C:\...` → `C:\...`）：canonicalize 语义不变。
/// 绑定侧 root/allowed 存储前去前缀，而 `canonicalize` 产物带前缀——比对两侧
/// 必须同口径，否则 `starts_with` 恒 false，白名单内变更会被误判越界。
pub(crate) fn simplify_path(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped.to_string()),
        None => path.to_path_buf(),
    }
}

/// 相对路径是否落在写白名单内（canonical 前缀判定，两侧同经 [`simplify_path`]
/// 去 verbatim；白名单空 = 未约束 → 放行）。
///
/// 与 [`check_whitelist`] 同一判定口径；十一期（二路）范围归属过滤复用。
/// Host-managed workspace internals (shell command wrappers/logs, artifact cache,
/// git metadata) are not deliverables and must not be attributed to a role's write
/// scope. `run_command` creates `.owo/shells/*` on every invocation; without this
/// exclusion a correctly scoped run_command fails as a scope violation.
pub fn is_host_internal_path(relative: &str) -> bool {
    let normalized = relative.replace('\\', "/");
    normalized == ".owo"
        || normalized.starts_with(".owo/")
        || normalized == ".git"
        || normalized.starts_with(".git/")
}

fn canonical_scope_candidate(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    loop {
        if let Ok(canonical) = current.canonicalize() {
            let canonical = simplify_path(&canonical);
            if let Ok(suffix) = path.strip_prefix(&current) {
                return canonical.join(suffix);
            }
            return canonical;
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return simplify_path(path),
        }
    }
}

pub fn path_in_allowed(relative: &str, root: &Path, allowed: &[PathBuf]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    // Canonicalize both the workspace root and each allowed prefix. Windows can
    // hand us a short-name/junction alias on one side and a long canonical path
    // on the other; stripping `\\?\\` alone does not make those identities equal.
    let base = canonical_scope_candidate(root);
    let candidate = canonical_scope_candidate(&base.join(relative));
    allowed
        .iter()
        .any(|prefix| candidate.starts_with(canonical_scope_candidate(prefix)))
}

/// 写白名单校验：`changed`（相对路径）必须全部落在 `allowed`（绝对前缀）内。
///
/// - `changed` 为空（无窗口新增变更 / 非 git 检测不到）→ 放行；
/// - `allowed` 为空 = 未声明白名单（工作区内可写）→ 放行；
/// - 否则逐条解析为绝对路径（canonicalize，不存在的目标按「父目录 canonicalize +
///   文件名」解析，与绑定侧口径一致；两侧比对前去 verbatim 前缀）；
/// - 越界 → `Err("scope_violation: …")`。
pub fn check_whitelist(changed: &[String], root: &Path, allowed: &[PathBuf]) -> Result<(), String> {
    if changed.is_empty() || allowed.is_empty() {
        return Ok(());
    }
    let offenders: Vec<String> = changed
        .iter()
        .filter(|relative| !path_in_allowed(relative, root, allowed))
        .cloned()
        .collect();
    if offenders.is_empty() {
        return Ok(());
    }
    let list = allowed
        .iter()
        .map(|prefix| prefix.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "scope_violation: 白名单外文件变更：{}（允许：{list}）",
        offenders.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::normalize_status_line;

    #[test]
    fn status_paths_are_rebased_to_workspace_root() {
        let prefix = "agent-sdk/team-runs/parkour3d/";
        assert_eq!(
            normalize_status_line("?? agent-sdk/team-runs/parkour3d/index.html", prefix),
            "?? index.html"
        );
        assert_eq!(
            normalize_status_line(
                "R  agent-sdk/team-runs/parkour3d/old.js -> agent-sdk/team-runs/parkour3d/new.js",
                prefix
            ),
            "R  old.js -> new.js"
        );
    }

    #[test]
    fn repo_root_prefix_keeps_paths_unchanged() {
        assert_eq!(normalize_status_line(" M src/lib.rs", ""), " M src/lib.rs");
    }
}
