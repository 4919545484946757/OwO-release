//! OwO Agent Daemon 扩展内核（M2）。
//!
//! ## 为什么是这五个模块
//!
//! 用 Tarjan 强连通分量分析 `owo-agent-core` 的 76 个模块依赖图后，得到 46 个分量；
//! 其中**只有五个模块同时满足“零出边 + 零入边”**——notes、cloud_exec、change_set、
//! change_set_store、automation（合 4,493 行）。这使它们成为整个 core 里唯一一批
//! 「切下来既不依赖别人、也没人依赖」的成规模集合，因此搬迁是**结构性安全**的：
//! core 只需保留同名别名 re-export，调用方零改动，且不可能形成 crate 环。
//!
//! ## 与指南的对应
//!
//! | 模块 | 指南位置 | 说明 |
//! |---|---|---|
//! | [`notes`] | §2.3「Notes/Automation → Daemon 插件」 | “插件接口足够，不必微服务化” |
//! | [`automation`] | 同上 | 定时任务，由 server 的后台循环驱动 |
//! | [`change_set`] / [`change_set_store`] | §2.4 第 4 条事务边界 | 写前快照 + 写入 + diff/revert 的**存储与状态机**侧 |
//! | [`cloud_exec`] | §2.3「Fleet/Cloud Exec → 可选扩展」 | 云端执行传输与任务队列 |
//!
//! ## 依赖方向
//!
//! ```text
//! owo-agent-extensions ──► owo-agent-kernel
//!          ▲                        （只用 cas_store / audit 两个内核模块）
//!          └── owo-agent-core 反向引用（仅 re-export，用于兼容既有路径）
//! ```
//!
//! 本 crate **不依赖** `owo-agent-core`、`owo-agent-server`，也不依赖 ONNX/Sherpa。
//!
//! ## 注意：change_set 的事务边界完整性
//!
//! 指南 §2.4 要求「文件写前快照、写入、diff 和 revert」保持在一个拥有者内。
//! 本 crate 只承载其中的**快照与恢复的状态机/存储**（`change_set` / `change_set_store`）；
//! 真正的文件写入仍由 core 的 `executor` / `tools` 与未来的 Tool Host 执行。
//! 这一点在迁到 Tool Host（指南 §9 A4）时必须整体复核，不要只搬一半。

pub mod automation;
pub mod change_set;
pub mod change_set_store;
pub mod cloud_exec;
pub mod notes;

mod atomic_io;
pub(crate) use atomic_io::atomic_write;

// 顶层再导出：与拆分前 `owo-agent-core` 的公共面保持兼容（glob 形式避免漏项，
// core 侧原有的逐条 `pub use` 继续照旧工作）。
pub use automation::*;
pub use change_set::*;
pub use change_set_store::*;
pub use cloud_exec::*;
pub use notes::*;
