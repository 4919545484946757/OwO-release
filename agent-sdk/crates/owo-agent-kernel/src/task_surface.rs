//! 桌面交互面契约（`TaskSurface`）。
//!
//! ## 为什么要下沉到内核（M5 的依赖倒置）
//!
//! 这个 trait 原本定义在 `owo-agent-core::computer_use`。M5 要把 `desktop_env`
//! （2,442 行的环境/状态机）外迁成独立 crate，而 `desktop_env` 对它**只有一条出边**：
//! `SurfaceEnvAdapter<S: crate::computer_use::TaskSurface>`。
//!
//! 按 ARCH §4 的判据：`desktop_env` 与 `computer_use` **不同迁**，且
//! `computer_use` 反向引用 `desktop_env`？——实测**没有**这条反向边。但即便如此，
//! 只要 `desktop_env` 依赖 `computer_use`，外迁后就会形成
//! `desktop_env → computer_use` 与 core 引用 `desktop_env` 的环。因此这条边必须倒置。
//!
//! 倒置方式：把**纯 I/O 契约**下沉到内核。trait 只用到 `&str` / `i32` /
//! `serde_json::Value`，不依赖任何业务类型，因此放内核不引入"某个执行器的数据类型"
//! （对比 ADR-001 里被否决的、把 `SandboxAuditEvent` 下沉内核的方案）。
//!
//! 边界归属说明：`TaskSurface` 是"观察 + 注入动作"的平台 I/O 抽象，与内核已有的
//! [`crate::platform`] 同类——多个运行边界都要用它，且它不绑定业务状态机。
//! 实现体（`SimTaskSurface` / `RealTaskSurface`）仍留在 `owo-agent-core::computer_use`，
//! 内核只承载契约。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一次窗口身份观察（Agent V2 §5 任务 1.1）。
///
/// 每个有副作用的桌面动作执行前都要重新采集并比对：应用标识之外还要包含
/// PID 与窗口句柄，避免"进程已重启但标题相同"被当成同一目标。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowIdentity {
    /// 进程名（小写、去掉 `.exe`）。
    pub app: String,
    /// 进程 ID。与 `hwnd` 一起用于识别 PID 复用。
    pub pid: u32,
    /// 顶层窗口句柄（非 Windows 平台为 0）。
    pub hwnd: i64,
    /// 窗口标题。
    pub title: String,
    /// 采集时是否为前台窗口。
    pub foreground: bool,
    /// 采集时间（RFC3339）。
    pub observed_at: String,
}

impl WindowIdentity {
    /// 两个观察是否指向同一个窗口实例（应用、PID、句柄都要一致）。
    pub fn same_window(&self, other: &Self) -> bool {
        self.app == other.app && self.pid == other.pid && self.hwnd == other.hwnd
    }
}

/// 感知闭环执行所需的桌面面抽象：感知（OCR 版面）与动作注入。
///
/// 运行环境用 `SimTaskSurface`（owo-sim-qq）；契约测试注入内存 Mock，
/// 使闭环在无网络/无真实桌面时完整可测。
///
/// 采用 `#[async_trait]` 以保持与 `owo-agent-core` 既有实现（`RealTaskSurface` /
/// `SimTaskSurface`）的 ABI 兼容：实现侧无需改动。
/// HWND/PID/name/AutomationId binding for a uniquely observed UIA edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiaEditIdentity {
    pub hwnd: i64,
    pub pid: u32,
    pub name: String,
    pub automation_id: String,
    pub native_window_handle: i64,
}

#[async_trait::async_trait]
pub trait TaskSurface: Send {
    /// 当前前台应用标识（用于目标应用匹配）。
    fn app(&self) -> String;
    /// 当前窗口身份快照（Agent V2 §5 任务 1.1）。
    ///
    /// 默认返回 `None`：内存 Mock 与模拟面没有真实窗口句柄。真实桌面实现必须覆盖，
    /// 否则动作前的身份复核会退化为"只比较应用名"。
    fn window_identity(&self) -> Option<WindowIdentity> {
        None
    }
    /// 当前 OCR 版面（screen_ocr 同构：lines 数组，每行 text/x/y/width/height/role_hint）。
    async fn ocr(&mut self) -> Result<Value, String>;
    async fn ocr_window(&mut self, _hwnd: i64) -> Result<Value, String> {
        self.ocr().await
    }
    async fn accessible_anchors(&mut self, _hwnd: i64, _name: &str) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }
    /// 与 `accessible_anchors` 同一窗口/名称绑定的受控 UIA 观察。
    /// OCR 为空但已解析到唯一、可写、非密码控件时，作为 `set_value` 的观察证据。
    async fn accessible_observation(
        &mut self,
        _hwnd: i64,
        _name: &str,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }
    async fn click(&mut self, x: i32, y: i32) -> Result<(), String>;
    async fn type_text(&mut self, text: &str) -> Result<(), String>;
    /// The identity is bound by the runner's approved UIA observation; the
    /// implementation must verify it and return the exact ValuePattern readback.
    async fn set_value(
        &mut self,
        _identity: &UiaEditIdentity,
        _value: &str,
    ) -> Result<String, String> {
        Err("This surface cannot set a named edit value".into())
    }
    async fn key(&mut self, key: &str) -> Result<(), String>;
    async fn launch(&mut self, target: &str) -> Result<(), String>;
}
