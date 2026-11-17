//! OwO Agent 共享契约内核（M8）。
//!
//! ## 内容：五块零出边的自包含数据契约
//!
//! | 模块 | 行数 | 职责 |
//! |---|---:|---|
//! | [`context`] | 32 | 项目规则（AGENTS.md 等）加载 |
//! | [`computer_task`] | 541 | ComputerTask 数据模型与状态机（含敏感 UI 关键词纯函数） |
//! | [`plan`] | 440 | 计划 DAG：校验、拓扑波次、持久化 |
//! | [`skill`] | 226 | 技能清单解析（frontmatter + 正文）与发现 |
//! | [`skill_health`] | 247 | 技能健康度与失败模式记录 |
//!
//! 合计 1,486 行。共同性质：**纯数据 + 纯函数契约**——没有业务状态机、无对外 I/O 副作用、
//! 不依赖 ONNX / Sherpa / rusqlite。
//!
//! ## 为什么它们能整体搬
//!
//! 判据与 M2 相同，但更严：这五个模块的 `crate::` 引用**实测为空集**（注释剥离后扫描），
//! 即真正零出边。因此：
//!
//! * 可独立编译，不需要任何依赖倒置；
//! * core 只需保留同名别名 re-export，调用方零改动；
//! * 不可能形成 crate 环。
//!
//! 这批模块之所以此前排在 M8 之后，是因为它们的入边来源（`agent`、`computer_use`、
//! `goal`、`workswarm`、`tools`、`learn`、`workflow`）当时还在 core 里；随着 M5–M7
//! 把 env / plugins 拆走，**它们的出边自然归零**，于是成为当前代价最低的一步。
//!
//! ## 依赖方向
//!
//! ```text
//! owo-agent-core ──► owo-agent-contracts        （零依赖根，比 kernel 更轻）
//! ```
//!
//! 本 crate 不依赖 kernel / core / server / ONNX / Sherpa / rusqlite，
//! 是整个 workspace 里**最轻的依赖根**（只有 serde 系 + chrono + uuid）。
//!
//! ## 与 `owo-agent-protocol` 的分工
//!
//! * `owo-agent-protocol`：**跨进程线格式**契约（Session/Turn/Item 等，指南 §5.1）；
//! * 本 crate：**进程内**共享的数据契约（Rust 类型）。
//!
//! 二者互补，不重叠。

pub mod computer_task;
pub mod context;
pub mod goal_progress;
pub mod plan;
pub mod skill;
pub mod skill_health;

// 顶层再导出：与拆分前 `owo-agent-core` 的公共面保持完全等价。
// 用 glob 而非手写符号表——M6 的教训（手写会抄错符号名，让编译器来证明等价）。
pub use computer_task::*;
pub use context::*;
pub use goal_progress::*;
pub use plan::*;
pub use skill::*;
pub use skill_health::*;
