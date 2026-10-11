//! M4 云端执行骨架（v0.2）
//!
//! 全链路契约：仓库快照 → 隔离执行 → diff 回传 → revert；凭据不落盘、任务间隔离、审计完整。
//!
//! v0.1 提供本地模拟执行器 `LocalSimExecutor`（零外部依赖，Windows cmd / Unix sh）。
//! v0.2 新增：
//! - `CloudTransport` 传输后端抽象（HTTP 远端 `HttpCloudTransport` + 不联网测试替身 `MockRemote`）；
//!   凭据只从环境变量读取（`OWO_CLOUD_HTTP_TOKEN`），结构体/持久化 JSON 不含凭据值。
//! - `CloudTaskQueue` 任务队列：JSON 持久化（临时/数据目录），重启可恢复。
//! - `CloudScheduler` 状态机 Queued→Running→Succeeded/Failed/Canceled + retry_count/指数退避。
//! - 进度事件流 `CloudEvent`（内存 mpsc channel 可订阅：快照/检出/执行/完成/失败/取消）。
//!
//! 安全基线：
//! - `CloudTaskSpec` 不携带任何凭据字段；子进程环境只透传 `env_passthrough` 白名单变量。
//! - 每个任务在工作区快照的隔离副本中执行，互不干扰，原工作区在显式 `apply_to` 前不被改动。
//! - 提交/执行/取消/重试/回滚全程写审计日志。

use owo_agent_kernel::audit::AuditLog;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 单文件变更（diff 的基本单元，可序列化回传/审阅/应用）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub change: DiffKind,
    /// 变更前内容（Deleted/Modified 时存在）。
    pub old: Option<String>,
    /// 变更后内容（Added/Modified 时存在）。
    pub new: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiffKind {
    Added,
    Modified,
    Deleted,
}

impl FileDiff {
    /// 计算应用目标路径并做越界防护（拒绝绝对路径与 `..` 父目录跳转，防 zip-slip 类攻击）。
    fn target_path(&self, root: &Path) -> Result<PathBuf, String> {
        let candidate = Path::new(&self.path);
        let escapes = candidate.is_absolute()
            || candidate.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::RootDir
                )
            });
        if escapes {
            return Err(format!(
                "diff 路径越界（拒绝绝对路径/父目录跳转）：{}",
                self.path
            ));
        }
        Ok(root.join(candidate))
    }

    /// 正向应用：把 root 下的文件从 old 推进到 new（新增/覆盖/删除）。
    pub fn apply(&self, root: &Path) -> Result<(), String> {
        let target = self.target_path(root)?;
        match self.change {
            DiffKind::Added | DiffKind::Modified => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("创建目录 {} 失败：{e}", parent.display()))?;
                }
                let content = self.new.as_deref().unwrap_or("");
                std::fs::write(&target, content)
                    .map_err(|e| format!("写入 {} 失败：{e}", target.display()))
            }
            DiffKind::Deleted => {
                if target.exists() {
                    std::fs::remove_file(&target)
                        .map_err(|e| format!("删除 {} 失败：{e}", target.display()))?;
                }
                Ok(())
            }
        }
    }

    /// 反向应用（revert）：把 root 下的文件恢复到 old 状态。
    pub fn reverse(&self, root: &Path) -> Result<(), String> {
        let reverted = match self.change {
            DiffKind::Added => FileDiff {
                path: self.path.clone(),
                change: DiffKind::Deleted,
                old: None,
                new: None,
            },
            DiffKind::Deleted => FileDiff {
                path: self.path.clone(),
                change: DiffKind::Added,
                old: None,
                new: self.old.clone(),
            },
            DiffKind::Modified => FileDiff {
                path: self.path.clone(),
                change: DiffKind::Modified,
                old: None,
                new: self.old.clone(),
            },
        };
        reverted.apply(root)
    }
}

/// 批量校验 diff 路径（合并审阅/应用前统一检查，任一越界即整体拒绝，不写盘）。
pub fn validate_batch(diffs: &[FileDiff], root: &Path) -> Result<(), String> {
    for d in diffs {
        d.target_path(root)?;
    }
    Ok(())
}

/// 多文件合并展示摘要：按变更类型计数 + 按路径排序列出（供 diff 审阅界面/CLI 输出）。
pub fn describe_diff(diffs: &[FileDiff]) -> String {
    let mut added: Vec<&str> = Vec::new();
    let mut modified: Vec<&str> = Vec::new();
    let mut deleted: Vec<&str> = Vec::new();
    for d in diffs {
        match d.change {
            DiffKind::Added => added.push(&d.path),
            DiffKind::Modified => modified.push(&d.path),
            DiffKind::Deleted => deleted.push(&d.path),
        }
    }
    let mut lines: Vec<String> = Vec::new();
    if !added.is_empty() {
        lines.push(format!("新增 {}：{}", added.len(), added.join(", ")));
    }
    if !modified.is_empty() {
        lines.push(format!("修改 {}：{}", modified.len(), modified.join(", ")));
    }
    if !deleted.is_empty() {
        lines.push(format!("删除 {}：{}", deleted.len(), deleted.join(", ")));
    }
    if lines.is_empty() {
        lines.push("无文件变更".to_string());
    }
    lines.join("；")
}

/// 任务用量计量（时长/改动量/重试次数，成本估算的原始数据）。
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
pub struct UsageMetrics {
    /// 本次执行耗时（毫秒，本端实测）。
    pub duration_ms: u64,
    /// diff 文件数。
    pub diff_count: usize,
    /// 重试次数。
    pub retry_count: u32,
}

/// 云端任务规格。注意：不提供任何凭据字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTaskSpec {
    pub name: String,
    /// 快照来源目录（v0.1 本地模拟时复制到隔离目录执行）。
    pub workspace_dir: PathBuf,
    /// 按序执行的命令。
    pub commands: Vec<String>,
    /// 环境变量白名单：只透传这些名字（值取自进程环境，不回写 spec）。
    pub env_passthrough: Vec<String>,
    /// 单条命令超时（秒）。
    pub timeout_secs: u64,
}

/// 任务执行结果：diff 携带回本地审阅/应用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTaskResult {
    pub task_id: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub diff: Vec<FileDiff>,
    /// diff 超过上限被截断时置 true。
    pub diff_truncated: bool,
}

