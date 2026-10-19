//! Agent SDK 公开线协议（v1 契约，HTTP JSON + SSE 事件）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// WorkSwarm 协同 DTO（§6.1 / §6.6）
// ---------------------------------------------------------------------------

/// 团队运行模式（对应技术文档 §6.1 三种形态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMode {
    /// 单一角色、短任务。
    Single,
    /// 角色明确、需要接力或评审。
    Team,
    /// 高频稳定流程、版本化模板固化。
    Swarmflow,
}

/// 团队成员运行时绑定（agent / human / worker）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeBinding {
    Agent { agent_id: String },
    Human { user_id: String },
    Worker { worker_name: String },
}

/// 成员健康状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberHealth {
    Active,
    Degraded,
    Offline,
    Fused,
}

/// 团队成员（§6.1 TeamMember）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub member_id: String,
    pub role: String,
    pub runtime_binding: RuntimeBinding,
    /// 能力声明（工具/感知/模型等）。
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 工具作用域白名单。
    #[serde(default)]
    pub tool_scope: Vec<String>,
    /// 可读资源范围。
    #[serde(default)]
    pub read_scope: Vec<String>,
    /// 可写资源范围。
    #[serde(default)]
    pub write_scope: Vec<String>,
    /// 预算限制（JSON 自由结构，由调度层解释）。
    #[serde(default)]
    pub budget: Value,
    /// 交接契约描述（下游期望的产物格式/语义）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_contract: Option<String>,
    #[serde(default = "default_member_health")]
    pub health: MemberHealth,
}

fn default_member_health() -> MemberHealth {
    MemberHealth::Active
}

/// 团队运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRunStatus {
    Created,
    Running,
    AwaitingHuman,
    Succeeded,
    Failed,
    Cancelled,
}

impl TeamRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// 团队运行（§6.1 TeamRun）。
///
/// Leader 产出的结构化协作计划，不是自然语言指令。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRun {
    pub team_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub mode: TeamMode,
    pub members: Vec<TeamMember>,
    /// 任务图引用（RunGraph id 或内联 DAG；具体载体由编排层决定）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_graph_ref: Option<String>,
    /// 关联 Project Space。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_space_id: Option<String>,
    /// 团队组建来源：命中的 TeamTemplate id（None = 动态组队）。
    /// 溯源字段：让用户看到「团队为什么被创建」（§9.0 S0 完成标准）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    /// 共享上下文引用（CAS / artifact refs）。
    #[serde(default)]
    pub shared_context_refs: Vec<String>,
    /// 全局预算（JSON 自由结构）。
    #[serde(default)]
    pub budget: Value,
    /// 人机协作策略（如 human_approval_required / auto_continue）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_policy: Option<String>,
    /// 五期：组队策略决策（TeamPlan 序列化：mode/requested/roles/parallelism/
    /// budget_calls_total/json_repair/reasons）。`auto` 判定理由供 UI 直接渲染；
    /// 旧记录反序列化为 None（additive，不破坏既有 wire）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_decision: Option<Value>,
    pub status: TeamRunStatus,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// 产物分类（§6.6 Artifact）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactClassification {
    Public,
    Private,
    Sensitive,
}

/// 产物评审状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Draft,
    PendingReview,
    Approved,
    Rejected,
    Superseded,
}

/// 共享产物（§6.6 Artifact）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_id: String,
    /// 产物类型（document / code / dataset / review / decision 等）。
    pub kind: String,
    pub version: u32,
    /// 生产者 member_id。
    pub producer: String,
    /// 内容引用（CAS hash / 文件路径 / URL）。
    pub content_ref: String,
    /// Schema 引用（可选，用于结构化校验）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_ref: Option<String>,
    /// 来源引用（trace / artifact / note ids）。
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default = "default_artifact_classification")]
    pub classification: ArtifactClassification,
    #[serde(default = "default_review_state")]
    pub review_state: ReviewState,
    /// 版本链：本版本所取代的上一版 artifact_id（首版为 None）。
    /// 评审闭环（R2）用于把同 kind 的历次产出串成可追溯链。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_artifact_id: Option<String>,
    pub created_at: String,
    // -- 七期（第三路）Artifact 交付扩展（全部 additive：旧记录 serde 缺省反序列化） --
    /// 所属团队运行（七期 additive；旧记录缺省为空）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub team_id: String,
    /// 创建此产物的任务身份；旧产物为空时按 legacy 处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// 创建此产物的尝试身份；旧产物为空时不能作为新任务验收证据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    /// 内容格式（七期：text|markdown|json|csv|research；旧记录缺省 "text"）。
    #[serde(default = "default_artifact_format")]
    pub format: String,
    /// 下载交付 media type（七期；旧记录缺省 "text/plain"）。
    #[serde(default = "default_artifact_media_type")]
    pub media_type: String,
    /// 下载交付文件名（七期；旧记录缺省为空）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_name: String,
    /// 内容 SHA-256（七期；与 content_ref 的 CAS 哈希一致。旧记录缺省为空，
    /// 服务端可由 content_ref 回退解析）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    /// 内容字节数（七期；旧记录缺省 0，服务端可由 CAS 内容回退计算）。
    #[serde(default)]
    pub size_bytes: u64,
    /// 证据引用链（七期：Worker 证据来源列表；旧记录缺省为空）。
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// 未解决问题（七期：Worker 如实上报；旧记录缺省为空）。
    #[serde(default)]
    pub open_issues: Vec<String>,
    /// 格式校验结果（七期：登记前门控产物；legacy 记录为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ArtifactValidation>,
    /// Worker 交接说明原文（七期：WorkerOutputV1.handoff；旧记录为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}

