//! Windows 命名管道服务端（OwO 输入法 Agent IPC 第三方 Agent 角色）。
//!
//! 设计对齐官方参考实现 `OwO-release/apps/agent_mock/main.cpp`：
//! - 默认端点 `\\.\pipe\OwO.Agent.External.v1`（本机管道，拒绝远程客户端）；
//! - 每连接一问一答：读一帧 → 处理 → 写一帧 → 断开并重建实例；
//! - ACL 仅允许**当前 Windows 用户**、SYSTEM 与管理员（SDDL 与 mock 完全一致）；
//! - 单次操作超时（默认 10s，钳位 500–30000ms，对齐协议文档）。

use std::sync::Arc;
use std::time::Duration;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::frame::{self, FrameError};

/// 默认管道端点。
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\OwO.Agent.External.v1";
/// 默认单次操作超时（毫秒）。
pub const DEFAULT_OP_TIMEOUT_MS: u64 = 10_000;
/// 超时钳位下限。
pub const MIN_OP_TIMEOUT_MS: u64 = 500;
/// 超时钳位上限。
pub const MAX_OP_TIMEOUT_MS: u64 = 30_000;

/// 一帧载荷的处理器（返回 `None` 表示不回包，直接断开）。
#[async_trait::async_trait]
pub trait FrameHandler: Send + Sync + 'static {
    async fn handle(&self, payload: Vec<u8>) -> Option<Vec<u8>>;
}

/// 管道服务错误。
#[derive(Debug, thiserror::Error)]
pub enum PipeError {
    #[error("管道 IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("帧编解码错误：{0}")]
    Frame(#[from] FrameError),
    #[error("管道安全描述符创建失败：{0}")]
    Security(String),
    #[error("管道名非法：{0}")]
    InvalidName(String),
}

/// 校验管道名：`\\.\pipe\` 前缀 + ASCII 字母数字点连字符下划线（1–128 字符）。
///
/// 规则与 OwO 连接器配置页一致：拒绝 UNC、远程管道、相对路径与目录穿越字符。
pub fn validate_pipe_name(name: &str) -> Result<(), PipeError> {
    const PREFIX: &str = r"\\.\pipe\";
    let Some(rest) = name.strip_prefix(PREFIX) else {
        return Err(PipeError::InvalidName(
            r"必须是 \\.\pipe\ 前缀的本机管道".to_string(),
        ));
    };
    if rest.is_empty() || rest.chars().count() > 128 {
        return Err(PipeError::InvalidName(
            "管道名长度须在 1–128 字符".to_string(),
        ));
    }
    if !rest
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(PipeError::InvalidName(
            "管道名仅允许 ASCII 字母、数字、点、连字符或下划线".to_string(),
        ));
    }
    Ok(())
}

/// 钳位超时到协议允许区间。
pub fn clamp_timeout_ms(ms: u64) -> u64 {
    ms.clamp(MIN_OP_TIMEOUT_MS, MAX_OP_TIMEOUT_MS)
}

/// 运行管道服务循环，直到 `shutdown` 变为 `true`。
///
/// 每接受一个连接就立即创建下一个实例（tokio 标准模式，允许多客户端并发）；
/// 单个连接内的读+处理+写整体受 `op_timeout` 约束。
pub async fn run_pipe_server(
    pipe_name: &str,
    op_timeout: Duration,
    handler: Arc<dyn FrameHandler>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), PipeError> {
    validate_pipe_name(pipe_name)?;
    let security = PipeSecurity::current_user().map_err(PipeError::Security)?;
    info!(
        pipe = pipe_name,
        timeout_ms = op_timeout.as_millis() as u64,
        "IME Agent 管道服务启动"
    );

    let mut first_instance = true;
    let mut security = security;
    loop {
        if *shutdown.borrow() {
            break;
        }

        let server = match create_instance(pipe_name, &mut security, first_instance) {
            Ok(server) => server,
            Err(error) if first_instance => {
                // 首次带 FILE_FLAG_FIRST_PIPE_INSTANCE 失败：多半是上一次异常退出残留了
                // 同名实例。降级重试一次（对齐 mock 行为）并给出警告。
                warn!(%error, "管道首实例创建失败（可能有残留实例），降级重试");
                create_instance(pipe_name, &mut security, false)?
            }
            Err(error) => return Err(PipeError::Io(error)),
        };
        first_instance = false;

        tokio::select! {
            result = server.connect() => {
                result?;
            }
            _ = shutdown.changed() => break,
        }

        let handler = Arc::clone(&handler);
        let timeout = op_timeout;
        tokio::spawn(async move {
            if let Err(error) = serve_connection(server, handler, timeout).await {
                debug!(%error, "IME 管道连接结束");
            }
        });
    }
    info!("IME Agent 管道服务已停止");
    Ok(())
}

