//! 统一 Task Runtime 的数据契约（Agent V2 §2.3/§2.4）。
//!
//! 设计边界：
//! - **Task 是用户委托的工作单元**，负责状态、预算、事件、审批、检查点和交付；
//! - Session 是对话上下文，Turn 是一次模型交互，都不再代替 Task 状态；
//! - 后端（Single Agent / WorkSwarm / Computer Use / Workflow / Cloud）只能提交
//!   执行事实和产物，**不能自行把 Task 标记为 Succeeded**——终态由 Runtime +
//!   Verification 决定。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Task Store 的 schema 版本（写入每条任务记录，便于后续迁移）。
/// v2：`tasks.payload_json`（后端专属输入，如 computer-use 的目标步骤）。
pub const TASK_SCHEMA_VERSION: u32 = 2;

/// 稳定错误码：HTTP 层与前端按 `code` 分类，不解析中文文案。
pub mod codes {
    pub const TASK_NOT_FOUND: &str = "TASK_NOT_FOUND";
    pub const INVALID_TRANSITION: &str = "INVALID_TRANSITION";
    pub const EMPTY_GOALS: &str = "EMPTY_GOALS";
    pub const INVALID_ACTION: &str = "INVALID_ACTION";
    pub const STALE_WINDOW_IDENTITY: &str = "STALE_WINDOW_IDENTITY";
    pub const TARGET_NOT_FOUND: &str = "TARGET_NOT_FOUND";
    pub const FOREGROUND_CHANGED: &str = "FOREGROUND_CHANGED";
    pub const ANCHOR_AMBIGUOUS: &str = "ANCHOR_AMBIGUOUS";
    pub const ACTION_NOT_AUTHORIZED: &str = "ACTION_NOT_AUTHORIZED";
    pub const POST_ACTION_VERIFY_FAILED: &str = "POST_ACTION_VERIFY_FAILED";
    pub const BUDGET_EXCEEDED: &str = "BUDGET_EXCEEDED";
    pub const APPROVAL_REQUIRED: &str = "APPROVAL_REQUIRED";
    pub const IDEMPOTENCY_CONFLICT: &str = "IDEMPOTENCY_CONFLICT";
    pub const UNSUPPORTED: &str = "UNSUPPORTED";
    pub const STORE_ERROR: &str = "STORE_ERROR";
    pub const VERIFICATION_FAILED: &str = "VERIFICATION_FAILED";
    pub const INVALID_REQUEST: &str = "INVALID_REQUEST";
}

/// 结构化任务错误。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskError {
    pub code: String,
    pub message: String,
}

impl TaskError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn not_found(task_id: &str) -> Self {
        Self::new(codes::TASK_NOT_FOUND, format!("任务 {task_id} 不存在"))
    }

    pub fn store(error: impl std::fmt::Display) -> Self {
        Self::new(codes::STORE_ERROR, error.to_string())
    }

    pub fn invalid_transition(from: TaskState, to: TaskState) -> Self {
        Self::new(
            codes::INVALID_TRANSITION,
            format!("任务状态不允许从 {} 迁移到 {}", from.as_str(), to.as_str()),
        )
    }
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TaskError {}

pub type TaskResult<T> = Result<T, TaskError>;

