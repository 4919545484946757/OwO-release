//! 统一 Task Runtime（Agent V2 Phase 2/3）。
//!
//! 组织方式：
//! - [`model`]：Task / Event / Checkpoint / Artifact / Approval 契约与稳定错误码；
//! - [`state_machine`]：唯一的状态迁移校验入口；
//! - [`store`]：SQLite Task Store（任务、事件、检查点、审批、产物、幂等键）；
//! - [`runtime`]：创建/启动/暂停/恢复/取消/审批/验证/启动恢复，以及执行后端契约。
//!
//! 边界：本模块不反向依赖 server / desktop；执行后端通过 `TaskExecutionBackend`
//! trait 注入，不能把内部状态直接写进 Task 终态。

pub mod model;
pub mod runtime;
pub mod state_machine;
pub mod store;

pub use model::*;
pub use runtime::*;
pub use state_machine::{can_transition, ensure_transition};
pub use store::TaskStore;
