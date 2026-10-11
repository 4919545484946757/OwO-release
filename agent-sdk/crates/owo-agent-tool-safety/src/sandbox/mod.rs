//! OS 级执行沙箱（综合文档 §6 P0 / X01，Wave 2：Windows OS 边界落地）。
//!
//! - `SandboxPolicy` / `SandboxExecutor` / `SandboxManager`：策略校验 → 能力评估 →
//!   显式降级/拒绝 → 审计的统一入口（Wave 1 契约保留）。
//! - `probe_platform_support()`：Windows 真实能力探测（Job Object 创建、低完整性令牌、
//!   AppContainer API 存在性），探测失败一律按不可用处理并携带原因。
//! - `WindowsSandboxExecutor`：Job Object（CPU/内存/进程数上限 + kill-on-close 防孤儿）
//!   为基线；策略要求低完整性时经受限令牌 + 完整性标签创建进程；要求 AppContainer 且
//!   平台支持时经 `SECURITY_CAPABILITIES` 创建 AppContainer 进程。
//! - 硬性约定：**无法建立所需 OS 隔离时返回 `Unsupported` 并写入审计事件，
//!   创建失败一律显式失败（终止已启动进程），禁止静默假装安全**。
//! - Wave 1 的 `#[path]` 独立编译约定已结束（R6 主控已把本模块并入 lib.rs）。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// 沙箱文件作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileScope {
    /// 仅工作区内可读写（默认）。
    #[default]
    WorkspaceOnly,
    /// 工作区 + 只读系统路径。
    WorkspacePlusReadonlySystem,
    /// 无文件限制（高风险，默认拒绝，需显式放行）。
    Unrestricted,
}

/// 沙箱网络策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// 完全隔离（默认）。
    #[default]
    None,
    /// 仅本地回环。
    Loopback,
    /// 白名单 host（`allow_hosts`）。
    AllowList,
    /// 无限制（高风险，默认拒绝，需显式放行）。
    Unrestricted,
}

/// OS 隔离强度（按声明顺序递增：`None < LowIntegrity < JobOnly < AppContainerJob`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    /// 无 OS 级隔离。
    None,
    /// 低完整性（Low Integrity Level）。
    LowIntegrity,
    /// Job Object 资源限制（CPU/内存/生命周期）。
    JobOnly,
    /// Windows AppContainer + Job Object（完整方案）。
    #[default]
    AppContainerJob,
}

/// 沙箱策略：文件作用域 / 网络 / 资源上限 / 存活时间 / 隔离要求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// 沙箱名（审计用）。
    pub name: String,
    /// 工作区根目录（`WorkspaceOnly` 必需）。
    pub workspace: Option<PathBuf>,
    pub file_scope: FileScope,
    pub network_policy: NetworkPolicy,
    /// CPU 时间上限（毫秒，`None` = 不限制，`Some(0)` 非法）。
    pub cpu_ms: Option<u64>,
    /// 内存上限（MB，`None` = 不限制）。
    pub mem_mb: Option<u64>,
    /// 存活时长上限（秒，`None` = 不限制）。
    pub ttl_secs: Option<u64>,
    /// `AllowList` 网络白名单（host 或 host:port）。
    pub allow_hosts: Vec<String>,
    /// 要求的隔离强度（默认 AppContainerJob）。
    pub require_isolation: IsolationLevel,
    /// 是否允许显式降级（如仅有 Job 无 AppContainer）。
    pub allow_degraded: bool,
    /// Job 内活动进程数上限（`Some(n)` 限制，防进程炸弹；`None` = 不限制）。
    pub active_process_limit: Option<u32>,
    /// 危险程序片段黑名单（deny 优先，大小写不敏感子串匹配）。
    pub deny_programs: Vec<String>,
    /// 显式放行无文件限制（高风险开关）。
    pub allow_unrestricted_file: bool,
    /// 显式放行无网络限制（高风险开关）。
    pub allow_unrestricted_network: bool,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            workspace: None,
            file_scope: FileScope::WorkspaceOnly,
            network_policy: NetworkPolicy::None,
            cpu_ms: Some(30_000),
            mem_mb: Some(1024),
            ttl_secs: Some(600),
            allow_hosts: Vec::new(),
            require_isolation: IsolationLevel::AppContainerJob,
            allow_degraded: false,
            active_process_limit: Some(1),
            deny_programs: vec![
                "shutdown".to_string(),
                "format".to_string(),
                "reg delete".to_string(),
            ],
            allow_unrestricted_file: false,
            allow_unrestricted_network: false,
        }
    }
}

