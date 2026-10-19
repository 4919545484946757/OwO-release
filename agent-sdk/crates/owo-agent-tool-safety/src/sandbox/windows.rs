//! Windows raw FFI 层：Job Object / 令牌 / AppContainer / 管道。
//! 全部 API 为系统自带导出（kernel32/advapi32/ntdll），**不引入新依赖**。
//! 结构布局与 Windows SDK 保持一致（repr(C)，测试含尺寸断言）。
#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(clippy::upper_case_acronyms)]

use super::*;
use std::ffi::c_void;
use std::io::Read;
use std::os::raw::c_char;

pub type BOOL = i32;
pub type DWORD = u32;
pub type Handle = *mut c_void;
pub type SIZE_T = usize;
pub type ULONG_PTR = usize;
pub type PSID = *mut c_void;
pub type PSECURITY_DESCRIPTOR = *mut c_void;

/// SDDL 版本（`ConvertStringSecurityDescriptorToSecurityDescriptorW` 的 revision）。
pub const SDDL_REVISION_1: DWORD = 1;
/// 管道安全描述符：DACL 放行所有人 + 强制标签 Low（no-write-up）。
///
/// 低完整性子进程要向父进程的管道写 stdout/stderr：对象若带默认（继承父进程的
/// Medium）标签，Low 进程的写入会被 MAC 直接拒绝，表现为**命令 exit 0 但输出全空**
/// （审查 N2 实测）。给管道显式打 Low 标签后 Low 进程可以写，父进程（Medium）
/// 写/读属于 write-down/read-down，不受 no-write-up 限制。
pub const PIPE_SECURITY_SDDL: &str = "D:(A;;GA;;;WD)S:(ML;;NW;;;LW)";

pub const TRUE: BOOL = 1;
pub const FALSE: BOOL = 0;
pub const INFINITE: DWORD = 0xFFFF_FFFF;
pub const STILL_ACTIVE: DWORD = 259;
pub const ERROR_NOT_FOUND: DWORD = 1168;

// Job Object
pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: DWORD = 0x0000_2000;
pub const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: DWORD = 0x0000_0008;
pub const JOB_OBJECT_LIMIT_JOB_MEMORY: DWORD = 0x0000_0200;
pub const JOB_OBJECT_LIMIT_JOB_TIME: DWORD = 0x0000_0004;
pub const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

// 令牌
pub const TOKEN_DUPLICATE: DWORD = 0x0002;
pub const TOKEN_QUERY: DWORD = 0x0008;
pub const MAXIMUM_ALLOWED: DWORD = 0x0200_0000;
pub const TOKEN_PRIMARY: DWORD = 1;
pub const SECURITY_IMPERSONATION: DWORD = 2;
pub const TOKEN_INTEGRITY_LEVEL: DWORD = 25;
pub const SYSTEM_MANDATORY_LABEL_ACE_TYPE: u8 = 0x11;
pub const SECURITY_MANDATORY_LOW_RID: DWORD = 0x1000;

// 进程/线程属性（AppContainer）
pub const EXTENDED_STARTUPINFO_PRESENT: DWORD = 0x0008_0000;
pub const PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES: ULONG_PTR = 0x0002_0009;

// STARTUPINFO / CreateProcess 标志与文件访问常量（审查 N2：受限令牌路径要真正
// 用上管道句柄、cwd 与环境块，否则子进程既拿不到 stdout 也看不到 cwd/env）。
/// `STARTUPINFO.dwFlags`：hStdInput/hStdOutput/hStdError 有效（必须设，否则
/// CreateProcess 忽略这些句柄，受限路径会丢 stdout）。
pub const STARTF_USESTDHANDLES: DWORD = 0x0000_0100;
/// CreateProcess 标志：lpEnvironment 是 Unicode 环境块。
pub const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x0000_0400;
pub const GENERIC_READ: DWORD = 0x8000_0000;
pub const FILE_SHARE_READ: DWORD = 0x0000_0001;
pub const FILE_SHARE_WRITE: DWORD = 0x0000_0002;
pub const OPEN_EXISTING: DWORD = 3;
pub const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

// 访问权限
pub const PROCESS_ALL_ACCESS: DWORD = 0x001F_0FFF;
pub const PROCESS_QUERY_INFORMATION: DWORD = 0x0400;

// Credential Manager
pub const CRED_TYPE_GENERIC: DWORD = 1;
pub const CRED_PERSIST_LOCAL_MACHINE: DWORD = 2;

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(lp_job_attributes: *const c_void, lp_name: *const u16) -> Handle;
    fn SetInformationJobObject(
        h_job: Handle,
        job_object_information_class: i32,
        lp_job_object_information: *const c_void,
        cb_job_object_information_length: DWORD,
    ) -> BOOL;
    fn AssignProcessToJobObject(h_job: Handle, h_process: Handle) -> BOOL;
    fn TerminateJobObject(h_job: Handle, u_exit_code: u32) -> BOOL;
    fn OpenProcess(
        dw_desired_access: DWORD,
        b_inherit_handle: BOOL,
        dw_process_id: DWORD,
    ) -> Handle;
    fn GetCurrentProcess() -> Handle;
    fn CloseHandle(h_object: Handle) -> BOOL;
    fn CreatePipe(
        h_read_pipe: *mut Handle,
        h_write_pipe: *mut Handle,
        lp_pipe_attributes: *const c_void,
        n_size: DWORD,
    ) -> BOOL;
    fn CreateFileW(
        lp_file_name: *const u16,
        dw_desired_access: DWORD,
        dw_share_mode: DWORD,
        lp_security_attributes: *const c_void,
        dw_creation_disposition: DWORD,
        dw_flags_and_attributes: DWORD,
        h_template_file: *mut c_void,
    ) -> Handle;
    fn CreateProcessW(
        lp_application_name: *const u16,
        lp_command_line: *mut u16,
        lp_process_attributes: *const c_void,
        lp_thread_attributes: *const c_void,
        b_inherit_handles: BOOL,
        dw_creation_flags: DWORD,
        lp_environment: *const c_void,
        lp_current_directory: *const u16,
        lp_startup_info: *mut c_void,
        lp_process_information: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn InitializeProcThreadAttributeList(
        lp_attribute_list: *mut c_void,
        dw_attribute_count: DWORD,
        dw_flags: DWORD,
        lp_size: *mut SIZE_T,
    ) -> BOOL;
    fn UpdateProcThreadAttribute(
        lp_attribute_list: *mut c_void,
        dw_flags: DWORD,
        attribute: ULONG_PTR,
        lp_value: *const c_void,
        cb_size: SIZE_T,
        lp_previous_value: *mut c_void,
        lp_return_size: *mut SIZE_T,
    ) -> BOOL;
    fn DeleteProcThreadAttributeList(lp_attribute_list: *mut c_void);
    fn ReadFile(
        h_file: Handle,
        lp_buffer: *mut u8,
        n_number_of_bytes_to_read: DWORD,
        lp_number_of_bytes_read: *mut DWORD,
        lp_overlapped: *mut c_void,
    ) -> BOOL;
    fn WaitForSingleObject(h_handle: Handle, dw_milliseconds: DWORD) -> DWORD;
    fn GetExitCodeProcess(h_process: Handle, lp_exit_code: *mut DWORD) -> BOOL;
    fn GetLastError() -> DWORD;
    fn LoadLibraryW(lp_file_name: *const u16) -> Handle;
    fn GetProcAddress(h_module: Handle, lp_proc_name: *const c_char) -> *mut c_void;
    fn TerminateProcess(h_process: Handle, u_exit_code: u32) -> BOOL;
    fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string_security_descriptor: *const u16,
        string_sd_revision: DWORD,
        security_descriptor: *mut PSECURITY_DESCRIPTOR,
        security_descriptor_size: *mut DWORD,
    ) -> BOOL;
    fn OpenProcessToken(
        process_handle: Handle,
        desired_access: DWORD,
        token_handle: *mut Handle,
    ) -> BOOL;
    fn DuplicateTokenEx(
        existing_token_handle: Handle,
        desired_access: DWORD,
        token_attributes: *const c_void,
        impersonation_level: DWORD,
        token_type: DWORD,
        new_token_handle: *mut Handle,
    ) -> BOOL;
    fn SetTokenInformation(
        token_handle: Handle,
        token_information_class: DWORD,
        token_information: *const c_void,
        token_information_length: DWORD,
    ) -> BOOL;
    fn CreateProcessAsUserW(
        h_token: Handle,
        lp_application_name: *const u16,
        lp_command_line: *mut u16,
        lp_process_attributes: *const c_void,
        lp_thread_attributes: *const c_void,
        b_inherit_handles: BOOL,
        dw_creation_flags: DWORD,
        lp_environment: *const c_void,
        lp_current_directory: *const u16,
        lp_startup_info: *mut c_void,
        lp_process_information: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn DeriveAppContainerSidFromAppContainerName(
        psz_app_container_name: *const u16,
        psid: *mut PSID,
    ) -> BOOL;
    fn FreeSid(psid: PSID);
    fn CredWriteW(credential: *const CREDENTIALW, flags: DWORD) -> BOOL;
    fn CredReadW(
        target_name: *mut u16,
        typ: DWORD,
        flags: DWORD,
        credential: *mut *mut CREDENTIALW,
    ) -> BOOL;
    fn CredDeleteW(target_name: *mut u16, typ: DWORD, flags: DWORD) -> BOOL;
    fn CredFree(buffer: *mut c_void);
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(lp_version_information: *mut OSVERSIONINFOW) -> i32;
}

