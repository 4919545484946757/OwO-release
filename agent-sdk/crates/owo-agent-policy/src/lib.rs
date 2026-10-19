//! 受信策略内核（M12）。
//!
//! 本 crate 是指南 §4 目标结构 `tool-host/crates/policy` 的第一段，承载
//! **"权限判定不可绕过"这条边界的判定侧**（指南 §2.2 / §13）：
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`permissions`] | 授权判定核心：`Level` / `Policy` / `Approver` / `PermissionProfile` / `PermissionRequest`，含 grant 命中、spec 收紧、效应类别参与决策 |
//! | [`permission_spec`] | 四维权限规格（filesystem / command / network / persistence）与其 `nearest_profile` 反推，保证"只收紧不放宽" |
//! | [`grant_store`] | 授权凭证存储：`GrantStore` / `GrantScope`，指纹稳定、按 workspace/host/任务域生效、可撤销、可过期 |
//! | [`tool_effects`] | 工具效应声明与矩阵：`EffectClass` / `ToolEffect`，内置矩阵 + MCP 注解降级 + 未声明即拒绝 |
//! | [`tool_names`] | 工具命名契约（`sanitize_tool_name`）：效应表按名字查表，命名必须与执行侧同源 |
//! | [`mcp_health`] | MCP 工具的健康与熔断：`McpHealthTracker` / `McpHealthConfig`，按**效应类别**判定可重试（只读可重试、写与执行不重试）+ 限流 + 半开探针（M13 从 core 归位：它唯一的出边就是 `tool_effects`，而 `tool_effects` 已随本 crate 下沉） |
//!
//! 边界（见 `docs/ARCH-MICROKERNEL.md` §14）：
//!
//! * 四个模块之间互相引用（`permissions ↔ permission_spec/grant_store/tool_effects`），
//!   **同迁一个 crate**，边不再跨 crate 边界，因此零倒置——与 M3 的
//!   `sandbox ↔ audit_chain`、M11 的 `memory ↔ observe` 是同一条判据；
//! * 对外的两条出边都不成环：`sanitize_tool_name` 随本 crate 下沉（core 的
//!   `tools.rs` 用 `pub(crate) use` 反向引用，可见性与公共面不变），
//!   `McpTool` 按「类型随域走」指向 [`owo_agent_mcp`]；
//! * **普通依赖里不得出现 `owo-agent-core`**：core 依赖本 crate。
//!
//! 本 crate 刻意不依赖 `owo-agent-kernel` / `owo-agent-contracts`：策略判定是
//! 纯逻辑 + serde + 哈希 + 授权凭证文件，不需要内核原语，依赖面越小越好。

pub mod command_guard;
pub mod grant_store;
pub mod mcp_health;
pub mod permission_spec;
pub mod permissions;
pub mod tool_effects;
pub mod tool_names;

// 迁移期约定（与 M0–M11 一致）：用 glob 再导出，让公共面等价性由编译器证明。
pub use command_guard::*;
pub use grant_store::*;
pub use mcp_health::*;
pub use permission_spec::*;
pub use permissions::*;
pub use tool_effects::*;
pub use tool_names::*;
