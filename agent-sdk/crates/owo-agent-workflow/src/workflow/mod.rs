//! §12 支柱 1：可组合工作流引擎（.owflow v1）
//!
//! `.owflow` = 触发器 + 步骤图 + 子流程 + 条件 + 人审节点 + 回滚点。
//! 底座只读复用 `action_program`（ProgramNode 编译目标）、`skill_health`（健康度门禁）、
//! `audit`（全程审计）。
//!
//! 设计口径（对应技术文档 §12.2 支柱 1）：
//! - 步骤类型：感知 / 定位 / 动作 / 断言 / 调用技能包 / 调用 MCP / 人审 / 通知 / 子流程 / 循环 / 条件 / 回滚点。
//! - 安全：每个跨应用步骤（发消息/写文件/联网）是独立权限节点，默认 deny；Ask 经审批人确认。
//! - 回滚：回滚点保存工作区快照，失败自动恢复到最近检查点。
//! - 健康度：调用技能包前查 SkillHealth（Disabled 拒绝、Degraded 需确认），执行后回写成功/失败。
//! - 状态机：Pending → Running → WaitingApproval → Succeeded / Failed / Aborted。

mod recovery;
pub use recovery::{
    recovery_fingerprint, WorkflowRecoveryState, WorkflowRecoveryStore, WORKFLOW_RECOVERY_REQUIRED,
};

use crate::action_program::ActionProgram;
use owo_agent_contracts::skill_health::{SkillHealthStore, SkillState};
use owo_agent_kernel::audit::AuditLog;
use owo_agent_memory::learn::{ActionType, SemanticAnchor};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// 触发器
// ---------------------------------------------------------------------------

/// 触发器类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerKind {
    /// 手动触发。
    Manual,
    /// 定时（v1 仅声明，运行时轮询器 P2）。
    Schedule { expression: String },
    /// 前台应用切换。
    ForegroundApp { app: String },
    /// 文件变化。
    FileChange { path: String },
    /// 剪贴板变化。
    Clipboard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowTrigger {
    pub id: String,
    pub kind: TriggerKind,
}

// ---------------------------------------------------------------------------
// 权限声明
// ---------------------------------------------------------------------------

/// 权限模式：默认 deny；显式 allow / ask。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermMode {
    #[default]
    Deny,
    Allow,
    Ask,
}

/// 权限声明：跨应用边界 scope（如 message.send / fs.write / network）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionClaim {
    pub scope: String,
    pub mode: PermMode,
}

// ---------------------------------------------------------------------------
// 步骤规格与步骤类型
// ---------------------------------------------------------------------------

/// 感知步骤规格。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SenseSpec {
    /// 感知目标（如 "clipboard" / "files" / "foreground"）。
    pub target: String,
}

/// 定位步骤规格。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocateSpec {
    pub target: String,
}

/// 动作步骤规格（跨应用边界，权限 scope）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActSpec {
    /// 动作名：write_file / append_file / send_message / launch / click / type。
    pub action: String,
    /// 目标（文件路径 / 应用 / 元素）。
    pub target: String,
    /// 值（写入内容 / 键入文本）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// 工作流步骤：条件/循环内嵌子步骤图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowStep {
    Sense {
        id: String,
        spec: SenseSpec,
    },
    Locate {
        id: String,
        spec: LocateSpec,
    },
    /// 动作：独立权限节点（scope 缺省按默认 deny）。
    Act {
        id: String,
        #[serde(default)]
        scope: String,
        spec: ActSpec,
    },
    /// 断言（表达式，见 `eval_expr`）。
    Assert {
        id: String,
        expr: String,
        #[serde(default = "default_assert_timeout_ms")]
        timeout_ms: u64,
    },
    /// 调用技能包（健康度门禁）。
    InvokeSkill {
        id: String,
        skill: String,
        #[serde(default)]
        args: BTreeMap<String, String>,
    },
    /// 调用 MCP 工具（跨应用边界，独立权限节点）。
    InvokeMcp {
        id: String,
        server: String,
        tool: String,
        #[serde(default)]
        args: serde_json::Value,
        #[serde(default)]
        scope: String,
    },
    /// 人审节点：等待审批（Approved 继续 / Rejected 中止）。
    HumanApprove {
        id: String,
        prompt: String,
    },
    /// 通知。
    Notify {
        id: String,
        message: String,
    },
    /// 子流程引用（深度上限由引擎控制）。
    Subflow {
        id: String,
        flow: String,
        #[serde(default)]
        args: BTreeMap<String, String>,
    },
    /// 循环（cond 为空时执行 max_iter 次；cond 为假提前退出）。
    Loop {
        id: String,
        body: Vec<WorkflowStep>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cond: Option<String>,
        #[serde(default = "default_loop_max_iter")]
        max_iter: u32,
    },
    /// 条件分支。
    Cond {
        id: String,
        expr: String,
        then: Vec<WorkflowStep>,
        #[serde(default)]
        otherwise: Vec<WorkflowStep>,
    },
    /// 回滚点：保存工作区快照。
    RollbackPoint {
        id: String,
    },
}

