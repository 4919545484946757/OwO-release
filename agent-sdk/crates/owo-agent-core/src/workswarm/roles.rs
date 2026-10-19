use owo_agent_protocol::TeamTemplateRole;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use super::error::{WorkSwarmError, WorkSwarmResult};
/// 角色规格（组队输入；模板角色的运行时展开）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RoleSpec {
    pub role: String,
    /// 承担者种类：`agent` | `human` | `worker`。
    pub assignee: String,
    /// 内层 worker 名（agent 角色缺省 `agent` 模型驱动；human 角色 = user_id）。
    pub worker: Option<String>,
    /// 上游角色（任务 DAG 边）。
    pub depends_on: Vec<String>,
    pub handoff_contract: Option<String>,
    /// 验证断言（字符串形式，见 [`parse_verify`]）。
    pub verify: Option<String>,
    /// 附加步骤输入（透传给内层 worker：echo 的 text / agent 的 prompt 覆盖等）。
    pub extra_input: Value,
    /// 角色模型（十一期 additive；None = 服务端缺省解析链）。
    pub model: Option<String>,
    /// 角色级写白名单（相对工作区根；空 = 工作区内可写）。
    /// 声明后写角色可与其他范围不重叠的写角色并发落盘（范围租约）。
    pub write_paths: Vec<String>,
    /// 职责能力。review 控制只读与评审语义；未知能力不授予工具权限。
    pub capabilities: Vec<String>,
    /// 模板声明的角色工具白名单（用户自定义工作流）：空 = 沿用角色画像的缺省工具面；
    /// 非空时作为**上界**收窄（见 [`crate::worker_profile::WorkerProfile::apply_declared_tools`]），
    /// 绝不会因为声明而放宽到画像能力面之外。
    pub tools: Vec<String>,
}

impl RoleSpec {
    pub fn agent(role: impl Into<String>) -> Self {
        let role = role.into();
        let capabilities = if super::util::is_review_role_name(&role) {
            vec!["review".to_string()]
        } else {
            Vec::new()
        };
        Self {
            role,
            assignee: "agent".to_string(),
            worker: None,
            depends_on: Vec::new(),
            handoff_contract: None,
            verify: None,
            extra_input: Value::Null,
            model: None,
            write_paths: Vec::new(),
            capabilities,
            tools: Vec::new(),
        }
    }

    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities
            .iter()
            .any(|value| value.eq_ignore_ascii_case(capability))
    }

    pub fn is_reviewer(&self) -> bool {
        super::util::is_review_role(&self.role, &self.capabilities)
    }
}

/// 校验角色级写白名单：相对路径、非空、不允许 `..`/根/盘符。
///
/// 写路径是权限声明（权限默认 deny 口径）：非法路径在组队期直接拒绝，
/// 不允许把越界语义带进运行期。
pub fn validate_role_write_paths(role: &str, paths: &[String]) -> Result<(), String> {
    validate_role_write_paths_with_capabilities(role, &[], paths)
}

pub fn validate_role_write_paths_with_capabilities(
    role: &str,
    capabilities: &[String],
    paths: &[String],
) -> Result<(), String> {
    if super::util::is_review_role(role, capabilities) && !paths.is_empty() {
        return Err(format!("评审角色 {role} 不能声明写路径"));
    }
    for raw in paths {
        let path = Path::new(raw.trim());
        if raw.trim().is_empty() {
            return Err(format!("角色 {role} 的写路径不能为空"));
        }
        if path.is_absolute() {
            return Err(format!("角色 {role} 的写路径必须是相对路径：{raw}"));
        }
        for component in path.components() {
            match component {
                Component::Normal(_) => {}
                Component::CurDir => {}
                _ => {
                    return Err(format!("角色 {role} 的写路径不允许 `..`/根/盘符：{raw}"));
                }
            }
        }
    }
    Ok(())
}

impl From<TeamTemplateRole> for RoleSpec {
    fn from(r: TeamTemplateRole) -> Self {
        let capabilities = if r.capabilities.is_empty() && super::util::is_review_role_name(&r.role)
        {
            vec!["review".to_string()]
        } else {
            r.capabilities
        };
        Self {
            role: r.role,
            assignee: r.assignee,
            worker: r.worker,
            depends_on: r.depends_on,
            handoff_contract: r.handoff_contract,
            verify: r.verify,
            extra_input: Value::Null,
            model: r.model,
            write_paths: r.write_paths,
            capabilities,
            tools: r.tools,
        }
    }
}