fn default_artifact_format() -> String {
    "text".to_string()
}

fn default_artifact_media_type() -> String {
    "text/plain".to_string()
}

/// Artifact 格式校验结果（七期 · 第三路：Artifact 校验与下载交付）。
///
/// 登记前门控：JSON 必须可解析且不得裹 Markdown 围栏；CSV 需表头 + 列数一致；
/// research 需至少一条有效证据（文件引用或 URL）；markdown 拒绝 TBD/空模板占位。
/// 校验未通过的产物**不登记**（不进版本链、不进 PendingReview）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactValidation {
    /// 被校验的格式（json|csv|research|markdown|text|…）。
    pub format: String,
    /// 是否通过。
    pub valid: bool,
    /// 失败原因（valid=true 时为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn default_artifact_classification() -> ArtifactClassification {
    ArtifactClassification::Private
}

fn default_review_state() -> ReviewState {
    ReviewState::Draft
}

/// 评审决定（§6.6 Artifact 评审闭环，V1-R2）。
///
/// - `approve`：批准（进入 approved head 候选）；
/// - `request_changes`：要求修改（产物回到 Draft 供返工出新版）；
/// - `reject`：驳回（终态拒绝，留审计）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactReviewDecision {
    Approve,
    RequestChanges,
    Reject,
}

/// 不可变评审记录（§6.6 Artifact 评审闭环，V1-R2）。
///
/// 一旦落盘永不修改/删除：评审历史只能追加（append-only）。
/// 同一 `idempotency_key` 重复提交不产生第二条记录（幂等回放）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactReviewRecord {
    pub review_id: String,
    /// 被评审的产物。
    pub artifact_id: String,
    /// 被评审的产物版本（与 `expected_version` 乐观并发目标对应）。
    pub artifact_version: u32,
    /// 产物所属团队运行。
    pub team_id: String,
    pub decision: ArtifactReviewDecision,
    /// 评审者（member_id / user_id / 角色名，如 "critic" / "human:u1"）。
    pub reviewer: String,
    /// 评语（可空）。
    #[serde(default)]
    pub comment: String,
    /// 幂等键（存储层唯一约束；同键重放返回既有记录）。
    pub idempotency_key: String,
    /// 评审时的产物内容引用（取证锚点：证明评审的是这份内容）。
    #[serde(default)]
    pub content_ref: String,
    /// 生产该产物的任务步骤（`s-{role}`；返工定位用。旧记录缺省为空）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub step_id: String,
    /// 生产者 member_id（冗余锚点：评审记录自足，不依赖产物表回查）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub producer_member_id: String,
    pub created_at: String,
}

/// 返工任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactReworkStatus {
    /// Request is durably recorded; coordinator dispatch may need replay after a crash.
    Dispatching,
    /// 已受理：步骤已重置并注入返工指令，等待重跑产出新版本。
    Requested,
    /// 新版本已登记（由版本链/评审闭环体现；任务记录保留供追溯）。
    Completed,
    /// 重跑失败（记录失败原因；需基于最新产物创建新评审后再次返工）。
    Failed,
}

/// Artifact 返工任务（V1 五期 · 第二路）。
///
/// 同一 `review_id` 至多一个返工任务（幂等）；重复请求返回原任务。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactReworkTask {
    pub rework_id: String,
    /// 被返工的产物（vN）。
    pub artifact_id: String,
    pub artifact_version: u32,
    /// 触发返工的 request_changes 评审（幂等键）。
    pub review_id: String,
    pub team_id: String,
    pub project_id: String,
    /// 生产步骤（重置目标；下游未完成节点一并重置）。
    pub step_id: String,
    /// 评审意见驱动的返工指令（注入步骤输入 `rework.instruction`）。
    pub instruction: String,
    pub idempotency_key: String,
    #[serde(default = "default_rework_status")]
    pub status: ArtifactReworkStatus,
    /// 返工产生的新版本 artifact_id（完成后回填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reworked_artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    pub created_at: String,
}

fn default_rework_status() -> ArtifactReworkStatus {
    ArtifactReworkStatus::Requested
}

// ---------------------------------------------------------------------------
// ChangeSet（八期 · 二路）：可审查/可接受/可拒绝/可撤销的代码变更闭环
// ---------------------------------------------------------------------------