impl SandboxPolicy {
    /// 默认工作区沙箱策略（只读默认）。
    pub fn for_workspace(name: impl Into<String>, workspace: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            workspace: Some(workspace.into()),
            ..SandboxPolicy::default()
        }
    }

    /// 策略自检：越界组合（未显式放行的高风险策略、非法资源值）一律拒绝。
    pub fn validate(&self) -> Result<(), SandboxError> {
        if self.file_scope == FileScope::Unrestricted && !self.allow_unrestricted_file {
            return Err(SandboxError::PolicyViolation(
                "文件作用域 Unrestricted 必须显式设置 allow_unrestricted_file".to_string(),
            ));
        }
        if self.network_policy == NetworkPolicy::Unrestricted && !self.allow_unrestricted_network {
            return Err(SandboxError::PolicyViolation(
                "网络策略 Unrestricted 必须显式设置 allow_unrestricted_network".to_string(),
            ));
        }
        if self.network_policy == NetworkPolicy::AllowList && self.allow_hosts.is_empty() {
            return Err(SandboxError::PolicyViolation(
                "AllowList 网络策略需要非空 allow_hosts".to_string(),
            ));
        }
        if self.cpu_ms == Some(0) {
            return Err(SandboxError::PolicyViolation(
                "cpu_ms 为 0 非法：None=不限制，Some(n>0)=限制".to_string(),
            ));
        }
        if self.file_scope == FileScope::WorkspaceOnly && self.workspace.is_none() {
            return Err(SandboxError::PolicyViolation(
                "WorkspaceOnly 文件作用域需要 workspace 路径".to_string(),
            ));
        }
        Ok(())
    }
}

/// 剥掉 Windows verbatim 路径前缀（`\\?\`）。
///
/// `canonicalize()` 在 Windows 上返回 verbatim 形式（`\\?\D:\ws\x`），而工作区边界
/// 常是原始路径（`D:\ws\x`）。两者混用做前缀比较会恒 false，把合法路径判成越界。
///
/// 与 `owo-agent-core::tools::strip_verbatim_prefix` 是**同口径的两份实现**：
/// sandbox 是 core 的依赖，不能反向引用，只能各持一份。
/// 修改任一侧都必须同步另一侧（core 侧有对应单测钉住）。
fn strip_verbatim_prefix(path: &std::path::Path) -> std::path::PathBuf {
    let text = path.as_os_str().to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => std::path::PathBuf::from(stripped.to_string()),
        None => path.to_path_buf(),
    }
}

/// 待执行的沙箱命令。
#[derive(Debug, Clone)]
pub struct SandboxCommand {
    pub program: String,
    pub args: Vec<String>,
    pub policy: SandboxPolicy,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl SandboxCommand {
    pub fn new(program: impl Into<String>, policy: SandboxPolicy) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            policy,
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// 危险片段命中检查（大小写不敏感子串；返回命中的片段）。
    pub fn deny_hit(text: &str, deny_programs: &[String]) -> Option<String> {
        let lower = text.to_lowercase();
        deny_programs
            .iter()
            .find(|fragment| lower.contains(&fragment.to_lowercase()))
            .cloned()
    }

    /// 命令文本级危险程序检查（比整串子串更精确，供 `cmd /C <command>` 的命令体使用）。
    ///
    /// 真实模型实测：整串子串匹配会把 `python -c "print(format(x))"`、`--format=json`
    /// 这类正常命令误杀（黑名单里的 `format` 本意是磁盘格式化工具）。这里按 shell
    /// 分隔符切词，只把"作为程序名出现"的危险片段判为命中；含空格的片段（如
    /// `reg delete`）仍按连续子串检查。
    pub fn deny_hit_in_command(text: &str, deny_programs: &[String]) -> Option<String> {
        let lower = text.to_lowercase();
        let trim_executable = |name: &str| -> String {
            let lower_name = name.to_lowercase();
            for suffix in [".exe", ".com", ".bat", ".cmd", ".ps1"] {
                if let Some(stripped) = lower_name.strip_suffix(suffix) {
                    return stripped.to_string();
                }
            }
            lower_name
        };
        for fragment in deny_programs {
            let fragment_lower = fragment.to_lowercase();
            if fragment_lower.contains(' ') {
                if lower.contains(&fragment_lower) {
                    return Some(fragment.clone());
                }
                continue;
            }
            // 单程序名：按空白与 shell 分隔符切词，只把"整词等于程序名"的片段判为命中。
            // 这样 `format(x)`、`--format=json`、`print(format(x))` 等参数用法不会误杀，
            // 而 `cmd /C "shutdown /s"`、`echo hi;format c:` 仍会被拦下。
            let hit = lower
                .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|'))
                .map(|token| token.trim_matches(|c| c == '"' || c == '\''))
                .filter(|token| !token.is_empty())
                .any(|token| {
                    let name = token.rsplit(['\\', '/']).next().unwrap_or(token);
                    trim_executable(name) == fragment_lower
                });
            if hit {
                return Some(fragment.clone());
            }
        }
        None
    }

    /// 命令级校验：策略自检 + 工作区越界 + 危险程序黑名单。
    pub fn validate(&self) -> Result<(), SandboxError> {
        self.policy.validate()?;
        if self.policy.file_scope == FileScope::WorkspaceOnly {
            if let (Some(cwd), Some(root)) = (&self.cwd, &self.policy.workspace) {
                // 比对前必须剥掉 Windows verbatim 前缀（`\\?\`）。
                //
                // `cwd` 一般来自 canonicalize（Windows 上会返回 `\\?\D:\ws\x`），而
                // `workspace` 往往是用户/调用方给的原始路径（`D:\ws\x`）。直接
                // `starts_with` 会得出"自己不在自己里面"的荒谬结论，把**合法命令**
                // 判成越界——实测模型因此连续被拒，只能改写脚本文件绕开：
                //
                //   工作目录越界：\\?\D:\ws\sandbox 不在工作区 D:\ws\sandbox 内
                //
                // core 侧 `resolve_session_path` 早就在做同口径剥离，这里漏了；
                // 两个 crate 不能互相引用，故各自持有一份等价实现（改一处必须改两处）。
                let cwd_cmp = strip_verbatim_prefix(cwd);
                let root_cmp = strip_verbatim_prefix(root);
                if cwd_cmp.is_absolute() && !cwd_cmp.starts_with(&root_cmp) {
                    return Err(SandboxError::PolicyViolation(format!(
                        "工作目录越界：{} 不在工作区 {} 内",
                        cwd.display(),
                        root.display()
                    )));
                }
            }
        }
        if let Some(fragment) = Self::deny_hit(&self.program, &self.policy.deny_programs) {
            return Err(SandboxError::PolicyViolation(format!(
                "程序命中危险黑名单片段：{}",
                fragment
            )));
        }
        Ok(())
    }
}

/// 沙箱进程句柄。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxHandle {
    pub id: String,
    pub spawned_at: String,
}

