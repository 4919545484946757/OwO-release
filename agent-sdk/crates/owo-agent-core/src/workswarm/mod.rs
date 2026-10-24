// R13:S0 编排层（WorkSwarm 最小协同闭环）
//! WorkSwarm 编排内核（Part 6 / S0：§6.1 编排内核、§6.6 Project Space、§6.7 团队经验、§9.0 完成标准）。
//!
//! 复用既有原语，不新建调度器：
//! - 任务图调度复用 [`crate::goal::GoalRunner`]：按「阶段」执行（已完成步骤永不重跑）；
//! - 产物经 [`crate::cas_store::CasStore`] 内容寻址落盘，**以 ref 传递**（版本化 Artifact）；
//! - ProjectSpace / Artifact / DecisionRecord / HandoffRecord / TeamRun 持久化复用
//!   [`crate::project_space_store`]；
//! - 跨成员消息经 [`crate::fleet::AgentBus`]（correlation_id 贯通父子，A3 语义）；
//! - 审计经 [`crate::audit::AuditLog`]（关键动作全部落审计，含 correlation_id）。
//!
//! S0 范围（本模块实现）：
//! - 组队：`single | team | swarmflow`；模板优先（[`TeamTemplateRegistry`]），动态组队 ≤5 个 Agent；
//! - 接力：步骤完成 → 版本化 Artifact + 结构化 [`HandoffRecord`]（context slice，A3）；
//! - 人节点：运行任务开「门闩」等待（cancel 可中断），结果录入后自动唤醒下游；
//! - steer：`continue / steer / replace / cancel / retry`——只改未完成节点，已完成产物永不丢失；
//!   变更必须留下 [`DecisionRecord`]（结论不留在聊天里）；
//! - 成功收尾：生成交付清单 + [`TeamTemplateProposal`]（**只提案，不自动启用**；
//!   用户采纳后才进 TeamTemplateRegistry）。
//!
//! 运行分工（三个独立关注点）：
//! - **监督**：[`supervisor`] 是宿主侧、确定性的进度监督器。它只读取持久化步骤、
//!   当前阶段领取记录和中断标记，输出
//!   `on_track/stalled/blocked/idle/complete/cancelled`、中断恢复提示和下一步建议；
//!   不调用模型、不放宽权限、不把"进程结束"误判成交付验收；
//! - **执行**：`task_worker` 能力节点由 `task_graph` 按依赖、工具面、写范围和估算
//!   工作量动态分配；执行结果回到 Artifact + Handoff + DeliveryGate；
//! - **对抗评审**：`review`/`critic` 只读节点独立复核，自动 reviewer 只在存在源码/
//!   高风险信号时加入，发现问题走有界 rework，而不是由执行者自评通过。
//!
//! 三条链路共享同一 TeamRun/审计/进度事件，但职责不互相替代：监督器不会替 Worker
//! 执行，评审器也不会替监督器宣布成功。
//!
//! 并发模型：
//! - 每个 team 一个运行任务（server 侧 tokio 任务）顺序驱动阶段循环；
//! - 磁盘状态变更临界区由 per-team 异步锁串行化（阶段边界 / steer / handoff / human-result）;
//! - `run-active` 标志只在阶段执行期间为真；人节点等待窗口为「暂停」，
//!   该窗口内 steer 直接改盘（下一阶段读取最新状态），阶段执行中 steer → Conflict(409)。