/// 任务 ID（newtype，JSON 序列化为字符串）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for TaskId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for TaskId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// 统一任务状态机（Agent V2 §2.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Draft,
    Queued,
    Clarifying,
    Planning,
    Running,
    AwaitingApproval,
    Verifying,
    Repairing,
    Succeeded,
    Paused,
    CancelRequested,
    Cancelled,
    Failed,
    Interrupted,
    NeedsUserInput,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Queued => "queued",
            Self::Clarifying => "clarifying",
            Self::Planning => "planning",
            Self::Running => "running",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Verifying => "verifying",
            Self::Repairing => "repairing",
            Self::Succeeded => "succeeded",
            Self::Paused => "paused",
            Self::CancelRequested => "cancel_requested",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::NeedsUserInput => "needs_user_input",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "draft" => Self::Draft,
            "queued" => Self::Queued,
            "clarifying" => Self::Clarifying,
            "planning" => Self::Planning,
            "running" => Self::Running,
            "awaiting_approval" => Self::AwaitingApproval,
            "verifying" => Self::Verifying,
            "repairing" => Self::Repairing,
            "succeeded" => Self::Succeeded,
            "paused" => Self::Paused,
            "cancel_requested" => Self::CancelRequested,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed,
            "interrupted" => Self::Interrupted,
            "needs_user_input" => Self::NeedsUserInput,
            _ => return None,
        })
    }

    /// 终态不可被普通更新覆盖；重新执行必须创建新 Attempt 或显式 Resume。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// 是否需要用户介入（前端据此显示"待处理"）。
    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::AwaitingApproval | Self::NeedsUserInput | Self::Paused | Self::Interrupted
        )
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 执行策略 / 后端种类（不是不同产品）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    SingleAgent,
    WorkSwarm,
    ComputerUse,
    Workflow,
    CloudRemote,
}

impl ExecutionStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SingleAgent => "single_agent",
            Self::WorkSwarm => "work_swarm",
            Self::ComputerUse => "computer_use",
            Self::Workflow => "workflow",
            Self::CloudRemote => "cloud_remote",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "single_agent" => Self::SingleAgent,
            "work_swarm" => Self::WorkSwarm,
            "computer_use" => Self::ComputerUse,
            "workflow" => Self::Workflow,
            "cloud_remote" => Self::CloudRemote,
            _ => return None,
        })
    }
}

/// 任务预算。`0` 表示该维度不额外限制。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskBudget {
    pub max_attempts: u32,
    pub max_repairs: u32,
    pub max_tool_calls: u32,
    pub max_model_tokens: u64,
    pub max_duration_ms: u64,
}

impl Default for TaskBudget {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            max_repairs: 2,
            max_tool_calls: 200,
            max_model_tokens: 0,
            max_duration_ms: 0,
        }
    }
}

/// 任务记录（`tasks` 表一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: TaskId,
    pub title: String,
    pub user_goal: String,
    pub state: TaskState,
    pub execution_strategy: ExecutionStrategy,
    pub current_attempt: u32,
    pub created_at: String,
    pub updated_at: String,
    pub last_checkpoint_id: Option<String>,
    pub terminal_summary: Option<String>,
    pub budget: TaskBudget,
    pub policy_profile: String,
    pub workspace_ref: Option<String>,
    pub session_refs: Vec<String>,
    /// 后端专属输入（不透明 JSON）。Single Agent 不用；Computer Use 用它承载
    /// `target_app` / `goals`。Runtime 不解释其内容，只在分发时原样交给后端。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    pub schema_version: u32,
}

impl TaskRecord {
    pub fn new(
        title: impl Into<String>,
        user_goal: impl Into<String>,
        execution_strategy: ExecutionStrategy,
        budget: TaskBudget,
        policy_profile: impl Into<String>,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            task_id: TaskId::new(),
            title: title.into(),
            user_goal: user_goal.into(),
            state: TaskState::Draft,
            execution_strategy,
            current_attempt: 0,
            created_at: now.clone(),
            updated_at: now,
            last_checkpoint_id: None,
            terminal_summary: None,
            budget,
            policy_profile: policy_profile.into(),
            workspace_ref: None,
            session_refs: Vec::new(),
            payload: None,
            schema_version: TASK_SCHEMA_VERSION,
        }
    }
}

/// 任务事件（`task_events` 表一行）。`sequence` 在单个任务内严格单调递增。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub task_id: TaskId,
    pub sequence: u64,
    pub event_id: String,
    pub event_type: String,
    pub occurred_at: String,
    pub attempt: u32,
    pub payload: Value,
    pub correlation_id: Option<String>,
}

