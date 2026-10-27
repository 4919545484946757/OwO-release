//! §12 回合执行域（turn）API 模块。
//!
//! 提取证明：自 `lib.rs` 逐字迁移 —— turn 编排器（附件注入/会话锁/并发许可/
//! 预算硬熔断/审批通道/SSE 事件流/trace 与用量归集）、respond_permission
//! （§5.4 审批响应 + Grant 临时授权）、ChannelApprover（Approver 实现，
//! 300s 截止 + 中止感知）、auto_approve_enabled、to_sse/to_event（协议 SSE
//! 映射，R10 统一携带 v 字段）。路由路径与 OpenAPI 登记零变化。
//!
//! `acquire_session_lock` 留守 lib.rs（session_api 八处引用的会话域共享基建）。
//! 本模块引用兄弟域（session_api/usage/audit_api/event_stream/logging）经
//! `crate::` 路径，故不可作 #[path] 独立编译目标（仅库内编译）。

mod approval;
mod handlers;
mod input_preflight;
mod queue;
mod wire;

#[cfg(test)]
mod tests;

pub(crate) use approval::*;
pub(crate) use handlers::*;
pub(crate) use input_preflight::input_preflight;