use async_trait::async_trait;
use owo_agent_protocol::{
    Artifact, ArtifactClassification, ArtifactValidation, DecisionRecord, HandoffRecord,
    MemberHealth, ProjectSpace, ProjectSpaceStatus, ReviewState, RuntimeBinding, TeamMember,
    TeamMode, TeamRun, TeamRunStatus, TeamTemplate, TeamTemplateProposal,
    TeamTemplateProposalStatus, TeamTemplateRole,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::artifact_pipeline::{
    effective_format, evidence_refs_of, file_name_of, media_type_of, validate_artifact_content,
};
use crate::audit::AuditLog;
use crate::cas_store::CasStore;
use crate::fleet::{new_correlation_id, AgentBus, MessageKind, OverflowPolicy};
use crate::goal::{
    Goal, GoalRunState, GoalRunner, GoalStatus, RunnerConfig, Worker, WorkerRegistry,
};
use crate::plan::{verify_output, Plan, StepSpec, StepStatus};
use crate::project_space_store::{ProjectSpaceStoreBackend, ProjectSpaceStoreError};
use crate::workswarm_output::WorkerOutputV1;

mod error;
mod registry;
mod roles;
mod run_state;
mod types;
mod util;

#[cfg(test)]
mod tests;

pub use error::*;
pub use registry::*;
pub use roles::*;
pub use types::*;
use util::*;

mod artifact_catalog;
mod artifact_read;
pub use artifact_read::DependencyArtifactRead;
mod context_snapshot;
mod context_timing;
mod coord_accessors;
mod coord_admission;
mod coord_artifacts;
mod coord_assignment;
mod coord_context;
mod coord_context_slice;
mod coord_create;
mod coord_delivery_gate;
mod coord_finalize;
mod coord_handoff;
mod coord_human;
mod coord_identity;
mod coord_lifecycle;
mod coord_load;
mod coord_progress;
mod coord_review;
mod coord_rework;
mod coord_run;
mod coord_steer;
mod coord_strategy;
mod delivery_gate_evidence;
mod phase_subplan;
mod review_evidence;
mod role_worker;
mod supervisor;
mod task_graph;
mod task_graph_policy;
mod task_profile;

pub use role_worker::*;
pub use supervisor::*;
pub use util::{is_review_role, is_review_role_name};
/// 阶段领取记录（进程内；进度视图 current_steps 的数据源）。
#[derive(Debug, Clone)]
pub(crate) struct PhaseClaim {
    pub(crate) epoch: u64,
    pub(crate) steps: Vec<ProgressStep>,
}

/// WorkSwarm 协调器：组队、阶段执行、接力注册、人节点、steer、模板提案。
///
/// 共享方式：`Arc<TeamCoordinator>`（内部状态均带锁；Clone 成本 = 若干 Arc）。
#[derive(Clone)]
pub struct TeamCoordinator {
    pub(crate) store: Arc<dyn ProjectSpaceStoreBackend>,
    pub(crate) templates: Arc<TeamTemplateRegistry>,
    pub(crate) cas: CasStore,
    pub(crate) bus: AgentBus,
    pub(crate) audit: Option<Arc<Mutex<AuditLog>>>,
    pub(crate) run_dir: PathBuf,
    /// 动态团队 Agent 成员上限（§6.1：默认不超过 5 个 Agent）。
    pub(crate) max_agent_members: usize,
    /// per-team 状态锁（磁盘状态变更临界区串行化）。
    pub(crate) team_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// per-team 运行中标志（阶段执行期间为真；人节点等待窗口为假 = 暂停）。
    pub(crate) run_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// per-team 运行循环存活标志（server 运行循环进程内声明；重启后为空 =
    /// 磁盘 Running 但无循环 → 可识别为 interrupted）。
    pub(crate) loop_alive: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// per-team 取消令牌。
    pub(crate) cancels: Arc<Mutex<HashMap<String, Arc<CancelToken>>>>,
    /// per-team 阶段代次（进程内单调）：领取阶段读取，cancel/retry/replace 等转向时 +1；
    /// 旧阶段的合并与产物回传凭代次校验，过期即丢弃（只记审计，不改状态）。
    pub(crate) phase_epochs: Arc<Mutex<HashMap<String, u64>>>,
    /// 当前阶段领取信息（进度视图 current_steps 数据源；每团队至多一条）。
    pub(crate) phase_claims: Arc<Mutex<HashMap<String, PhaseClaim>>>,
    /// per-team 进度事件序号（进程内单调；状态转移时 +1）。
    pub(crate) progress_seqs: Arc<Mutex<HashMap<String, u64>>>,
    /// Last emitted supervisor health per team, so only transitions enter the audit stream.
    pub(crate) supervisor_states: Arc<Mutex<HashMap<String, String>>>,
    /// Per-team watch channel for event-driven runtime waits (human handoff, progress).
    pub(crate) progress_watchers: Arc<Mutex<HashMap<String, tokio::sync::watch::Sender<u64>>>>,
    /// Host-bound workspace roots used only by the registered read-only Validator lane.
    pub(crate) verification_workspaces: Arc<Mutex<HashMap<String, PathBuf>>>,
    /// Bounded one-shot index for matching context assembly timings to Worker spans.
    context_assembly_timings: Arc<Mutex<context_timing::ContextAssemblyTimingStore>>,
    /// Cross-Worker bounded cache of immutable, hash-verified CAS artifact previews.
    artifact_preview_cache: Arc<Mutex<artifact_read::SharedArtifactPreviewCache>>,
    /// Bounded immutable inputs shared by Workers in the active Team phase.
    phase_context_snapshots: Arc<Mutex<context_snapshot::PhaseContextSnapshotCache>>,
}

/// 产物登记载荷（七期 · 第三路：legacy 纯文本 / 契约 V1 两条路径的统一内部形状）。
///
/// `open_issues` / `known_risks` 为 `None` 时按 legacy 语义从内容反解析
/// （[`TeamCoordinator::parse_optional_json_lists`]）；为 `Some` 时以结构化值为准。
/// `validation` 为 `None` 表示未做格式门控（legacy 路径，空内容照旧登记）；
/// 为 `Some` 表示登记前已按 [`crate::artifact_pipeline::validate_artifact_content`]
/// 通过门控。
#[derive(Debug, Clone)]
pub(crate) struct StepOutput {
    /// 交付物正文（CAS 内容本体）。
    pub(crate) content: String,
    /// 版本链 / 评审口径的产物分类（角色链 kind）。
    pub(crate) kind: String,
    /// 落盘内容格式（有效校验格式）。
    pub(crate) format: String,
    /// 下载交付 media type。
    pub(crate) media_type: String,
    /// 下载交付文件名。
    pub(crate) file_name: String,
    /// Worker 证据引用链（Artifact.evidence_refs / HandoffRecord.evidence_refs 同源）。
    pub(crate) evidence_refs: Vec<String>,
    pub(crate) open_issues: Option<Vec<String>>,
    pub(crate) known_risks: Option<Vec<String>>,
    pub(crate) validation: Option<ArtifactValidation>,
    /// Worker 交接说明原文（WorkerOutputV1.handoff；critic 为评审结论）。
    pub(crate) handoff_note: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OutputAttemptBinding<'a> {
    pub(crate) phase_epoch: Option<u64>,
    pub(crate) attempt_id: Option<&'a str>,
    /// Host context snapshot actually supplied to this reviewer invocation.
    pub(crate) reviewed_sources: Option<&'a [serde_json::Value]>,
}