#[repr(C)]
pub struct OSVERSIONINFOW {
    pub dw_os_version_info_size: DWORD,
    pub dw_major_version: DWORD,
    pub dw_minor_version: DWORD,
    pub dw_build_number: DWORD,
    pub dw_platform_id: DWORD,
    pub sz_csd_version: [u16; 128],
}

#[repr(C)]
pub struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    pub per_process_user_time_limit: i64,
    pub per_job_user_time_limit: i64,
    pub limit_flags: DWORD,
    pub minimum_working_set_size: SIZE_T,
    pub maximum_working_set_size: SIZE_T,
    pub active_process_limit: DWORD,
    pub affinity: ULONG_PTR,
    pub priority_class: DWORD,
    pub scheduling_class: DWORD,
}

#[repr(C)]
pub struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    pub basic_limit_information: JOBOBJECT_BASIC_LIMIT_INFORMATION,
    pub io_info: [u64; 6],
    pub process_memory_limit: SIZE_T,
    pub job_memory_limit: SIZE_T,
    pub peak_process_memory_used: SIZE_T,
    pub peak_job_memory_used: SIZE_T,
}

#[repr(C)]
pub struct PROCESS_INFORMATION {
    pub h_process: Handle,
    pub h_thread: Handle,
    pub dw_process_id: DWORD,
    pub dw_thread_id: DWORD,
}

#[repr(C)]
pub struct STARTUPINFOW {
    pub cb: DWORD,
    pub lp_reserved: *mut u16,
    pub lp_desktop: *mut u16,
    pub lp_title: *mut u16,
    pub dw_x: DWORD,
    pub dw_y: DWORD,
    pub dw_x_size: DWORD,
    pub dw_y_size: DWORD,
    pub dw_x_count_chars: DWORD,
    pub dw_y_count_chars: DWORD,
    pub dw_fill_attribute: DWORD,
    pub dw_flags: DWORD,
    pub w_show_window: u16,
    pub cb_reserved2: u16,
    pub lp_reserved2: *mut u8,
    pub h_std_input: Handle,
    pub h_std_output: Handle,
    pub h_std_error: Handle,
}

#[repr(C)]
pub struct STARTUPINFOEXW {
    pub startup_info: STARTUPINFOW,
    pub lp_attribute_list: *mut c_void,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SID_AND_ATTRIBUTES {
    pub sid: PSID,
    pub attributes: DWORD,
}

#[repr(C)]
pub struct SECURITY_CAPABILITIES {
    pub app_container_sid: PSID,
    pub capabilities: *mut SID_AND_ATTRIBUTES,
    pub capability_count: DWORD,
    pub reserved: DWORD,
}

#[repr(C)]
pub struct ACE_HEADER {
    pub ace_type: u8,
    pub ace_flags: u8,
    pub ace_size: u16,
}

#[repr(C)]
pub struct SID_MINIMAL {
    pub revision: u8,
    pub sub_authority_count: u8,
    pub identifier_authority: [u8; 6],
    pub sub_authority: [DWORD; 1],
}

#[repr(C)]
pub struct SYSTEM_MANDATORY_LABEL_ACE {
    pub header: ACE_HEADER,
    pub mask: DWORD,
    pub sid_start: SID_MINIMAL,
}

#[repr(C)]
pub struct CREDENTIALW {
    pub flags: DWORD,
    pub cred_type: DWORD,
    pub target_name: *mut u16,
    pub comment: *mut u16,
    pub last_written: [DWORD; 2],
    pub credential_blob_size: DWORD,
    pub credential_blob: *mut u8,
    pub persist: DWORD,
    pub attribute_count: DWORD,
    pub attributes: *mut c_void,
    pub target_alias: *mut u16,
    pub user_name: *mut u16,
}

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub n_length: DWORD,
    pub lp_security_descriptor: *mut c_void,
    pub b_inherit_handle: BOOL,
}

pub fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn last_error() -> DWORD {
    unsafe { GetLastError() }
}

pub fn close_handle(handle: Handle) {
    if !handle.is_null() {
        unsafe {
            CloseHandle(handle);
        }
    }
}

pub fn terminate_job(job: Handle, exit_code: u32) {
    if !job.is_null() {
        unsafe {
            TerminateJobObject(job, exit_code);
        }
    }
}

/// 创建受限 Job（kill-on-close + 资源上限）。返回句柄，失败返回 None。
pub fn create_job(policy: &SandboxPolicy) -> Option<Handle> {
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return None;
    }
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if let Some(limit) = policy.active_process_limit {
        flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.basic_limit_information.active_process_limit = limit;
    }
    if let Some(mem_mb) = policy.mem_mb {
        flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
        info.job_memory_limit = (mem_mb as usize).saturating_mul(1024 * 1024);
    }
    if let Some(cpu_ms) = policy.cpu_ms {
        flags |= JOB_OBJECT_LIMIT_JOB_TIME;
        // 100ns 单位
        info.basic_limit_information.per_job_user_time_limit =
            (cpu_ms as i64).saturating_mul(10_000);
    }
    info.basic_limit_information.limit_flags = flags;
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
        )
    };
    if ok == FALSE {
        close_handle(job);
        return None;
    }
    Some(job)
}

/// 把 PID 进程挂入 Job。
pub fn assign_pid_to_job(job: Handle, pid: u32) -> bool {
    let process = unsafe { OpenProcess(PROCESS_ALL_ACCESS, FALSE, pid) };
    if process.is_null() {
        return false;
    }
    let ok = unsafe { AssignProcessToJobObject(job, process) };
    close_handle(process);
    ok == TRUE
}

/// OS 版本探测（RtlGetVersion；失败按保守处理）。
pub fn os_version() -> (u32, u32) {
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dw_os_version_info_size = std::mem::size_of::<OSVERSIONINFOW>() as DWORD;
    let status = unsafe { RtlGetVersion(&mut info) };
    if status != 0 {
        return (0, 0);
    }
    (info.dw_major_version, info.dw_minor_version)
}