/// 统一事件类型（§6 任务 2.4）。
pub mod event_type {
    pub const TASK_CREATED: &str = "task.created";
    pub const TASK_QUEUED: &str = "task.queued";
    pub const TASK_STATE_CHANGED: &str = "task.state.changed";
    pub const TASK_PLANNING_STARTED: &str = "task.planning.started";
    pub const TASK_PLAN_UPDATED: &str = "task.plan.updated";
    pub const TASK_EXECUTION_STARTED: &str = "task.execution.started";
    /// 后端提交执行事实（不含成功结论）。
    pub const TASK_BACKEND_COMPLETED: &str = "task.backend.completed";
    /// 后端报告执行失败。
    pub const TASK_BACKEND_FAILED: &str = "task.backend.failed";
    /// 后端要求人工处理（如 computer-use 任务需要先批准）。
    pub const TASK_USER_INPUT_REQUIRED: &str = "task.user_input.required";
    /// computer-use 闭环执行事实。
    pub const TASK_COMPUTER_STARTED: &str = "task.computer.started";
    /// 单步执行事实及后续观察分别持久化，同一 action_id 关联两个阶段。
    pub const TASK_COMPUTER_ACTION_INTENDED: &str = "task.computer.action.intended";
    pub const TASK_COMPUTER_ACTION_RECORDED: &str = "task.computer.action.recorded";
    pub const TASK_COMPUTER_COMPLETED: &str = "task.computer.completed";
    pub const TASK_STEP_STARTED: &str = "task.step.started";
    pub const TASK_TOOL_STARTED: &str = "task.tool.started";
    pub const TASK_TOOL_COMPLETED: &str = "task.tool.completed";
    pub const TASK_APPROVAL_REQUESTED: &str = "task.approval.requested";
    pub const TASK_APPROVAL_DECIDED: &str = "task.approval.decided";
    pub const TASK_CHECKPOINT_CREATED: &str = "task.checkpoint.created";
    pub const TASK_VERIFICATION_STARTED: &str = "task.verification.started";
    pub const TASK_VERIFICATION_COMPLETED: &str = "task.verification.completed";
    pub const TASK_REPAIR_STARTED: &str = "task.repair.started";
    pub const TASK_ARTIFACT_CREATED: &str = "task.artifact.created";
    pub const TASK_PAUSED: &str = "task.paused";
    pub const TASK_RESUMED: &str = "task.resumed";
    pub const TASK_CANCEL_REQUESTED: &str = "task.cancel_requested";
    pub const TASK_CANCELLED: &str = "task.cancelled";
    pub const TASK_FAILED: &str = "task.failed";
    pub const TASK_INTERRUPTED: &str = "task.interrupted";
    pub const TASK_SUCCEEDED: &str = "task.succeeded";
}

/// 检查点只保存恢复所需的最小信息（§7 任务 3.2）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCheckpoint {
    pub checkpoint_id: String,
    pub task_id: TaskId,
    pub attempt: u32,
    pub created_at: String,
    pub phase: String,
    /// 状态快照引用（文件 / 对象存储路径），不把大对象塞进事件表。
    pub state_snapshot_ref: String,
    pub workspace_revision: Option<String>,
    pub artifact_refs: Vec<String>,
    /// 当前计划版本、下一步意图、预算使用等可恢复元数据。
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactVerification {
    Pending,
    Verified,
    Failed,
    NotApplicable,
}

/// 可交付产物。大型内容用 URI/路径引用，`sha256` 用于校验。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskArtifact {
    pub artifact_id: String,
    pub task_id: TaskId,
    pub attempt: u32,
    pub kind: String,
    pub uri_or_path: String,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
    pub created_at: String,
    pub verification_status: ArtifactVerification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

impl ApprovalDecision {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "approved" | "approve" => Some(Self::Approved),
            "rejected" | "reject" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// 审批对象。批准某一次操作不等于批准整个任务后续所有操作。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskApproval {
    pub approval_id: String,
    pub task_id: TaskId,
    /// 操作主体（工具名或动作名）。
    pub action: String,
    /// 本次批准的权限范围（目标应用/文件/工作区等）。
    pub scope: Value,
    pub requested_at: String,
    pub expires_at: Option<String>,
    pub decided_at: Option<String>,
    pub decision: Option<ApprovalDecision>,
    pub decided_by: Option<String>,
}

impl TaskApproval {
    pub fn is_pending(&self) -> bool {
        self.decision.is_none()
    }
}
