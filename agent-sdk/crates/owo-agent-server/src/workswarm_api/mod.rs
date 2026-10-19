//! WorkSwarm S0 HTTP 路由（§8.5 / §9.0 S0）。
//!
//! 路由面：
//! - `POST /teams` 创建 single/team/swarmflow 运行（后台运行循环驱动阶段推进）；
//! - `GET /teams` 团队运行列表（含运行中标志）；
//! - `GET /teams/{id}` 成员、预算、状态（+ 任务视图 + 审计尾迹）；
//! - `GET /teams/{id}/tasks` TeamRun 的任务图（步骤 × 状态）；
//! - `GET /teams/{id}/events` 团队事件流 SSE（审计重放 + 状态轮询；`?format=json` 一次性快照，
//!   五期起快照含 `progress`——轮询降级也能看到当前步骤）；
//! - `GET /teams/{id}/metrics` TeamRun 指标（角色 span 聚合 / token·费用 / 最慢 Worker /
//!   失败·返工次数 / Artifact 版本数 / 预算状态；五期 · 第三路）；
//! - `GET /teams/{id}/diagnostic` 脱敏诊断导出（TeamRun/任务/产物/评审/交接/指标/审计；
//!   凭据类键值与令牌已脱敏、超长文本截断；五期 · 第三路）；
//! - `POST /teams/{id}/steer` continue/steer/replace/cancel/retry（R2：retry 局部重试 + 中断恢复）；
//! - `GET  /teams/{id}/change-sets` 团队 ChangeSet 列表 + 批准门控状态（八期 · 二路）；
//! - `GET  /change-sets/{id}` 单个 ChangeSet；
//! - `POST /change-sets/{id}/accept|reject|revert` 接受/拒绝/撤销（reject/revert
//!   安全恢复：用户改过的文件 409 + conflicted，不覆盖；八期 · 二路）；
//! - `GET /projects/{id}` Project Space 摘要；
//! - `GET /projects/{id}/artifacts` 版本化共享产物（ref 列表）；
//! - `GET /projects/{id}/workspace/changes` Worker 代码变更追踪（七期 · 二路：
//!   变更文件 + diff 摘要 + 逐步骤记录 + 白名单越界原因）；
//! - `POST /tasks/{id}/handoff` 结构化接力（team_id 在请求体）；
//! - `POST /tasks/{id}/human-result` Human 节点提交结果（team_id 在请求体）；
//! - `GET /teams/templates` 已采纳模板；
//! - `GET /teams/templates/proposals` 模板提案（只提案，不自动启用）；
//! - `POST /teams/templates/proposals/{proposal_id}/adopt` 采纳提案；
//! - `POST /teams/templates/proposals/{proposal_id}/reject` 拒绝提案（保留记录，可审计）。
//!
//! 五期（第三路）指标接线：`build_run_registry` 用 `MeasuredRoleWorker` 包装每个
//! `RoleWorker`（span 指标 JSONL 落盘 TeamRun 数据目录，重启可读）；`run_team_loop`
//! 在每阶段前做指标预算门（`TeamRun.budget` additive 支持 max_cost_usd / max_wall_secs / max_model_calls，
//! 超限停止调度下一阶段并留审计）。指标/脱敏实现见子模块 [`workswarm_metrics`]。
//!
//! 七期（第二路）权限接线：`build_run_registry` 按角色画像（`WorkerProfile`）装配
//! 每个角色的工具注册表（注册表面即权限边界，模板 `budget_calls_per_role` → 真实
//! `max_turns`）；写角色经单写租约互斥，执行前后 git 快照 → 变更摘要/diff ref 落盘
//! （子模块 [`workspace_change_tracker`]；白名单越界 → `scope_violation` 步骤失败）；
//! 团队取消令牌经桥接任务置位共享 abort 标志，运行中 Worker 协作即时中断。
//!
//! 十一期（二路）并行开发接线：角色可声明 `model`（每角色独立模型）与 `write_paths`
//! （角色写范围）——写范围互不重叠的写角色经 [`write_lease`] 范围租约**并发落盘**，
//! 未声明范围的写角色保持原单写者语义（全局互斥）；范围归属过滤避免并发窗口把
//! 其他写者的变更误判为本步骤越界（见 [`workers::TrackedRoleWorker`]）。
//!
//! S0 边界：产物经 CAS ref 传递（大对象不进响应体）；agent 角色经
//! `Agent::run_subagent` 模型驱动；内置 echo/sleep/fail worker 供测试与演示。

// 六期（第二路）：项目工作区绑定（真实目录 / 只读 / 写白名单）。
// 独立文件，经本模块 router 合并挂载（子模块可访问本模块私有项）。
#[path = "../project_workspace_api.rs"]
pub(crate) mod project_workspace;

// 七期（第二路）：工作区变更追踪（Worker 执行前后 git 快照 + 写白名单校验 +
// 变更摘要/diff ref 落盘；经本模块 router 挂载读取面）。
#[path = "../workspace_change_tracker/mod.rs"]
pub(crate) mod workspace_change_tracker;

// 八期（第二路）：ChangeSet 审批、接受与安全撤销（独立文件，经本模块 router
// 合并挂载；子模块可访问本模块私有项：error_response 与 project_workspace）。
#[path = "../change_set_api.rs"]
pub(crate) mod change_set_api;

// 七期（第二路）：角色画像（工具面/只读/写白名单/回合上限）驱动真实 Worker。

// 五期（第三路）：指标/预算/脱敏实现（#[path] 子模块声明 = 零 lib.rs 接线，
// 物理文件 crates/owo-agent-server/src/workswarm_metrics.rs；接线归第四路，
// 其后续若要在 crate 根登记 `pub mod workswarm_metrics;` 可平移，无语义差异）。
#[path = "../workswarm_metrics/mod.rs"]
pub mod workswarm_metrics;

mod dto;

// 统计模块等兄弟模块需要读取团队审计尾部（同源实现，避免复制）。
pub(crate) use dto::audit_tail;
mod handlers;
mod registry_builder;
mod runtime;
mod state;
mod workers;
/// 十一期（二路）：范围写租约——声明写范围的写角色可并发落盘（范围不重叠时）。
mod write_lease;

/// Hold the same workspace-wide lease used by Team writers while DeliveryGate
/// hashes workspace subjects and commits its accepted manifest. This prevents
/// another in-process TeamRun from changing checked files between validation
/// and delivery publication.
pub(crate) async fn acquire_workspace_delivery_lease(
    workspace_root: &std::path::Path,
) -> write_lease::WriteLeaseGuard {
    write_lease::WriteLease::new(
        write_lease::manager_for_workspace(workspace_root),
        write_lease::WriteScope::global(),
    )
    .acquire()
    .await
}

#[cfg(test)]
mod tests;

pub(crate) use dto::error_response;
pub use handlers::*;
pub(crate) use runtime::run_team_loop;
pub(crate) use state::WorkSwarmState;