/// AppContainer API 是否存在（Win8+；动态解析避免旧系统加载失败）。
fn app_container_api_present() -> bool {
    let advapi = unsafe { LoadLibraryW(to_wide("advapi32.dll").as_ptr()) };
    if advapi.is_null() {
        return false;
    }
    let proc = unsafe {
        GetProcAddress(
            advapi,
            c"DeriveAppContainerSidFromAppContainerName".as_ptr(),
        )
    };
    // advapi32 恒驻留，无需 FreeLibrary。
    !proc.is_null()
}

/// 探测 AppContainer：API 存在 + 派生 SID 成功。
pub fn probe_app_container() -> (bool, String) {
    if !app_container_api_present() {
        return (
            false,
            "AppContainer API 不可用（需要 Windows 8+）".to_string(),
        );
    }
    let name = to_wide("owo-agent-cap-probe");
    let mut sid: PSID = std::ptr::null_mut();
    let ok = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
    if ok == TRUE && !sid.is_null() {
        unsafe {
            FreeSid(sid);
        }
        return (true, "AppContainer API 可用".to_string());
    }
    (
        false,
        format!("AppContainer SID 派生失败（错误 {}）", last_error()),
    )
}

/// `SE_GROUP_INTEGRITY`：TOKEN_MANDATORY_LABEL.Label.Attributes 的必需取值。
pub const SE_GROUP_INTEGRITY: DWORD = 0x0000_0020;

/// 低完整性标签 SID：`S-1-16-4096`（`SECURITY_MANDATORY_LOW_RID`）。
///
/// 审查 N2 第 1 点：旧实现把 `SYSTEM_MANDATORY_LABEL_ACE`（ACE 布局，掩码语义也
/// 不对）直接传给 `SetTokenInformation(TokenIntegrityLevel)`，系统返回 998 /
/// 1299，低完整性令牌**永远探测不到**，受限令牌路径静默降级为 Job-only。
///
/// 关键实现约束（实测得出，勿改回去）：
/// 1. 长度 = `sizeof(TOKEN_MANDATORY_LABEL) + GetSidLengthRequired(1)` = 28；
/// 2. SID **不能**用 `&mut self` 里的指针字段持有——结构体按值返回时指针会指向
///    已失效的栈帧，系统校验失败（1299）。指针必须在调用点从仍然存活的 SID
///    数组现取（[`Self::token_label`]）；
/// 3. SID 用 `[u32; 3]` 存放，保证 4 字节对齐。
pub struct LowIntegrityLabel {
    sid: [u32; 3],
}

impl LowIntegrityLabel {
    /// `sizeof(TOKEN_MANDATORY_LABEL) + 12`（SID 1 个子权威）。
    pub const LEN: DWORD = 28;

    pub fn new() -> Self {
        let mut label = Self { sid: [0; 3] };
        let bytes = label.sid.as_mut_ptr() as *mut u8;
        unsafe {
            *bytes = 1; // SID revision
            *bytes.add(1) = 1; // sub-authority count
            std::ptr::copy_nonoverlapping([0u8, 0, 0, 0, 0, 16].as_ptr(), bytes.add(2), 6);
            std::ptr::copy_nonoverlapping(
                SECURITY_MANDATORY_LOW_RID.to_le_bytes().as_ptr(),
                bytes.add(8),
                4,
            );
        }
        label
    }

    /// 生成指向本对象 SID 的 `TOKEN_MANDATORY_LABEL`（指针在此处现取，随对象存活）。
    pub fn token_label(&mut self) -> SID_AND_ATTRIBUTES {
        SID_AND_ATTRIBUTES {
            sid: self.sid.as_mut_ptr() as PSID,
            attributes: SE_GROUP_INTEGRITY,
        }
    }

    pub fn len(&self) -> DWORD {
        Self::LEN
    }
}

/// 探测低完整性令牌：复制当前令牌并设置 Low IL（不改动当前令牌，安全）。
pub fn probe_low_integrity() -> (bool, String) {
    let current = unsafe { GetCurrentProcess() };
    let mut token: Handle = std::ptr::null_mut();
    if unsafe { OpenProcessToken(current, TOKEN_DUPLICATE, &mut token) } == FALSE {
        return (
            false,
            format!("OpenProcessToken 失败（错误 {}）", last_error()),
        );
    }
    let mut duplicate: Handle = std::ptr::null_mut();
    let dup_ok = unsafe {
        DuplicateTokenEx(
            token,
            MAXIMUM_ALLOWED,
            std::ptr::null(),
            SECURITY_IMPERSONATION,
            TOKEN_PRIMARY,
            &mut duplicate,
        )
    };
    close_handle(token);
    if dup_ok == FALSE || duplicate.is_null() {
        return (
            false,
            format!("DuplicateTokenEx 失败（错误 {}）", last_error()),
        );
    }
    let mut label = LowIntegrityLabel::new();
    let token_label = label.token_label();
    let set_ok = unsafe {
        SetTokenInformation(
            duplicate,
            TOKEN_INTEGRITY_LEVEL,
            &token_label as *const SID_AND_ATTRIBUTES as *const c_void,
            label.len(),
        )
    };
    if set_ok == FALSE {
        close_handle(duplicate);
        return (
            false,
            format!("SetTokenInformation(Low IL) 失败（错误 {}）", last_error()),
        );
    }
    // 端到端自测：受限令牌路径必须能捕获子进程输出。只测"令牌能标 Low"不够——
    // 若管道句柄无法继承进 Low 子进程，命令会 exit 0 但 stdout/stderr 全空，
    // 比"没有隔离"更危险（结果静默丢失）。自测不过就诚实报告不可用，
    // 让调用方回落到 Job-only 路径（审查 N2：能力报告必须与真实行为一致）。
    let capture = verify_low_integrity_capture();
    close_handle(duplicate);
    match capture {
        Ok(()) => (
            true,
            "低完整性令牌可用（受限路径端到端输出捕获通过）".to_string(),
        ),
        Err(reason) => (false, reason),
    }
}

/// 低完整性自测：走**与管理器完全相同的代码路径**（`SandboxManager::spawn` →
/// `create_process` → 受限令牌）跑一条最小命令，确认输出可被捕获。
///
/// 为什么必须走管理器路径：只测"令牌能标 Low"或直接调 `create_process_with_token`
/// 都不足以证明可用——真实路径上的管道继承/句柄 MAC 校验可能失败，表现为
/// **命令 exit 0 但 stdout/stderr 全空**，比"没有隔离"更危险（结果静默丢失）。
fn verify_low_integrity_capture() -> Result<(), String> {
    let support = PlatformSupport {
        os: "windows".to_string(),
        app_container: false,
        job_object: true,
        low_integrity: true,
        reason: "low-integrity self-test".to_string(),
    };
    let executor = WindowsSandboxExecutor::detect(&support)
        .ok_or_else(|| "低完整性自测：Job 执行器不可用".to_string())?;
    let mut manager = SandboxManager::new(Box::new(executor), support);
    let policy = SandboxPolicy {
        name: "low-integrity-probe".to_string(),
        workspace: None,
        file_scope: FileScope::WorkspacePlusReadonlySystem,
        network_policy: NetworkPolicy::Loopback,
        cpu_ms: Some(10_000),
        mem_mb: None,
        ttl_secs: Some(30),
        require_isolation: IsolationLevel::LowIntegrity,
        allow_degraded: false,
        ..SandboxPolicy::default()
    };
    let command = SandboxCommand::new("cmd", policy)
        .with_args(vec!["/C".to_string(), "echo owo-lowil-probe".to_string()]);
    let mut process = manager
        .spawn(&command)
        .map_err(|error| format!("低完整性自测创建失败：{error}"))?;
    let info = process
        .wait_output()
        .map_err(|error| format!("低完整性自测等待失败：{error}"))?;
    let stdout = String::from_utf8_lossy(&info.stdout);
    if !stdout.contains("owo-lowil-probe") {
        return Err(format!(
            "低完整性令牌可用但受限路径无法捕获输出（exit={}，stdout={stdout:?}）；\
             保持 Job-only 以免命令结果静默丢失",
            info.exit_code
        ));
    }
    Ok(())
}