fn default_assert_timeout_ms() -> u64 {
    3_000
}

fn default_loop_max_iter() -> u32 {
    10
}

// ---------------------------------------------------------------------------
// 工作流定义
// ---------------------------------------------------------------------------

/// .owflow 定义（JSON 声明式）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub triggers: Vec<WorkflowTrigger>,
    #[serde(default)]
    pub permissions: Vec<PermissionClaim>,
    /// 前置条件表达式（全部成立才启动）。
    #[serde(default)]
    pub preconditions: Vec<String>,
    /// 回滚点（也可用 RollbackPoint 步骤内嵌）。
    #[serde(default)]
    pub rollback_points: Vec<String>,
    pub steps: Vec<WorkflowStep>,
    /// 单次运行总步数上限（防死循环）。
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    /// 子流程嵌套深度上限。
    #[serde(default = "default_subflow_depth")]
    pub subflow_depth_limit: u32,
}

fn default_version() -> u32 {
    1
}

fn default_max_steps() -> u32 {
    500
}

fn default_subflow_depth() -> u32 {
    5
}

impl Default for WorkflowDefinition {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: 1,
            triggers: Vec::new(),
            permissions: Vec::new(),
            preconditions: Vec::new(),
            rollback_points: Vec::new(),
            steps: Vec::new(),
            max_steps: 500,
            subflow_depth_limit: 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Schema 校验
// ---------------------------------------------------------------------------

fn collect_step_ids(steps: &[WorkflowStep], out: &mut Vec<String>) {
    for step in steps {
        let id = match step {
            WorkflowStep::Sense { id, .. }
            | WorkflowStep::Locate { id, .. }
            | WorkflowStep::Act { id, .. }
            | WorkflowStep::Assert { id, .. }
            | WorkflowStep::InvokeSkill { id, .. }
            | WorkflowStep::InvokeMcp { id, .. }
            | WorkflowStep::HumanApprove { id, .. }
            | WorkflowStep::Notify { id, .. }
            | WorkflowStep::Subflow { id, .. }
            | WorkflowStep::Loop { id, .. }
            | WorkflowStep::Cond { id, .. }
            | WorkflowStep::RollbackPoint { id, .. } => id.clone(),
        };
        out.push(id);
        match step {
            WorkflowStep::Loop { body, .. } => collect_step_ids(body, out),
            WorkflowStep::Cond {
                then, otherwise, ..
            } => {
                collect_step_ids(then, out);
                collect_step_ids(otherwise, out);
            }
            _ => {}
        }
    }
}

mod compile;

pub use compile::{
    compile_to_program, eval_expr, validate_definition, EXPRESSION_SEMANTICS_VERSION,
};

// ---------------------------------------------------------------------------
// 执行后端与审批抽象
// ---------------------------------------------------------------------------

/// 动作后端：真实桌面/文件/MCP 动作的抽象（v1 用 MockBackend；真实接入由主控后续做）。
#[async_trait::async_trait]
pub trait ActionBackend: Send {
    async fn sense(&mut self, spec: &SenseSpec) -> Result<serde_json::Value, String>;
    async fn locate(&mut self, spec: &LocateSpec) -> Result<serde_json::Value, String>;
    async fn act(&mut self, spec: &ActSpec) -> Result<serde_json::Value, String>;
    async fn invoke_skill(
        &mut self,
        skill: &str,
        args: &BTreeMap<String, String>,
    ) -> Result<serde_json::Value, String>;
    async fn invoke_mcp(
        &mut self,
        server: &str,
        tool: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, (String, bool)>;
    async fn notify(&mut self, message: &str) -> Result<(), String>;
}

/// 人审批复（人审节点）。
#[async_trait::async_trait]
pub trait HumanApprover: Send {
    async fn request(&self, prompt: &str) -> Approval;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Approved,
    Rejected,
}

/// 测试/默认审批人：统一放行或拒绝。
pub struct AutoApprover {
    pub approve: bool,
}

#[async_trait::async_trait]
impl HumanApprover for AutoApprover {
    async fn request(&self, _prompt: &str) -> Approval {
        if self.approve {
            Approval::Approved
        } else {
            Approval::Rejected
        }
    }
}

// ---------------------------------------------------------------------------
// 执行结果与状态机
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    Pending,
    Running,
    /// 人审等待中。
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub id: String,
    pub kind: String,
    pub ok: bool,
    pub detail: String,
    #[serde(default)]
    pub reused: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowRunMetrics {
    pub tool_calls: usize,
    pub reused_invocations: usize,
    pub reused_tool_calls: usize,
    pub reserved_task_tool_calls: usize,
}

#[derive(Debug, Clone)]
pub struct WorkflowOutcome {
    pub metrics: WorkflowRunMetrics,
    pub state: WorkflowState,
    pub steps: Vec<StepRecord>,
    /// 失败时回滚到的检查点（最近回滚点 id）。
    pub rollback_to: Option<String>,
}

/// 检查点：工作区快照目录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRef {
    pub id: String,
    pub snapshot_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// 引擎
// ---------------------------------------------------------------------------

/// 工作流执行引擎：权限门禁 → 健康度门禁 → 步骤执行 → 回滚/审计。
pub struct WorkflowEngine {
    flow: WorkflowDefinition,
    flows: HashMap<String, WorkflowDefinition>,
    backend: Box<dyn ActionBackend>,
    approver: Box<dyn HumanApprover>,
    health: SkillHealthStore,
    audit: AuditLog,
    /// 执行上下文（步骤输出写回，供断言/条件引用）。
    ctx: BTreeMap<String, serde_json::Value>,
    checkpoints: Vec<CheckpointRef>,
    work_root: PathBuf,
    snapshots_root: PathBuf,
    executed_steps: u32,
    step_log: Vec<StepRecord>,
    state: WorkflowState,
    depth: u32,
    last_rollback: Option<String>,
    disabled_skills: HashSet<String>,
    /// 协作式取消标志：外部（HTTP abort / 统一任务暂停）置位后，引擎在步骤边界停下。
    cancel: Option<Arc<AtomicBool>>,
    recovery: Option<recovery::RecoverySession>,
    restored_context: recovery::CapturedContext,
    metrics: WorkflowRunMetrics,
}

impl WorkflowEngine {
    pub fn new(
        flow: WorkflowDefinition,
        flows: HashMap<String, WorkflowDefinition>,
        backend: Box<dyn ActionBackend>,
        approver: Box<dyn HumanApprover>,
        health: SkillHealthStore,
        work_root: PathBuf,
    ) -> Self {
        Self {
            flow,
            flows,
            backend,
            approver,
            health,
            audit: AuditLog::default(),
            ctx: BTreeMap::new(),
            checkpoints: Vec::new(),
            // 快照目录必须在 work_root 外部：rollback 会删除并重建 work_root，
            // 快照放内部会被一并删除导致回滚源丢失。
            snapshots_root: work_root
                .parent()
                .map(|parent| {
                    parent.join(format!(
                        ".wf-checkpoints-{}",
                        work_root.file_name().unwrap_or_default().to_string_lossy()
                    ))
                })
                .unwrap_or_else(|| work_root.join(".wf-checkpoints-outside")),
            work_root,
            executed_steps: 0,
            step_log: Vec::new(),
            state: WorkflowState::Pending,
            depth: 0,
            last_rollback: None,
            disabled_skills: HashSet::new(),
            cancel: None,
            recovery: None,
            restored_context: BTreeMap::new(),
            metrics: WorkflowRunMetrics::default(),
        }
    }