/// 并行开发角色组（十一期）：`lead`（只读拆解）→ `w1..wN`（依赖 lead，彼此无依赖 =
/// 同一 wave 真并行）→ `leader`（依赖全部 writer，汇总交付）。
///
/// - `writers` 收敛到 2..=8（≥2 才有并行收益；上限防误配爆并发/成员上限）；
/// - lead 被要求输出版本化 TaskGraphV1：任务数与 worker 槽位解耦，校验后按依赖绑定任务；
/// - 运行期据此**动态**给 writer 收窄写范围 + 注入任务验收与验证要求；
/// - 未声明 `--write` 时 writer 初始为只读（权限默认 deny），由 lead 分配写范围后
///   升级为写角色；若 lead 产物不可解析，writer 保持只读但产物仍可经 Artifact 交付。
pub fn parallel_roles(writers: usize) -> Vec<RoleSpec> {
    let writers = writers.clamp(2, 8);
    let writer_names: Vec<String> = (1..=writers).map(|i| format!("w{i}")).collect();

    let mut lead = RoleSpec::agent("lead");
    lead.handoff_contract = Some(format!(
        "只读拆分目标，输出 TaskGraphV1 JSON。任务数 1 到 128，可多于 {writers} 个 Worker 槽位。\
         对象包含 version=1 和 tasks 数组；每项包含 task_id、worker（可省略）、task、depends_on、requirement_quotes、\
         read_refs、write_paths、contract_refs、required_capabilities、estimated_effort、verification、risk、priority、acceptance。requirement_quotes 必须是从当前用户目标原文逐字引用的数组；每条引用也必须逐字出现在当前任务的 task 或 acceptance 字段；每个任务至少引用一条实际由该任务负责的用户要求，且显式验收清单中的每项都必须被至少一个任务精确引用。模型只估算 estimated_effort；宿主按该值和 Worker 角色上限计算每任务每次尝试的模型请求上限，模型不得输出或覆盖预算字段。\
         依赖引用 task_id；重叠写范围必须有依赖顺序；目标和验收不能为空。\
         代码实现任务必须包含 required=true 的 workspace-command-success-v1 行为检查，scope.kind=workspace_paths 且 relative_paths 覆盖被改代码；arguments.command 使用宿主登记的测试入口，并在 required_capabilities 声明 run_command。\
         scope 路径必须位于该任务 write_paths 内；required=true，resources 用 cpu_slots=1、memory_mb=8..128、exclusive_workspace=false、timeout_ms=1..30000。\
         仅对报告文本断言使用 non_empty、contains:<文本> 或 equals:<文本>。\
         验证命令必须是单条测试命令，执行仍经现有工具审批与沙箱；文件存在/非空/文本包含只能证明静态条件，不能替代行为测试。risk 使用 low/normal/high/critical。\
         不要无意义拆分，只声明任务确需写入的路径。"
    ));
    lead.verify = Some("non_empty".to_string());

    let mut roles = vec![lead];
    for (index, name) in writer_names.iter().enumerate() {
        let mut writer = RoleSpec::agent(name.clone());
        writer.depends_on = vec!["lead".to_string()];
        writer.handoff_contract = Some(format!(
            "你是并行执行者 {name}（第 {} 路）：只执行当前步骤输入中的 assigned_task，\
             按 assigned_acceptance 与 assigned_verification 验收；写权限仅限 assigned_write_paths。若声明宿主命令检查，必须执行完全相同的命令，检查后不得再修改受测文件；不要承担队友任务；\
             完成当前任务后交付结果与证据。",
            index + 1
        ));
        writer.verify = Some("non_empty".to_string());
        roles.push(writer);
    }

    let mut leader = RoleSpec::agent("leader");
    leader.depends_on = writer_names;
    leader.handoff_contract = Some(
        "汇总全部 writer 产物：逐条核对子任务验收与证据，标出未完成/冲突/越界项，\
         产出最终交付物与交付清单（不要重做已完成的子任务）。"
            .to_string(),
    );
    leader.verify = Some("non_empty".to_string());
    roles.push(leader);
    roles
}

/// 运行元数据（sidecar 文件 `<run_dir>/<team_id>-meta.json`）：
/// correlation_id + 角色规格（worker 构建的唯一来源；replace 在此生效）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMeta {
    pub team_id: String,
    pub correlation_id: String,
    pub roles: Vec<RoleSpec>,
    /// 八期一路：模板 id（内置模板 → 角色专属 Prompt 段；动态组队为 None）。
    #[serde(default)]
    pub template_id: Option<String>,
    /// 八期一路：角色 → 调用预算（模板 `budget_calls_per_role`；Prompt 预算段与
    /// 运行期跳过的 `saved_budget_calls` 口径来源）。旧 sidecar 缺省为空。
    #[serde(default)]
    pub budgets: BTreeMap<String, usize>,
    /// 十一期：并行开发模式标记（lead 拆解 → w1..wN 并行 → leader 汇总；
    /// 运行期动态应用 lead 产物的子任务分配）。旧 sidecar 缺省 false。
    #[serde(default)]
    pub parallel: bool,
}