/// Windows 真实能力探测。
pub fn probe_windows_support() -> PlatformSupport {
    let (major, minor) = os_version();
    let version_note = format!("Windows {major}.{minor}");
    let mut reasons = Vec::new();

    let job = create_job(&SandboxPolicy::default());
    let job_object = job.is_some();
    if let Some(job) = job {
        terminate_job(job, 1);
        close_handle(job);
    }
    reasons.push(if job_object {
        "Job Object 创建成功".to_string()
    } else {
        format!("Job Object 不可用（错误 {}）", last_error())
    });

    let (low_integrity, low_reason) = probe_low_integrity();
    reasons.push(if low_integrity {
        "低完整性令牌可用".to_string()
    } else {
        low_reason
    });

    let (app_container, ac_reason) = probe_app_container();
    reasons.push(if app_container {
        "AppContainer 可用".to_string()
    } else {
        ac_reason
    });

    PlatformSupport {
        os: "windows".to_string(),
        app_container,
        job_object,
        low_integrity,
        reason: format!("{}；{}", version_note, reasons.join("；")),
    }
}

// The Job is owned by the process. The executor keeps only weak references, so
// completed jobs do not accumulate handles or authorize killing a reused PID.
struct JobControl {
    handle: Handle,
}
// Windows Job handles support concurrent terminate/wait; Arc keeps the handle
// open through each operation and only the final owner closes it.
unsafe impl Send for JobControl {}
unsafe impl Sync for JobControl {}

impl JobControl {
    fn terminate(&self, exit_code: u32) -> Result<(), SandboxError> {
        if unsafe { TerminateJobObject(self.handle, exit_code) } == FALSE {
            return Err(SandboxError::Kill(format!(
                "TerminateJobObject failed: {}",
                last_error()
            )));
        }
        Ok(())
    }
}

impl Drop for JobControl {
    fn drop(&mut self) {
        terminate_job(self.handle, 1);
        close_handle(self.handle);
    }
}

const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
const CAPTURE_TRUNCATED: &[u8] = b"\n[output truncated: capture limit reached]\n";

fn append_captured_output(out: &mut Vec<u8>, bytes: &[u8]) {
    let remaining = MAX_CAPTURE_BYTES.saturating_sub(out.len());
    out.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
    if bytes.len() > remaining && !out.ends_with(CAPTURE_TRUNCATED) {
        out.extend_from_slice(CAPTURE_TRUNCATED);
    }
}

fn capture_reader<R: Read>(reader: Option<R>) -> Result<Vec<u8>, SandboxError> {
    let mut out = Vec::new();
    if let Some(mut reader) = reader {
        let mut buffer = [0u8; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            append_captured_output(&mut out, &buffer[..count]);
        }
    }
    Ok(out)
}

/// Windows 沙箱执行器：Job 基线 + LowIL/AppContainer 按策略升级。
pub struct WindowsSandboxExecutor {
    support: PlatformSupport,
    jobs: Mutex<std::collections::HashMap<String, std::sync::Weak<JobControl>>>,
}