/// ChangeSet 文件哈希条目。
///
/// `base_hashes` 侧三态（恢复语义）：
///
/// - `(Some(hash), content_available=true)`：基线内容在 CAS，可自动恢复；
/// - `(None, content_available=true)`：执行前不存在（新建文件，恢复即删除）；
/// - `(_, content_available=false)`：基线不可用/未知（超限/读取失败/快照不完整），
///   恢复按冲突处理（409 + conflicted，绝不误删）。
///
/// `result_hashes` 侧恒为 `content_available=false`（结果内容以磁盘为准，恢复用基线）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeSetFileHash {
    /// 相对工作区根路径（`/` 分隔，与 git porcelain / 变更追踪口径一致）。
    pub path: String,
    /// 内容 SHA-256（十六进制）；`None` = 该状态下文件不存在。
    #[serde(default)]
    pub sha256: Option<String>,
    /// 基线内容是否可自动恢复（见结构体文档三态；结果侧恒 false）。
    #[serde(default = "default_content_available")]
    pub content_available: bool,
}

fn default_content_available() -> bool {
    true
}

/// ChangeSet 状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetStatus {
    /// 待人工审查（写 Worker 成功落盘变更后自动进入）。
    PendingReview,
    /// 已接受：保留文件现状，该团队代码 Artifact 允许批准为最终 approved head。
    Accepted,
    /// 已拒绝：该 ChangeSet 修改的文件已恢复到基线。
    Rejected,
    /// 已撤销：同拒绝（语义为「撤销变更」，文件恢复到基线）。
    Reverted,
    /// 冲突：恢复前检测到用户改过文件（当前哈希 ≠ 结果哈希 ≠ 基线哈希），
    /// 不覆盖用户新内容；用户处理后可重试 accept/reject/revert。
    Conflicted,
}

/// ChangeSet 决定记录（幂等 + 审计锚点）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeSetDecision {
    /// accept / reject / revert。
    pub action: String,
    /// 幂等键：同动作重放零副作用（返回现状，`replayed: true`）。
    pub idempotency_key: String,
    pub decided_at: String,
    /// 备注（可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// ChangeSet：一次写角色执行的「工作区变更集合」（八期 · 二路）。
///
/// 由服务端 `TrackedRoleWorker` 在写角色成功路径自动生成：
/// 执行前对允许路径做内容基线快照（进 CAS）→ 执行 → 窗口内 git 变更差集 →
/// ChangeSet 落盘（`<run_dir>/<team_id>-change-sets.json`）。reject/revert 只恢复
/// 该 ChangeSet 修改的文件；未接受（pending_review/conflicted）时该团队代码
/// Artifact 可评审但不能成为最终 approved head。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeSet {
    pub change_set_id: String,
    pub team_id: String,
    /// 产生该变更集合的步骤（`s-{role}`）。
    pub step_id: String,
    /// 产生该变更集合的任务尝试；旧记录缺失时不能与新 attempt 收据绑定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    /// 产生该变更集合的角色。
    pub role: String,
    /// 执行前基线（仅本次变更涉及的文件；三态见 [`ChangeSetFileHash`]）。
    pub base_hashes: Vec<ChangeSetFileHash>,
    /// 执行后结果哈希（与 changed_files 一一对应）。
    pub result_hashes: Vec<ChangeSetFileHash>,
    /// 本次执行窗口内新增变更的文件（相对路径）。
    pub changed_files: Vec<String>,
    /// diff 补丁引用（`<team_id>-changes/<file>.patch`，相对 run_dir；可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_ref: Option<String>,
    pub status: ChangeSetStatus,
    pub created_at: String,
    /// 决定记录（accept/reject/revert 各至多一条；幂等重放零副作用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ChangeSetDecision>,
    /// 最近一次冲突文件清单（conflicted 时非空；落决定后清空）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

impl ChangeSet {
    /// 状态中文名（UI/审计展示）。
    pub fn status_label(&self) -> &'static str {
        match self.status {
            ChangeSetStatus::PendingReview => "待评审",
            ChangeSetStatus::Accepted => "已接受",
            ChangeSetStatus::Rejected => "已拒绝",
            ChangeSetStatus::Reverted => "已撤销",
            ChangeSetStatus::Conflicted => "冲突",
        }
    }
}

/// 决策记录（§6.6 DecisionRecord）。
///
/// 每项会改变任务路线的结论必须写成 DecisionRecord，不能只留在聊天中。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision_id: String,
    /// 提出者 member_id 或 user_id。
    pub proposer: String,
    /// 选择/结论摘要。
    pub choice: String,
    /// 理由与依据。
    pub rationale: String,
    /// 受影响的任务/资产 ID。
    #[serde(default)]
    pub affected_refs: Vec<String>,
    pub created_at: String,
}