impl RunMeta {
    pub(super) fn file_path(run_dir: &Path, team_id: &str) -> PathBuf {
        run_dir.join(format!("{team_id}-meta.json"))
    }

    pub(super) fn save(&self, run_dir: &Path) -> WorkSwarmResult<()> {
        std::fs::create_dir_all(run_dir).map_err(|e| WorkSwarmError::Io(e.to_string()))?;
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(Self::file_path(run_dir, &self.team_id), json)
            .map_err(|e| WorkSwarmError::Io(e.to_string()))
    }

    pub(super) fn load(run_dir: &Path, team_id: &str) -> WorkSwarmResult<Self> {
        let path = Self::file_path(run_dir, team_id);
        let raw = std::fs::read_to_string(&path)
            .map_err(|_| WorkSwarmError::NotFound(format!("运行元数据 {team_id} 不存在")))?;
        serde_json::from_str(&raw).map_err(WorkSwarmError::from)
    }
}

/// 默认接力样例角色（§9.0：planner → builder → critic → leader）。
pub fn default_relay_roles() -> Vec<RoleSpec> {
    let mut planner = RoleSpec::agent("planner");
    planner.handoff_contract = Some(
        "把目标拆解为可执行的建设方案：明确产物要求、验收要点与风险，输出方案大纲（非代码）。"
            .to_string(),
    );
    planner.verify = Some("non_empty".to_string());
    let mut builder = RoleSpec::agent("builder");
    builder.depends_on = vec!["planner".to_string()];
    builder.handoff_contract = Some(
        "依据上游方案（planner 产物）产出主交付物草稿；输出产物正文本身，不要解释过程。"
            .to_string(),
    );
    builder.verify = Some("non_empty".to_string());
    let mut critic = RoleSpec::agent("critic");
    critic.depends_on = vec!["builder".to_string()];
    critic.handoff_contract = Some(
        "只读评审上游草稿（不修改原文）：检查完整性、一致性与风险，输出 JSON {\"approved\":bool,\"score\":0-100,\"comments\":[..]}。"
            .to_string(),
    );
    critic.verify = Some("non_empty".to_string());
    let mut leader = RoleSpec::agent("leader");
    leader.depends_on = vec!["critic".to_string()];
    leader.handoff_contract =
        Some("最终裁决：综合上游草稿与评审意见采纳或修正，输出最终交付物与交付清单。".to_string());
    leader.verify = Some("non_empty".to_string());
    vec![planner, builder, critic, leader]
}

// ---------------------------------------------------------------------------
// 取消令牌（跨阶段取消传播；A3：fan-out/运行支持父任务取消）
// ---------------------------------------------------------------------------

/// 团队运行取消令牌（watch 语义；运行任务每阶段/每次人节点等待都监听）。
#[derive(Debug, Clone)]
pub struct CancelToken {
    tx: tokio::sync::watch::Sender<bool>,
    aborted: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        let (tx, _) = tokio::sync::watch::channel(false);
        Self {
            tx,
            aborted: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.aborted
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.tx.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.aborted.load(std::sync::atomic::Ordering::SeqCst) || *self.tx.borrow()
    }

    /// Shared abort flag for in-flight GoalRunner work; cancellation sets it before waking watchers.
    pub fn abort_signal(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.aborted)
    }

    pub fn rx(&self) -> tokio::sync::watch::Receiver<bool> {
        self.tx.subscribe()
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// 等待取消（值变 true 返回；发送端关闭返回 false）。
pub async fn wait_cancel(token: &CancelToken) -> bool {
    let mut rx = token.rx();
    if *rx.borrow() {
        return true;
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return true;
        }
    }
    false
}
#[cfg(test)]
mod cancel_token_tests {
    use super::CancelToken;
    use std::sync::atomic::Ordering;

    #[test]
    fn cancel_sets_shared_abort_flag_before_returning() {
        let token = CancelToken::new();
        let abort = token.abort_signal();
        assert!(!abort.load(Ordering::SeqCst));

        token.cancel();

        assert!(abort.load(Ordering::SeqCst));
        assert!(token.is_cancelled());
    }
}