impl WindowsSandboxExecutor {
    pub fn detect(support: &PlatformSupport) -> Option<Self> {
        if !support.job_object {
            return None;
        }
        Some(Self {
            support: support.clone(),
            jobs: Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// 按策略选择隔离创建方式：AppContainer → LowIL → Job-only。
    fn create_process(
        &self,
        command: &SandboxCommand,
        job: Handle,
    ) -> Result<OsChild, SandboxError> {
        let required = command.policy.require_isolation;
        if required >= IsolationLevel::AppContainerJob && self.support.app_container {
            return self.spawn_app_container(command, job);
        }
        if required >= IsolationLevel::LowIntegrity && self.support.low_integrity {
            return self.spawn_low_integrity(command, job);
        }
        self.spawn_plain(command, job)
    }

    /// 普通路径：std::process::Command（可靠 quoting）+ Job 挂接。
    fn spawn_plain(&self, command: &SandboxCommand, job: Handle) -> Result<OsChild, SandboxError> {
        // raw_arg（`CommandExt`）用于 cmd 命令体原样透传，见下方说明。
        use std::os::windows::process::CommandExt as _;
        let mut cmd = std::process::Command::new(&command.program);
        // `cmd /C|/K <命令体>`：命令体必须用 raw_arg 原样透传。
        // std 的 Windows 参数转义会给含空格的参数加引号并把内部 `"` 写成 `\"`，
        // cmd 的 /C 旧行为再剥掉首尾引号，模型最常写的
        // `python -c "print(2+2)"` 就变成 `python -c \"print(2+2)\"` →
        // python 实际执行的是**字符串字面量**，`exit_code=0` 却 stdout 全空
        // （带分号的变体则直接 SyntaxError）。这类"静默成功"最难排查。
        if is_cmd_shell(&command.program) && command.args.iter().any(|arg| is_cmd_switch(arg)) {
            for arg in &command.args {
                cmd.raw_arg(arg);
            }
        } else {
            cmd.args(&command.args);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(cwd) = &command.cwd {
            cmd.current_dir(cwd);
        }
        for (key, value) in &command.env {
            cmd.env(key, value);
        }
        let mut child = cmd
            .spawn()
            .map_err(|error| SandboxError::Spawn(format!("{}：{error}", command.program)))?;
        let pid = child.id();
        if !assign_pid_to_job(job, pid) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SandboxError::Spawn(format!(
                "进程 {pid} 无法挂入 Job（错误 {}），已终止",
                last_error()
            )));
        }
        Ok(OsChild::StdChild { child })
    }

    /// 低完整性路径：受限令牌 + Low IL 标签 + CreateProcessAsUserW。
    fn spawn_low_integrity(
        &self,
        command: &SandboxCommand,
        job: Handle,
    ) -> Result<OsChild, SandboxError> {
        let current = unsafe { GetCurrentProcess() };
        let mut token: Handle = std::ptr::null_mut();
        if unsafe { OpenProcessToken(current, TOKEN_DUPLICATE, &mut token) } == FALSE {
            return Err(SandboxError::Spawn(format!(
                "OpenProcessToken 失败（错误 {}）",
                last_error()
            )));
        }
        let mut primary: Handle = std::ptr::null_mut();
        let dup_ok = unsafe {
            DuplicateTokenEx(
                token,
                MAXIMUM_ALLOWED,
                std::ptr::null(),
                SECURITY_IMPERSONATION,
                TOKEN_PRIMARY,
                &mut primary,
            )
        };
        close_handle(token);
        if dup_ok == FALSE || primary.is_null() {
            return Err(SandboxError::Spawn(format!(
                "DuplicateTokenEx 失败（错误 {}）",
                last_error()
            )));
        }
        let mut label = LowIntegrityLabel::new();
        let token_label = label.token_label();
        let set_ok = unsafe {
            SetTokenInformation(
                primary,
                TOKEN_INTEGRITY_LEVEL,
                &token_label as *const SID_AND_ATTRIBUTES as *const c_void,
                label.len(),
            )
        };
        if set_ok == FALSE {
            close_handle(primary);
            return Err(SandboxError::Spawn(format!(
                "SetTokenInformation(Low IL) 失败（错误 {}）",
                last_error()
            )));
        }
        let (pi, pipes) = create_process_with_token(command, job, |context| {
            let result = unsafe {
                CreateProcessAsUserW(
                    primary,
                    std::ptr::null(),
                    context.command_line(),
                    std::ptr::null(),
                    std::ptr::null(),
                    TRUE,
                    context.creation_flags(),
                    context.environment(),
                    context.current_directory(),
                    context.startup_info_ptr(),
                    context.process_information(),
                )
            };
            if result == FALSE {
                Err(SandboxError::Spawn(format!(
                    "CreateProcessAsUserW(Low IL) 失败（错误 {}）",
                    last_error()
                )))
            } else {
                Ok(())
            }
        })?;
        close_handle(primary);
        Ok(OsChild::OsChild {
            pi,
            stdout_read: pipes.read_stdout,
            stderr_read: pipes.read_stderr,
        })
    }

    /// AppContainer 路径：SECURITY_CAPABILITIES 属性 + CreateProcessW。
    fn spawn_app_container(
        &self,
        command: &SandboxCommand,
        job: Handle,
    ) -> Result<OsChild, SandboxError> {
        let name = to_wide("owo-agent-container");
        let mut sid: PSID = std::ptr::null_mut();
        if unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) } == FALSE
            || sid.is_null()
        {
            return Err(SandboxError::Spawn(format!(
                "AppContainer SID 派生失败（错误 {}）",
                last_error()
            )));
        }
        // 网络能力白名单：按策略生成 SID 并校验（隔离策略不得带网络能力）。
        let capability_sids = app_container_network_capabilities(&command.policy);
        validate_app_container_network(&command.policy, &capability_sids)?;
        let sid_boxes: Vec<Box<[u8]>> = capability_sids
            .iter()
            .map(|sid| sid.clone().into_boxed_slice())
            .collect();
        let attrs: Vec<SID_AND_ATTRIBUTES> = sid_boxes
            .iter()
            .map(|boxed| SID_AND_ATTRIBUTES {
                sid: boxed.as_ptr() as PSID,
                attributes: 0,
            })
            .collect();
        let capabilities = SECURITY_CAPABILITIES {
            app_container_sid: sid,
            capabilities: attrs.as_ptr() as *mut SID_AND_ATTRIBUTES,
            capability_count: attrs.len() as DWORD,
            reserved: 0,
        };
        let (pi, pipes) = create_process_with_token(command, job, move |context| {
            let mut size: SIZE_T = 0;
            let size_ok =
                unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
            if size_ok == FALSE && size == 0 {
                return Err(SandboxError::Spawn(
                    "InitializeProcThreadAttributeList 尺寸获取失败".to_string(),
                ));
            }
            let mut buffer = vec![0u8; size];
            let list = buffer.as_mut_ptr() as *mut c_void;
            if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == FALSE {
                return Err(SandboxError::Spawn(format!(
                    "InitializeProcThreadAttributeList 失败（错误 {}）",
                    last_error()
                )));
            }
            let updated = unsafe {
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
                    &capabilities as *const SECURITY_CAPABILITIES as *const c_void,
                    std::mem::size_of::<SECURITY_CAPABILITIES>(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if updated == FALSE {
                unsafe {
                    DeleteProcThreadAttributeList(list);
                }
                return Err(SandboxError::Spawn(format!(
                    "UpdateProcThreadAttribute 失败（错误 {}）",
                    last_error()
                )));
            }
            // 把属性列表挂上 STARTUPINFOEXW：不挂等于建了列表却不用，进程不会进容器。
            context.set_attribute_list(list);
            let result = unsafe {
                CreateProcessW(
                    std::ptr::null(),
                    context.command_line(),
                    std::ptr::null(),
                    std::ptr::null(),
                    TRUE,
                    context.creation_flags(),
                    context.environment(),
                    context.current_directory(),
                    context.startup_info_ptr(),
                    context.process_information(),
                )
            };
            // 属性列表仅需存活到 CreateProcessW 返回。
            unsafe {
                DeleteProcThreadAttributeList(list);
            }
            if result == FALSE {
                Err(SandboxError::Spawn(format!(
                    "CreateProcessW(AppContainer) 失败（错误 {}）",
                    last_error()
                )))
            } else {
                Ok(())
            }
        })?;
        unsafe {
            FreeSid(sid);
        }
        Ok(OsChild::OsChild {
            pi,
            stdout_read: pipes.read_stdout,
            stderr_read: pipes.read_stderr,
        })
    }
}

/// 受限令牌 / AppContainer 创建路径共享的 `CreateProcess` 参数。
///
/// 审查 N2 修复的四个 Win32 坑：
/// 1. `STARTUPINFOEXW` 必须通过 `dwCreationFlags=EXTENDED_STARTUPINFO_PRESENT`
///    声明，否则系统按 `STARTUPINFOW` 解释，`lpAttributeList`（AppContainer
///    安全能力）被**静默忽略**——进程根本没进容器却报告已隔离；
/// 2. 设了 `hStdOutput/hStdError` 就必须设 `STARTF_USESTDHANDLES`，否则管道句柄
///    被忽略，受限路径的命令输出全部丢失；
/// 3. `lpCurrentDirectory` / `lpEnvironment` 以前恒为 null：cwd 与 env 覆盖被
///    丢掉（命令跑在守护进程目录、看不到注入的环境变量）；
/// 4. stdin 以前没有句柄：命令读 stdin 会拿到无效句柄而报错，这里统一接 `NUL`。
struct CreateProcessContext {
    startup: *mut STARTUPINFOEXW,
    command_line: *mut u16,
    environment: *const c_void,
    current_directory: *const u16,
    creation_flags: DWORD,
    process_information: *mut PROCESS_INFORMATION,
}

impl CreateProcessContext {
    fn command_line(&self) -> *mut u16 {
        self.command_line
    }

    fn environment(&self) -> *const c_void {
        self.environment
    }

    fn current_directory(&self) -> *const u16 {
        self.current_directory
    }

    fn creation_flags(&self) -> DWORD {
        self.creation_flags
    }

    fn startup_info_ptr(&self) -> *mut c_void {
        self.startup as *mut c_void
    }

    fn process_information(&self) -> *mut PROCESS_INFORMATION {
        self.process_information
    }

    /// AppContainer 路径：把属性列表挂到 STARTUPINFOEXW（否则能力被忽略）。
    fn set_attribute_list(&mut self, list: *mut c_void) {
        unsafe {
            (*self.startup).lp_attribute_list = list;
        }
    }
}

/// 打开 `NUL` 作为 stdin：让命令读 stdin 时立即 EOF，而不是继承到无效句柄。
///
/// 注意必须是**可继承**句柄：`STARTF_USESTDHANDLES` 的三个句柄都要能被子进程继承，
/// 否则受限令牌路径下子进程拿到无效句柄、输出全部丢失（实测 stdout 为空）。
fn null_device_handle() -> Result<Handle, SandboxError> {
    let name = to_wide("NUL");
    // 同样带 Low 强制标签：低完整性子进程要继承这个句柄，对象标签必须允许 Low 访问。
    let descriptor = low_integrity_pipe_descriptor();
    let attributes = SECURITY_ATTRIBUTES {
        n_length: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
        lp_security_descriptor: descriptor,
        b_inherit_handle: TRUE,
    };
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &attributes as *const SECURITY_ATTRIBUTES as *const c_void,
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if !descriptor.is_null() {
        unsafe {
            LocalFree(descriptor);
        }
    }
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(SandboxError::Spawn(format!(
            "打开 NUL 设备失败（错误 {}）",
            last_error()
        )));
    }
    Ok(handle)
}

/// 构造 CreateProcess 的 Unicode 环境块（`K=V\0…\0\0`，按键排序）。
///
/// 合并父进程环境与 `command.env` 覆盖项；键名含 `=` 或为空会被 Windows 截断，直接跳过。
fn environment_block(overrides: &[(String, String)]) -> Vec<u16> {
    use std::collections::BTreeMap;
    let mut vars: BTreeMap<String, String> = std::env::vars().collect();
    for (key, value) in overrides {
        vars.insert(key.clone(), value.clone());
    }
    let mut block: Vec<u16> = Vec::new();
    for (key, value) in vars {
        if key.is_empty() || key.contains('=') {
            continue;
        }
        block.extend(format!("{key}={value}").encode_utf16());
        block.push(0);
    }
    // 环境块必须以双 NUL 结尾（空环境也要两个 NUL）。
    block.push(0);
    block.push(0);
    block
}

/// 统一进程创建：管道 + STARTUPINFOEX + Job 挂接。
/// `create` 闭包负责调用 CreateProcessW 族并填充 `PROCESS_INFORMATION`。
fn create_process_with_token<F>(
    command: &SandboxCommand,
    job: Handle,
    create: F,
) -> Result<(PROCESS_INFORMATION, PipePair), SandboxError>
where
    F: FnOnce(&mut CreateProcessContext) -> Result<(), SandboxError>,
{
    let pipes = PipePair::create()
        .map_err(|error| SandboxError::Spawn(format!("CreatePipe 失败：{error}")))?;
    let stdin_handle = match null_device_handle() {
        Ok(handle) => handle,
        Err(error) => {
            close_handle(pipes.read_stdout);
            close_handle(pipes.read_stderr);
            close_handle(pipes.write_stdout);
            close_handle(pipes.write_stderr);
            return Err(error);
        }
    };
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.startup_info.cb = std::mem::size_of::<STARTUPINFOEXW>() as DWORD;
    // 关键：句柄有效必须声明 STARTF_USESTDHANDLES（旧代码误把创建标志
    // EXTENDED_STARTUPINFO_PRESENT 填进 dwFlags，等于没声明任何句柄）。
    startup.startup_info.dw_flags = STARTF_USESTDHANDLES;
    startup.startup_info.h_std_input = stdin_handle;
    startup.startup_info.h_std_output = pipes.write_stdout;
    startup.startup_info.h_std_error = pipes.write_stderr;
    let mut cmdline = command_line(&command.program, &command.args);
    let env = environment_block(&command.env);
    let cwd = command
        .cwd
        .as_ref()
        .map(|path| to_wide(&path.to_string_lossy()));
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let mut context = CreateProcessContext {
        startup: &mut startup as *mut _,
        command_line: cmdline.as_mut_ptr(),
        environment: env.as_ptr() as *const c_void,
        current_directory: cwd
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr()),
        // EXTENDED_STARTUPINFO_PRESENT 是**创建标志**：没有它 STARTUPINFOEXW 的
        // 属性列表按 STARTUPINFOW 解释被忽略（AppContainer 静默失效）。
        creation_flags: EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
        process_information: &mut pi as *mut _,
    };
    let creation = create(&mut context);
    close_handle(stdin_handle);
    // 子进程已创建（或失败）：关闭父侧写端副本。
    close_handle(pipes.write_stdout);
    close_handle(pipes.write_stderr);
    creation?;
    if pi.h_process.is_null() {
        return Err(SandboxError::Spawn(
            "CreateProcessW 未返回进程句柄".to_string(),
        ));
    }
    let assigned = unsafe { AssignProcessToJobObject(job, pi.h_process) };
    if assigned == FALSE {
        unsafe {
            TerminateProcess(pi.h_process, 1);
            WaitForSingleObject(pi.h_process, INFINITE);
        }
        return Err(SandboxError::Spawn(format!(
            "进程挂入 Job 失败（错误 {}），已终止",
            last_error()
        )));
    }
    Ok((pi, pipes))
}