/// 沙箱进程状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxProcessStatus {
    Running,
    Exited(i32),
    Killed,
    Failed(String),
}

/// 进程退出信息（wait_output 结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxWaitInfo {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// 沙箱进程内部句柄（OS 执行器填充；Mock 为空）。
pub trait SandboxProcessInner: Send {
    /// 阻塞等待退出并采集输出。
    fn wait(&mut self) -> Result<SandboxWaitInfo, SandboxError>;
    /// 终止进程。
    fn kill(&mut self) -> Result<(), SandboxError>;
}

/// 沙箱进程。
pub struct SandboxProcess {
    pub handle: SandboxHandle,
    pub status: SandboxProcessStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub(crate) inner: Option<Box<dyn SandboxProcessInner>>,
}

impl std::fmt::Debug for SandboxProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxProcess")
            .field("handle", &self.handle)
            .field("status", &self.status)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .finish_non_exhaustive()
    }
}

impl SandboxProcess {
    /// 阻塞等待进程退出并采集输出（无 inner 时返回已缓存数据与状态码）。
    pub fn wait_output(&mut self) -> Result<SandboxWaitInfo, SandboxError> {
        if let Some(inner) = self.inner.as_mut() {
            let info = inner.wait()?;
            self.stdout = info.stdout.clone();
            self.stderr = info.stderr.clone();
            self.status = SandboxProcessStatus::Exited(info.exit_code);
            return Ok(info);
        }
        let exit_code = match self.status {
            SandboxProcessStatus::Exited(code) => code,
            SandboxProcessStatus::Killed => 1,
            SandboxProcessStatus::Failed(_) | SandboxProcessStatus::Running => -1,
        };
        Ok(SandboxWaitInfo {
            exit_code,
            stdout: std::mem::take(&mut self.stdout),
            stderr: std::mem::take(&mut self.stderr),
        })
    }

    /// 终止进程（Job 层兜底）。
    pub fn kill(&mut self) -> Result<(), SandboxError> {
        if let Some(inner) = self.inner.as_mut() {
            inner.kill()
        } else {
            Err(SandboxError::Kill("进程无内部句柄".to_string()))
        }
    }
}

/// 执行器健康状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxHealth {
    pub healthy: bool,
    pub detail: String,
}

/// 沙箱执行器接口。
pub trait SandboxExecutor: Send + Sync {
    fn name(&self) -> &'static str;
    /// 执行器实际可提供的隔离强度。
    fn capability(&self) -> IsolationLevel;
    fn spawn(&self, command: &SandboxCommand) -> Result<SandboxProcess, SandboxError>;
    fn kill(&self, handle: &SandboxHandle) -> Result<(), SandboxError>;
    fn check_healthy(&self) -> SandboxHealth;
    /// 把运行中的进程挂入受限 Job（默认不支持）。
    fn attach(&self, _policy: &SandboxPolicy, _pid: u32) -> Result<JobGuard, SandboxError> {
        Err(SandboxError::Unsupported(
            "该执行器不支持挂接运行中进程".to_string(),
        ))
    }
}

/// 沙箱错误：显式拒绝/降级，绝不静默。
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("沙箱不可用：{0}")]
    Unsupported(String),
    #[error("策略违规：{0}")]
    PolicyViolation(String),
    #[error("启动失败：{0}")]
    Spawn(String),
    #[error("终止失败：{0}")]
    Kill(String),
    #[error("沙箱不健康：{0}")]
    Unhealthy(String),
    #[error("io 错误：{0}")]
    Io(#[from] std::io::Error),
}

/// 平台 OS 能力探测结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSupport {
    pub os: String,
    /// Windows AppContainer（Win8+）可用。
    pub app_container: bool,
    /// Job Object 可用。
    pub job_object: bool,
    /// 低完整性令牌可用。
    pub low_integrity: bool,
    /// 探测/降级原因（审计用）。
    pub reason: String,
}