impl CloudTaskResult {
    /// 把 diff 正向应用到 root（“带回本地”）。失败即中断，返回已应用条目以便回滚。
    pub fn apply_to(&self, root: &Path) -> Result<usize, (usize, String)> {
        if let Err(e) = validate_batch(&self.diff, root) {
            return Err((0, e));
        }
        for (i, d) in self.diff.iter().enumerate() {
            d.apply(root).map_err(|e| (i, e))?;
        }
        Ok(self.diff.len())
    }

    /// 回滚已经应用到 root 的 diff（与 apply_to 成对使用）。
    pub fn revert_from(&self, root: &Path) -> Result<usize, String> {
        validate_batch(&self.diff, root)?;
        let mut applied = 0usize;
        for d in self.diff.iter().rev() {
            d.reverse(root)
                .map_err(|e| format!("回滚 {} 失败:{e}", d.path))?;
            applied += 1;
        }
        Ok(applied)
    }
}

/// 任务句柄：submit 后持有，run 后携带结果。
#[derive(Debug)]
pub struct CloudTask {
    pub task_id: String,
    pub spec: CloudTaskSpec,
    pub(crate) temp_dir: Option<PathBuf>,
    pub result: Option<CloudTaskResult>,
}

/// 执行器抽象：submit → run → revert。未来可加 SSH/容器实现。
#[async_trait::async_trait]
pub trait CloudExecutor: Send {
    /// 提交任务并返回 task_id（v0.1 同步完成快照与隔离目录创建）。
    fn submit(&mut self, spec: CloudTaskSpec) -> Result<String, String>;
    /// 执行任务，产出 diff 结果。
    async fn run(&mut self, task_id: &str) -> Result<CloudTaskResult, String>;
    /// 回滚：销毁隔离执行环境并审计（原工作区由调用方经 apply/revert 控制）。
    async fn revert(&mut self, task_id: &str) -> Result<(), String>;
    /// 审计日志（只读）。
    fn audit(&self) -> &AuditLog;
}

const DIFF_LIMIT: usize = 200;

/// 断线重连：轮询/拉取瞬时传输错误的最大重试次数。
const POLL_RETRY_MAX: u32 = 4;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

/// 本地模拟执行器：把工作区快照复制到 `scratch_root/<task_id>/` 隔离执行。
pub struct LocalSimExecutor {
    scratch_root: PathBuf,
    tasks: BTreeMap<String, CloudTask>,
    audit: AuditLog,
    counter: u64,
}

impl LocalSimExecutor {
    pub fn new(scratch_root: PathBuf) -> Self {
        Self {
            scratch_root,
            tasks: BTreeMap::new(),
            audit: AuditLog::default(),
            counter: 0,
        }
    }

    fn snapshot(root: &Path) -> Result<BTreeMap<String, String>, String> {
        let mut snap = BTreeMap::new();
        let mut total: u64 = 0;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir)
                .map_err(|e| format!("读取目录 {} 失败：{e}", dir.display()))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("读取目录项失败：{e}"))?;
                let path = entry.path();
                let rel = path
                    .strip_prefix(root)
                    .map_err(|_| "路径越界".to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                let kind = entry
                    .file_type()
                    .map_err(|e| format!("读取类型失败：{e}"))?;
                if kind.is_dir() {
                    stack.push(path);
                } else if kind.is_file() {
                    let meta = entry
                        .metadata()
                        .map_err(|e| format!("读取元数据失败：{e}"))?;
                    if meta.len() > MAX_FILE_BYTES {
                        return Err(format!("文件超过上限：{rel}（{}B）", meta.len()));
                    }
                    total += meta.len();
                    if total > MAX_SNAPSHOT_BYTES {
                        return Err(format!("快照总大小超过上限（{MAX_SNAPSHOT_BYTES}B）"));
                    }
                    let bytes = std::fs::read(&path)
                        .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
                    snap.insert(rel, String::from_utf8_lossy(&bytes).to_string());
                }
            }
        }
        Ok(snap)
    }

    fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dst).map_err(|e| format!("创建 {} 失败：{e}", dst.display()))?;
        for entry in
            std::fs::read_dir(src).map_err(|e| format!("读取 {} 失败：{e}", src.display()))?
        {
            let entry = entry.map_err(|e| format!("读取目录项失败：{e}"))?;
            let kind = entry
                .file_type()
                .map_err(|e| format!("读取类型失败：{e}"))?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if kind.is_dir() {
                Self::copy_tree(&from, &to)?;
            } else if kind.is_file() {
                std::fs::copy(&from, &to)
                    .map_err(|e| format!("复制 {} → {} 失败：{e}", from.display(), to.display()))?;
            }
        }
        Ok(())
    }

    /// 计算 after 相对 before 的 diff（按路径排序，超限截断）。
    fn compute_diff(before_root: &Path, after_root: &Path) -> (Vec<FileDiff>, bool) {
        let before = Self::snapshot(before_root).unwrap_or_default();
        let after = Self::snapshot(after_root).unwrap_or_default();
        let mut diff = Vec::new();
        let mut truncated = false;
        for (path, new_content) in &after {
            match before.get(path) {
                None => diff.push(FileDiff {
                    path: path.clone(),
                    change: DiffKind::Added,
                    old: None,
                    new: Some(new_content.clone()),
                }),
                Some(old) if old != new_content => diff.push(FileDiff {
                    path: path.clone(),
                    change: DiffKind::Modified,
                    old: Some(old.clone()),
                    new: Some(new_content.clone()),
                }),
                _ => {}
            }
        }
        for path in before.keys() {
            if !after.contains_key(path) {
                diff.push(FileDiff {
                    path: path.clone(),
                    change: DiffKind::Deleted,
                    old: before.get(path).cloned(),
                    new: None,
                });
            }
        }
        if diff.len() > DIFF_LIMIT {
            diff.truncate(DIFF_LIMIT);
            truncated = true;
        }
        (diff, truncated)
    }

    fn shell() -> (&'static str, &'static str) {
        if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        }
    }
}