/// 命令行拼接（lpCommandLine）：程序 + 参数；含空白参数加双引号。
pub fn command_line(program: &str, args: &[String]) -> Vec<u16> {
    to_wide(&command_line_string(program, args))
}

/// 命令行拼接的字符串形态（可单测；[`command_line`] 只负责转 UTF-16）。
///
/// `cmd /C|/K <命令体>` 是**唯一例外**：命令体必须原样透传，不能套引号、
/// 更不能把内部 `"` 翻倍成 `""`。否则 cmd 的 `/C` 旧行为会剥掉首尾引号，
/// 残留的 `""` 再被解释成字面 `"`，于是模型最常写的
/// `python -c "print(2+2)"` 实际执行成 `python -c ""print(2+2)""` ——
/// python 拿到空的 `-c` 参数，静默无输出（审计里 exit_code=0 却 stdout 为空），
/// 带引号的 `python -c "import pptx; print(...)"` 则直接 SyntaxError。
/// 实测证据见 2026-10-02 的 run_command 审计（PPT 请求因此连续失败）。
pub fn command_line_string(program: &str, args: &[String]) -> String {
    let parts = std::iter::once(program.to_string()).chain(args.iter().cloned());
    if is_cmd_shell(program) && args.iter().any(|arg| is_cmd_switch(arg)) {
        return parts.collect::<Vec<_>>().join(" ");
    }
    parts
        .map(|part| {
            if part.contains(' ') || part.contains('\t') {
                format!("\"{}\"", part.replace('"', "\"\""))
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 程序是否为 Windows 命令解释器（`cmd` / `cmd.exe`，忽略路径与大小写）。
pub(crate) fn is_cmd_shell(program: &str) -> bool {
    let stem = program.rsplit(['\\', '/']).next().unwrap_or(program);
    let stem = stem.strip_suffix(".exe").unwrap_or(stem);
    stem.eq_ignore_ascii_case("cmd")
}

/// 参数是否为 cmd 的命令串开关（`/C` / `/K`，忽略大小写）。
pub(crate) fn is_cmd_switch(arg: &str) -> bool {
    let trimmed = arg.trim();
    trimmed.eq_ignore_ascii_case("/c") || trimmed.eq_ignore_ascii_case("/k")
}

/// 管道对（父侧读端 + 子侧写端）。
pub struct PipePair {
    pub read_stdout: Handle,
    pub read_stderr: Handle,
    pub write_stdout: Handle,
    pub write_stderr: Handle,
}

impl PipePair {
    pub fn create() -> std::io::Result<Self> {
        let mut read_stdout: Handle = std::ptr::null_mut();
        let mut write_stdout: Handle = std::ptr::null_mut();
        let mut read_stderr: Handle = std::ptr::null_mut();
        let mut write_stderr: Handle = std::ptr::null_mut();
        // 强制标签 Low 的安全描述符：低完整性子进程必须能写管道（见 PIPE_SECURITY_SDDL）。
        // 取不到描述符时退回默认属性（等价旧行为，不阻断创建）。
        let descriptor = low_integrity_pipe_descriptor();
        let attrs = SECURITY_ATTRIBUTES {
            n_length: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
            lp_security_descriptor: descriptor,
            b_inherit_handle: TRUE,
        };
        let ok1 = unsafe {
            CreatePipe(
                &mut read_stdout,
                &mut write_stdout,
                &attrs as *const _ as *const c_void,
                0,
            )
        };
        let ok2 = unsafe {
            CreatePipe(
                &mut read_stderr,
                &mut write_stderr,
                &attrs as *const _ as *const c_void,
                0,
            )
        };
        if !descriptor.is_null() {
            unsafe {
                LocalFree(descriptor);
            }
        }
        if ok1 == FALSE || ok2 == FALSE {
            close_handle(read_stdout);
            close_handle(write_stdout);
            close_handle(read_stderr);
            close_handle(write_stderr);
            return Err(std::io::Error::last_os_error());
        }
        // 低完整性子进程要继承这四个句柄：管道在创建时已带 Low 强制标签
        // （见 low_integrity_pipe_descriptor / PIPE_SECURITY_SDDL）。
        Ok(Self {
            read_stdout,
            read_stderr,
            write_stdout,
            write_stderr,
        })
    }
}

/// 构造带 Low 强制标签的管道安全描述符；失败返回 null（调用方走默认属性）。
fn low_integrity_pipe_descriptor() -> PSECURITY_DESCRIPTOR {
    let sddl = to_wide(PIPE_SECURITY_SDDL);
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == FALSE || descriptor.is_null() {
        return std::ptr::null_mut();
    }
    descriptor
}

impl Drop for PipePair {
    fn drop(&mut self) {
        close_handle(self.read_stdout);
        close_handle(self.read_stderr);
        close_handle(self.write_stdout);
        close_handle(self.write_stderr);
    }
}

/// 管道句柄包装（raw handle 跨线程转移用；所有权唯一，可安全 Send）。
#[derive(Clone, Copy)]
pub struct PipeHandle(pub Handle);

// 句柄值可跨线程转移（不并发使用即安全），标准 Windows 实践。
unsafe impl Send for PipeHandle {}

/// 读取管道（跨线程辅助：整体传递 PipeHandle，避免字段级捕获 raw 指针）。
pub fn read_pipe_handle(handle: PipeHandle) -> Vec<u8> {
    read_pipe(handle.0)
}

/// 读管道直到 EOF。
pub fn read_pipe(handle: Handle) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let mut read: DWORD = 0;
        let ok = unsafe {
            ReadFile(
                handle,
                buffer.as_mut_ptr(),
                buffer.len() as DWORD,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == FALSE || read == 0 {
            break;
        }
        append_captured_output(&mut out, &buffer[..read as usize]);
    }
    out
}

/// 进程是否存活（句柄可打开且退出码仍为 STILL_ACTIVE）。
pub fn process_alive(pid: u32) -> bool {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, FALSE, pid) };
    if handle.is_null() {
        return false;
    }
    let mut code: DWORD = 0;
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
    close_handle(handle);
    ok == TRUE && code == STILL_ACTIVE
}

/// Job 内的进程（Job-only 用 std Child；OS 创建用 hProcess + 管道）。
pub enum OsChild {
    StdChild {
        child: std::process::Child,
    },
    OsChild {
        pi: PROCESS_INFORMATION,
        stdout_read: Handle,
        stderr_read: Handle,
    },
}

impl OsChild {
    pub fn pid(&self) -> Option<u32> {
        match self {
            OsChild::StdChild { child } => Some(child.id()),
            OsChild::OsChild { pi, .. } => Some(pi.dw_process_id),
        }
    }

    pub fn wait(&mut self) -> Result<SandboxWaitInfo, SandboxError> {
        match self {
            OsChild::StdChild { child } => {
                let stdout_pipe = child.stdout.take();
                let stderr_pipe = child.stderr.take();
                // Both pipes must keep draining even after the retained preview is full.
                let (stdout, stderr) = std::thread::scope(|scope| {
                    let out = scope.spawn(move || capture_reader(stdout_pipe));
                    let err = scope.spawn(move || capture_reader(stderr_pipe));
                    (out.join(), err.join())
                });
                let stdout = stdout
                    .map_err(|_| SandboxError::Unhealthy("stdout reader panicked".into()))??;
                let stderr = stderr
                    .map_err(|_| SandboxError::Unhealthy("stderr reader panicked".into()))??;
                let status = child.wait().map_err(SandboxError::Io)?;
                Ok(SandboxWaitInfo {
                    exit_code: status.code().unwrap_or(-1),
                    stdout,
                    stderr,
                })
            }
            OsChild::OsChild {
                pi,
                stdout_read,
                stderr_read,
            } => {
                // 并行读两个管道（避免管道满死锁），再等进程退出。
                let (stdout, stderr) = std::thread::scope(|scope| {
                    let out_handle = PipeHandle(*stdout_read);
                    let err_handle = PipeHandle(*stderr_read);
                    let t1 = scope.spawn(move || read_pipe_handle(out_handle));
                    let t2 = scope.spawn(move || read_pipe_handle(err_handle));
                    (t1.join().unwrap_or_default(), t2.join().unwrap_or_default())
                });
                unsafe {
                    WaitForSingleObject(pi.h_process, INFINITE);
                }
                let mut code: DWORD = 0;
                unsafe {
                    GetExitCodeProcess(pi.h_process, &mut code);
                }
                Ok(SandboxWaitInfo {
                    exit_code: code as i32,
                    stdout,
                    stderr,
                })
            }
        }
    }

    pub fn kill(&mut self) {
        match self {
            OsChild::StdChild { child } => {
                let _ = child.kill();
            }
            OsChild::OsChild { pi, .. } => unsafe {
                TerminateProcess(pi.h_process, 1);
            },
        }
    }
}

impl Drop for OsChild {
    fn drop(&mut self) {
        match self {
            OsChild::StdChild { child } => {
                // 丢弃时若仍运行则终止（防孤儿）。
                let _ = child.kill();
            }
            OsChild::OsChild {
                pi,
                stdout_read,
                stderr_read,
            } => unsafe {
                TerminateProcess(pi.h_process, 1);
                WaitForSingleObject(pi.h_process, 5_000);
                close_handle(pi.h_process);
                close_handle(pi.h_thread);
                close_handle(*stdout_read);
                close_handle(*stderr_read);
            },
        }
    }
}

/// Windows 进程内部句柄（inner：进程 + Job）。
pub struct WindowsProcess {
    pub os_child: OsChild,
    job: Arc<JobControl>,
}

// 句柄值可跨线程转移（进程/Job 句柄由 WindowsProcess 独占管理），标准 Windows 实践。
unsafe impl Send for WindowsProcess {}
unsafe impl Send for OsChild {}

impl SandboxProcessInner for WindowsProcess {
    fn wait(&mut self) -> Result<SandboxWaitInfo, SandboxError> {
        self.os_child.wait()
    }

    fn kill(&mut self) -> Result<(), SandboxError> {
        self.job.terminate(1)
    }
}

impl Drop for WindowsProcess {
    fn drop(&mut self) {
        // Stop the whole tree before OsChild drops or waits on inherited pipes.
        let _ = self.job.terminate(1);
    }
}

impl SandboxExecutor for WindowsSandboxExecutor {
    fn name(&self) -> &'static str {
        "windows-job"
    }

    fn capability(&self) -> IsolationLevel {
        super::available_isolation(&self.support)
    }

    fn spawn(&self, command: &SandboxCommand) -> Result<SandboxProcess, SandboxError> {
        // 网络 egress 边界（R9）：AllowList/Unrestricted 网络策略只能在
        // AppContainer 路径强制；仅 Job/LowIL 隔离无法限制网络 → 显式拒绝。
        let uses_app_container = command.policy.require_isolation
            >= IsolationLevel::AppContainerJob
            && self.support.app_container;
        if network_requires_app_container(&command.policy) && !uses_app_container {
            return Err(SandboxError::Unsupported(format!(
                "网络策略 {:?} 需要 AppContainer 隔离才能强制网络白名单，\
                 当前执行路径仅提供 {:?}（显式拒绝，不静默放开网络）",
                command.policy.network_policy,
                super::available_isolation(&self.support)
            )));
        }
        let job = Arc::new(JobControl {
            handle: create_job(&command.policy).ok_or_else(|| {
                SandboxError::Unsupported(format!("Job Object creation failed: {}", last_error()))
            })?,
        });
        let os_child = self.create_process(command, job.handle)?;
        let pid = os_child.pid().unwrap_or(0);
        let id = format!("win-{pid}-{}", uuid::Uuid::new_v4());
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| SandboxError::Unhealthy("sandbox job registry poisoned".into()))?;
        jobs.retain(|_, job| job.strong_count() > 0);
        jobs.insert(id.clone(), Arc::downgrade(&job));
        drop(jobs);
        Ok(SandboxProcess {
            handle: SandboxHandle {
                id,
                spawned_at: Utc::now().to_rfc3339(),
            },
            status: SandboxProcessStatus::Running,
            stdout: Vec::new(),
            stderr: Vec::new(),
            inner: Some(Box::new(WindowsProcess { os_child, job })),
        })
    }

    fn kill(&self, handle: &SandboxHandle) -> Result<(), SandboxError> {
        let job = self
            .jobs
            .lock()
            .map_err(|_| SandboxError::Kill("sandbox job registry poisoned".into()))?
            .get(&handle.id)
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                SandboxError::Kill(format!(
                    "unknown or completed sandbox handle: {}",
                    handle.id
                ))
            })?;
        job.terminate(1)
    }

    fn check_healthy(&self) -> SandboxHealth {
        SandboxHealth {
            healthy: true,
            detail: "Windows Job 执行器可用".to_string(),
        }
    }

    fn attach(&self, policy: &SandboxPolicy, pid: u32) -> Result<JobGuard, SandboxError> {
        let job = create_job(policy).ok_or_else(|| {
            SandboxError::Unsupported(format!("Job Object 创建失败（错误 {}）", last_error()))
        })?;
        if !assign_pid_to_job(job, pid) {
            close_handle(job);
            return Err(SandboxError::Spawn(format!(
                "进程 {pid} 无法挂入 Job（错误 {}）",
                last_error()
            )));
        }
        Ok(JobGuard { pid, job })
    }
}