    /// 协作式取消：外部把标志置为 true 后，引擎在下一个步骤边界停止并进入
    /// `Aborted`。单个动作一旦开始就执行完——取消不谎报"已中断"。
    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|flag| flag.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    pub fn state(&self) -> WorkflowState {
        self.state
    }

    /// Historical values restored from a protected checkpoint are Null here;
    /// their expression decisions remain available internally.
    pub fn ctx(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.ctx
    }

    /// 显式禁用技能（v1 测试/运行期能力；优先于健康度存储判定）。
    pub fn disable_skill(&mut self, name: &str) {
        self.disabled_skills.insert(name.to_string());
    }

    /// 运行工作流（失败含回滚，返回终态 outcome）。
    pub async fn run(&mut self) -> Result<WorkflowOutcome, String> {
        // 前置条件
        for expr in &self.flow.preconditions {
            if !self.evaluate_expression(expr)? {
                self.state = WorkflowState::Failed;
                self.audit.record(
                    "workflow",
                    "workflow.precondition_failed",
                    Some(self.flow.id.clone()),
                    None,
                    if self.recovery.is_some() {
                        "前置条件不成立（未复制表达式原文）".to_string()
                    } else {
                        format!("前置条件不成立：{expr}")
                    },
                );
                return Ok(self.finish());
            }
        }
        if self.state == WorkflowState::Aborted {
            self.audit.record(
                "workflow",
                "workflow.abort",
                Some(self.flow.id.clone()),
                None,
                "启动前已中止",
            );
            return Ok(self.finish());
        }
        self.state = WorkflowState::Running;
        self.audit.record(
            "workflow",
            "workflow.start",
            Some(self.flow.id.clone()),
            None,
            format!("工作流 {}（{}）启动", self.flow.name, self.flow.id),
        );
        let result = self.execute_steps(&self.flow.steps.clone()).await;
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.contains(WORKFLOW_RECOVERY_REQUIRED))
        {
            return Err(result.unwrap_err());
        }
        if self.cancelled() {
            // 取消是用户主动停止：保留现场，不回滚（回滚属于失败语义）。
            self.state = WorkflowState::Aborted;
            self.audit.record(
                "workflow",
                "workflow.abort",
                Some(self.flow.id.clone()),
                None,
                "执行中收到中止请求",
            );
        } else if let Err(error) = result {
            if error.contains(WORKFLOW_RECOVERY_REQUIRED) {
                return Err(error);
            }
            // Mark recovery as closed before rollback can alter prior effects.
            // A crash during rollback must never replay stale successful facts.
            self.finish_recovery(false)?;
            let rollback_to = self.rollback_to_latest();
            self.last_rollback = rollback_to.clone();
            self.state = WorkflowState::Failed;
            self.audit.record(
                "workflow",
                "workflow.failed",
                Some(self.flow.id.clone()),
                None,
                format!("{error}；回滚={rollback_to:?}"),
            );
        } else {
            self.state = WorkflowState::Succeeded;
            self.audit.record(
                "workflow",
                "workflow.succeeded",
                Some(self.flow.id.clone()),
                None,
                format!("工作流完成，共 {} 步", self.step_log.len()),
            );
        }
        if self.state == WorkflowState::Succeeded {
            self.finish_recovery(true)?;
        } else if self.state == WorkflowState::Failed {
            self.finish_recovery(false)?;
        }
        Ok(self.finish())
    }