#[async_trait::async_trait]
impl CloudExecutor for LocalSimExecutor {
    fn submit(&mut self, spec: CloudTaskSpec) -> Result<String, String> {
        if spec.commands.is_empty() {
            return Err("任务至少需要一条命令".to_string());
        }
        if spec.env_passthrough.iter().any(|k| k.contains('=')) {
            return Err("env_passthrough 只允许变量名，禁止内联值".to_string());
        }
        if !spec.workspace_dir.is_dir() {
            return Err(format!(
                "工作区目录不存在：{}",
                spec.workspace_dir.display()
            ));
        }
        Self::snapshot(&spec.workspace_dir)?; // 先校验快照可读
        self.counter += 1;
        let task_id = format!("cloud-{:05}", self.counter);
        let temp_dir = self.scratch_root.join(&task_id);
        Self::copy_tree(&spec.workspace_dir, &temp_dir)?;
        let task = CloudTask {
            task_id: task_id.clone(),
            spec,
            temp_dir: Some(temp_dir),
            result: None,
        };
        self.audit.record(
            "cloud",
            "cloud.submit",
            None,
            None,
            format!(
                "task_id={task_id} 命令数={} 隔离目录已就绪",
                task.spec.commands.len()
            ),
        );
        self.tasks.insert(task_id.clone(), task);
        Ok(task_id)
    }

    async fn run(&mut self, task_id: &str) -> Result<CloudTaskResult, String> {
        let (workdir, commands, passthrough, timeout, workspace_dir) = {
            let task = self
                .tasks
                .get(task_id)
                .ok_or_else(|| format!("任务不存在：{task_id}"))?;
            (
                task.temp_dir
                    .clone()
                    .ok_or_else(|| format!("任务已回滚：{task_id}"))?,
                task.spec.commands.clone(),
                task.spec.env_passthrough.clone(),
                task.spec.timeout_secs,
                task.spec.workspace_dir.clone(),
            )
        };
        let (shell, flag) = Self::shell();
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = None;
        let mut timed_out = false;
        for cmd in &commands {
            let mut builder = tokio::process::Command::new(shell);
            builder.arg(flag).arg(cmd).current_dir(&workdir);
            builder.env_clear().kill_on_drop(true);
            // env_clear 后恒保留 PATH/SystemRoot（非机密），其余仅透传白名单。
            for keep in ["PATH", "SystemRoot", "COMSPEC"] {
                if let Ok(value) = std::env::var(keep) {
                    builder.env(keep, value);
                }
            }
            for key in &passthrough {
                if let Ok(value) = std::env::var(key) {
                    builder.env(key, value);
                }
            }
            let output = match tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                builder.output(),
            )
            .await
            {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => return Err(format!("命令启动失败：{e}")),
                Err(_) => {
                    timed_out = true;
                    break;
                }
            };
            stdout.push_str(&String::from_utf8_lossy(&output.stdout));
            stderr.push_str(&String::from_utf8_lossy(&output.stderr));
            exit_code = output.status.code();
            if exit_code != Some(0) {
                break;
            }
        }
        let (diff, truncated) = Self::compute_diff(&workspace_dir, &workdir);
        let result = CloudTaskResult {
            task_id: task_id.to_string(),
            exit_code,
            stdout,
            stderr,
            diff,
            diff_truncated: truncated,
        };
        self.audit.record(
            "cloud",
            "cloud.run",
            None,
            None,
            format!(
                "task_id={task_id} 退出码={:?} diff 条目={} 截断={} 超时={}",
                result.exit_code,
                result.diff.len(),
                truncated,
                timed_out
            ),
        );
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.result = Some(result.clone());
        }
        if timed_out {
            return Err(format!("命令执行超时（>{timeout}s）"));
        }
        Ok(result)
    }

    async fn revert(&mut self, task_id: &str) -> Result<(), String> {
        let task = self
            .tasks
            .remove(task_id)
            .ok_or_else(|| format!("任务不存在：{task_id}"))?;
        if let Some(temp_dir) = task.temp_dir {
            if temp_dir.exists() {
                std::fs::remove_dir_all(&temp_dir).map_err(|e| format!("清理隔离目录失败：{e}"))?;
            }
        }
        self.audit.record(
            "cloud",
            "cloud.revert",
            None,
            None,
            format!("task_id={task_id} 隔离环境已销毁，原工作区未被改动"),
        );
        Ok(())
    }

    fn audit(&self) -> &AuditLog {
        &self.audit
    }
}

mod transport;

pub use transport::{
    cloud_token_from_env, CloudTransport, HttpTransport, MockRemoteTransport, RemoteStatus,
};

// ----------------------------------------------------------------------------
// 任务队列：状态机（Queued→Running→Succeeded/Failed/Canceled）+ 重试退避 + JSON 持久化
// ----------------------------------------------------------------------------

/// 本地任务状态机状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

/// 任务记录：唯一可持久化结构（序列化不含任何凭据；spec 本身无凭据字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: String,
    pub remote_id: Option<String>,
    pub spec: CloudTaskSpec,
    pub state: TaskState,
    pub retry_count: u32,
    pub last_error: Option<String>,
    pub result: Option<CloudTaskResult>,
    pub created_at: String,
    /// 本次执行耗时（毫秒，本端实测；恢复旧 JSON 时缺省 0）。
    #[serde(default)]
    pub duration_ms: u64,
}

/// 进度事件序列（快照/提交/执行/回传/重试/终态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudProgress {
    Snapshotting { task_id: String },
    Submitting { task_id: String },
    Submitted { task_id: String, remote_id: String },
    Executing { task_id: String },
    Fetching { task_id: String },
    Retrying { task_id: String, retry_count: u32 },
    Succeeded { task_id: String, diff_count: usize },
    Failed { task_id: String, error: String },
    Canceled { task_id: String },
}

/// 进度订阅接口：内存 channel（UnboundedSender）或测试收集器均可。
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: &CloudProgress);
}

/// 空实现：CLI/不需要订阅的场景。
pub struct NullSink;

impl ProgressSink for NullSink {
    fn emit(&self, _event: &CloudProgress) {}
}

impl ProgressSink for tokio::sync::mpsc::UnboundedSender<CloudProgress> {
    fn emit(&self, event: &CloudProgress) {
        let _ = self.send(event.clone());
    }
}

/// 内存收集器（测试用）。
pub struct CollectingSink {
    events: std::sync::Mutex<Vec<CloudProgress>>,
}

impl CollectingSink {
    pub fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn all(&self) -> Vec<CloudProgress> {
        self.events.lock().unwrap().clone()
    }
}