#[cfg(test)]
mod integrity_label_probe_tests {
    use super::*;

    /// 低完整性标签必须能被系统接受：旧实现（ACE 结构 / 悬空 SID 指针）返回
    /// 998/1299，导致受限令牌路径静默降级（审查 N2）。
    #[test]
    fn low_integrity_label_is_accepted_by_set_token_information() {
        let current = unsafe { GetCurrentProcess() };
        let mut token: Handle = std::ptr::null_mut();
        if unsafe { OpenProcessToken(current, TOKEN_DUPLICATE, &mut token) } == FALSE {
            eprintln!("SKIP: OpenProcessToken 失败（错误 {}）", last_error());
            return;
        }
        let mut primary: Handle = std::ptr::null_mut();
        let duplicated = unsafe {
            DuplicateTokenEx(
                token,
                MAXIMUM_ALLOWED,
                std::ptr::null(),
                SECURITY_IMPERSONATION,
                TOKEN_PRIMARY,
                &mut primary,
            )
        };
        close_handle(token);
        if duplicated == FALSE || primary.is_null() {
            eprintln!("SKIP: DuplicateTokenEx 失败（错误 {}）", last_error());
            return;
        }
        let mut label = LowIntegrityLabel::new();
        let token_label = label.token_label();
        let accepted = unsafe {
            SetTokenInformation(
                primary,
                TOKEN_INTEGRITY_LEVEL,
                &token_label as *const SID_AND_ATTRIBUTES as *const c_void,
                label.len(),
            )
        };
        let error = last_error();
        close_handle(primary);
        assert_eq!(
            accepted, TRUE,
            "低完整性标签应被接受（错误 {error}）；旧实现 998/1299 会静默降级为 Job-only"
        );
        assert_eq!(
            LowIntegrityLabel::LEN,
            28,
            "长度必须为 TOKEN_MANDATORY_LABEL + SID(12)"
        );
    }
}

