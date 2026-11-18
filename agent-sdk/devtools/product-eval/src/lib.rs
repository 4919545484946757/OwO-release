//! OwO Agent ProductEval 底座（**开发工具包**，不进用户默认运行时）。
//!
//! ## 为什么单独成 crate
//!
//! 指南 §1.1 把 `product_eval.rs`（3,317 行）列为必须从生产基础链移出的四个大文件
//! 之一，§3 的目标位置是 `devtools/eval/`，并明确要求“从 release sidecar 和启动链删除”。
//! 本 crate 是该迁移的 Rust 侧落地：评测、统计、报告与数据集构建留在 Rust（因为要真实
//! 跑 Agent/Turn/工具/权限），但以独立 crate + 可选 feature 的形式与基础聊天启动链解耦。
//!
//! ## 依赖方向
//!
//! ```text
//! owo-agent-product-eval ──► owo-agent-core ──► owo-agent-kernel
//!          ▲
//!          └── 仅由 core 的 `product-eval` feature 反向引入（默认开启；
//!              `--no-default-features` 即彻底移出编译闭包与启动链）
//! ```
//!
//! 本 crate 不依赖 `owo-agent-server`，也不注册任何 HTTP 路由；服务端的
//! `/product-eval/*`、`/eval/*` 路由在 `owo-agent-server` 侧，由 feature 决定是否挂载。
//!
//! ## 模块
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`product_eval`] | 任务与 suite schema、输入校验、检查器、指标聚合和兼容 API 门面 |
//! | [`product_eval::paired_report`]（内部模块；现有 API 继续由 `product_eval` 重导出） | 配对报告 schema、Single/Team 单元对齐、任务聚类统计与比较 JSON |
//! | [`product_eval::freeze`]（内部模块；现有 API 继续由 `product_eval` 重导出） | suite 任务、权限、预算、模型与版本的冻结清单及漂移校验 |
//! | [`product_eval::matrix_runner`]（内部模块；现有 API 继续由 `product_eval` 重导出） | 矩阵计划、顺序配对执行、journal 续跑与逐单元报告 |
//! | [`product_eval::single_agent`] | Route 1 真实单 Agent 执行器（审批器 + 范围工具） |
//! | [`product_eval::workswarm_executor`] | Route 2 WorkSwarm 真实 TeamRun 适配器（与 Route 1 同任务定义/同权限/同预算） |
//! | [`eval`] | 面向工程回归的内置 demo 单轮套件（`builtin_suite` / `run_suite`） |
//! | [`dataset_builder`] | 把 transition trace 清洗为可版本化训练数据集 |
//!
//! `workswarm_executor` 在 core 中曾以 `#[path = "product_eval/workswarm_executor.rs"]`
//! 挂在 crate 根（模块名 `product_eval_workswarm`）。迁移后它是 [`product_eval`] 的正式
//! 子模块，core 侧通过 `owo_agent_core::product_eval_workswarm` 别名维持原路径不变。

pub mod dataset_builder;
pub mod eval;
#[path = "product_eval/external_drivers.rs"]
pub mod external_drivers;
pub mod product_eval;

// 顶层再导出：与拆分前 `owo-agent-core` 的 `pub use` 面保持兼容。
//
// `product_eval` 用 glob：该模块有 70+ 公共条目（含统计、冻结点、配对报告），逐条列举
// 既容易漏项，也会让“迁移即改公共 API”的风险凭空出现。glob 让迁移后的公共面与拆分前
// **完全等价**，由编译器验证；core 侧原有的逐条 `pub use product_eval::{...}` 仍照旧工作。
pub use dataset_builder::{
    build_dataset, load_manifest, save_manifest, BuildResult, DatasetBuilderConfig,
    DatasetManifest, RejectReason, Rejection,
};
pub use eval::{builtin_suite, eval_suite_path, run_suite, EvalCase, EvalReport, EvalSuite};
pub use product_eval::*;

/// WorkSwarm 适配器别名：core 侧原模块名为 `product_eval_workswarm`（`#[path]` 挂在根），
/// 这里保留同名别名，使 `owo_agent_core::product_eval_workswarm::*` 与
/// `owo_agent_product_eval::product_eval_workswarm::*` 都继续有效。
pub use product_eval::workswarm_executor as product_eval_workswarm;
pub use product_eval::workswarm_executor::{
    ArtifactObservation, StrategyObservation, TeamRunObservation, WorkSwarmExecutor,
    WorkSwarmExecutorConfig, WorkerObservation,
};