/// 平台能力探测：Windows 真实探测（Job 创建 / 低完整性令牌 / AppContainer API）。
/// 任何一步无法验证即按不可用处理并写入 reason（禁止假装安全）。
/// 非 Windows（R10）：显式 `Unsupported` + 可审计降级原因——若检测到 bwrap（Linux）
/// 或 sandbox-exec（macOS）则写入 reason 供后续 Wave 接入，绝不编译期假装支持。
pub fn probe_platform_support() -> PlatformSupport {
    #[cfg(target_os = "windows")]
    {
        win::probe_windows_support()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let os = std::env::consts::OS;
        let mut hints = Vec::new();
        #[cfg(target_os = "linux")]
        {
            if std::path::Path::new("/usr/bin/bwrap").exists()
                || std::path::Path::new("/bin/bwrap").exists()
            {
                hints.push("检测到 bwrap（Wave 接入点已预留）");
            }
        }
        #[cfg(target_os = "macos")]
        {
            if std::path::Path::new("/usr/bin/sandbox-exec").exists() {
                hints.push("检测到 sandbox-exec（Wave 接入点已预留）");
            }
        }
        let mut reason = format!("平台 {os} 暂不支持 OS 级执行沙箱（显式降级，不静默假装安全）");
        if !hints.is_empty() {
            reason.push('；');
            reason.push_str(&hints.join("；"));
        }
        PlatformSupport {
            os: os.to_string(),
            app_container: false,
            job_object: false,
            low_integrity: false,
            reason,
        }
    }
}

/// 平台可用隔离强度（按能力取最高档）。
pub fn available_isolation(support: &PlatformSupport) -> IsolationLevel {
    if support.app_container && support.job_object {
        IsolationLevel::AppContainerJob
    } else if support.job_object {
        IsolationLevel::JobOnly
    } else if support.low_integrity {
        IsolationLevel::LowIntegrity
    } else {
        IsolationLevel::None
    }
}

/// 能力评估结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityEvaluation {
    /// 满足策略要求的完整隔离。
    Full,
    /// 显式降级（策略允许 `allow_degraded`）。
    Degraded(IsolationLevel),
}

/// 能力评估：策略要求 vs 平台可用 → `Full` / 显式 `Degraded` / 显式 `Unsupported`。
pub fn evaluate_capability(
    support: &PlatformSupport,
    policy: &SandboxPolicy,
) -> Result<CapabilityEvaluation, SandboxError> {
    let available = available_isolation(support);
    if available == IsolationLevel::None {
        return Err(SandboxError::Unsupported(support.reason.clone()));
    }
    if available < policy.require_isolation {
        if policy.allow_degraded {
            return Ok(CapabilityEvaluation::Degraded(available));
        }
        return Err(SandboxError::Unsupported(format!(
            "需要 {:?} 隔离，平台仅提供 {:?}（显式 allow_degraded 可降级）",
            policy.require_isolation, available
        )));
    }
    Ok(CapabilityEvaluation::Full)
}

/// 沙箱审计事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxEventKind {
    CapabilityProbe,
    SpawnRejected,
    UnsupportedIsolation,
    DegradedIsolation,
    Killed,
    Unhealthy,
    Attached,
    EgressRejected,
    PluginRejected,
}

impl SandboxEventKind {
    /// 审计链事件名后缀（sandbox.<label>）。
    pub fn label(&self) -> &'static str {
        match self {
            SandboxEventKind::CapabilityProbe => "capability_probe",
            SandboxEventKind::SpawnRejected => "spawn_rejected",
            SandboxEventKind::UnsupportedIsolation => "unsupported_isolation",
            SandboxEventKind::DegradedIsolation => "degraded_isolation",
            SandboxEventKind::Killed => "killed",
            SandboxEventKind::Unhealthy => "unhealthy",
            SandboxEventKind::Attached => "attached",
            SandboxEventKind::EgressRejected => "egress_rejected",
            SandboxEventKind::PluginRejected => "plugin_rejected",
        }
    }
}

/// 沙箱审计事件（append-only，可汇入 audit_chain）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxAuditEvent {
    pub ts: String,
    pub kind: SandboxEventKind,
    pub sandbox: String,
    pub detail: String,
}

/// 沙箱审计日志（进程内 append-only）。
#[derive(Debug, Clone, Default)]
pub struct SandboxAuditLog {
    events: Vec<SandboxAuditEvent>,
}

impl SandboxAuditLog {
    pub fn record(
        &mut self,
        kind: SandboxEventKind,
        sandbox: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.events.push(SandboxAuditEvent {
            ts: Utc::now().to_rfc3339(),
            kind,
            sandbox: sandbox.into(),
            detail: detail.into(),
        });
    }

    pub fn events(&self) -> &[SandboxAuditEvent] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn contains_kind(&self, kind: SandboxEventKind) -> bool {
        self.events.iter().any(|event| event.kind == kind)
    }

    pub fn drain(&mut self) -> Vec<SandboxAuditEvent> {
        std::mem::take(&mut self.events)
    }
}