#[cfg(test)]
mod token_spawn_plumbing_tests {
    use super::*;

    fn duplicate_primary_token() -> Handle {
        let current = unsafe { GetCurrentProcess() };
        let mut token: Handle = std::ptr::null_mut();
        unsafe { OpenProcessToken(current, TOKEN_DUPLICATE, &mut token) };
        let mut primary: Handle = std::ptr::null_mut();
        unsafe {
            DuplicateTokenEx(
                token,
                MAXIMUM_ALLOWED,
                std::ptr::null(),
                SECURITY_IMPERSONATION,
                TOKEN_PRIMARY,
                &mut primary,
            );
        }
        close_handle(token);
        primary
    }

    fn probe_command() -> SandboxCommand {
        SandboxCommand {
            program: "cmd".to_string(),
            args: vec![
                "/C".to_string(),
                "echo out-token & echo err-token 1>&2".to_string(),
            ],
            policy: SandboxPolicy {
                name: "plumbing".to_string(),
                workspace: None,
                file_scope: FileScope::WorkspacePlusReadonlySystem,
                network_policy: NetworkPolicy::Loopback,
                cpu_ms: Some(30_000),
                ttl_secs: Some(60),
                require_isolation: IsolationLevel::JobOnly,
                ..SandboxPolicy::default()
            },
            cwd: None,
            env: Vec::new(),
        }
    }

    /// 受限令牌共享的创建助手必须真正让子进程继承管道句柄。
    #[test]
    fn create_process_with_token_inherits_pipes() {
        let command = probe_command();
        let Some(job) = create_job(&command.policy) else {
            eprintln!("SKIP: Job 创建失败");
            return;
        };
        let token = duplicate_primary_token();
        let created = create_process_with_token(&command, job, |context| {
            let result = unsafe {
                CreateProcessAsUserW(
                    token,
                    std::ptr::null(),
                    context.command_line(),
                    std::ptr::null(),
                    std::ptr::null(),
                    TRUE,
                    context.creation_flags(),
                    context.environment(),
                    context.current_directory(),
                    context.startup_info_ptr(),
                    context.process_information(),
                )
            };
            if result == FALSE {
                Err(SandboxError::Spawn(format!(
                    "CreateProcessAsUserW 失败（错误 {}）",
                    last_error()
                )))
            } else {
                Ok(())
            }
        });
        close_handle(token);
        match created {
            Ok((pi, pipes)) => {
                let mut child = OsChild::OsChild {
                    pi,
                    stdout_read: pipes.read_stdout,
                    stderr_read: pipes.read_stderr,
                };
                let info = child.wait().expect("等待应成功");
                let stdout = String::from_utf8_lossy(&info.stdout);
                let stderr = String::from_utf8_lossy(&info.stderr);
                eprintln!(
                    "plumbing exit={} stdout={stdout:?} stderr={stderr:?}",
                    info.exit_code
                );
                assert!(
                    stdout.contains("out-token"),
                    "受限令牌路径必须捕获 stdout：stdout={stdout:?} stderr={stderr:?}"
                );
                assert!(
                    stderr.contains("err-token"),
                    "受限令牌路径必须捕获 stderr：stderr={stderr:?}"
                );
            }
            Err(error) => panic!("受限令牌创建失败：{error}"),
        }
    }
}

/// 结构布局断言（与 Windows SDK 一致；防 ABI 漂移）。
pub(crate) fn assert_struct_layouts() -> bool {
    let mut ok = true;
    if std::mem::size_of::<usize>() == 8 {
        // x64 期望值（与 SDK 编译对齐一致）。
        ok &= std::mem::size_of::<JOBOBJECT_BASIC_LIMIT_INFORMATION>() == 64;
        ok &= std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() == 144;
        ok &= std::mem::size_of::<SYSTEM_MANDATORY_LABEL_ACE>() == 20;
        // TOKEN_MANDATORY_LABEL(16) + SID(12)：结构体因指针对齐补到 32，
        // 但 SetTokenInformation 的长度按 API 公式是 28（见 LowIntegrityLabel::len）。
        ok &= std::mem::size_of::<SID_AND_ATTRIBUTES>() == 16;
        ok &= (std::mem::size_of::<SID_AND_ATTRIBUTES>() + 12) == 28;
        ok &= std::mem::size_of::<STARTUPINFOW>() == 104;
        ok &= std::mem::size_of::<STARTUPINFOEXW>() == 112;
        ok &= std::mem::size_of::<CREDENTIALW>() == 80;
    } else {
        // x86 期望值。
        ok &= std::mem::size_of::<JOBOBJECT_BASIC_LIMIT_INFORMATION>() == 48;
        ok &= std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() == 112;
    }
    ok
}
