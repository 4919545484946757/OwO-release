//! 执行工作区提供者（Agent V2 Phase 6）。
//!
//! 目标：代码任务在隔离工作区里执行，完成后给出可审阅的 Diff，用户可以选择
//! 应用、拒绝或回滚；并行任务不共享可写目录，删除任务/工作区不影响用户原始仓库。
//!
//! 本版只把 **本地 Git Worktree** 做完整：
//! - `prepare`：在数据目录创建 detached worktree，记录基线 commit；
//! - `diff`：暂存 worktree 内的改动并输出二进制安全的 unified diff；
//! - `apply`：把 Diff 应用到用户原工作区（冲突时如实报错，不强行覆盖）；
//! - `rollback`：反向应用已应用的 Diff；
//! - `reject` / `release`：丢弃 worktree 与改动，用户原工作区不受影响。
//!
//! `current_workspace` 是显式的不隔离模式：只允许查看 Diff，应用/拒绝/回滚都返回
//! `WORKSPACE_NOT_ISOLATED`，避免把「看见的改动」误当成「可撤销的改动」。
//! `local_sandbox` 与 `remote_workspace` 尚未达到契约，统一返回 `WORKSPACE_UNSUPPORTED`，
//! 不伪装成可用。

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const CODE_WORKSPACE_NOT_GIT: &str = "WORKSPACE_NOT_GIT";
pub const CODE_WORKSPACE_GIT_FAILED: &str = "WORKSPACE_GIT_FAILED";
pub const CODE_WORKSPACE_UNSUPPORTED: &str = "WORKSPACE_UNSUPPORTED";
pub const CODE_WORKSPACE_NOT_ISOLATED: &str = "WORKSPACE_NOT_ISOLATED";
pub const CODE_WORKSPACE_APPLY_CONFLICT: &str = "WORKSPACE_APPLY_CONFLICT";
pub const CODE_WORKSPACE_INVALID_STATE: &str = "WORKSPACE_INVALID_STATE";
pub const CODE_WORKSPACE_NOT_FOUND: &str = "WORKSPACE_NOT_FOUND";

/// 执行工作区的隔离方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// 直接在原工作区执行（无隔离，仅用于/查看 Diff）。
    CurrentWorkspace,
    /// 每个任务一个独立 Git worktree（推荐）。
    GitWorktree,
    /// 本地目录沙箱（尚未实现）。
    LocalSandbox,
    /// 远端工作区（Remote Agent Runtime 尚未达到契约）。
    RemoteWorkspace,
}

impl WorkspaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CurrentWorkspace => "current_workspace",
            Self::GitWorktree => "git_worktree",
            Self::LocalSandbox => "local_sandbox",
            Self::RemoteWorkspace => "remote_workspace",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "current_workspace" | "current" => Some(Self::CurrentWorkspace),
            "git_worktree" | "worktree" => Some(Self::GitWorktree),
            "local_sandbox" | "sandbox" => Some(Self::LocalSandbox),
            "remote_workspace" | "remote" => Some(Self::RemoteWorkspace),
            _ => None,
        }
    }
}

/// 工作区租约的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState {
    /// 已创建，任务可以写入；Diff 尚未应用。
    Prepared,
    /// 改动已应用到用户原工作区，可用 rollback 撤销。
    Applied,
    /// 改动被拒绝或已回滚，原工作区保持基线内容。
    Rejected,
    /// worktree / 沙箱目录已释放。
    Released,
}

impl WorkspaceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Applied => "applied",
            Self::Rejected => "rejected",
            Self::Released => "released",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceError {
    pub code: &'static str,
    pub message: String,
}

impl WorkspaceError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}（{}）", self.message, self.code)
    }
}

impl std::error::Error for WorkspaceError {}