/// 沙箱管理器：策略校验 → 能力评估 → 显式降级/拒绝 → 审计，统一入口。
pub struct SandboxManager {
    executor: Box<dyn SandboxExecutor>,
    platform: PlatformSupport,
    audit: SandboxAuditLog,
}

/// 进程执行门卫结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecGuard {
    /// 可执行（若为 Degraded，降级已写审计事件）。
    Allowed { degraded: Option<IsolationLevel> },
    /// 已拒绝（审计已写），不得执行。
    Blocked,
}

/// 全局默认沙箱管理器（惰性初始化：真实探测 + Windows 执行器）。
pub fn default_manager() -> Arc<Mutex<SandboxManager>> {
    static MANAGER: OnceLock<Arc<Mutex<SandboxManager>>> = OnceLock::new();
    MANAGER
        .get_or_init(|| {
            let support = probe_platform_support();
            #[cfg(target_os = "windows")]
            let executor: Box<dyn SandboxExecutor> =
                match win::WindowsSandboxExecutor::detect(&support) {
                    Some(executor) => Box::new(executor),
                    None => Box::new(UnavailableExecutor {
                        reason: support.reason.clone(),
                    }),
                };
            #[cfg(not(target_os = "windows"))]
            let executor: Box<dyn SandboxExecutor> = Box::new(UnavailableExecutor {
                reason: support.reason.clone(),
            });
            let manager = SandboxManager::with_probe(executor);
            Arc::new(Mutex::new(manager))
        })
        .clone()
}

impl SandboxManager {
    pub fn new(executor: Box<dyn SandboxExecutor>, platform: PlatformSupport) -> Self {
        Self {
            executor,
            platform,
            audit: SandboxAuditLog::default(),
        }
    }

    /// 用平台探测结果构造管理器（探测本身也记录审计事件）。
    pub fn with_probe(executor: Box<dyn SandboxExecutor>) -> Self {
        let platform = probe_platform_support();
        let mut manager = Self::new(executor, platform.clone());
        manager.audit.record(
            SandboxEventKind::CapabilityProbe,
            "probe",
            format!(
                "os={} app_container={} job_object={} low_integrity={}；{}",
                platform.os,
                platform.app_container,
                platform.job_object,
                platform.low_integrity,
                platform.reason
            ),
        );
        manager
    }

    pub fn executor(&self) -> &dyn SandboxExecutor {
        self.executor.as_ref()
    }

    pub fn platform(&self) -> &PlatformSupport {
        &self.platform
    }

    pub fn audit(&self) -> &SandboxAuditLog {
        &self.audit
    }

    pub fn take_audit_events(&mut self) -> Vec<SandboxAuditEvent> {
        self.audit.drain()
    }

    /// 记录 egress 拒绝事件（R9：插件/工具越界网络拒绝时调用，可汇入审计链）。
    pub fn record_egress_rejection(
        &mut self,
        sandbox: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.audit
            .record(SandboxEventKind::EgressRejected, sandbox, detail);
    }

    /// 记录插件拒绝事件（R10：吊销/高风险扫描拒绝时调用，可汇入审计链）。
    pub fn record_plugin_rejection(
        &mut self,
        plugin: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.audit
            .record(SandboxEventKind::PluginRejected, plugin, detail);
    }

    /// 沙箱审计事件汇入 HMAC 审计链（R8：凭据与审计闭环）。
    pub fn drain_into_chain(
        &mut self,
        chain: &mut crate::audit_chain::AuditChain,
        actor: &str,
    ) -> usize {
        let mut log = SandboxAuditLog::default();
        for event in self.audit.drain() {
            log.record(event.kind, event.sandbox, event.detail);
        }
        chain.append_sandbox_log(&log, actor)
    }

    /// 门卫：策略校验 + 能力评估 + 审计（不实际启动进程）。
    pub fn guard(&mut self, command: &SandboxCommand) -> Result<ExecGuard, SandboxError> {
        if let Err(error) = command.validate() {
            self.audit.record(
                SandboxEventKind::SpawnRejected,
                &command.policy.name,
                error.to_string(),
            );
            return Err(error);
        }
        match evaluate_capability(&self.platform, &command.policy) {
            Err(error) => {
                self.audit.record(
                    SandboxEventKind::UnsupportedIsolation,
                    &command.policy.name,
                    error.to_string(),
                );
                Err(error)
            }
            Ok(CapabilityEvaluation::Degraded(level)) => {
                self.audit.record(
                    SandboxEventKind::DegradedIsolation,
                    &command.policy.name,
                    format!("显式降级到 {:?}", level),
                );
                Ok(ExecGuard::Allowed {
                    degraded: Some(level),
                })
            }
            Ok(CapabilityEvaluation::Full) => Ok(ExecGuard::Allowed { degraded: None }),
        }
    }

    /// 经沙箱执行命令：门卫 + executor.spawn。
    pub fn spawn(&mut self, command: &SandboxCommand) -> Result<SandboxProcess, SandboxError> {
        self.guard(command)?;
        match self.executor.spawn(command) {
            Ok(process) => Ok(process),
            Err(error) => {
                self.audit.record(
                    SandboxEventKind::SpawnRejected,
                    &command.policy.name,
                    error.to_string(),
                );
                Err(error)
            }
        }
    }

