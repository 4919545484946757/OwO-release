//! R8:<server> 服务端韧性 完成，待主控接线。
//!
//! - 全局并发 turn 上限：`ShutdownGate::try_acquire_turn`（信号量 + 活跃计数）；
//! - 优雅关闭：`request_shutdown`（停止接收）→ `await_drain`（完成在途）→ 主控 flush → 退出；
//! - 强杀恢复：`PidFile`（<data_root>/server.pid）+ `recover_force_kill`（陈旧 pid 清理）。
//!
//! 模块约定：不引用 `crate::`/`super::`，可被测试以 #[path] mod 独立编译。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 默认全局并发回合上限（环境变量 OWO_SERVER_MAX_CONCURRENT_TURNS 可调）。
pub const DEFAULT_MAX_CONCURRENT_TURNS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnBusy {
    AtCapacity,
    ShuttingDown,
}

impl std::fmt::Display for TurnBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnBusy::AtCapacity => {
                write!(formatter, "并发回合已达上限，请稍后重试")
            }
            TurnBusy::ShuttingDown => {
                write!(formatter, "服务正在优雅关闭，停止接收新回合")
            }
        }
    }
}

/// 优雅关闭结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GracefulOutcome {
    /// 请求关闭时的在途回合数。
    pub active_at_request: usize,
    /// 等待期限内完成的回合数（active_at_request - remaining）。
    pub drained: usize,
    /// 超时后仍在途的回合数（>0 表示需要强杀路径兜底）。
    pub remaining: usize,
}

pub struct ShutdownGate {
    semaphore: Arc<tokio::sync::Semaphore>,
    active: Arc<AtomicUsize>,
    shutting_down: Arc<AtomicBool>,
    request: Arc<tokio::sync::Notify>,
    max_concurrent: usize,
}

pub struct TurnPermit {
    _semaphore: tokio::sync::OwnedSemaphorePermit,
    active: Arc<AtomicUsize>,
}

impl std::fmt::Debug for TurnPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TurnPermit")
            .field("active", &self.active.load(Ordering::SeqCst))
            .finish()
    }
}

impl Drop for TurnPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ShutdownGate {
    /// 环境变量构造：OWO_SERVER_MAX_CONCURRENT_TURNS（默认 4，最小 1）。
    pub fn from_env() -> Self {
        let max = std::env::var("OWO_SERVER_MAX_CONCURRENT_TURNS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MAX_CONCURRENT_TURNS)
            .max(1);
        Self::new(max)
    }

    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_concurrent.max(1))),
            active: Arc::new(AtomicUsize::new(0)),
            shutting_down: Arc::new(AtomicBool::new(false)),
            request: Arc::new(tokio::sync::Notify::new()),
            max_concurrent: max_concurrent.max(1),
        }
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// 当前在途回合数。
    pub fn active_turns(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    pub fn shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }

    /// 获取回合许可；上限已满或正在关闭时拒绝（不等待、不排队）。
    pub fn try_acquire_turn(&self) -> Result<TurnPermit, TurnBusy> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(TurnBusy::ShuttingDown);
        }
        let permit = self
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| TurnBusy::AtCapacity)?;
        self.active.fetch_add(1, Ordering::SeqCst);
        Ok(TurnPermit {
            _semaphore: permit,
            active: Arc::clone(&self.active),
        })
    }

    /// 请求优雅关闭：停止接收新回合，返回当时在途数。
    pub fn request_shutdown(&self) -> usize {
        self.shutting_down.store(true, Ordering::Release);
        self.request.notify_waiters();
        self.active_turns()
    }

    /// 等待关闭请求（在途回合完成前的信号）。
    pub async fn wait_shutdown_request(&self) {
        self.request.notified().await;
    }

    /// 等待在途回合完成，超时返回剩余数（>0 由强杀恢复路径兜底）。
    pub async fn await_drain(&self, timeout: Duration) -> usize {
        let deadline = tokio::time::Instant::now() + timeout;
        while self.active_turns() > 0 {
            if tokio::time::Instant::now() >= deadline {
                return self.active_turns();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        0
    }

    /// 完整优雅关闭：停止接收 → 完成在途（限时）。
    pub async fn graceful_shutdown(&self, timeout: Duration) -> GracefulOutcome {
        let active_at_request = self.request_shutdown();
        let remaining = self.await_drain(timeout).await;
        GracefulOutcome {
            active_at_request,
            drained: active_at_request.saturating_sub(remaining),
            remaining,
        }
    }
}

/// 强杀恢复结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForceKillRecovery {
    /// 陈旧 pid 文件记录的进程号（可能为 None = 内容损坏）。
    pub stale_pid: Option<u32>,
    /// 是否清理了陈旧 pid 文件。
    pub cleaned: bool,
}

