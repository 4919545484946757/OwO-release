use owo_agent_protocol::TeamMode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::roles::RoleSpec;
/// 阶段结果（server 运行循环据此推进）。
#[derive(Debug, Clone)]
pub enum PhaseOutcome {
    /// 运行已进入终态（无需再推进）。
    Finished,
    /// 本阶段 agent 批次完成；后续仍有就绪步骤 → 运行循环继续。
    MoreReady,
    /// 等待人节点；运行任务开门闩等待结果（结果录入后自动唤醒下游）。
    AwaitingHuman { waits: Vec<HumanWait> },
    /// 全部步骤成功；可收尾（交付清单 + 模板提案）。
    Done,
    /// 运行失败（team 已置 Failed，产物保留）。
    Failed,
    /// 运行被取消（team 已置 Cancelled，产物保留）。
    Aborted,
}

/// 人节点等待项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanWait {
    pub step_id: String,
    pub member_id: String,
    pub user_id: String,
    pub role: String,
}

/// 共享事实发布载荷；正文进入 CAS，元数据按团队 revision 原子提交。
#[derive(Debug, Clone)]
pub struct SharedContextFactDraft {
    pub key: String,
    pub value: String,
    pub producer: String,
    pub task_id: Option<String>,
    pub source_refs: Vec<String>,
    pub file_hash: Option<String>,
}

/// 组队请求。
#[derive(Debug, Clone)]
pub struct CreateTeamRequest {
    pub goal_id: Option<String>,
    pub objective: String,
    pub mode: TeamMode,
    /// 显式指定模板（swarmflow 必填；team 可选；single 忽略）。
    pub template_id: Option<String>,
    /// 动态角色（空 = 按模式取模板默认 / 内置默认接力）。
    pub roles: Vec<RoleSpec>,
    pub budget: Value,
    pub human_policy: Option<String>,
    /// 五期：组队策略（auto 判定 / single / team 强制；缺省 auto——
    /// 默认不再盲目启用多 Agent，由 TeamStrategyEngine 按任务画像判定）。
    pub strategy: Option<crate::team_strategy::TeamSelectionMode>,
    /// 十一期：团队统一模型（所有 agent 步骤缺省使用；`RoleSpec.model` 显式覆盖）。
    pub model: Option<String>,
    /// 十一期：并行开发模式——配合 [`super::roles::parallel_roles`]：lead 拆解 →
    /// w1..wN 并行 → leader 汇总；运行期把 lead 产物的 `subtasks`（子任务说明 +
    /// 写范围）动态应用到对应 writer 角色（见 `TeamCoordinator::run_phase`）。
    pub parallel: bool,
    /// 十一期：Agent 成员上限覆盖（并行模式 lead+writers+leader > 默认 5）。
    pub max_agent_members: Option<usize>,
    /// 由 Daemon 从同工作区父会话生成的受限 CoreSpec 快照（JSON）。
    pub parent_context_snapshot: Option<String>,
}

impl CreateTeamRequest {
    pub fn new(objective: impl Into<String>, mode: TeamMode) -> Self {
        Self {
            goal_id: None,
            objective: objective.into(),
            mode,
            template_id: None,
            roles: Vec::new(),
            budget: Value::Null,
            human_policy: None,
            strategy: None,
            model: None,
            parallel: false,
            max_agent_members: None,
            parent_context_snapshot: None,
        }
    }
}