/// 交接记录（§6.6 HandoffRecord）。
///
/// Agent 交接任务时必须填写的结构化信息，下游从 Project Space 读取而非猜测。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffRecord {
    pub handoff_id: String,
    /// 来源 member_id。
    pub from_member: String,
    /// 目标 member_id（或 * 表示任意下游）。
    pub to_member: String,
    /// 已完成内容摘要。
    pub completed_summary: String,
    /// 未解决问题。
    #[serde(default)]
    pub open_issues: Vec<String>,
    /// 输出产物引用。
    #[serde(default)]
    pub output_artifact_refs: Vec<String>,
    /// 证据引用（trace / snapshot / assertion）。
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// 建议下游动作。
    #[serde(default)]
    pub suggested_next_actions: Vec<String>,
    /// 已知风险。
    #[serde(default)]
    pub known_risks: Vec<String>,
    pub created_at: String,
    /// Worker 交接说明原文（七期 · 第三路：WorkerOutputV1.handoff 逐字落盘，
    /// 不再依赖自动摘要/反解析；旧记录为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_note: Option<String>,
}

/// 项目空间状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSpaceStatus {
    Active,
    Archived,
    Completed,
}

/// 可追溯共享事实的元数据；正文单独存放在 CAS。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedContextFact {
    pub key: String,
    pub value_ref: String,
    pub revision: u64,
    pub producer: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub file_hash: Option<String>,
    pub confidence: String,
    pub status: String,
    pub created_at: String,
}

/// 团队共享事实快照；revision 由 SQLite CAS 写入更新。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SharedContextSnapshot {
    pub revision: u64,
    #[serde(default)]
    pub facts: Vec<SharedContextFact>,
}

/// 项目空间（§6.6 ProjectSpace）。
///
/// WorkSwarm 式协同的核心：统一的任务、产物、决策、审批和活动流工作空间。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSpace {
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// 任务 ID 列表（完整任务图由 task_graph_ref 承载）。
    #[serde(default)]
    pub tasks: Vec<String>,
    /// 产物 ID 列表。
    #[serde(default)]
    pub artifacts: Vec<String>,
    /// 决策 ID 列表。
    #[serde(default)]
    pub decisions: Vec<String>,
    /// 审批 ID 列表。
    #[serde(default)]
    pub approvals: Vec<String>,
    /// 讨论消息 ID 列表（讨论允许自然语言，但结论须写 DecisionRecord）。
    #[serde(default)]
    pub discussions: Vec<String>,
    /// 活动流条目 ID（按时间排序）。
    #[serde(default)]
    pub activity_stream: Vec<String>,
    /// 交付清单引用（最终产物集合）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_manifest_ref: Option<String>,
    /// Artifact 返工任务（按 review_id 幂等；随空间 JSON 持久化）。
    #[serde(default)]
    pub rework_tasks: Vec<ArtifactReworkTask>,
    pub version: u32,
    #[serde(default = "default_project_status")]
    pub status: ProjectSpaceStatus,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_project_status() -> ProjectSpaceStatus {
    ProjectSpaceStatus::Active
}

// ---------------------------------------------------------------------------
// WorkSwarm 团队模板（§6.1 TeamTemplateRegistry / §6.7 团队经验）
// ---------------------------------------------------------------------------

/// 团队模板中的角色定义（角色组合 + 依赖 + 交付契约，不含历史敏感上下文）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTemplateRole {
    /// 宽松反序列化：缺失字段按空串处理，由 `user_workflow` 校验器给出可读原因
    ///（AI 生成与社区包导入不该因为少一个字段就得到 serde 解析错误）。
    #[serde(default)]
    pub role: String,
    /// 承担者种类：`agent` | `human` | `worker`。
    #[serde(default)]
    pub assignee: String,
    /// 建议 worker 名（agent 角色默认 `agent` 模型驱动；其余为内置 worker 名）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    /// 依赖的上游角色（任务 DAG 边）。
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// 交接契约（下游期望的产物格式/语义）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_contract: Option<String>,
    /// 验证断言（`non_empty` / `contains:<text>` / `equals:<text>`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<String>,
    /// 角色模型（十一期 additive）：None = 服务端缺省解析链
    /// （`input.model` 未注入时回退 `OWO_AGENT_MODEL` → 内置缺省）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 角色级写白名单（相对工作区根；空 = 工作区内可写）。
    /// 声明后写角色可与其他**范围不重叠**的写角色并发落盘（范围租约）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_paths: Vec<String>,
    /// 职责能力（例如 review）；缺失时由旧角色名兼容推导。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// 角色可见工具白名单（用户自定义工作流）：空 = 沿用角色运行档默认可见集。
    /// 元素必须是内置工具名（策略矩阵认得）或 `mcp__{server}__{tool}` 命名空间形式。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