/// Windows 下的进程探测结果。
#[cfg(windows)]
enum ProcessProbe {
    /// 进程不存在（或 pid 非法）。
    Dead,
    /// 进程存在；`image` 为可执行文件全路径（权限不足时为 None）。
    Alive { image: Option<String> },
}

/// 进程存活探测（纯存活语义：pid 对应进程存在即 true；其他平台 kill(pid, 0)）。
///
/// Windows 的 `GetLastError` 是线程本地状态，调用 `OpenProcess` 前必须清零；
/// 否则陈旧错误码会把已退出进程的 pid 文件误判为活跃实例，阻塞服务恢复。
///
/// 注意：这是**纯存活**判定——它会被 pid 复用骗过（见 [`process_is_previous_server`]）。
/// 判断"上一代服务是否还在跑"请用后者。
pub fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        matches!(probe_process(pid), ProcessProbe::Alive { .. })
    }
    #[cfg(not(windows))]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        unsafe { kill(pid as i32, 0) == 0 }
    }
}

/// 该 pid 是否是"我们的"上一代服务进程（存活 **且** 映像名是 owo-agent*）。
///
/// **只看 pid 存活是不够的**（实测踩坑）：Windows 会复用 pid——上一代核心退出后，
/// 它的 pid 很快被系统里无关进程拿走，`OpenProcess` 照样成功，于是 `recover_force_kill`
/// 把"别人的进程"当成运行中的上一代，桌面壳的新核心在 40s 等待后按双开冲突拒绝启动，
/// **而且每次重启都如此（锁死不自愈）**。所以陈旧 pid 判定必须叠加映像名校验：
/// 只有真的是 owo-agent 才算"上一代还在跑"，否则按陈旧 pid 清理。
///
/// 权限不足拿不到映像名时保守返回 true（宁可等待，也不误开双实例）。
pub fn process_is_previous_server(pid: u32) -> bool {
    #[cfg(windows)]
    {
        match probe_process(pid) {
            ProcessProbe::Alive { image: Some(image) } => image_is_owo_agent(&image),
            ProcessProbe::Alive { image: None } => true,
            ProcessProbe::Dead => false,
        }
    }
    #[cfg(not(windows))]
    {
        process_alive(pid)
    }
}

/// 映像名（全路径或文件名均可）是否属于 owo-agent 服务。
///
/// 只认文件名前缀，不看所在目录——否则把项目放在 `D:\owo-agent-src\` 下的
/// 任何 exe 都会被误判成上一代服务。兼容 `owo-agent.exe` / `owo_agent.exe`。
pub fn image_is_owo_agent(image_path: &str) -> bool {
    let file = image_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(image_path)
        .to_ascii_lowercase();
    file.starts_with("owo-agent") || file.starts_with("owo_agent")
}