impl Default for CollectingSink {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressSink for CollectingSink {
    fn emit(&self, event: &CloudProgress) {
        self.events.lock().unwrap().push(event.clone());
    }
}

/// 重试退避：base_secs * 2^retry_count，封顶 60 秒（纯函数，便于测试）。
pub fn backoff_delay(base_secs: u64, retry_count: u32) -> std::time::Duration {
    let secs = base_secs.saturating_mul(1u64 << retry_count.min(6));
    std::time::Duration::from_secs(secs.min(60))
}

fn validate_task_spec(spec: &CloudTaskSpec, allowlist: &[String]) -> Result<(), String> {
    validate_commands(&spec.commands, allowlist)?;
    if spec.commands.is_empty() {
        return Err("任务至少需要一条命令".to_string());
    }
    if spec.env_passthrough.iter().any(|key| key.contains('=')) {
        return Err("env_passthrough 只允许变量名，禁止内联值".to_string());
    }
    if !spec.workspace_dir.is_dir() {
        return Err(format!(
            "工作区目录不存在：{}",
            spec.workspace_dir.display()
        ));
    }
    Ok(())
}

/// 命令校验：危险模式恒拒绝；提供前缀白名单时，非白名单命令也拒绝。
pub fn validate_commands(commands: &[String], allowlist: &[String]) -> Result<(), String> {
    const DANGEROUS: &[&str] = &[
        "rm -rf /",
        "rm -rf *",
        "format c:",
        "del /s /q",
        "> /dev/sda",
        "shutdown",
        "taskkill /f /im",
        "rd /s /q c:\\",
        "del c:\\",
    ];
    for command in commands {
        let trimmed = command.trim();
        for pattern in DANGEROUS {
            if trimmed.to_lowercase().contains(pattern) {
                return Err(format!("命令含危险模式被拒绝：{command}（命中 {pattern}）"));
            }
        }
        if !allowlist.is_empty() {
            let allowed = allowlist.iter().any(|prefix| trimmed.starts_with(prefix));
            if !allowed {
                return Err(format!(
                    "命令不在白名单内被拒绝：{command}（允许前缀：{}）",
                    allowlist.join(" | ")
                ));
            }
        }
    }
    Ok(())
}

/// 任务队列：状态机 + 重试 + 持久化（`<dir>/<task_id>.json`）+ 审计。
pub struct CloudTaskQueue {
    dir: PathBuf,
    tasks: BTreeMap<String, TaskRecord>,
    transport: Box<dyn CloudTransport>,
    audit: AuditLog,
    max_retries: u32,
    base_backoff_secs: u64,
    command_allowlist: Vec<String>,
    poll_interval: std::time::Duration,
}

impl CloudTaskQueue {
    pub fn new(dir: PathBuf, transport: Box<dyn CloudTransport>) -> Self {
        Self {
            dir,
            tasks: BTreeMap::new(),
            transport,
            audit: AuditLog::default(),
            max_retries: 2,
            base_backoff_secs: 1,
            command_allowlist: Vec::new(),
            poll_interval: std::time::Duration::from_millis(50),
        }
    }

    /// Persist a task without borrowing the long-lived execution queue.
    /// Callers must serialize this operation across writers so sequential IDs
    /// remain unique. The execution owner refreshes this durable record later.
    pub fn submit_persisted(
        dir: &Path,
        spec: CloudTaskSpec,
        command_allowlist: &[String],
    ) -> Result<TaskRecord, String> {
        validate_task_spec(&spec, command_allowlist)?;
        std::fs::create_dir_all(dir).map_err(|error| format!("创建队列目录失败：{error}"))?;
        let mut max_id = 0u32;
        for entry in std::fs::read_dir(dir).map_err(|error| format!("读取队列目录失败：{error}"))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let Some(number) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.strip_prefix("cloud-"))
                .and_then(|suffix| suffix.parse::<u32>().ok())
            else {
                continue;
            };
            max_id = max_id.max(number);
        }
        let next_id = max_id
            .checked_add(1)
            .ok_or_else(|| "云端任务 ID 空间已耗尽".to_string())?;
        let task_id = format!("cloud-{next_id:04}");
        let record = TaskRecord {
            task_id,
            remote_id: None,
            spec,
            state: TaskState::Queued,
            retry_count: 0,
            last_error: None,
            result: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            duration_ms: 0,
        };
        let content = serde_json::to_string_pretty(&record).map_err(|error| error.to_string())?;
        crate::atomic_write(
            &dir.join(format!("{}.json", record.task_id)),
            content.as_bytes(),
        )
        .map_err(|error| format!("任务持久化失败：{error}"))?;
        Ok(record)
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_command_allowlist(mut self, allowlist: Vec<String>) -> Self {
        self.command_allowlist = allowlist;
        self
    }