/// 团队模板：已验证的角色组合、Swarmflow 与适用条件（§6.1）。
///
/// 只接受经用户采纳的提案进入注册表（§6.7）；复用任务图与交付契约，
/// 不复制历史敏感上下文。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTemplate {
    /// 宽松反序列化（AI 生成/社区包常见缺字段）：缺失交给 `user_workflow` 校验器
    /// 逐条报告，而不是在 serde 层就失败。
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub name: String,
    /// 缺省 `team`（多角色工作流最常见形态；single 由校验器检查角色数）。
    #[serde(default = "default_team_mode")]
    pub mode: TeamMode,
    #[serde(default)]
    pub roles: Vec<TeamTemplateRole>,
    /// 适用条件描述（模板优先匹配的语义依据）。
    #[serde(default)]
    pub applicability: String,
    /// 模板来源团队（溯源）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_team_id: Option<String>,
    #[serde(default)]
    pub created_at: String,
    /// 来源：`user`（用户自定义）/ `community`（导入的社区包）/ None（内置或历史记录）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// 人类可读描述（用户自定义/社区工作流用于列表与分享）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 版本号（社区包演进用；缺省视为 1.0.0）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 标签（检索/分类；不参与权限判定）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// 角色默认值（新字段扩展时保持旧字面量可用：`..Default::default()`）。
impl Default for TeamTemplateRole {
    fn default() -> Self {
        Self {
            role: String::new(),
            assignee: "agent".to_string(),
            worker: None,
            depends_on: Vec::new(),
            handoff_contract: None,
            verify: None,
            model: None,
            write_paths: Vec::new(),
            capabilities: Vec::new(),
            tools: Vec::new(),
        }
    }
}

/// 模板默认值（同上）。
impl Default for TeamTemplate {
    fn default() -> Self {
        Self {
            template_id: String::new(),
            name: String::new(),
            mode: TeamMode::Team,
            roles: Vec::new(),
            applicability: String::new(),
            source_team_id: None,
            created_at: String::new(),
            origin: None,
            description: None,
            version: None,
            tags: Vec::new(),
        }
    }
}

/// 团队模板提案状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamTemplateProposalStatus {
    /// 已生成，等待用户采纳（默认）。
    Proposed,
    /// 用户已采纳，进入 TeamTemplateRegistry。
    Adopted,
    /// 用户已拒绝（保留记录，可审计）。
    Rejected,
}

fn default_proposal_status() -> TeamTemplateProposalStatus {
    TeamTemplateProposalStatus::Proposed
}

/// 团队模板提案（§6.7）：从一次已验证运行生成，**只做提案，不自动启用**；
/// 至少经过一次用户采纳后才进入 TeamTemplateRegistry。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTemplateProposal {
    pub proposal_id: String,
    pub template: TeamTemplate,
    /// 产生该提案的运行（team_id）。
    pub source_team_id: String,
    /// 证据引用（最终产物 artifact ids）。
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default = "default_proposal_status")]
    pub status: TeamTemplateProposalStatus,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub workspace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub workspace: String,
    pub model: String,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub fork_point: Option<usize>,
    /// 会话级目标模式状态（`/goal`）；`None` = 未设定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<SessionGoalV1>,
}

/// 会话级目标模式状态（与 core `SessionGoal` 同构；线格式契约）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionGoalV1 {
    pub objective: String,
    #[serde(default)]
    pub iterations: usize,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub updated_at: String,
    /// `user`（用户 `/goal` 设定）或 `model`（`goal_request` 工具登记，待宿主接管）。
    #[serde(default = "default_goal_origin")]
    pub origin: String,
}

fn default_goal_origin() -> String {
    "user".to_string()
}

fn default_team_mode() -> TeamMode {
    TeamMode::Team
}

/// Restricted names for desktop-managed custom credentials. No arbitrary environment lookup.
pub fn custom_credential_env_names(reference: &str) -> Option<(String, String)> {
    if reference.len() != 32 || !reference.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some((
        format!("OWO_CUSTOM_MODEL_SECRET_{reference}"),
        format!("OWO_CUSTOM_MODEL_BINDING_{reference}"),
    ))
}

/// A bounded, durable view of the last session event and last host terminal record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTurnHistoryV1 {
    pub last_turn_id: String,
    pub latest_seq: u64,
    pub last_event_at: String,
    pub last_event_type: String,
    pub last_terminal: Option<TurnEventRecord>,
    pub last_stats: Option<TurnEventRecord>,
}

/// Ephemeral, explicit connection for one turn. Never saved with the session.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomModelConnection {
    pub model: String,
    pub base_url: String,
    #[serde(default)]
    pub api_format: String,
    #[serde(default)]
    pub use_full_url: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