#[cfg(windows)]
fn probe_process(pid: u32) -> ProcessProbe {
    type WinHandle = *mut core::ffi::c_void;
    unsafe extern "system" {
        fn OpenProcess(
            dw_desired_access: u32,
            b_inherit_handle: i32,
            dw_process_id: u32,
        ) -> WinHandle;
        fn GetLastError() -> u32;
        fn SetLastError(dw_err_code: u32);
        fn CloseHandle(h_object: WinHandle) -> i32;
        fn QueryFullProcessImageNameW(
            h_process: WinHandle,
            dw_flags: u32,
            lp_exe_name: *mut u16,
            lpdw_size: *mut u32,
        ) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const ERROR_ACCESS_DENIED: u32 = 5;
    // `OpenProcess` 成功不保证会重置 last-error，先清零才能可信地解释失败路径。
    unsafe { SetLastError(0) };
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        // 权限不足 → 存活但身份未知（由调用方决定保守策略）；其余（不存在、
        // 无效参数、错误码未设置）都按"进程已死"处理，绝不阻塞恢复。
        return if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
            ProcessProbe::Alive { image: None }
        } else {
            ProcessProbe::Dead
        };
    }
    const NAME_MAX_LEN: u32 = 1024;
    let mut name_buf = [0u16; NAME_MAX_LEN as usize];
    let mut name_len = NAME_MAX_LEN;
    let image =
        if unsafe { QueryFullProcessImageNameW(handle, 0, name_buf.as_mut_ptr(), &mut name_len) }
            != 0
        {
            Some(String::from_utf16_lossy(&name_buf[..name_len as usize]))
        } else {
            None
        };
    unsafe { CloseHandle(handle) };
    ProcessProbe::Alive { image }
}

/// 服务 pid 文件（存活标记；正常退出 Drop 时清理，强杀后由 recover_force_kill 清理）。
pub struct PidFile {
    path: PathBuf,
    removed: bool,
}

impl PidFile {
    pub fn create(data_root: &std::path::Path) -> Result<Self, String> {
        let path = data_root.join("server.pid");
        std::fs::write(&path, std::process::id().to_string())
            .map_err(|e| format!("写入 pid 文件失败：{e}"))?;
        Ok(Self {
            path,
            removed: false,
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if !self.removed {
            let _ = std::fs::remove_file(&self.path);
            self.removed = true;
        }
    }
}

/// 强杀恢复：pid 文件存在但进程已死（或该 pid 已被无关进程复用）→ 清理（恢复干净状态）；
/// 确认是我们自己的服务进程仍存活 → 显式报错（不允许双实例写同一数据目录）。
///
/// 判定用 [`process_is_previous_server`] 而非 [`process_alive`]：后者会被 pid 复用
/// 骗过，导致陈旧锁把服务恢复永久卡死（见该函数注释）。
pub fn recover_force_kill(
    data_root: &std::path::Path,
) -> Result<Option<ForceKillRecovery>, String> {
    let path = data_root.join("server.pid");
    if !path.is_file() {
        return Ok(None);
    }
    let stale_pid = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| content.trim().parse::<u32>().ok());
    match stale_pid {
        Some(pid) if process_is_previous_server(pid) => {
            return Err(format!(
                "检测到运行中的服务（pid={pid}，pid 文件 {}）：请先停止该进程再启动",
                path.display()
            ));
        }
        _ => {}
    }
    std::fs::remove_file(&path)
        .map_err(|e| format!("清理陈旧 pid 文件失败（{}）：{e}", path.display()))?;
    Ok(Some(ForceKillRecovery {
        stale_pid,
        cleaned: true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_name_matches_only_our_server_binary() {
        for ours in [
            "owo-agent.exe",
            "OWO-AGENT.EXE",
            "owo_agent.exe",
            r"C:\app\OwO-Agent\owo-agent.exe",
            "/usr/local/bin/owo-agent",
        ] {
            assert!(image_is_owo_agent(ours), "{ours} 应被认作 owo-agent");
        }
        // 目录名里含 owo-agent 不算（只认文件名），别人的进程更不算。
        for foreign in [
            r"D:\owo-agent-src\target\debug\deps\shutdown_tests-abc123.exe",
            r"C:\Windows\explorer.exe",
            r"C:\Windows\System32\svchost.exe",
            r"C:\Users\me\AppData\Local\Programs\electron.exe",
        ] {
            assert!(
                !image_is_owo_agent(foreign),
                "{foreign} 不应被认作 owo-agent"
            );
        }
    }
}