/// steer 指令（§6.1：continue | steer | replace | cancel；R2 追加：retry）。
///
/// S0 语义：只改未完成节点，已完成产物永不丢失；变更必须留下 DecisionRecord。
#[derive(Debug, Clone)]
pub enum SteerCommand {
    /// 失败/取消后重跑：重置未完成步骤（已完成不重跑），清除失败/取消现场。
    Continue,
    /// 修改未完成节点：`step_id` 为空 = 全部未完成节点；`new_input` 合并进步骤输入。
    Steer {
        step_id: Option<String>,
        new_input: Option<Value>,
        note: String,
    },
    /// 更换成员承担者（agent 换 worker / 人节点换用户）：仅影响该成员未完成步骤。
    Replace {
        role: String,
        new_worker: Option<String>,
        new_user_id: Option<String>,
        note: String,
    },
    /// 取消运行（传播到运行中阶段；已完成产物保留）。
    Cancel,
    /// 局部重试（R2）：仅允许指定一个 Failed/Aborted/中断中的步骤；
    /// 只重置目标步骤及其尚未完成的下游节点——已成功步骤、已有 Artifact、
    /// Handoff 与 DecisionRecord 一律不动。重复发送同一 retry 不产生额外副作用。
    Retry { step_id: String, note: String },
}

/// 进程中断识别记录（sidecar 文件 `<run_dir>/<team_id>-interrupted.json`）。
///
/// 进程重启后磁盘状态仍为 Running、且当前无活动运行/运行循环时，
/// 由 [`TeamCoordinator::detect_interrupted`] 识别为 interrupted 并落盘本记录：
/// - 原 Running 步骤转为可恢复状态（Aborted，带中断说明）；
/// - 通过 `continue` 或 `retry` 显式恢复（禁止启动时静默重复执行写操作）；
/// - 恢复成功后本记录被清除。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptedRun {
    pub team_id: String,
    /// 识别时间（RFC3339）。
    pub detected_at: String,
    /// 中断时正在执行的步骤（原 record.status == Running）。
    pub interrupted_steps: Vec<String>,
    pub reason: String,
}

/// 全量中断扫描报告（[`TeamCoordinator::detect_interrupted`]）。
#[derive(Debug, Clone, Default)]
pub struct InterruptionScan {
    /// 本次新识别的中断团队（已识别过的不再重复报告）。
    pub interrupted: Vec<InterruptedRun>,
    /// 状态文件无法解析的团队（原样保留、未覆盖；需人工修复后才能恢复操作）。
    pub unreadable_states: Vec<String>,
}

/// 手动交接输入（§8.5 POST /tasks/{id}/handoff）。
#[derive(Debug, Clone, Default)]
pub struct HandoffFields {
    /// 目标成员（缺省 `*` = 任意下游）。
    pub to_member: Option<String>,
    pub completed_summary: String,
    pub open_issues: Vec<String>,
    pub output_artifact_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub suggested_next_actions: Vec<String>,
    pub known_risks: Vec<String>,
}

/// 进度视图中的执行中步骤（SSE `progress` 事件用）。
#[derive(Debug, Clone, Serialize)]
pub struct ProgressStep {
    pub step_id: String,
    /// 角色名（成员名去掉 `m-` 前缀）。
    pub worker: String,
    pub status: String,
    pub attempts: u32,
    /// 调度器领取时间；尚未进入 Worker 时 started_at 为空。
    pub claimed_at: String,
    /// Worker 包装层实际开始调用的时间；Claimed 阶段为空。
    pub started_at: String,
}

/// 步骤计数（进度视图；`aborted` 为附加口径，前四项与既定契约一致）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProgressCounts {
    pub pending: u32,
    /// Steps claimed for this phase but not yet inside Worker::run.
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub claimed: u32,
    /// Steps that have entered Worker::run.
    pub running: u32,
    pub succeeded: u32,
    pub failed: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub aborted: u32,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// 团队实时进度快照（`seq` 单调递增；变化即代表有状态转移）。
#[derive(Debug, Clone, Serialize)]
pub struct TeamProgress {
    pub seq: u64,
    pub team_id: String,
    pub status: String,
    pub active: bool,
    pub current_steps: Vec<ProgressStep>,
    pub counts: ProgressCounts,
    pub updated_at: String,
    /// Host-side supervision derived from this snapshot (never a model opinion).
    pub supervisor: super::supervisor::ProgressSupervisor,
}