impl std::fmt::Debug for CustomModelConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomModelConnection")
            .field("model", &self.model)
            .field("api_format", &self.api_format)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_connection: Option<CustomModelConnection>,
    /// Optional request restriction; false/omitted never relaxes the host policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// Optional preallocated UUID for scoped cancellation and safe recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// 附件 ID（由 `POST /session/{id}/attachments` 返回；发送时注入路径上下文）。
    #[serde(default)]
    pub attachments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkRequest {
    pub message_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindRequest {
    pub keep: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRunRequest {
    pub suite_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionResponse {
    pub allow: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remember: Option<bool>,
    /// §5.4 审批选项：once（默认，仅本次）/ session（此会话）/ one_hour（此项目一小时）/
    /// always_readonly（始终允许此只读动作）。破坏性操作不允许 always_readonly。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub healthy: bool,
    pub version: String,
    /// HTTP 契约版本。桌面壳用它拒绝连接到只占用同一端口、但 API 面不兼容的旧核心。
    #[serde(default)]
    pub api_version: String,
    pub auto_approve: bool,
    /// 构建信息（十期一路：统一构建入口生成 build-info.json；additive 字段，旧客户端自然忽略）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildInfo>,
    /// 桌面壳注入的实例身份（§4.2 实例握手；未注入 = 开发模式，不序列化）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    /// 服务进程 pid（非秘密；桌面壳用于核对"这个服务是我启动的子进程"）。
    #[serde(default)]
    pub pid: u32,
    /// 启动阶段（当前恒为 "ready"；为 §4.3 分阶段启动诊断预留）。
    #[serde(default)]
    pub stage: String,
    /// 构建标识（build-info.json 的 git_commit；缺失时 "unknown"）。
    #[serde(default)]
    pub build_id: String,
}

/// `/health.build` 构建信息（对应 agent-sdk/build-info.json 的 git_commit/git_dirty/built_at）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildInfo {
    pub commit: String,
    pub dirty: bool,
    pub built_at: String,
}

/// 指南 §2.3：Daemon 发现描述符（`<data_root>/runtime/daemon.json` 的唯一契约）。
///
/// 由 Daemon 在端口绑定后**原子替换**写入；CLI / TUI / 桌面壳经 `owo-agent-client`
/// 读取并校验 PID、`instance_id`、`/health`、API 版本与 build 兼容性。
/// **token 绝不写入本文件**（认证仍走权限受控的 `auth/token` 或桌面配对）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonDescriptor {
    /// 服务进程 pid（用于存活校验与 stale 恢复）。
    pub pid: u32,
    /// 实际监听端口（`--port 0` 时为系统分配的临时端口）。
    pub port: u16,
    /// 桌面壳注入的实例身份（开发模式为空串）。
    #[serde(default)]
    pub instance_id: String,
    /// HTTP 契约版本（与 `/health.api_version`、`OWO_API_VERSION` 同源）。
    pub api_version: String,
    /// 构建标识（build-info.json 的 git_commit；缺失时 "unknown"）。
    #[serde(default)]
    pub build_id: String,
    /// 启动时刻 RFC3339（UTC `Z`）。
    pub started_at: String,
    /// 数据根标识（脱敏；仅用于诊断区分，不暴露绝对路径）。
    #[serde(default)]
    pub data_root: String,
}

/// SSE 事件协议版本（R10：所有 SSE 事件帧 data 统一携带 `v` 字段）。
/// 变更策略：破坏性事件结构变更 → 递增版本并登记 RFC 注释（弃用期 ≥2 个 minor）。
pub const SSE_PROTOCOL_VERSION: u32 = 1;

/// Per-request token usage reported by a provider; None means unknown, never zero-filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTokenUsageV1 {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// Redacted model request telemetry exposed with a completed turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequestMetricV1 {
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub usage: Option<ModelTokenUsageV1>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub succeeded: bool,
}

/// Shared completion meaning across ordinary Agent turns and Team delivery.
/// A finished response is not the same claim as an accepted code change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStatusV1 {
    /// The conversational turn produced a final response without tracked file changes.
    ResponseComplete,
    /// The host observed file changes, but no task-scoped validation has accepted them.
    Candidate,
    /// All required host validations and version-bound delivery checks passed.
    Accepted,
    /// The turn ended, but required behavior or source evidence is missing or stale.
    #[default]
    Unverified,
    /// A required validation or blocking issue prevents acceptance.
    Blocked,
    /// The caller explicitly cancelled the run before delivery was committed.
    Aborted,
}