fn create_instance(
    pipe_name: &str,
    security: &mut PipeSecurity,
    first_instance: bool,
) -> std::io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first_instance);
    // Safety: `security` 的 SECURITY_ATTRIBUTES 在本调用期间有效；
    // CreateNamedPipeW 只在调用期间读取安全描述符（随后复制进内核对象）。
    unsafe { options.create_with_security_attributes_raw(pipe_name, security.attributes_ptr()) }
}

/// 处理一次连接：读一帧 → handler → 写一帧 → 断开。
async fn serve_connection(
    mut server: NamedPipeServer,
    handler: Arc<dyn FrameHandler>,
    timeout: Duration,
) -> Result<(), PipeError> {
    let exchange = async {
        let payload = frame::read_frame(&mut server).await?;
        if let Some(response) = handler.handle(payload).await {
            frame::write_frame(&mut server, &response).await?;
        }
        Ok::<(), PipeError>(())
    };

    match tokio::time::timeout(timeout, exchange).await {
        Ok(result) => result,
        Err(_) => Err(PipeError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "IME 管道单次操作超时",
        ))),
    }
}

// ────────────────────────────── 当前用户 ACL ──────────────────────────────

/// 当前用户安全描述符（SDDL 与 mock `CurrentUserSecurity` 完全一致）：
/// `D:P(A;;GA;;;<当前用户SID>)(A;;GA;;;SY)(A;;GA;;;BA)S:(ML;;NW;;;LW)`
struct PipeSecurity {
    descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
    attributes: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
}

impl PipeSecurity {
    fn current_user() -> Result<Self, String> {
        use std::ffi::c_void;
        use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
        use windows_sys::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        // 1) 读取当前进程 token 的 TokenUser。
        let mut token: HANDLE = std::ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err("OpenProcessToken 失败".to_string());
        }
        let sid_string = (|| -> Result<String, String> {
            let mut size: u32 = 0;
            unsafe {
                GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
            }
            if size == 0 {
                return Err("GetTokenInformation 长度探测失败".to_string());
            }
            let mut storage = vec![0_u8; size as usize];
            if unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    storage.as_mut_ptr() as *mut c_void,
                    size,
                    &mut size,
                )
            } == 0
            {
                return Err("GetTokenInformation 读取失败".to_string());
            }
            let user = unsafe { &*(storage.as_ptr() as *const TOKEN_USER) };
            let mut sid_raw: *mut u16 = std::ptr::null_mut();
            if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_raw) } == 0 {
                return Err("ConvertSidToStringSidW 失败".to_string());
            }
            let mut len = 0_usize;
            while unsafe { *sid_raw.add(len) } != 0 {
                len += 1;
            }
            let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_raw, len) });
            unsafe { LocalFree(sid_raw as *mut c_void) };
            Ok(sid)
        })();
        unsafe { CloseHandle(token) };
        let sid = sid_string?;

        // 2) 组装 SDDL 并转换为安全描述符。
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)(A;;GA;;;BA)S:(ML;;NW;;;LW)")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut descriptor_size: u32 = 0;
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                &mut descriptor_size,
            )
        } == 0
        {
            return Err("ConvertStringSecurityDescriptorToSecurityDescriptorW 失败".to_string());
        }

        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        Ok(Self {
            descriptor,
            attributes,
        })
    }

    fn attributes_ptr(&mut self) -> *mut std::ffi::c_void {
        &mut self.attributes as *mut _ as *mut std::ffi::c_void
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::LocalFree(
                    self.descriptor as windows_sys::Win32::Foundation::HLOCAL,
                );
            }
        }
    }
}

// Safety: 安全描述符由 `LocalAlloc` 分配、仅供本进程使用，构造后到 `Drop`（LocalFree）
// 前只读（`create_with_security_attributes_raw` 调用期间由 Windows 读取副本进内核对象）。
// 跨线程移动不产生数据竞争；显式实现仅为弥补裸指针导致的自动推导失效。
unsafe impl Send for PipeSecurity {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_validation() {
        assert!(validate_pipe_name(DEFAULT_PIPE_NAME).is_ok());
        assert!(validate_pipe_name(r"\\.\pipe\owo-test.v1").is_ok());
        assert!(validate_pipe_name(r"\\server\pipe\evil").is_err());
        assert!(validate_pipe_name(r"\\.\pipe\..\..\evil").is_err());
        assert!(validate_pipe_name(r"\\.\pipe\").is_err());
        assert!(validate_pipe_name("relative-name").is_err());
    }

    #[test]
    fn timeout_clamped() {
        assert_eq!(clamp_timeout_ms(10), MIN_OP_TIMEOUT_MS);
        assert_eq!(clamp_timeout_ms(10_000), 10_000);
        assert_eq!(clamp_timeout_ms(999_999), MAX_OP_TIMEOUT_MS);
    }

    #[test]
    fn current_user_security_descriptor_builds() {
        let security = PipeSecurity::current_user().expect("当前用户 ACL 必须可构造");
        assert!(!security.descriptor.is_null());
        drop(security); // 触发 LocalFree，不得崩溃
    }
}