    /// 中止：立即停止并保留现场。
    pub fn abort(&mut self) {
        if self.state == WorkflowState::Pending || self.state == WorkflowState::Running {
            self.state = WorkflowState::Aborted;
            self.audit.record(
                "workflow",
                "workflow.abort",
                Some(self.flow.id.clone()),
                None,
                "用户中止",
            );
        }
    }

    fn finish(&self) -> WorkflowOutcome {
        WorkflowOutcome {
            metrics: self.metrics(),
            state: self.state,
            steps: self.step_log.clone(),
            rollback_to: self.last_rollback.clone(),
        }
    }

    fn count_step(&mut self, id: &str, kind: &str, ok: bool, detail: String) -> Result<(), String> {
        self.executed_steps += 1;
        if self.executed_steps > self.flow.max_steps {
            return Err(format!("超过单次运行步数上限（{}）", self.flow.max_steps));
        }
        let detail = if self.recovery.is_some() {
            format!("{kind}: {}", if ok { "completed" } else { "not completed" })
        } else {
            detail
        };
        self.step_log.push(StepRecord {
            id: id.to_string(),
            kind: kind.to_string(),
            ok,
            detail,
            reused: false,
        });
        Ok(())
    }

    /// 权限门禁：scope 声明表查找，默认 deny；Ask 经审批人。
    async fn gate(&mut self, scope: &str) -> Result<(), String> {
        let mode = self
            .flow
            .permissions
            .iter()
            .find(|claim| claim.scope == scope)
            .map(|claim| claim.mode)
            .unwrap_or(PermMode::Deny);
        match mode {
            PermMode::Allow => Ok(()),
            PermMode::Deny => {
                self.audit.record(
                    "workflow",
                    "workflow.permission_deny",
                    Some(self.flow.id.clone()),
                    Some(false),
                    format!("权限拒绝：{scope}"),
                );
                Err(format!("权限拒绝（默认 deny）：{scope}"))
            }
            PermMode::Ask => {
                let approval = self
                    .approver
                    .request(&format!("工作流 {} 请求权限：{scope}", self.flow.name))
                    .await;
                if approval == Approval::Approved {
                    self.audit.record(
                        "workflow",
                        "workflow.permission_ask_approved",
                        Some(self.flow.id.clone()),
                        Some(true),
                        format!("权限确认：{scope}"),
                    );
                    Ok(())
                } else {
                    self.audit.record(
                        "workflow",
                        "workflow.permission_ask_rejected",
                        Some(self.flow.id.clone()),
                        Some(false),
                        format!("权限被拒：{scope}"),
                    );
                    Err(format!("权限被拒：{scope}"))
                }
            }
        }
    }

