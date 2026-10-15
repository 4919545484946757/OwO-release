//! OwO 输入法 Agent IPC v3 适配层（`owo-agent serve-ime` 的支撑库）。
//!
//! 模块规划（按施工顺序）：
//! - [`protocol`]：协议 v3 类型与校验（E1.1，已完成）
//! - [`frame`] / [`pipe`]：4 字节小端长度帧 + 命名管道服务端（E1.2，已完成）
//! - [`commands`] / [`state`]：响应构造器 + 会话与任务状态机（E1.3，已完成）
//! - [`sse`] / [`http`] / [`bridge`]：SSE 解析 + HTTP 回环 + turn 异步适配（E1.4）
//! - 命令生成扩展：turn 结果 → 候选的完整映射（E1.5）
//!
//! 协议规范：`OwO-release/docs/plugins/agent-ipc-integration.md` 与
//! `OwO-release/docs/plugins/agent-ipc-v3.schema.json`；
//! 行为参考实现：`OwO-release/apps/agent_mock/main.cpp`。

pub mod bridge;
pub mod commands;
pub mod frame;
pub mod http;
pub mod protocol;
pub mod sse;
pub mod state;

#[cfg(windows)]
pub mod pipe;

pub use bridge::{compose_prompt, run_turn_task, ImeBridge, TurnAccumulator};
pub use http::{BridgeError, OwoHttpClient};
pub use protocol::{
    Action, AgentIpcRequest, AgentIpcResponse, ApplicationView, Command, ContextEntry, InputView,
    PinyinRange, PrivacyFlags, ProtocolError, RiskLevel, SlotUpdate, Status, TaskDraft, TaskSlot,
    AGENT_PLUGIN_ID, AGENT_PLUGIN_SERVICE, AGENT_PROTOCOL_VERSION, MAXIMUM_AGENT_COMMANDS,
    MAXIMUM_AGENT_CONTEXT_ENTRIES, MAXIMUM_AGENT_PAYLOAD_BYTES, MAXIMUM_AGENT_SLOT_UPDATES,
    MAXIMUM_AGENT_TASK_RANGES, MAXIMUM_AGENT_TASK_SLOTS, MINIMUM_AGENT_PROTOCOL_VERSION,
    SUPPORTED_CAPABILITIES,
};
pub use sse::{SseFrame, SseParser};
pub use state::{
    CommandAction, ImeState, PendingHandle, PendingSlot, PermissionNotice, StartTurnRequest,
    StateAction, DEFAULT_SESSION_TTL,
};

#[cfg(windows)]
pub use pipe::{
    clamp_timeout_ms, run_pipe_server, validate_pipe_name, FrameHandler, PipeError,
    DEFAULT_OP_TIMEOUT_MS, DEFAULT_PIPE_NAME,
};