/// Durable, host-produced completion decision shared by Single and Team.
///
/// The record identifies the task attempt, decision, evidence receipts, and the
/// candidate version those receipts cover. It records a decision; it does not
/// replace the validators that produced the referenced evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCompletionRecordV1 {
    pub task_id: String,
    pub attempt_id: String,
    pub status: CompletionStatusV1,
    #[serde(default)]
    pub evidence_receipt_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_version_sha256: Option<String>,
    pub decided_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    Progress {
        message: String,
    },
    ToolUse {
        id: String,
        tool: String,
        args: Value,
    },
    ToolResult {
        id: String,
        tool: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// 结果预览（截断）：前端步骤 chip 展开时展示"这一步产出了什么"。
        /// 附加可选字段（非破坏性变更，SSE_PROTOCOL_VERSION 不变）；
        /// 老客户端忽略即可，完整结果仍以会话记录为准。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
    },
    PermissionRequest {
        request_id: String,
        tool: String,
        args: Value,
        reason: String,
        /// §5.5 脱敏参数视图（秘密字段只显示类型和长度）；审批卡默认展示。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        redacted_args: Option<Value>,
        /// §5.5 等级（read/write/execute/inject；前端据此隐藏「始终允许」）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<String>,
        /// §5.5 风险说明（审批卡展示）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        risk_note: Option<String>,
        /// §5.5 可解释摘要（将做什么/影响哪里/能否撤销）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        explain: Option<Value>,
    },
    Final {
        text: String,
    },
    TokenDelta {
        delta: String,
    },
    /// 深度思考增量（模型 reasoning_content；不写入对话历史）。
    ReasoningDelta {
        delta: String,
    },
    /// 任务计划更新（P2-5，`todo` 工具）：前端渲染步骤进度。
    PlanUpdate {
        /// `[{ "content": "...", "status": "pending|in_progress|completed" }]`。
        steps: Value,
    },
    Compaction {
        summary: String,
    },
    /// 审批已解决（与提问的 `UserAnswered` 对称）：前端关闭审批卡行动区并显示结果。
    /// 此前只有 `PermissionRequest`，300s 超时/回合中止后卡片仍留在界面上可点。
    PermissionResolved {
        request_id: String,
        /// 来源：`user`=用户点了允许/拒绝；`timeout`=300s 未响应；`aborted`=回合中止。
        #[serde(default)]
        source: String,
        /// 最终是否放行（超时/中止一律按拒绝）。
        #[serde(default)]
        allowed: bool,
    },
    /// 模型请求向用户提问（ask_user 工具）：前端展示提问卡，回合挂起等待回答。
    /// 用户通过 `POST /session/{id}/answer/{question_id}` 提交答案。
    UserQuestion {
        question_id: String,
        question: String,
        #[serde(default)]
        options: Vec<String>,
    },
    /// 用户已回答（或提问超时/中止）：前端关闭提问卡并回显答案。
    UserAnswered {
        question_id: String,
        answer: String,
        /// 回答来源：user=用户提交；timeout=超时未答；aborted=回合中止。
        #[serde(default)]
        source: String,
    },
    /// 回合失败终态（异常/中断）：必须显式下发，避免前端停留在"执行中"。
    TurnFailed {
        message: String,
        #[serde(default)]
        completion_status: CompletionStatusV1,
        /// Host decision for this failed/aborted attempt when task identity is available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completion_record: Option<TaskCompletionRecordV1>,
    },
    /// 回合统计（`run_turn` 结束后补发）：前端回合汇报卡展示耗时/步数/消耗。
    TurnStats {
        steps: usize,
        duration_ms: u64,
        #[serde(default)]
        prompt_tokens: u64,
        #[serde(default)]
        completion_tokens: u64,
        #[serde(default)]
        total_tokens: u64,
        #[serde(default)]
        cost_usd: f64,
        /// Completion state shared with Team DeliveryGate; older clients may ignore it.
        #[serde(default)]
        completion_status: CompletionStatusV1,
        /// Host-produced decision with candidate/version and validation-receipt identity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completion_record: Option<TaskCompletionRecordV1>,
        /// Request-level provider usage and latency; unknown values remain absent.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        model_calls: Vec<ModelRequestMetricV1>,
    },
}

/// Durable turn event. `seq` is strictly increasing within its session and is the replay cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnEventRecord {
    pub session_id: String,
    pub turn_id: String,
    pub seq: u64,
    pub created_at: String,
    pub payload: SseEvent,
}

/// Durable replay endpoint state. `active` also means the current bounded page has more
/// persisted events to fetch; `interrupted` means the process no longer owns the turn and no
/// completed/failed terminal event was persisted before it stopped.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnReplayState {
    Active,
    Completed,
    Failed,
    Interrupted,
}

/// Bounded replay response returned by GET /session/{id}/turn/events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnEventReplayPage {
    pub events: Vec<TurnEventRecord>,
    pub active: bool,
    pub state: TurnReplayState,
    pub next_after_seq: u64,
}

#[cfg(test)]
mod health_build_tests {
    use super::*;

    /// 十期一路：/health additive build 契约——无 build-info.json 时 build 键
    /// 不得序列化（skip_serializing_if），旧客户端零感知。
    #[test]
    fn health_without_build_omits_field() {
        let legacy = HealthResponse {
            healthy: true,
            version: "0.1.0".into(),
            api_version: "0.7".into(),
            auto_approve: false,
            build: None,
            instance_id: None,
            pid: 0,
            stage: String::new(),
            build_id: String::new(),
        };
        let v = serde_json::to_value(&legacy).unwrap();
        assert!(v.get("build").is_none(), "build=None 时不得出现 build 键");
        assert!(
            v.get("instance_id").is_none(),
            "instance_id=None 时不得出现 instance_id 键（开发模式不泄漏字段）"
        );
        assert_eq!(v["healthy"], serde_json::json!(true));
        assert_eq!(v["version"], "0.1.0");
        assert_eq!(v["api_version"], "0.7");
        assert_eq!(v["auto_approve"], serde_json::json!(false));
    }