    async fn execute_steps(&mut self, steps: &[WorkflowStep]) -> Result<(), String> {
        // 子流程/嵌套步骤经 execute_step 间接递归，Rust 要求 async 递归 Box::pin。
        let future = Box::pin(Self::execute_steps_inner(self, steps));
        future.await
    }

    async fn execute_steps_inner(this: &mut Self, steps: &[WorkflowStep]) -> Result<(), String> {
        for step in steps {
            if this.cancelled() {
                return Err("工作流已中止".to_string());
            }
            if this.state == WorkflowState::Aborted {
                return Err("工作流已中止".to_string());
            }
            this.execute_step(step).await?;
        }
        Ok(())
    }

    async fn execute_step(&mut self, step: &WorkflowStep) -> Result<(), String> {
        self.execute_recoverable_step(step).await
    }

    async fn execute_step_inner(&mut self, step: &WorkflowStep) -> Result<(), String> {
        match step {
            WorkflowStep::Sense { id, spec } => {
                self.begin_tool_call()?;
                let value = self.backend.sense(spec).await?;
                self.set_context_value(id.clone(), value.clone());
                self.count_step(id, "sense", true, value.to_string())?;
                Ok(())
            }
            WorkflowStep::Locate { id, spec } => {
                self.begin_tool_call()?;
                let value = self.backend.locate(spec).await?;
                self.set_context_value(id.clone(), value.clone());
                self.count_step(id, "locate", true, value.to_string())?;
                Ok(())
            }
            WorkflowStep::Act { id, scope, spec } => {
                self.gate(scope).await?;
                self.begin_effect()?;
                self.begin_tool_call()?;
                let value = self.backend.act(spec).await?;
                self.set_context_value(id.clone(), value.clone());
                self.count_step(
                    id,
                    "act",
                    true,
                    format!("{} -> {}", spec.action, spec.target),
                )?;
                Ok(())
            }
            WorkflowStep::Assert { id, expr, .. } => {
                let ok = self.evaluate_expression(expr)?;
                self.count_step(id, "assert", ok, format!("{expr} = {ok}"))?;
                if ok {
                    Ok(())
                } else {
                    Err(format!("断言不成立：{expr}（步骤 {id}）"))
                }
            }
            WorkflowStep::InvokeSkill { id, skill, args } => {
                // 健康度门禁
                let state = if self.disabled_skills.contains(skill) {
                    SkillState::Disabled
                } else {
                    self.health.state(skill)
                };
                match state {
                    SkillState::Disabled => {
                        self.count_step(
                            id,
                            "invoke_skill",
                            false,
                            format!("技能 {skill} 已被禁用"),
                        )?;
                        return Err(format!("技能 {skill} 已被禁用（健康度门禁）"));
                    }
                    SkillState::Degraded => {
                        let approval = self
                            .approver
                            .request(&format!("技能 {skill} 健康度 Degraded，是否继续执行？"))
                            .await;
                        if approval != Approval::Approved {
                            self.count_step(
                                id,
                                "invoke_skill",
                                false,
                                format!("技能 {skill} Degraded 未获确认"),
                            )?;
                            return Err(format!("技能 {skill} Degraded 未获确认"));
                        }
                    }
                    SkillState::Active => {}
                }
                self.begin_effect()?;
                self.begin_tool_call()?;
                let result = self.backend.invoke_skill(skill, args).await;
                let ok = result.is_ok();
                let detail = match &result {
                    Ok(value) => value.to_string(),
                    Err(e) => e.clone(),
                };
                self.health.record(skill, ok, None).ok();
                self.count_step(id, "invoke_skill", ok, detail.clone())?;
                if let Ok(value) = result {
                    self.set_context_value(id.clone(), value);
                    Ok(())
                } else {
                    Err(format!("技能执行失败：{detail}"))
                }
            }
            WorkflowStep::InvokeMcp {
                id,
                server,
                tool,
                args,
                scope,
            } => {
                self.gate(scope).await?;
                self.begin_effect()?;
                self.begin_tool_call()?;
                let value = match self.backend.invoke_mcp(server, tool, args).await {
                    Ok(value) => value,
                    Err((_, true)) => {
                        self.begin_tool_call()?;
                        self.audit.record(
                            "workflow",
                            "workflow.step_retry",
                            Some(self.flow.id.clone()),
                            None,
                            format!("Retrying MCP step {id} once"),
                        );
                        self.backend
                            .invoke_mcp(server, tool, args)
                            .await
                            .map_err(|(error, _)| error)?
                    }
                    Err((error, false)) => return Err(error),
                };
                self.set_context_value(id.clone(), value.clone());
                self.count_step(
                    id,
                    "invoke_mcp",
                    true,
                    format!("{server}/{tool} -> {value}"),
                )?;
                Ok(())
            }
            WorkflowStep::HumanApprove { id, prompt } => {
                self.state = WorkflowState::WaitingApproval;
                let approval = self.approver.request(prompt).await;
                self.state = WorkflowState::Running;
                let ok = approval == Approval::Approved;
                self.count_step(id, "human_approve", ok, prompt.clone())?;
                if ok {
                    Ok(())
                } else {
                    Err(format!("人审拒绝：{prompt}"))
                }
            }
            WorkflowStep::Notify { id, message } => {
                self.begin_effect()?;
                self.begin_tool_call()?;
                self.backend.notify(message).await?;
                self.count_step(id, "notify", true, message.clone())?;
                Ok(())
            }
            WorkflowStep::Subflow { id, flow, args } => {
                if self.depth >= self.flow.subflow_depth_limit {
                    self.count_step(
                        id,
                        "subflow",
                        false,
                        format!(
                            "子流程嵌套超过深度上限（{}）",
                            self.flow.subflow_depth_limit
                        ),
                    )?;
                    return Err(format!(
                        "子流程嵌套超过深度上限（{}）",
                        self.flow.subflow_depth_limit
                    ));
                }
                let definition = self
                    .flows
                    .get(flow)
                    .ok_or_else(|| format!("子流程不存在：{flow}"))?
                    .clone();
                // 子流程上下文：前缀注入参数，互不污染父流程。
                let child_ctx: BTreeMap<String, serde_json::Value> = args
                    .iter()
                    .map(|(k, v)| (format!("{id}.{k}"), serde_json::Value::String(v.clone())))
                    .collect();
                let saved_ctx = self.ctx.clone();
                let saved_restored = self.restored_context.clone();
                for (key, value) in child_ctx {
                    self.set_context_value(key, value);
                }
                self.depth += 1;
                let result = self.execute_steps(&definition.steps).await;
                self.depth -= 1;
                match result {
                    Ok(()) => {
                        let summary = serde_json::json!({
                            "flow": flow,
                            "steps": self.step_log.len(),
                        });
                        self.ctx = saved_ctx;
                        self.restored_context = saved_restored;
                        self.set_context_value(id.clone(), summary);
                        self.count_step(id, "subflow", true, format!("子流程 {flow} 完成"))?;
                        Ok(())
                    }
                    Err(e) => {
                        self.ctx = saved_ctx;
                        self.restored_context = saved_restored;
                        self.count_step(id, "subflow", false, format!("子流程 {flow} 失败：{e}"))?;
                        Err(format!("子流程 {flow} 失败：{e}"))
                    }
                }
            }
            WorkflowStep::Loop {
                id,
                body,
                cond,
                max_iter,
            } => {
                let mut iterations = 0u32;
                loop {
                    if self.cancelled() || self.state == WorkflowState::Aborted {
                        return Err("工作流已中止".to_string());
                    }
                    if iterations >= *max_iter {
                        break;
                    }
                    // 循环变量：`{id}.iteration` 可在 cond/body 中引用（从 0 递增）。
                    self.set_context_value(
                        format!("{id}.iteration"),
                        serde_json::json!(iterations),
                    );
                    if let Some(cond_expr) = cond {
                        if !self.evaluate_expression(cond_expr)? {
                            break;
                        }
                    }
                    self.execute_steps(body).await?;
                    iterations += 1;
                }
                self.count_step(
                    id,
                    "loop",
                    true,
                    format!("循环 {} 次（上限 {}）", iterations, max_iter),
                )?;
                Ok(())
            }
            WorkflowStep::Cond {
                id,
                expr,
                then,
                otherwise,
            } => {
                let take_then = self.evaluate_expression(expr)?;
                let branch = if take_then { then } else { otherwise };
                if !branch.is_empty() {
                    self.execute_steps(branch).await?;
                }
                self.count_step(
                    id,
                    "cond",
                    true,
                    format!(
                        "{expr} = {take_then}（{} 分支）",
                        if take_then { "then" } else { "otherwise" }
                    ),
                )?;
                Ok(())
            }
            WorkflowStep::RollbackPoint { id } => {
                self.begin_effect()?;
                self.begin_tool_call()?;
                self.save_checkpoint(id)?;
                self.count_step(id, "rollback_point", true, "已保存快照".to_string())?;
                Ok(())
            }
        }
    }