/// 一个任务的执行工作区租约。`path` 是任务实际写入的目录；`base_root` 是用户原工作区。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceLease {
    pub workspace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub kind: WorkspaceKind,
    pub state: WorkspaceState,
    pub base_root: PathBuf,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceFileChange {
    pub path: String,
    /// `A` 新增 / `M` 修改 / `D` 删除 / `R` 重命名 / `?` 未跟踪。
    pub status: String,
    pub insertions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDiff {
    pub workspace_id: String,
    pub state: WorkspaceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    /// 可直接交给 `git apply` 的 unified diff（含二进制补丁）。
    pub patch: String,
    pub files: Vec<WorkspaceFileChange>,
}

impl WorkspaceDiff {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.patch.trim().is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct PrepareWorkspaceRequest {
    pub workspace_id: String,
    pub task_id: Option<String>,
    pub kind: WorkspaceKind,
    /// 用户原工作区（必须是已存在的目录）。
    pub base_root: PathBuf,
}

/// 工作区提供者：创建、审阅、应用与清理隔离工作区。
pub trait WorkspaceProvider: Send + Sync {
    fn kind(&self) -> WorkspaceKind;
    fn prepare(&self, request: &PrepareWorkspaceRequest) -> Result<WorkspaceLease, WorkspaceError>;
    fn diff(&self, lease: &WorkspaceLease) -> Result<WorkspaceDiff, WorkspaceError>;
    fn apply(&self, lease: &mut WorkspaceLease) -> Result<WorkspaceDiff, WorkspaceError>;
    fn reject(&self, lease: &mut WorkspaceLease) -> Result<(), WorkspaceError>;
    fn rollback(&self, lease: &mut WorkspaceLease) -> Result<(), WorkspaceError>;
    fn release(&self, lease: &mut WorkspaceLease) -> Result<(), WorkspaceError>;
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, WorkspaceError> {
    // 固定 `core.autocrlf=false`：worktree 检出的字节必须与基线一致，Diff 应用/回滚
    // 才是可逆的；否则 Windows 全局 autocrlf=true 会把 LF 文件改写成 CRLF。
    let output = Command::new("git")
        .arg("-c")
        .arg("core.autocrlf=false")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| {
            WorkspaceError::new(
                CODE_WORKSPACE_GIT_FAILED,
                format!("无法执行 git：{error}（需要本机安装 Git）"),
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(WorkspaceError::new(
            CODE_WORKSPACE_GIT_FAILED,
            format!("git {} 失败：{}", args.join(" "), stderr),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-c")
        .arg("core.autocrlf=false")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn repo_root(dir: &Path) -> Result<PathBuf, WorkspaceError> {
    if !dir.is_dir() {
        return Err(WorkspaceError::new(
            CODE_WORKSPACE_GIT_FAILED,
            format!("工作区不存在或不是目录：{}", dir.display()),
        ));
    }
    if !git_ok(dir, &["rev-parse", "--is-inside-work-tree"]) {
        return Err(WorkspaceError::new(
            CODE_WORKSPACE_NOT_GIT,
            format!("{} 不是 Git 工作区，无法创建隔离 worktree", dir.display()),
        ));
    }
    let root = run_git(dir, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(root.trim()))
}

fn head_commit(repo: &Path) -> Result<String, WorkspaceError> {
    let out = run_git(repo, &["rev-parse", "HEAD"]).map_err(|_| {
        WorkspaceError::new(
            CODE_WORKSPACE_NOT_GIT,
            "工作区还没有提交（HEAD 不存在），无法记录基线 commit",
        )
    })?;
    Ok(out.trim().to_string())
}

fn parse_name_status(text: &str) -> Vec<(String, String)> {
    // `git diff --name-status` 每行：状态\t路径（重命名是 R100\t旧\t新）。
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let status = parts.next()?.trim();
            let path = parts.nth(if status.starts_with('R') || status.starts_with('C') {
                1
            } else {
                0
            })?;
            if path.trim().is_empty() {
                return None;
            }
            let code = status.chars().next().unwrap_or('M').to_string();
            Some((code, path.trim().to_string()))
        })
        .collect()
}

fn parse_numstat(text: &str) -> std::collections::HashMap<String, (u64, u64)> {
    let mut map = std::collections::HashMap::new();
    for line in text.lines() {
        let mut parts = line.split('\t');
        let insertions = parts.next().unwrap_or("0");
        let deletions = parts.next().unwrap_or("0");
        let Some(path) = parts.next() else { continue };
        let count = |value: &str| value.parse::<u64>().unwrap_or(0);
        map.insert(
            path.trim().to_string(),
            (count(insertions), count(deletions)),
        );
    }
    map
}

/// Git Worktree 提供者：工作区目录由调用方给定（通常是数据根下的 `workspaces/`）。
#[derive(Debug, Clone)]
pub struct GitWorktreeProvider {
    worktree_root: PathBuf,
}

impl GitWorktreeProvider {
    pub fn new(worktree_root: impl Into<PathBuf>) -> Self {
        Self {
            worktree_root: worktree_root.into(),
        }
    }
}

impl WorkspaceProvider for GitWorktreeProvider {
    fn kind(&self) -> WorkspaceKind {
        WorkspaceKind::GitWorktree
    }

    fn prepare(&self, request: &PrepareWorkspaceRequest) -> Result<WorkspaceLease, WorkspaceError> {
        let repo = repo_root(&request.base_root)?;
        let base_commit = head_commit(&repo)?;
        std::fs::create_dir_all(&self.worktree_root).map_err(|error| {
            WorkspaceError::new(
                CODE_WORKSPACE_GIT_FAILED,
                format!("无法创建 worktree 根目录：{error}"),
            )
        })?;
        let path = self
            .worktree_root
            .join(format!("owo-ws-{}", request.workspace_id));
        if path.exists() {
            return Err(WorkspaceError::new(
                CODE_WORKSPACE_INVALID_STATE,
                format!("工作区目录已存在：{}", path.display()),
            ));
        }
        let path_text = path.display().to_string();
        run_git(
            &repo,
            &["worktree", "add", "--detach", &path_text, &base_commit],
        )?;
        Ok(WorkspaceLease {
            workspace_id: request.workspace_id.clone(),
            task_id: request.task_id.clone(),
            kind: WorkspaceKind::GitWorktree,
            state: WorkspaceState::Prepared,
            base_root: repo,
            path,
            base_commit: Some(base_commit),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    fn diff(&self, lease: &WorkspaceLease) -> Result<WorkspaceDiff, WorkspaceError> {
        if lease.state == WorkspaceState::Released {
            return Err(WorkspaceError::new(
                CODE_WORKSPACE_INVALID_STATE,
                "工作区已释放，Diff 不可用",
            ));
        }
        let base = lease.base_commit.as_deref().ok_or_else(|| {
            WorkspaceError::new(CODE_WORKSPACE_INVALID_STATE, "租约缺少基线 commit")
        })?;
        // 只暂存 worktree 自己的索引，不触碰用户原工作区。
        run_git(&lease.path, &["add", "-A"])?;
        let patch = run_git(&lease.path, &["diff", "--cached", "--binary", base])?;
        let name_status = run_git(&lease.path, &["diff", "--cached", "--name-status", base])?;
        let numstat = run_git(&lease.path, &["diff", "--cached", "--numstat", base])?;
        let stats = parse_numstat(&numstat);
        let files = parse_name_status(&name_status)
            .into_iter()
            .map(|(status, path)| {
                let (insertions, deletions) = stats.get(&path).copied().unwrap_or((0, 0));
                WorkspaceFileChange {
                    path,
                    status,
                    insertions,
                    deletions,
                }
            })
            .collect();
        Ok(WorkspaceDiff {
            workspace_id: lease.workspace_id.clone(),
            state: lease.state,
            base_commit: lease.base_commit.clone(),
            patch,
            files,
        })
    }

    fn apply(&self, lease: &mut WorkspaceLease) -> Result<WorkspaceDiff, WorkspaceError> {
        if lease.state != WorkspaceState::Prepared {
            return Err(WorkspaceError::new(
                CODE_WORKSPACE_INVALID_STATE,
                format!(
                    "只有 prepared 工作区可以应用改动，当前 {}",
                    lease.state.as_str()
                ),
            ));
        }
        let diff = self.diff(lease)?;
        if diff.is_empty() {
            return Err(WorkspaceError::new(
                CODE_WORKSPACE_INVALID_STATE,
                "工作区没有改动，无需应用",
            ));
        }
        // 用户在原工作区改了同一批文件时不能强行覆盖：git apply 失败会如实返回冲突。
        apply_patch(&lease.base_root, &diff.patch, false)?;
        lease.state = WorkspaceState::Applied;
        let mut applied = diff;
        applied.state = lease.state;
        Ok(applied)
    }

    fn reject(&self, lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        if lease.state == WorkspaceState::Released {
            return Err(WorkspaceError::new(
                CODE_WORKSPACE_INVALID_STATE,
                "工作区已释放，不能再拒绝",
            ));
        }
        if lease.state == WorkspaceState::Applied {
            self.rollback(lease)?;
        }
        remove_worktree(&lease.base_root, &lease.path)?;
        lease.state = WorkspaceState::Rejected;
        Ok(())
    }

    fn rollback(&self, lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        if lease.state != WorkspaceState::Applied {
            return Err(WorkspaceError::new(
                CODE_WORKSPACE_INVALID_STATE,
                format!("只有 applied 工作区可以回滚，当前 {}", lease.state.as_str()),
            ));
        }
        let diff = self.diff(lease)?;
        apply_patch(&lease.base_root, &diff.patch, true)?;
        lease.state = WorkspaceState::Rejected;
        Ok(())
    }

    fn release(&self, lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        remove_worktree(&lease.base_root, &lease.path)?;
        lease.state = WorkspaceState::Released;
        Ok(())
    }
}

fn apply_patch(repo: &Path, patch: &str, reverse: bool) -> Result<(), WorkspaceError> {
    use std::io::Write;
    let mut command = Command::new("git");
    command
        .arg("-c")
        .arg("core.autocrlf=false")
        .arg("-C")
        .arg(repo)
        .arg("apply");
    if reverse {
        command.arg("-R");
    }
    command
        .arg("--whitespace=nowarn")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        WorkspaceError::new(
            CODE_WORKSPACE_GIT_FAILED,
            format!("无法执行 git apply：{error}"),
        )
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(patch.as_bytes()).map_err(|error| {
            WorkspaceError::new(
                CODE_WORKSPACE_GIT_FAILED,
                format!("写入 Diff 失败：{error}"),
            )
        })?;
    }
    let output = child.wait_with_output().map_err(|error| {
        WorkspaceError::new(
            CODE_WORKSPACE_GIT_FAILED,
            format!("等待 git apply 失败：{error}"),
        )
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(WorkspaceError::new(
        CODE_WORKSPACE_APPLY_CONFLICT,
        if reverse {
            format!("回滚失败：原工作区在应用后又被修改，需人工处理。{stderr}")
        } else {
            format!("应用失败：原工作区存在冲突或已修改同一文件，未强行覆盖。{stderr}")
        },
    ))
}

fn remove_worktree(repo: &Path, path: &Path) -> Result<(), WorkspaceError> {
    let path_text = path.display().to_string();
    let _ = run_git(repo, &["worktree", "remove", "--force", &path_text]);
    if path.exists() {
        std::fs::remove_dir_all(path).map_err(|error| {
            WorkspaceError::new(
                CODE_WORKSPACE_GIT_FAILED,
                format!("无法删除工作区目录 {}：{error}", path.display()),
            )
        })?;
    }
    let _ = run_git(repo, &["worktree", "prune"]);
    Ok(())
}

/// 当前工作区（无隔离）：只提供 Diff 审阅，不提供应用/回滚。
#[derive(Debug, Clone)]
pub struct CurrentWorkspaceProvider {
    root: PathBuf,
}

impl CurrentWorkspaceProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl WorkspaceProvider for CurrentWorkspaceProvider {
    fn kind(&self) -> WorkspaceKind {
        WorkspaceKind::CurrentWorkspace
    }

    fn prepare(&self, request: &PrepareWorkspaceRequest) -> Result<WorkspaceLease, WorkspaceError> {
        let repo = repo_root(&self.root)?;
        let base_commit = head_commit(&repo).ok();
        Ok(WorkspaceLease {
            workspace_id: request.workspace_id.clone(),
            task_id: request.task_id.clone(),
            kind: WorkspaceKind::CurrentWorkspace,
            state: WorkspaceState::Prepared,
            path: repo.clone(),
            base_root: repo,
            base_commit,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    fn diff(&self, lease: &WorkspaceLease) -> Result<WorkspaceDiff, WorkspaceError> {
        // 只读：不暂存、不写索引。未跟踪文件不出现在 patch 里，files 会明确标注。
        let patch = run_git(&lease.path, &["diff", "--binary"])?;
        let name_status = run_git(&lease.path, &["diff", "--name-status"])?;
        let numstat = run_git(&lease.path, &["diff", "--numstat"])?;
        let stats = parse_numstat(&numstat);
        let mut files: Vec<WorkspaceFileChange> = parse_name_status(&name_status)
            .into_iter()
            .map(|(status, path)| {
                let (insertions, deletions) = stats.get(&path).copied().unwrap_or((0, 0));
                WorkspaceFileChange {
                    path,
                    status,
                    insertions,
                    deletions,
                }
            })
            .collect();
        let untracked = run_git(&lease.path, &["ls-files", "--others", "--exclude-standard"])?;
        for path in untracked
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            files.push(WorkspaceFileChange {
                path: path.to_string(),
                status: "?".to_string(),
                insertions: 0,
                deletions: 0,
            });
        }
        Ok(WorkspaceDiff {
            workspace_id: lease.workspace_id.clone(),
            state: lease.state,
            base_commit: lease.base_commit.clone(),
            patch,
            files,
        })
    }

    fn apply(&self, _lease: &mut WorkspaceLease) -> Result<WorkspaceDiff, WorkspaceError> {
        Err(WorkspaceError::new(
            CODE_WORKSPACE_NOT_ISOLATED,
            "当前工作区没有隔离：改动已经在原工作区，无法再“应用”一次",
        ))
    }

    fn reject(&self, _lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::new(
            CODE_WORKSPACE_NOT_ISOLATED,
            "当前工作区没有隔离：拒绝不会撤销已发生的改动",
        ))
    }

    fn rollback(&self, _lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::new(
            CODE_WORKSPACE_NOT_ISOLATED,
            "当前工作区没有隔离：没有可回滚的已应用 Diff",
        ))
    }

    fn release(&self, lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        // 原工作区不能删除；只结束租约。
        lease.state = WorkspaceState::Released;
        Ok(())
    }
}

/// 尚未达到契约的提供者：明确拒绝，而不是静默降级成原工作区。
#[derive(Debug, Clone)]
pub struct UnsupportedWorkspaceProvider {
    kind: WorkspaceKind,
    reason: &'static str,
}

impl UnsupportedWorkspaceProvider {
    pub fn local_sandbox() -> Self {
        Self {
            kind: WorkspaceKind::LocalSandbox,
            reason: "本地目录沙箱尚未实现：请使用 git_worktree，当前不会在不隔离的情况下执行",
        }
    }

    pub fn remote_workspace() -> Self {
        Self {
            kind: WorkspaceKind::RemoteWorkspace,
            reason: "远端 Agent Runtime 尚未达到契约：CloudExec 目前只提供命令执行与 Diff 回传",
        }
    }
}

impl WorkspaceProvider for UnsupportedWorkspaceProvider {
    fn kind(&self) -> WorkspaceKind {
        self.kind
    }

    fn prepare(
        &self,
        _request: &PrepareWorkspaceRequest,
    ) -> Result<WorkspaceLease, WorkspaceError> {
        Err(WorkspaceError::new(CODE_WORKSPACE_UNSUPPORTED, self.reason))
    }

    fn diff(&self, _lease: &WorkspaceLease) -> Result<WorkspaceDiff, WorkspaceError> {
        Err(WorkspaceError::new(CODE_WORKSPACE_UNSUPPORTED, self.reason))
    }

    fn apply(&self, _lease: &mut WorkspaceLease) -> Result<WorkspaceDiff, WorkspaceError> {
        Err(WorkspaceError::new(CODE_WORKSPACE_UNSUPPORTED, self.reason))
    }

    fn reject(&self, _lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::new(CODE_WORKSPACE_UNSUPPORTED, self.reason))
    }

    fn rollback(&self, _lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::new(CODE_WORKSPACE_UNSUPPORTED, self.reason))
    }

    fn release(&self, _lease: &mut WorkspaceLease) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::new(CODE_WORKSPACE_UNSUPPORTED, self.reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .expect("git 必须可执行");
            assert!(
                output.status.success(),
                "git {} 失败：{}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init"]);
        run(&["config", "user.email", "owo@test.local"]);
        run(&["config", "user.name", "OwO Test"]);
        std::fs::write(dir.join("app.txt"), "base\n").unwrap();
        run(&["add", "app.txt"]);
        run(&["commit", "-m", "base"]);
    }

    fn request(id: &str, base: &Path) -> PrepareWorkspaceRequest {
        PrepareWorkspaceRequest {
            workspace_id: id.to_string(),
            task_id: Some(format!("task-{id}")),
            kind: WorkspaceKind::GitWorktree,
            base_root: base.to_path_buf(),
        }
    }

    #[test]
    fn worktree_changes_are_isolated_and_reviewable() {
        if !git_available() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let provider = GitWorktreeProvider::new(temp.path().join("worktrees"));

        let mut lease = provider.prepare(&request("a", &repo)).unwrap();
        assert!(lease.path.is_dir());
        assert_ne!(lease.path, repo);
        assert_eq!(lease.state, WorkspaceState::Prepared);
        assert!(lease.base_commit.is_some());

        std::fs::write(lease.path.join("app.txt"), "base\nfrom-task\n").unwrap();
        std::fs::write(lease.path.join("new.txt"), "created\n").unwrap();
        let diff = provider.diff(&lease).unwrap();
        assert!(!diff.is_empty());
        assert!(diff.patch.contains("app.txt"));
        let paths: Vec<&str> = diff.files.iter().map(|file| file.path.as_str()).collect();
        assert!(paths.contains(&"app.txt"));
        assert!(paths.contains(&"new.txt"));
        assert_eq!(
            std::fs::read_to_string(repo.join("app.txt")).unwrap(),
            "base\n",
            "任务写入不能污染用户原工作区"
        );
        assert!(!repo.join("new.txt").exists());

        provider.apply(&mut lease).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("app.txt")).unwrap(),
            "base\nfrom-task\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("new.txt")).unwrap(),
            "created\n"
        );
        provider.release(&mut lease).unwrap();
        assert!(!lease.path.exists());
    }

    #[test]
    fn rollback_restores_the_original_workspace() {
        if !git_available() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let provider = GitWorktreeProvider::new(temp.path().join("worktrees"));
        let mut lease = provider.prepare(&request("b", &repo)).unwrap();
        std::fs::write(lease.path.join("app.txt"), "changed\n").unwrap();
        provider.apply(&mut lease).unwrap();
        assert_eq!(lease.state, WorkspaceState::Applied);
        provider.rollback(&mut lease).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("app.txt")).unwrap(),
            "base\n"
        );
        assert_eq!(lease.state, WorkspaceState::Rejected);
        provider.release(&mut lease).unwrap();
    }

    #[test]
    fn reject_never_touches_the_original_workspace() {
        if !git_available() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let provider = GitWorktreeProvider::new(temp.path().join("worktrees"));
        let mut lease = provider.prepare(&request("c", &repo)).unwrap();
        std::fs::write(lease.path.join("app.txt"), "rejected\n").unwrap();
        provider.reject(&mut lease).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("app.txt")).unwrap(),
            "base\n"
        );
        assert!(!lease.path.exists());
        assert_eq!(lease.state, WorkspaceState::Rejected);
        // 再次释放是幂等的（目录已删除，只改状态）。
        provider.release(&mut lease).unwrap();
    }

    #[test]
    fn parallel_leases_use_distinct_directories() {
        if !git_available() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let provider = GitWorktreeProvider::new(temp.path().join("worktrees"));
        let mut first = provider.prepare(&request("p1", &repo)).unwrap();
        let mut second = provider.prepare(&request("p2", &repo)).unwrap();
        assert_ne!(first.path, second.path);
        std::fs::write(first.path.join("app.txt"), "one\n").unwrap();
        std::fs::write(second.path.join("app.txt"), "two\n").unwrap();
        provider.apply(&mut first).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("app.txt")).unwrap(),
            "one\n"
        );
        // 第二个任务的工作区仍是自己的内容，不受第一个任务应用影响。
        assert_eq!(
            std::fs::read_to_string(second.path.join("app.txt")).unwrap(),
            "two\n"
        );
        provider.release(&mut first).unwrap();
        provider.release(&mut second).unwrap();
        // 两个任务都释放后，原工作区仍保留第一个任务已应用的改动。
        assert_eq!(
            std::fs::read_to_string(repo.join("app.txt")).unwrap(),
            "one\n"
        );
    }

    #[test]
    fn non_git_workspace_is_rejected_instead_of_silently_unisolated() {
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let provider = GitWorktreeProvider::new(temp.path().join("worktrees"));
        let error = provider.prepare(&request("d", &plain)).unwrap_err();
        if git_available() {
            assert_eq!(error.code, CODE_WORKSPACE_NOT_GIT);
        }
    }

    #[test]
    fn current_workspace_can_review_but_not_apply() {
        if !git_available() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        std::fs::write(repo.join("app.txt"), "edited\n").unwrap();
        let provider = CurrentWorkspaceProvider::new(&repo);
        let mut lease = provider.prepare(&request("e", &repo)).unwrap();
        let diff = provider.diff(&lease).unwrap();
        assert!(diff.files.iter().any(|file| file.path == "app.txt"));
        assert_eq!(
            provider.apply(&mut lease).unwrap_err().code,
            CODE_WORKSPACE_NOT_ISOLATED
        );
        assert_eq!(
            provider.rollback(&mut lease).unwrap_err().code,
            CODE_WORKSPACE_NOT_ISOLATED
        );
    }

    #[test]
    fn unimplemented_providers_refuse_instead_of_falling_back() {
        let temp = tempfile::tempdir().unwrap();
        let request = request("f", temp.path());
        let sandbox = UnsupportedWorkspaceProvider::local_sandbox();
        let remote = UnsupportedWorkspaceProvider::remote_workspace();
        for provider in [&sandbox as &dyn WorkspaceProvider, &remote] {
            let error = provider.prepare(&request).unwrap_err();
            assert_eq!(error.code, CODE_WORKSPACE_UNSUPPORTED);
        }
        assert_eq!(
            WorkspaceKind::parse("git_worktree"),
            Some(WorkspaceKind::GitWorktree)
        );
        assert_eq!(WorkspaceKind::parse("nope"), None);
    }
}