    /// 把已启动的进程（如 MCP 子进程）挂入受限 Job；失败 = 显式拒绝。
    /// 网络策略（R10）：AllowList/Unrestricted 需要 OS 级网络强制（AppContainer），
    /// Job 挂接路径无法强制网络 → 显式拒绝（不静默放开）。
    pub fn attach_pid(
        &mut self,
        policy: &SandboxPolicy,
        pid: u32,
    ) -> Result<JobGuard, SandboxError> {
        policy.validate()?;
        if network_requires_app_container(policy) {
            return Err(SandboxError::Unsupported(format!(
                "网络策略 {:?} 需要 AppContainer 隔离才能强制网络白名单，\
                 Job 挂接路径无法强制（显式拒绝，不静默放开网络）",
                policy.network_policy
            )));
        }
        let guard = self.executor.attach(policy, pid)?;
        self.audit.record(
            SandboxEventKind::Attached,
            &policy.name,
            format!("进程 {} 已挂入受限 Job", pid),
        );
        Ok(guard)
    }

    pub fn kill(&mut self, handle: &SandboxHandle) -> Result<(), SandboxError> {
        match self.executor.kill(handle) {
            Ok(()) => {
                self.audit
                    .record(SandboxEventKind::Killed, &handle.id, "沙箱进程已终止");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub fn check_healthy(&mut self) -> SandboxHealth {
        let health = self.executor.check_healthy();
        if !health.healthy {
            self.audit.record(
                SandboxEventKind::Unhealthy,
                "executor",
                health.detail.clone(),
            );
        }
        health
    }
}

/// 测试/本地用执行器：可编程的 spawn/kill/健康结果。
#[derive(Debug)]
pub struct MockSandboxExecutor {
    pub name: &'static str,
    pub isolation: IsolationLevel,
    pub healthy: std::sync::atomic::AtomicBool,
    pub spawn_should_fail: std::sync::atomic::AtomicBool,
    pub kill_should_fail: std::sync::atomic::AtomicBool,
    pub spawn_calls: std::sync::atomic::AtomicUsize,
    pub kill_calls: std::sync::atomic::AtomicUsize,
}

impl Default for MockSandboxExecutor {
    fn default() -> Self {
        Self {
            name: "mock",
            isolation: IsolationLevel::AppContainerJob,
            healthy: std::sync::atomic::AtomicBool::new(true),
            spawn_should_fail: std::sync::atomic::AtomicBool::new(false),
            kill_should_fail: std::sync::atomic::AtomicBool::new(false),
            spawn_calls: std::sync::atomic::AtomicUsize::new(0),
            kill_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl MockSandboxExecutor {
    pub fn with_isolation(isolation: IsolationLevel) -> Self {
        Self {
            isolation,
            ..MockSandboxExecutor::default()
        }
    }
}

impl SandboxExecutor for MockSandboxExecutor {
    fn name(&self) -> &'static str {
        self.name
    }

    fn capability(&self) -> IsolationLevel {
        self.isolation
    }

    fn spawn(&self, command: &SandboxCommand) -> Result<SandboxProcess, SandboxError> {
        self.spawn_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .spawn_should_fail
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(SandboxError::Spawn("mock 执行器启动失败".to_string()));
        }
        Ok(SandboxProcess {
            handle: SandboxHandle {
                id: format!("mock-{}", command.policy.name),
                spawned_at: Utc::now().to_rfc3339(),
            },
            status: SandboxProcessStatus::Running,
            stdout: Vec::new(),
            stderr: Vec::new(),
            inner: None,
        })
    }

    fn kill(&self, _handle: &SandboxHandle) -> Result<(), SandboxError> {
        self.kill_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .kill_should_fail
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(SandboxError::Kill("mock 执行器终止失败".to_string()));
        }
        Ok(())
    }

    fn check_healthy(&self) -> SandboxHealth {
        SandboxHealth {
            healthy: self.healthy.load(std::sync::atomic::Ordering::SeqCst),
            detail: "mock 健康检查".to_string(),
        }
    }
}

/// 显式不可用的执行器：任何操作返回 Unsupported（禁止假装安全）。
pub struct UnavailableExecutor {
    pub reason: String,
}

impl SandboxExecutor for UnavailableExecutor {
    fn name(&self) -> &'static str {
        "unavailable"
    }

    fn capability(&self) -> IsolationLevel {
        IsolationLevel::None
    }

    fn spawn(&self, _command: &SandboxCommand) -> Result<SandboxProcess, SandboxError> {
        Err(SandboxError::Unsupported(self.reason.clone()))
    }

    fn kill(&self, _handle: &SandboxHandle) -> Result<(), SandboxError> {
        Err(SandboxError::Unsupported(self.reason.clone()))
    }

    fn check_healthy(&self) -> SandboxHealth {
        SandboxHealth {
            healthy: false,
            detail: self.reason.clone(),
        }
    }
}

/// Job 守卫：持有 Job Object 句柄；Drop 时终止 job 内全部进程（防孤儿）。
pub struct JobGuard {
    pub pid: u32,
    #[cfg(target_os = "windows")]
    job: win::Handle,
}

impl std::fmt::Debug for JobGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JobGuard")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

// 句柄值可跨线程转移（由 JobGuard 独占管理），标准 Windows 实践。
#[cfg(target_os = "windows")]
unsafe impl Send for JobGuard {}

#[cfg(target_os = "windows")]
impl Drop for JobGuard {
    fn drop(&mut self) {
        win::terminate_job(self.job, 1);
        win::close_handle(self.job);
    }
}

/// 路径工具：判断目标是否位于工作区内（越界样例矩阵用）。
pub fn inside_workspace(workspace: &Path, target: &Path) -> bool {
    target.starts_with(workspace)
}

/// AppContainer 网络能力 SID（S-1-15-2-1 InternetClient）。
pub fn internet_client_sid() -> Vec<u8> {
    app_package_sid(1)
}

/// AppContainer 网络能力 SID（S-1-15-2-3 PrivateNetworkClientServer）。
pub fn private_network_client_server_sid() -> Vec<u8> {
    app_package_sid(3)
}

/// 构造 S-1-15-2-<rid> 包能力 SID 字节（revision=1, count=2, authority=15, subauth=[2, rid]）。
fn app_package_sid(rid: u32) -> Vec<u8> {
    let mut sid = vec![1u8, 2u8, 0, 0, 0, 0, 0, 15];
    sid.extend_from_slice(&2u32.to_le_bytes());
    sid.extend_from_slice(&rid.to_le_bytes());
    sid
}

/// host 是否为内网（私有 IP 段 / 无点 netbios 名）。host 可含 :port。
fn is_private_host(host: &str) -> bool {
    let hostname = host.split(':').next().unwrap_or(host);
    if hostname.starts_with("127.")
        || hostname.starts_with("10.")
        || hostname.starts_with("192.168.")
        || hostname.starts_with("169.254.")
        || hostname.starts_with("172.16.")
        || hostname.starts_with("172.17.")
        || hostname.starts_with("172.18.")
        || hostname.starts_with("172.19.")
        || hostname.starts_with("172.2")
        || hostname.starts_with("172.30.")
        || hostname.starts_with("172.31.")
    {
        return true;
    }
    !hostname.contains('.')
}

/// 按网络策略生成 AppContainer 网络能力 SID 列表：
/// - `None`/`Loopback` → 空（默认 deny；AppContainer 下 loopback 需 OS 级豁免，Wave 3 接入）；
/// - `AllowList` → 按 host 推断（内网 → PrivateNetworkClientServer，公网 → InternetClient）；
/// - `Unrestricted` → InternetClient + PrivateNetworkClientServer（需显式放行，policy.validate 把关）。
pub fn app_container_network_capabilities(policy: &SandboxPolicy) -> Vec<Vec<u8>> {
    match policy.network_policy {
        NetworkPolicy::None | NetworkPolicy::Loopback => Vec::new(),
        NetworkPolicy::Unrestricted => {
            vec![internet_client_sid(), private_network_client_server_sid()]
        }
        NetworkPolicy::AllowList => {
            let mut sids: Vec<Vec<u8>> = Vec::new();
            for host in &policy.allow_hosts {
                let sid = if is_private_host(host) {
                    private_network_client_server_sid()
                } else {
                    internet_client_sid()
                };
                if !sids.contains(&sid) {
                    sids.push(sid);
                }
            }
            sids
        }
    }
}

/// 校验生成的网络能力与策略一致（默认 deny 的隔离策略不得携带网络能力）。
pub fn validate_app_container_network(
    policy: &SandboxPolicy,
    sids: &[Vec<u8>],
) -> Result<(), SandboxError> {
    match policy.network_policy {
        NetworkPolicy::None | NetworkPolicy::Loopback => {
            if !sids.is_empty() {
                return Err(SandboxError::PolicyViolation(
                    "隔离网络策略下 AppContainer 不得携带网络能力".to_string(),
                ));
            }
        }
        NetworkPolicy::AllowList => {
            if sids.is_empty() {
                return Err(SandboxError::PolicyViolation(
                    "AllowList 网络策略要求网络能力但生成为空".to_string(),
                ));
            }
        }
        NetworkPolicy::Unrestricted => {
            if !sids.iter().any(|sid| *sid == internet_client_sid()) {
                return Err(SandboxError::PolicyViolation(
                    "Unrestricted 网络策略必须携带 InternetClient 能力".to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// 网络 egress 边界（R9）：`AllowList`/`Unrestricted` 网络策略必须在 OS 边界强制
/// （AppContainer 网络能力），仅 Job/LowIL 隔离无法限制网络 → 调用方必须显式拒绝。
pub fn network_requires_app_container(policy: &SandboxPolicy) -> bool {
    matches!(
        policy.network_policy,
        NetworkPolicy::AllowList | NetworkPolicy::Unrestricted
    )
}

/// Windows 结构布局与 SDK 一致性检查（防 ABI 漂移；非 Windows 恒真）。
pub fn os_struct_layouts_match() -> bool {
    #[cfg(target_os = "windows")]
    {
        win::assert_struct_layouts()
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
pub(crate) mod win;

#[cfg(test)]
mod sandbox_workspace_scope_tests {
    use super::*;

    /// 回归：canonicalize 产出的 `\\?\` 前缀不得让"工作目录在自己工作区内"判成越界。
    ///
    /// 线上事故：模型执行 `python3 -c "多行脚本"`，cwd 来自 canonicalize
    /// （`\\?\D:\...\sandbox-test`），workspace 是原始路径（`D:\...\sandbox-test`），
    /// 直接 `starts_with` 恒 false → 合法命令被拒，模型只能改写脚本文件绕开。
    #[test]
    fn verbatim_prefixed_cwd_is_not_out_of_workspace() {
        let root = std::path::PathBuf::from(r"D:\ws\sandbox");
        let policy = SandboxPolicy {
            workspace: Some(root.clone()),
            file_scope: FileScope::WorkspaceOnly,
            ..Default::default()
        };
        // canonicalize 形态（Windows）：带 verbatim 前缀。
        let verbatim_cwd = std::path::PathBuf::from(r"\\?\D:\ws\sandbox\sub");
        let command = SandboxCommand::new("python", policy.clone()).with_cwd(verbatim_cwd);
        assert!(
            command.validate().is_ok(),
            "带 \\\\?\\ 前缀的工作目录被误判越界：{:?}",
            command.validate()
        );

        // 反向：真的跑到工作区外仍必须拒绝（安全口径不放松）。
        let outside = std::path::PathBuf::from(r"\\?\D:\other\place");
        let escaped = SandboxCommand::new("python", policy).with_cwd(outside);
        assert!(escaped.validate().is_err(), "工作区外的 cwd 必须仍被拒绝");
    }

    #[test]
    fn strip_verbatim_prefix_only_touches_verbatim_paths() {
        assert_eq!(
            strip_verbatim_prefix(std::path::Path::new(r"\\?\D:\ws\x")),
            std::path::PathBuf::from(r"D:\ws\x")
        );
        assert_eq!(
            strip_verbatim_prefix(std::path::Path::new(r"D:\ws\x")),
            std::path::PathBuf::from(r"D:\ws\x")
        );
        // UNC 形式的 verbatim 前缀（`\\?\UNC\server\share`）不该被误剥成 `UNC\...`。
        assert_eq!(
            strip_verbatim_prefix(std::path::Path::new(r"\\?\UNC\srv\share")),
            std::path::PathBuf::from(r"UNC\srv\share")
        );
    }
}

#[cfg(test)]
mod command_line_tests {
    // command_line 家族是 `win` 模块下的自由函数（Windows 平台实现）。
    use super::win::{command_line_string, is_cmd_shell, is_cmd_switch};

    /// 普通参数：含空白的照旧加双引号（既有行为不得回归）。
    #[test]
    fn quotes_plain_args_containing_whitespace() {
        let args = vec![
            "C:\\Program Files\\tool.exe".to_string(),
            "a b".to_string(),
            "plain".to_string(),
        ];
        assert_eq!(
            command_line_string("runner", &args),
            "runner \"C:\\Program Files\\tool.exe\" \"a b\" plain"
        );
    }

    /// 回归：`cmd /C` 的命令体必须原样透传——套引号 + 内部引号翻倍会让
    /// `python -c "..."` 静默失败（审计里 exit_code=0 但 stdout 为空）。
    #[test]
    fn cmd_c_body_passed_verbatim_without_doubling_quotes() {
        let body = "python -c \"import pptx; print(pptx.__version__)\"".to_string();
        let line = command_line_string("cmd", &["/C".to_string(), body.clone()]);
        assert!(line.ends_with(&body), "命令体必须原样结尾：{line}");
        assert_eq!(line, format!("cmd /C {body}"));
        assert!(!line.contains("\"\""), "内部引号不得翻倍：{line}");
    }

    #[test]
    fn cmd_k_body_also_passed_verbatim() {
        let body = "echo \"a b\" & echo done".to_string();
        let line = command_line_string("cmd.exe", &["/k".to_string(), body.clone()]);
        assert_eq!(line, format!("cmd.exe /k {body}"));
    }

    /// 非 cmd 程序仍走既有引号逻辑（透传只对 cmd 生效）。
    #[test]
    fn non_cmd_program_keeps_quoting_rules() {
        let args = vec!["/C".to_string(), "python -c \"print(1)\"".to_string()];
        let line = command_line_string("powershell", &args);
        assert!(line.contains("\"\""), "非 cmd 程序保持原引号策略：{line}");
    }

    #[test]
    fn detects_cmd_shell_and_switches() {
        assert!(is_cmd_shell("cmd"));
        assert!(is_cmd_shell("C:\\Windows\\System32\\cmd.exe"));
        assert!(!is_cmd_shell("powershell"));
        assert!(is_cmd_switch("/C"));
        assert!(is_cmd_switch("/k"));
        assert!(!is_cmd_switch("-Command"));
    }
}