    /// 十期一路：有构建信息时三字段齐备并可往返；旧载荷（无 build 键）反序列化兼容。
    #[test]
    fn health_with_build_round_trips_and_legacy_payload_compat() {
        let full = HealthResponse {
            healthy: true,
            version: "0.1.0".into(),
            api_version: "0.7".into(),
            auto_approve: true,
            build: Some(BuildInfo {
                commit: "8412021".into(),
                dirty: true,
                built_at: "2026-08-30T04:00:00Z".into(),
            }),
            instance_id: Some("a1b2c3d4e5f60718293a4b5c6d7e8f90".into()),
            pid: 4242,
            stage: "ready".into(),
            build_id: "8412021".into(),
        };
        let text = serde_json::to_string(&full).unwrap();
        assert!(
            text.contains("\"build\""),
            "Some(build) 必须序列化 build 键"
        );
        let back: HealthResponse = serde_json::from_str(&text).unwrap();
        let build = back.build.expect("build 应存在");
        assert_eq!(build.commit, "8412021");
        assert!(build.dirty);
        assert_eq!(build.built_at, "2026-08-30T04:00:00Z");
        // §4.2 实例握手字段往返。
        assert_eq!(
            back.instance_id.as_deref(),
            Some("a1b2c3d4e5f60718293a4b5c6d7e8f90")
        );
        assert_eq!(back.pid, 4242);
        assert_eq!(back.stage, "ready");
        assert_eq!(back.build_id, "8412021");

        // 旧载荷（无 build 键）→ #[serde(default)] 兼容。
        let old: HealthResponse =
            serde_json::from_str(r#"{"healthy":true,"version":"0.1.0","auto_approve":false}"#)
                .unwrap();
        assert!(old.build.is_none());
        assert!(
            old.api_version.is_empty(),
            "旧载荷保持可解析，但不会通过桌面版本握手"
        );
    }
}

#[cfg(test)]
mod turn_failed_completion_status_tests {
    use super::{CompletionStatusV1, SseEvent, TaskCompletionRecordV1};

    #[test]
    fn turn_failed_status_is_backward_compatible_and_round_trips() {
        let legacy: SseEvent =
            serde_json::from_str(r#"{"type":"turn_failed","message":"cancelled"}"#).unwrap();
        match legacy {
            SseEvent::TurnFailed {
                completion_status, ..
            } => assert_eq!(completion_status, CompletionStatusV1::Unverified),
            _ => panic!("expected TurnFailed"),
        }

        let record = TaskCompletionRecordV1 {
            task_id: "single-turn:turn-1".to_string(),
            attempt_id: "turn-1".to_string(),
            status: CompletionStatusV1::Aborted,
            evidence_receipt_ids: vec!["receipt-1".to_string()],
            candidate_version_sha256: Some("sha256:candidate".to_string()),
            decided_at: "2026-10-05T00:00:00Z".to_string(),
        };
        let current = SseEvent::TurnFailed {
            message: "cancelled".to_string(),
            completion_status: CompletionStatusV1::Aborted,
            completion_record: Some(record.clone()),
        };
        let json = serde_json::to_value(&current).unwrap();
        assert_eq!(json["completion_status"], "aborted");
        let restored: SseEvent = serde_json::from_value(json).unwrap();
        match restored {
            SseEvent::TurnFailed {
                completion_record, ..
            } => {
                assert_eq!(completion_record.as_ref(), Some(&record));
            }
            _ => panic!("expected TurnFailed"),
        }
    }
}

#[cfg(test)]
mod turn_request_constraint_tests {
    use super::TurnRequest;
    #[test]
    fn old_payloads_remain_compatible_and_readonly_is_optional() {
        let old: TurnRequest = serde_json::from_str(r#"{"prompt":"fixture"}"#).unwrap();
        assert_eq!(old.read_only, None);
        let value = serde_json::to_value(&old).unwrap();
        assert!(value.get("read_only").is_none());
        let read_only: TurnRequest =
            serde_json::from_str(r#"{"prompt":"fixture","read_only":true}"#).unwrap();
        assert_eq!(read_only.read_only, Some(true));
    }
}

pub fn canonical_turn_id(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    for (index, byte) in bytes.iter().enumerate() {
        if [8, 13, 18, 23].contains(&index) {
            if *byte != b'-' {
                return None;
            }
        } else if !byte.is_ascii_hexdigit() {
            return None;
        }
    }
    Some(value.to_ascii_lowercase())
}

/// Stateless, tool-free text request. Unknown capability fields are rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionRequest {
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionResponse {
    pub text: String,
}