    /// 提交：校验（工作区/命令/白名单）→ Queued → 持久化 → 审计。
    pub fn submit(&mut self, spec: CloudTaskSpec) -> Result<String, String> {
        validate_task_spec(&spec, &self.command_allowlist)?;
        let next_id = self
            .tasks
            .keys()
            .filter_map(|id| {
                id.strip_prefix("cloud-")
                    .and_then(|n| n.parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        let task_id = format!("cloud-{next_id:04}");
        let record = TaskRecord {
            task_id: task_id.clone(),
            remote_id: None,
            spec,
            state: TaskState::Queued,
            retry_count: 0,
            last_error: None,
            result: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            duration_ms: 0,
        };
        self.audit.record(
            "cloud",
            "cloud.submit",
            Some(task_id.clone()),
            None,
            format!(
                "任务入队：命令数={} 传输={}",
                record.spec.commands.len(),
                self.transport.kind()
            ),
        );
        self.tasks.insert(task_id.clone(), record);
        self.persist(&task_id)?;
        Ok(task_id)
    }

    /// 执行队列中第一个 Queued 任务，推进到终态；返回该任务 id。
    /// 失败未超重试上限 → 回 Queued（等待 retry()/下一轮 run_next），超限 → Failed。
    pub async fn run_next(&mut self, sink: &dyn ProgressSink) -> Result<Option<String>, String> {
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let result = self.run_next_with_cancel(sink, cancel_rx).await;
        drop(cancel_tx);
        result
    }

    pub async fn run_next_with_cancel(
        &mut self,
        sink: &dyn ProgressSink,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Option<String>, String> {
        let task_id = self
            .tasks
            .iter()
            .find(|(_, task)| task.state == TaskState::Queued)
            .map(|(id, _)| id.clone());
        let Some(task_id) = task_id else {
            return Ok(None);
        };
        self.run_task_with_cancel(&task_id, sink, cancel_rx).await
    }

    /// Execute one specific queued task. The server dispatcher uses this to
    /// drain a fixed queue snapshot without accidentally rerunning a retry that
    /// was returned to Queued during the same drain.
    pub async fn run_task_with_cancel(
        &mut self,
        task_id: &str,
        sink: &dyn ProgressSink,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Option<String>, String> {
        if !self
            .tasks
            .get(task_id)
            .is_some_and(|task| task.state == TaskState::Queued)
        {
            return Ok(None);
        }
        let task_id = task_id.to_string();

        // A cancellation signal may predate runner subscription (for example,
        // an HTTP cancel racing background dispatch). Honor its current value
        // before marking the task Running or starting remote work.
        let canceled_before_start = *cancel_rx.borrow();
        if canceled_before_start {
            self.cancel(&task_id).await?;
            sink.emit(&CloudProgress::Canceled {
                task_id: task_id.clone(),
            });
            return Ok(Some(task_id));
        }

        self.set_state(&task_id, TaskState::Running, None)?;
        let started_at = std::time::Instant::now();
        sink.emit(&CloudProgress::Snapshotting {
            task_id: task_id.clone(),
        });
        sink.emit(&CloudProgress::Submitting {
            task_id: task_id.clone(),
        });

        let spec = self.tasks.get(&task_id).unwrap().spec.clone();
        let outcome = self
            .run_via_transport(&task_id, &spec, sink, &mut cancel_rx)
            .await;

        let canceled_during_run = *cancel_rx.borrow();
        if outcome.is_err() && canceled_during_run {
            self.cancel(&task_id).await?;
            sink.emit(&CloudProgress::Canceled {
                task_id: task_id.clone(),
            });
            return Ok(Some(task_id));
        }

        match outcome {
            Ok(result) => {
                if let Some(record) = self.tasks.get_mut(&task_id) {
                    record.result = Some(result.clone());
                    record.remote_id = Some(result.task_id.clone());
                    record.duration_ms = started_at.elapsed().as_millis() as u64;
                }
                self.set_state(&task_id, TaskState::Succeeded, None)?;
                sink.emit(&CloudProgress::Succeeded {
                    task_id: task_id.clone(),
                    diff_count: result.diff.len(),
                });
            }
            Err(error) => {
                let retry_count = self.tasks.get(&task_id).map(|t| t.retry_count).unwrap_or(0);
                if let Some(record) = self.tasks.get_mut(&task_id) {
                    record.last_error = Some(error.clone());
                    record.retry_count = retry_count + 1;
                    record.duration_ms = started_at.elapsed().as_millis() as u64;
                }
                let retry_count = retry_count + 1;
                if retry_count <= self.max_retries {
                    self.set_state(&task_id, TaskState::Queued, Some(error.clone()))?;
                    self.audit.record(
                        "cloud",
                        "cloud.retry",
                        Some(task_id.clone()),
                        None,
                        format!(
                            "第 {retry_count} 次重试，退避 {}s：{error}",
                            backoff_delay(self.base_backoff_secs, retry_count).as_secs()
                        ),
                    );
                    sink.emit(&CloudProgress::Retrying {
                        task_id: task_id.clone(),
                        retry_count,
                    });
                } else {
                    self.set_state(&task_id, TaskState::Failed, Some(error.clone()))?;
                    sink.emit(&CloudProgress::Failed {
                        task_id: task_id.clone(),
                        error,
                    });
                }
            }
        }
        Ok(Some(task_id))
    }

    /// Retry one task through a terminal state, applying cancellable backoff
    /// between attempts. Queue owners should call this when a dispatch token is
    /// consumed so a retry cannot be stranded in Queued.
    pub async fn run_task_to_terminal_with_cancel(
        &mut self,
        task_id: &str,
        sink: &dyn ProgressSink,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Option<String>, String> {
        loop {
            let result = self
                .run_task_with_cancel(task_id, sink, cancel_rx.clone())
                .await?;
            let Some(result_id) = result else {
                return Ok(None);
            };
            let Some(record) = self.tasks.get(task_id) else {
                return Err(format!("任务执行后记录丢失：{task_id}"));
            };
            if record.state != TaskState::Queued {
                return Ok(Some(result_id));
            }

            let delay = backoff_delay(self.base_backoff_secs, record.retry_count);
            let canceled = *cancel_rx.borrow();
            if canceled {
                self.cancel(task_id).await?;
                sink.emit(&CloudProgress::Canceled {
                    task_id: task_id.to_string(),
                });
                return Ok(Some(result_id));
            }
            tokio::select! {
                biased;
                changed = cancel_rx.changed() => {
                    if changed.is_err() || *cancel_rx.borrow() {
                        self.cancel(task_id).await?;
                        sink.emit(&CloudProgress::Canceled {
                            task_id: task_id.to_string(),
                        });
                        return Ok(Some(result_id));
                    }
                }
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }

    async fn run_via_transport(
        &mut self,
        task_id: &str,
        spec: &CloudTaskSpec,
        sink: &dyn ProgressSink,
        cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<CloudTaskResult, String> {
        let remote_id = self.transport.submit(spec).await?;
        sink.emit(&CloudProgress::Submitted {
            task_id: task_id.to_string(),
            remote_id: remote_id.clone(),
        });
        if let Some(record) = self.tasks.get_mut(task_id) {
            record.remote_id = Some(remote_id.clone());
        }
        self.persist(task_id)?;

        sink.emit(&CloudProgress::Executing {
            task_id: task_id.to_string(),
        });
        // 轮询远端状态，超时熔断（命令超时 × 2 作为总预算）。
        let budget = std::time::Duration::from_secs(spec.timeout_secs.max(1) * 2);
        let deadline = tokio::time::Instant::now() + budget;
        let mut poll_attempts = 0u32;
        let status = loop {
            let status_result = tokio::select! {
                biased;
                changed = cancel_rx.changed() => {
                    if changed.is_err() || *cancel_rx.borrow() {
                        return Err("cancelled".to_string());
                    }
                    continue;
                }
                result = self.transport.status(&remote_id) => result,
            };
            let status = match status_result {
                Ok(status) => status,
                Err(e) => {
                    // 断线重连：瞬时传输错误按退避重试（最多 4 次），不直接判失败。
                    if poll_attempts >= POLL_RETRY_MAX {
                        return Err(format!(
                            "远端状态轮询失败（已重试 {poll_attempts} 次）：{e}"
                        ));
                    }
                    poll_attempts += 1;
                    sink.emit(&CloudProgress::Retrying {
                        task_id: task_id.to_string(),
                        retry_count: poll_attempts,
                    });
                    tokio::select! {
                        biased;
                        changed = cancel_rx.changed() => {
                            if changed.is_err() || *cancel_rx.borrow() {
                                return Err("cancelled".to_string());
                            }
                        }
                        _ = tokio::time::sleep(backoff_delay(1, poll_attempts)) => {}
                    }
                    continue;
                }
            };
            match status {
                RemoteStatus::Succeeded => break RemoteStatus::Succeeded,
                RemoteStatus::Failed(reason) => return Err(reason),
                RemoteStatus::Canceled => return Err("远端任务已被取消".to_string()),
                RemoteStatus::Queued | RemoteStatus::Running => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(format!("远端任务轮询超时（{budget:?}）"));
                    }
                    tokio::select! {
                        biased;
                        changed = cancel_rx.changed() => {
                            if changed.is_err() || *cancel_rx.borrow() {
                                return Err("cancelled".to_string());
                            }
                        }
                        _ = tokio::time::sleep(self.poll_interval) => {}
                    }
                }
            }
        };
        let _ = status;
        sink.emit(&CloudProgress::Fetching {
            task_id: task_id.to_string(),
        });
        let result = loop {
            let fetch_result = tokio::select! {
                biased;
                changed = cancel_rx.changed() => {
                    if changed.is_err() || *cancel_rx.borrow() {
                        return Err("cancelled".to_string());
                    }
                    continue;
                }
                result = self.transport.fetch_result(&remote_id) => result,
            };
            match fetch_result {
                Ok(result) => break result,
                Err(e) => {
                    if poll_attempts >= POLL_RETRY_MAX {
                        return Err(format!(
                            "远端结果拉取失败（已重试 {poll_attempts} 次）：{e}"
                        ));
                    }
                    poll_attempts += 1;
                    sink.emit(&CloudProgress::Retrying {
                        task_id: task_id.to_string(),
                        retry_count: poll_attempts,
                    });
                    tokio::select! {
                        biased;
                        changed = cancel_rx.changed() => {
                            if changed.is_err() || *cancel_rx.borrow() {
                                return Err("cancelled".to_string());
                            }
                        }
                        _ = tokio::time::sleep(backoff_delay(1, poll_attempts)) => {}
                    }
                }
            }
        };
        if result.exit_code != Some(0) {
            return Err(format!(
                "远端执行失败：退出码={:?} stderr={}",
                result.exit_code,
                result.stderr.trim()
            ));
        }
        Ok(result)
    }

    /// 手工重试：Failed/Canceled → Queued。
    pub fn retry(&mut self, task_id: &str) -> Result<(), String> {
        let record = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("任务不存在：{task_id}"))?;
        match record.state {
            TaskState::Failed | TaskState::Canceled => {
                record.state = TaskState::Queued;
                record.last_error = None;
            }
            _ => return Err(format!("任务 {task_id} 处于 {:?}，无法重试", record.state)),
        }
        self.audit.record(
            "cloud",
            "cloud.retry.manual",
            Some(task_id.to_string()),
            None,
            "手工重试入队",
        );
        self.persist(task_id)
    }

    /// 取消：Canceling → Canceled（远端 cancel + 审计 + 持久化）。
    pub async fn cancel(&mut self, task_id: &str) -> Result<(), String> {
        let record = self
            .tasks
            .get(task_id)
            .ok_or_else(|| format!("任务不存在：{task_id}"))?;
        if record.state == TaskState::Canceled {
            return Ok(());
        }
        if record.state == TaskState::Succeeded || record.state == TaskState::Failed {
            return Err(format!("任务 {task_id} 已终结，无法取消"));
        }
        if let Some(remote_id) = &record.remote_id {
            self.transport.cancel(remote_id).await?;
        }
        self.set_state(task_id, TaskState::Canceled, Some("用户取消".to_string()))?;
        self.audit.record(
            "cloud",
            "cloud.cancel",
            Some(task_id.to_string()),
            None,
            "任务取消",
        );
        Ok(())
    }

    /// 把任务 diff 应用到本地工作区（审阅后带回）。失败返回 (已应用条数, 错误)。
    pub async fn apply_to(&mut self, task_id: &str, root: &Path) -> Result<usize, (usize, String)> {
        let record = self
            .tasks
            .get(task_id)
            .ok_or_else(|| (0, format!("任务不存在：{task_id}")))?;
        let Some(result) = &record.result else {
            return Err((0, format!("任务 {task_id} 尚无结果")));
        };
        let applied = result.apply_to(root)?;
        self.audit.record(
            "cloud",
            "cloud.apply",
            Some(task_id.to_string()),
            Some(true),
            format!("diff 已应用到本地，{applied} 条"),
        );
        Ok(applied)
    }

    /// 回滚已应用的 diff。
    pub async fn revert_from(&mut self, task_id: &str, root: &Path) -> Result<usize, String> {
        let record = self
            .tasks
            .get(task_id)
            .ok_or_else(|| format!("任务不存在：{task_id}"))?;
        let Some(result) = &record.result else {
            return Err(format!("任务 {task_id} 尚无结果"));
        };
        let reverted = result.revert_from(root)?;
        self.audit.record(
            "cloud",
            "cloud.revert",
            Some(task_id.to_string()),
            Some(true),
            format!("diff 已回滚，{reverted} 条"),
        );
        Ok(reverted)
    }

    /// 任务用量计量（时长/改动量/重试次数）。
    pub fn usage(&self, task_id: &str) -> Option<UsageMetrics> {
        self.record(task_id).map(|r| UsageMetrics {
            duration_ms: r.duration_ms,
            diff_count: r.result.as_ref().map(|res| res.diff.len()).unwrap_or(0),
            retry_count: r.retry_count,
        })
    }

    pub fn record(&self, task_id: &str) -> Option<&TaskRecord> {
        self.tasks.get(task_id)
    }

    pub fn diff(&self, task_id: &str) -> Option<&[FileDiff]> {
        self.tasks
            .get(task_id)?
            .result
            .as_ref()
            .map(|r| r.diff.as_slice())
    }

    pub fn list(&self) -> Vec<TaskRecord> {
        self.tasks.values().cloned().collect()
    }

    /// 从持久化目录恢复任务：Queued/Running 保持语义；Running 重置为 Queued（可重跑）。
    pub fn recover(&mut self) -> Result<usize, String> {
        let mut recovered = 0usize;
        if !self.dir.is_dir() {
            return Ok(0);
        }
        for entry in std::fs::read_dir(&self.dir).map_err(|e| format!("读取队列目录失败：{e}"))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut record: TaskRecord = serde_json::from_str(&content)
                .map_err(|e| format!("任务记录解析失败（{}）：{e}", path.display()))?;
            let file_task_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| format!("任务记录文件名无效：{}", path.display()))?;
            if record.task_id != file_task_id
                || !record.task_id.starts_with("cloud-")
                || !record
                    .task_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err(format!("任务记录身份与文件名不匹配：{}", path.display()));
            }
            let was_running = record.state == TaskState::Running;
            if was_running {
                record.state = TaskState::Queued;
                record.last_error = Some("进程重启，任务恢复为待执行".to_string());
            }
            recovered += 1;
            let task_id = record.task_id.clone();
            self.tasks.insert(task_id.clone(), record);
            if was_running {
                self.persist(&task_id)?;
            }
        }
        self.audit.record(
            "cloud",
            "cloud.recover",
            None,
            None,
            format!("从 {} 恢复任务 {recovered} 个", self.dir.display()),
        );
        Ok(recovered)
    }

    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    /// 当前传输后端类型（mock/http），供 UI/CLI 展示。
    pub fn transport_kind(&self) -> &'static str {
        self.transport.kind()
    }

    fn set_state(
        &mut self,
        task_id: &str,
        state: TaskState,
        error: Option<String>,
    ) -> Result<(), String> {
        if let Some(record) = self.tasks.get_mut(task_id) {
            record.state = state;
            record.last_error = error;
        }
        self.persist(task_id)
    }

    fn persist(&self, task_id: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("创建队列目录失败：{e}"))?;
        let record = self
            .tasks
            .get(task_id)
            .ok_or_else(|| format!("任务不存在：{task_id}"))?;
        let content = serde_json::to_string_pretty(record).map_err(|e| e.to_string())?;
        crate::atomic_write(
            &self.dir.join(format!("{task_id}.json")),
            content.as_bytes(),
        )
        .map_err(|e| format!("任务持久化失败：{e}"))
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct WaitingTransport {
        submitted: std::sync::Arc<AtomicBool>,
        status_started: std::sync::Arc<AtomicBool>,
        canceled: std::sync::Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl CloudTransport for WaitingTransport {
        fn kind(&self) -> &'static str {
            "waiting-test"
        }

        async fn submit(&self, _spec: &CloudTaskSpec) -> Result<String, String> {
            self.submitted.store(true, Ordering::SeqCst);
            Ok("remote-test-1".to_string())
        }

        async fn status(&self, _remote_id: &str) -> Result<RemoteStatus, String> {
            self.status_started.store(true, Ordering::SeqCst);
            std::future::pending().await
        }

        async fn fetch_result(&self, _remote_id: &str) -> Result<CloudTaskResult, String> {
            unreachable!("status never completes in this test")
        }

        async fn cancel(&self, remote_id: &str) -> Result<(), String> {
            assert_eq!(remote_id, "remote-test-1");
            self.canceled.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancellation_interrupts_pending_remote_status_and_persists_terminal_state() {
        let root = std::env::temp_dir().join(format!("owo-cloud-cancel-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let queue_dir = root.join("queue");
        std::fs::create_dir_all(&workspace).unwrap();

        let submitted = std::sync::Arc::new(AtomicBool::new(false));
        let status_started = std::sync::Arc::new(AtomicBool::new(false));
        let canceled = std::sync::Arc::new(AtomicBool::new(false));
        let transport = WaitingTransport {
            submitted: submitted.clone(),
            status_started: status_started.clone(),
            canceled: canceled.clone(),
        };
        let mut queue = CloudTaskQueue::new(queue_dir.clone(), Box::new(transport));
        let task_id = queue
            .submit(CloudTaskSpec {
                name: "cancel pending status".to_string(),
                workspace_dir: workspace,
                commands: vec!["echo ok".to_string()],
                env_passthrough: Vec::new(),
                timeout_secs: 30,
            })
            .unwrap();
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let sink = std::sync::Arc::new(CollectingSink::new());
        let run_sink = sink.clone();
        let runner = tokio::spawn(async move {
            let result = queue
                .run_next_with_cancel(run_sink.as_ref(), cancel_rx)
                .await;
            (queue, result)
        });

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !status_started.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("remote status should start");
        cancel_tx.send(true).unwrap();

        let (queue, run_result) = tokio::time::timeout(std::time::Duration::from_secs(2), runner)
            .await
            .expect("cancellation should interrupt pending status")
            .unwrap();
        assert_eq!(run_result.unwrap().as_deref(), Some(task_id.as_str()));
        assert!(submitted.load(Ordering::SeqCst));
        assert!(canceled.load(Ordering::SeqCst));
        assert_eq!(queue.record(&task_id).unwrap().state, TaskState::Canceled);
        assert_eq!(
            sink.all()
                .iter()
                .filter(|event| matches!(event, CloudProgress::Canceled { .. }))
                .count(),
            1
        );

        let persisted = std::fs::read_to_string(queue_dir.join(format!("{task_id}.json"))).unwrap();
        let restored: TaskRecord = serde_json::from_str(&persisted).unwrap();
        assert_eq!(restored.state, TaskState::Canceled);
        let _ = std::fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn cancellation_already_requested_before_runner_start_skips_remote_submission() {
        let root = std::env::temp_dir().join(format!(
            "owo-cloud-cancel-before-run-{}",
            uuid::Uuid::new_v4()
        ));
        let workspace = root.join("workspace");
        let queue_dir = root.join("queue");
        std::fs::create_dir_all(&workspace).unwrap();
        let submitted = std::sync::Arc::new(AtomicBool::new(false));
        let status_started = std::sync::Arc::new(AtomicBool::new(false));
        let canceled = std::sync::Arc::new(AtomicBool::new(false));
        let transport = WaitingTransport {
            submitted: submitted.clone(),
            status_started: status_started.clone(),
            canceled: canceled.clone(),
        };
        let mut queue = CloudTaskQueue::new(queue_dir.clone(), Box::new(transport));
        let task_id = queue
            .submit(CloudTaskSpec {
                name: "cancel before runner subscription".to_string(),
                workspace_dir: workspace,
                commands: vec!["echo ok".to_string()],
                env_passthrough: Vec::new(),
                timeout_secs: 30,
            })
            .unwrap();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(true);
        let sink = CollectingSink::new();

        assert_eq!(
            queue
                .run_next_with_cancel(&sink, cancel_rx)
                .await
                .unwrap()
                .as_deref(),
            Some(task_id.as_str())
        );
        assert!(!submitted.load(Ordering::SeqCst));
        assert!(!status_started.load(Ordering::SeqCst));
        assert!(!canceled.load(Ordering::SeqCst));
        assert_eq!(queue.record(&task_id).unwrap().state, TaskState::Canceled);
        assert_eq!(
            sink.all()
                .iter()
                .filter(|event| matches!(event, CloudProgress::Canceled { .. }))
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod queue_recovery_tests {
    use super::*;

    struct RetryThenSuccessTransport {
        attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CloudTransport for RetryThenSuccessTransport {
        fn kind(&self) -> &'static str {
            "retry-test"
        }

        async fn submit(&self, _spec: &CloudTaskSpec) -> Result<String, String> {
            Ok("remote-retry".to_string())
        }

        async fn status(&self, _remote_id: &str) -> Result<RemoteStatus, String> {
            let attempt = self
                .attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if attempt == 0 {
                Ok(RemoteStatus::Failed("temporary failure".to_string()))
            } else {
                Ok(RemoteStatus::Succeeded)
            }
        }

        async fn fetch_result(&self, remote_id: &str) -> Result<CloudTaskResult, String> {
            Ok(CloudTaskResult {
                task_id: remote_id.to_string(),
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                diff: Vec::new(),
                diff_truncated: false,
            })
        }

        async fn cancel(&self, _remote_id: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn spec(workspace: PathBuf, name: &str) -> CloudTaskSpec {
        CloudTaskSpec {
            name: name.to_string(),
            workspace_dir: workspace,
            commands: vec!["echo ok".to_string()],
            env_passthrough: Vec::new(),
            timeout_secs: 5,
        }
    }

    #[test]
    fn detached_submit_allocates_next_id_without_rewriting_active_task_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "owo-cloud-detached-submit-{}",
            uuid::Uuid::new_v4()
        ));
        let workspace = root.join("workspace");
        let queue_dir = root.join("queue");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut owner = CloudTaskQueue::new(
            queue_dir.clone(),
            Box::new(MockRemoteTransport::new(root.join("scratch"))),
        );
        let first = owner
            .submit(spec(workspace.clone(), "already running"))
            .unwrap();
        let first_path = queue_dir.join(format!("{first}.json"));
        let mut active = owner.record(&first).unwrap().clone();
        active.state = TaskState::Running;
        std::fs::write(&first_path, serde_json::to_vec(&active).unwrap()).unwrap();

        let second = CloudTaskQueue::submit_persisted(
            &queue_dir,
            spec(workspace, "accepted during execution"),
            &[],
        )
        .unwrap();
        assert_eq!(first, "cloud-0001");
        assert_eq!(second.task_id, "cloud-0002");
        let still_active: TaskRecord =
            serde_json::from_slice(&std::fs::read(first_path).unwrap()).unwrap();
        assert_eq!(still_active.state, TaskState::Running);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn specific_task_execution_does_not_consume_an_earlier_queued_task() {
        let root =
            std::env::temp_dir().join(format!("owo-cloud-specific-task-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let queue_dir = root.join("queue");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut queue = CloudTaskQueue::new(
            queue_dir,
            Box::new(MockRemoteTransport::new(root.join("scratch"))),
        );
        let first = queue.submit(spec(workspace.clone(), "first")).unwrap();
        let second = queue.submit(spec(workspace, "second")).unwrap();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(true);
        let sink = CollectingSink::new();

        assert_eq!(
            queue
                .run_task_with_cancel(&second, &sink, cancel_rx)
                .await
                .unwrap()
                .as_deref(),
            Some(second.as_str())
        );
        assert_eq!(queue.record(&first).unwrap().state, TaskState::Queued);
        assert_eq!(queue.record(&second).unwrap().state, TaskState::Canceled);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn dispatch_retry_runs_with_backoff_until_terminal_instead_of_stranding_queued() {
        let root =
            std::env::temp_dir().join(format!("owo-cloud-retry-terminal-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut queue = CloudTaskQueue::new(
            root.join("queue"),
            Box::new(RetryThenSuccessTransport {
                attempts: attempts.clone(),
            }),
        )
        .with_max_retries(1);
        queue.base_backoff_secs = 0;
        let task_id = queue.submit(spec(workspace, "retry to terminal")).unwrap();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let sink = CollectingSink::new();

        assert_eq!(
            queue
                .run_task_to_terminal_with_cancel(&task_id, &sink, cancel_rx)
                .await
                .unwrap()
                .as_deref(),
            Some(task_id.as_str())
        );
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(queue.record(&task_id).unwrap().state, TaskState::Succeeded);
        assert_eq!(queue.record(&task_id).unwrap().retry_count, 1);
        let events = sink.all();
        assert!(events
            .iter()
            .any(|event| matches!(event, CloudProgress::Retrying { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, CloudProgress::Succeeded { .. })));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_persists_running_as_queued_and_rejects_filename_identity_mismatch() {
        let root = std::env::temp_dir().join(format!("owo-cloud-recover-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let queue_dir = root.join("queue");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut original = CloudTaskQueue::new(
            queue_dir.clone(),
            Box::new(MockRemoteTransport::new(root.join("scratch"))),
        );
        let task_id = original.submit(spec(workspace, "recover")).unwrap();
        let path = queue_dir.join(format!("{task_id}.json"));
        let mut record = original.record(&task_id).unwrap().clone();
        record.state = TaskState::Running;
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();

        let mut recovered = CloudTaskQueue::new(
            queue_dir.clone(),
            Box::new(MockRemoteTransport::new(root.join("scratch"))),
        );
        assert_eq!(recovered.recover().unwrap(), 1);
        assert_eq!(recovered.record(&task_id).unwrap().state, TaskState::Queued);
        let persisted: TaskRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.state, TaskState::Queued);

        let mut mismatched = recovered.record(&task_id).unwrap().clone();
        mismatched.task_id = "cloud-escape".to_string();
        std::fs::write(&path, serde_json::to_vec(&mismatched).unwrap()).unwrap();
        let mut invalid = CloudTaskQueue::new(
            queue_dir,
            Box::new(MockRemoteTransport::new(root.join("scratch"))),
        );
        assert!(invalid.recover().is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