    fn save_checkpoint(&mut self, id: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.snapshots_root)
            .map_err(|e| format!("创建快照目录失败：{e}"))?;
        let snapshot_dir = self.snapshots_root.join(id);
        if snapshot_dir.exists() {
            std::fs::remove_dir_all(&snapshot_dir).map_err(|e| format!("清理旧快照失败：{e}"))?;
        }
        copy_tree(&self.work_root, &snapshot_dir)?;
        // 快照目录自身不参与快照
        if self.snapshots_root.exists() {
            let _ = std::fs::remove_dir_all(self.snapshots_root.join(id).join(".wf-checkpoints"));
        }
        self.checkpoints.push(CheckpointRef {
            id: id.to_string(),
            snapshot_dir,
        });
        self.audit.record(
            "workflow",
            "workflow.checkpoint",
            Some(self.flow.id.clone()),
            None,
            format!("回滚点 {id} 已保存"),
        );
        Ok(())
    }

    /// 回滚到最近检查点：恢复工作区文件树。
    fn rollback_to_latest(&mut self) -> Option<String> {
        let checkpoint = self.checkpoints.pop()?;
        // 快照位于 work_root 内部（.wf-checkpoints/），先复制到 work_root 外的暂存目录，
        // 再清空 work_root 恢复，避免删除 work_root 时连带销毁快照。
        let staging = self.work_root.with_extension("wf-staging");
        let _ = std::fs::remove_dir_all(&staging);
        let ok = copy_tree(&checkpoint.snapshot_dir, &staging).is_ok();
        if ok {
            let _ = std::fs::remove_dir_all(&self.work_root);
            let _ = copy_tree(&staging, &self.work_root);
        }
        let _ = std::fs::remove_dir_all(&staging);
        self.audit.record(
            "workflow",
            "workflow.rollback",
            Some(self.flow.id.clone()),
            None,
            format!("已回滚到检查点 {}（恢复={ok}）", checkpoint.id),
        );
        Some(checkpoint.id)
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("创建 {} 失败：{e}", dst.display()))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("读取 {} 失败：{e}", src.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() == ".wf-checkpoints" {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else if kind.is_file() {
            std::fs::copy(&from, &to).map_err(|e| format!("复制 {} 失败：{e}", from.display()))?;
        }
    }
    Ok(())
}

mod mock_backend;

pub use mock_backend::MockBackend;
