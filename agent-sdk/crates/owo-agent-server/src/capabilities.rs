//! 功能目录（§8.3）：`CapabilityDescriptor` 注册表——UI 功能目录、CLI `capabilities`、
//! 诊断页与帮助文档的**共同来源**。
//!
//! 设计约束（审计 §8.3）：
//! - 按「任务/能力」组织，不按内部模块组织；
//! - 只有存在用户入口、权限说明、契约测试和诊断状态的能力才能标记 `Stable`
//!   （当前注册表从保守起步：均 Beta/Experimental，满足四要素后逐项晋升）；
//! - 独立编译约束（沿 notes_api 惯例）：不使用 `crate::`/`super::`，server 类型
//!   写全限定名 `owo_agent_server::AppState`。

use axum::extract::State;
use axum::Json;
use owo_agent_core::EffectClass;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

/// 文档命名对齐：审计 §8.3 称 `ToolEffectClass`，core 实名 `EffectClass`。
pub type ToolEffectClass = EffectClass;

/// 能力唯一标识（`'static` 字符串，跨 UI/CLI/文档引用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityId(pub &'static str);

impl CapabilityId {
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// 成熟度：`Stable`（四要素齐备）/ `Beta`（主流程可用，诊断面未全）/ `Experimental`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMaturity {
    Stable,
    Beta,
    Experimental,
}

impl CapabilityMaturity {
    pub fn as_str(self) -> &'static str {
        match self {
            CapabilityMaturity::Stable => "stable",
            CapabilityMaturity::Beta => "beta",
            CapabilityMaturity::Experimental => "experimental",
        }
    }
}

/// 用户入口：UI 面板 / CLI 命令 / HTTP 路由（kind ∈ {"ui","cli","http"}）。
#[derive(Debug, Clone, Copy)]
pub struct Entrypoint {
    pub kind: &'static str,
    pub target: &'static str,
    pub label: &'static str,
}

/// 单个用户能力的描述符（§8.3 结构）。
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub user_name: &'static str,
    pub summary: &'static str,
    pub maturity: CapabilityMaturity,
    pub entrypoints: &'static [Entrypoint],
    pub required_effects: &'static [ToolEffectClass],
    /// 高级能力：默认折叠，不进入新手主流程。
    pub advanced: bool,
    pub dependencies: &'static [CapabilityId],
}

const E_PLUGIN_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "cli",
        target: "owo-agent plugin catalog|validate|check|sign|install|update|uninstall",
        label: "Agent Plugins 规范包：校验、签名、安装与更新",
    },
    Entrypoint {
        kind: "http",
        target: "GET /plugins/market, POST /plugins/market/install|update|verify|scan",
        label: "插件市场：本地索引、规范包安装/更新（吊销列表生效）",
    },
    Entrypoint {
        kind: "ui",
        target: "工作台 →「插件与技能」面板",
        label: "查看包格式、MCP 服务器与 hooks 注入",
    },
    Entrypoint {
        kind: "service",
        target: "plugin.json / mcp.json / skills/ / plugin.sig.json",
        label: "清单 + 技能 + MCP + 可选 ed25519 包签名（可移植包格式）",
    },
];

const E_SHAREDCORE_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "cli",
        target: "owo-agent plugin install plugins/owo-sharedcore",
        label: "安装/启用 SharedCore 插件（禁用即移除工具面）",
    },
    Entrypoint {
        kind: "service",
        target: "sharedcore_init|status|context|task_add|plan|claim|update|contract|event|brief",
        label: "10 个协调工具（MCP，经宿主权限/审计）",
    },
    Entrypoint {
        kind: "cli",
        target: "python agent-sdk/scripts/sharedcore-selfcheck.py",
        label: "端到端自检（批次/冲突/契约 CAS/简报）",
    },
    Entrypoint {
        kind: "ui",
        target: "工作台 →「SharedCore」面板",
        label: "看任务 DAG、并行批次与共享事件（后续接入）",
    },
];

const E_PLAYBOOK_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "cli",
        target: "owo-agent playbook validate|plan|init|next|record|status|list",
        label: "剧本定义、校验与逐步骤运行",
    },
    Entrypoint {
        kind: "cli",
        target: "owo-agent playbook archive|history|diff",
        label: "运行归档、历史与退化对比",
    },
    Entrypoint {
        kind: "http",
        target:
            "GET /playbooks, GET /playbooks/status, GET /playbooks/history, POST /playbooks/record",
        label: "桌面面板数据面（只读写，不执行命令）",
    },
    Entrypoint {
        kind: "ui",
        target: "工作台 →「任务剧本」面板",
        label: "看进度、历史与逐步记录结果",
    },
    Entrypoint {
        kind: "service",
        target: "playbook_next / playbook_record / playbook_check / playbook_run",
        label: "模型侧工具组（需开启 tool_capabilities.playbooks；执行经审批）",
    },
];

const E_SESSION_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "http",
        target: "POST /session",
        label: "新建会话",
    },
    Entrypoint {
        kind: "http",
        target: "POST /session/{id}/turn",
        label: "发起回合",
    },
    Entrypoint {
        kind: "http",
        target: "GET /sessions",
        label: "会话列表",
    },
    Entrypoint {
        kind: "cli",
        target: "owo-agent-cli",
        label: "命令行会话",
    },
    Entrypoint {
        kind: "cli",
        target: "owo-agent sessions [--json] [--all] [--limit N]",
        label: "会话列表（标出 turn --continue 会续接的会话）",
    },
    Entrypoint {
        kind: "cli",
        target: "owo-agent turn --continue|--session <id>",
        label: "非交互续接会话（对标 codex exec resume）",
    },
];

const E_WORKFLOW_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "cli",
        target:
            "owo-agent workflow list|show|validate|review|create|delete|draft|adopt|export|import",
        label: "自定义工作流全链路（validate/review 离线，其余经 Daemon）",
    },
    Entrypoint {
        kind: "http",
        target: "GET/POST /workflows/user + POST /workflows/user/validate",
        label: "用户工作流 CRUD 与校验（权限面仍由策略与审批决定）",
    },
    Entrypoint {
        kind: "http",
        target: "POST /workflows/draft （以及 /workflows/draft/{id}/adopt）",
        label: "AI 生成候选工作流：只提案不启用，人工采纳才进注册表",
    },
    Entrypoint {
        kind: "http",
        target: "POST /workflows/user/{id}/export 与 /workflows/import(/review)",
        label: "社区包分享：sha256 校验 + 能力面评审门禁（block 即拒绝落盘）",
    },
];

const E_REVIEW_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "cli",
        target: "owo-agent review [--path <文件>] [--accept/--reject <序号|all>] [--json]",
        label: "逐 hunk 审阅工作树改动（离线，依赖 git）",
    },
    Entrypoint {
        kind: "http",
        target: "GET /session/{id}/hunks",
        label: "会话改动的逐 hunk 视图（含 conflicts: 已被用户改动的文件）",
    },
    Entrypoint {
        kind: "http",
        target: "POST /session/{id}/hunks/apply",
        label: "按序号接受/拒绝 hunk 并写回（用户改过文件 → 409 拒绝覆盖）",
    },
];

const E_MCP_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "cli",
        target: "owo-agent mcp list|get|add|remove",
        label: "命令行管理（有核心时热连接，否则写配置下次启动生效）",
    },
    Entrypoint {
        kind: "http",
        target: "GET /mcp",
        label: "已接入服务器列表",
    },
    Entrypoint {
        kind: "http",
        target: "POST /mcp/add",
        label: "接入服务器",
    },
    Entrypoint {
        kind: "http",
        target: "POST /mcp/remove",
        label: "移除服务器",
    },
    Entrypoint {
        kind: "http",
        target: "GET /mcp/health",
        label: "服务器健康状态",
    },
];

const E_TRACES_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "http",
        target: "GET /traces",
        label: "回合列表",
    },
    Entrypoint {
        kind: "http",
        target: "GET /traces/{index}",
        label: "回放详情",
    },
];

const E_MEMORY_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "http",
        target: "GET /memory/recall",
        label: "记忆检索",
    },
    Entrypoint {
        kind: "http",
        target: "GET /memory/observations",
        label: "观察记录",
    },
    Entrypoint {
        kind: "http",
        target: "POST /memory/mine-skill",
        label: "挖掘流程技能",
    },
    Entrypoint {
        kind: "http",
        target: "POST /memory/clear",
        label: "清空记忆",
    },
];

const E_USAGE_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "http",
        target: "GET /usage",
        label: "模型用量与预算",
    },
    Entrypoint {
        kind: "http",
        target: "POST /usage/topup",
        label: "预算加额",
    },
];

const E_FLEET_ENTRYPOINTS: &[Entrypoint] = &[Entrypoint {
    kind: "http",
    target: "GET /fleet/*",
    label: "舰队节点与任务",
}];

const E_DIAG_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "http",
        target: "GET /health",
        label: "服务健康",
    },
    Entrypoint {
        kind: "http",
        target: "GET /usage",
        label: "用量面板",
    },
];

const E_TRUST_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "http",
        target: "GET /workspace/trust",
        label: "查看工作区信任状态",
    },
    Entrypoint {
        kind: "http",
        target: "POST /workspace/trust",
        label: "信任/撤销信任",
    },
    Entrypoint {
        kind: "cli",
        target: "owo-agent trust",
        label: "命令行信任管理",
    },
    Entrypoint {
        kind: "ui",
        target: "工作区页 · 工作区信任",
        label: "桌面端信任按钮",
    },
];

const E_TASKS_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "ui",
        target: "工作台 · 任务运行",
        label: "统一任务列表、详情、检查点、审批与验证",
    },
    Entrypoint {
        kind: "http",
        target: "GET/POST /agent-tasks",
        label: "列出/创建统一任务（幂等键去重）",
    },
    Entrypoint {
        kind: "http",
        target: "POST /agent-tasks/{id}/start|pause|resume|cancel",
        label: "任务状态机动作（涉及写入/命令时仍需主界面审批）",
    },
    Entrypoint {
        kind: "http",
        target: "GET /agent-tasks/{id}/events/stream",
        label: "SSE 实时事件（after 游标断线补拉）",
    },
    Entrypoint {
        kind: "http",
        target: "POST /agent-tasks/{id}/verify",
        label: "独立验证：只认可观察结果，不以模型自述代替",
    },
];

const E_WORKSPACE_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "ui",
        target: "任务运行 · 工作区改动",
        label: "查看 Diff，应用/拒绝/回滚/释放（拒绝需二次确认）",
    },
    Entrypoint {
        kind: "http",
        target: "POST /workspaces",
        label: "为任务创建隔离工作区（Git worktree）",
    },
    Entrypoint {
        kind: "http",
        target: "GET /workspaces/{id}/diff",
        label: "读取隔离工作区的改动",
    },
    Entrypoint {
        kind: "http",
        target: "POST /workspaces/{id}/apply|reject|rollback",
        label: "应用、拒绝或回滚改动",
    },
    Entrypoint {
        kind: "http",
        target: "DELETE /workspaces/{id}",
        label: "释放工作区租约",
    },
];

const E_AUTOMATION_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "ui",
        target: "导航 · 定时任务",
        label: "创建间隔/每天/单次提醒或任务，查看执行记录",
    },
    Entrypoint {
        kind: "http",
        target: "GET/POST /automations",
        label: "列出/创建自动化任务",
    },
    Entrypoint {
        kind: "http",
        target: "GET /automations/runs",
        label: "执行记录（含 skipped/failed 原因）",
    },
    Entrypoint {
        kind: "http",
        target: "POST /automations/{id}/toggle 与 DELETE /automations/{id}",
        label: "启停与删除",
    },
];

const E_BRIDGE_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "http",
        target: "POST /agent-bridge/messages",
        label: "外部入口投递消息（protocol_version=1，message_id 幂等）",
    },
    Entrypoint {
        kind: "http",
        target: "GET /agent-bridge/tasks/{id}/status",
        label: "查询任务状态（只读）",
    },
    Entrypoint {
        kind: "http",
        target: "GET/PUT /agent-bridge/preferences",
        label: "通知偏好；授权仍只在主界面完成",
    },
];

/// 功能目录注册表（§8.3 单一来源）。新增能力：在此追加条目 + 同步路由契约测试。
pub static CAPABILITIES: &[CapabilityDescriptor] = &[
    CapabilityDescriptor {
        id: CapabilityId("plugins.agent-plugins"),
        user_name: "Agent Plugins 规范包（技能 + MCP + 签名）",
        summary: "可移植的插件包：plugin.json + skills/ + mcp.json，可选 ed25519 包签名；经市场安装/更新并受吊销列表约束，安装前做静态风险扫描",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_PLUGIN_ENTRYPOINTS,
        required_effects: &[
            ToolEffectClass::Read,
            ToolEffectClass::Write,
            ToolEffectClass::Execute,
        ],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("workflow.playbooks"),
        user_name: "任务剧本（按进度推进的工作流）",
        summary: "把一类任务写成可校验的步骤剧本：定义/校验/计划/逐步骤执行与记录、归档与退化对比；可由 /advance --playbook 自主推进，桌面面板可看进度",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_PLAYBOOK_ENTRYPOINTS,
        required_effects: &[
            ToolEffectClass::Read,
            ToolEffectClass::Write,
            ToolEffectClass::Execute,
        ],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("agents.sharedcore"),
        user_name: "多 Agent 共享上下文并行开发",
        summary: "SharedCore 插件：项目级结构化上下文（规则/契约/任务 DAG/事件）由多个子 Agent 共享；按写范围拆分并行批次，契约变更走版本 CAS，状态落在工作区 .sharedcore/",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_SHAREDCORE_ENTRYPOINTS,
        required_effects: &[
            ToolEffectClass::Read,
            ToolEffectClass::Write,
            ToolEffectClass::Execute,
        ],
        advanced: false,
        dependencies: &[
            CapabilityId("plugins.agent-plugins"),
            CapabilityId("sessions.turns"),
        ],
    },
    CapabilityDescriptor {
        id: CapabilityId("workflows.custom"),
        user_name: "用户自定义多 Agent 工作流",
        summary: "用角色/依赖/工具/交付契约描述任意特定功能的工作流：本地校验与能力面评审、AI 生成候选、社区包导入导出",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_WORKFLOW_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read, ToolEffectClass::Write],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("changes.review"),
        user_name: "逐 hunk 变更审阅",
        summary: "按 hunk 审阅工作树相对 HEAD 的改动，可单独接受/拒绝（拒绝即还原该段，便于回灌模型改法）",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_REVIEW_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read, ToolEffectClass::Write],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("sessions.turns"),
        user_name: "会话与回合",
        summary: "创建会话、发起多轮回合，附件、diff/revert 与审计全程可追溯",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_SESSION_ENTRYPOINTS,
        required_effects: &[],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("mcp.servers"),
        user_name: "MCP 服务器接入",
        summary: "接入/移除 stdio 或 http 的 MCP 服务器，查看每服务器健康与熔断状态",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_MCP_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Execute],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("traces.replay"),
        user_name: "回合回放",
        summary: "按序浏览历史回合的步骤、用量与最终输出，用于复盘与诊断",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_TRACES_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read],
        advanced: false,
        dependencies: &[CapabilityId("sessions.turns")],
    },
    CapabilityDescriptor {
        id: CapabilityId("memory.skill-mining"),
        user_name: "记忆与技能沉淀",
        summary: "检索情景记忆，并把观察到的操作序列泛化为可复用的流程技能包",
        maturity: CapabilityMaturity::Experimental,
        entrypoints: E_MEMORY_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read, ToolEffectClass::Write],
        advanced: true,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("usage.budget"),
        user_name: "用量与预算",
        summary: "查看模型用量、成本估算与预算熔断状态，必要时加额解除熔断",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_USAGE_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("fleet.control"),
        user_name: "舰队控制面",
        summary: "注册与管理多节点舰队，把目标分发到已注册节点执行",
        maturity: CapabilityMaturity::Experimental,
        entrypoints: E_FLEET_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Execute],
        advanced: true,
        dependencies: &[CapabilityId("sessions.turns")],
    },
    CapabilityDescriptor {
        id: CapabilityId("diagnostics"),
        user_name: "健康与诊断",
        summary: "服务健康、运行状态与用量诊断信息汇总",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_DIAG_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("workspace.trust"),
        user_name: "工作区信任",
        summary: "未信任的工作区不加载 hooks/插件 MCP/权限放宽/白名单（受限模式），显式信任后生效",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_TRUST_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("media.image"),
        user_name: "文生图",
        summary: "按提示词生成图片并写入工作区 outputs/；小图（≤512 KiB）直接在对话里预览，需要 OPENAI_API_KEY 或本机端点",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_MEDIA_IMAGE_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Execute, ToolEffectClass::Write],
        advanced: true,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("media.speech"),
        user_name: "语音合成",
        summary: "把文本合成音频并写入工作区 outputs/；短音频（≤512 KiB）可在对话里直接试听，端点与密钥同文生图",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_MEDIA_SPEECH_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Execute, ToolEffectClass::Write],
        advanced: true,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("tasks.runtime"),
        user_name: "统一任务运行",
        summary: "创建、暂停、恢复、取消统一任务，查看检查点、审批、验证与 SSE 实时事件；\
                  幂等键保证同一时段或同一次请求不会重复建任务",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_TASKS_ENTRYPOINTS,
        required_effects: &[],
        advanced: false,
        dependencies: &[],
    },
    CapabilityDescriptor {
        id: CapabilityId("workspace.isolation"),
        user_name: "隔离工作区与 Diff 审阅",
        summary: "在 Git worktree 中执行改动，先在任务详情审阅 Diff 再应用/拒绝/回滚；\
                  非 Git 工作区只能只读审阅，不支持隐式回退到直接改工作树",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_WORKSPACE_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read, ToolEffectClass::Write],
        advanced: false,
        dependencies: &[CapabilityId("tasks.runtime")],
    },
    CapabilityDescriptor {
        id: CapabilityId("automations.schedule"),
        user_name: "定时任务",
        summary: "按间隔/每天（可指定 UTC 或固定时区偏移）/单次触发提醒或无人值守任务；\
                  执行记录标注跳过与失败原因，同一时段重复 tick 不会重复建任务",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_AUTOMATION_ENTRYPOINTS,
        required_effects: &[],
        advanced: false,
        dependencies: &[CapabilityId("tasks.runtime")],
    },
    CapabilityDescriptor {
        id: CapabilityId("bridge.external"),
        user_name: "外部入口桥接",
        summary: "protocol_version=1 的外部消息接口与任务状态查询；审批只在主界面完成，\
                  外部入口不能代替用户授权",
        maturity: CapabilityMaturity::Experimental,
        entrypoints: E_BRIDGE_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Read],
        advanced: true,
        dependencies: &[CapabilityId("tasks.runtime")],
    },
    CapabilityDescriptor {
        id: CapabilityId("chat.conversation"),
        user_name: "纯对话模式",
        summary: "面向问答与学习：只保留联网检索和「向你提问」，不开放本地文件读写、命令执行与桌面控制；\
                  需要改代码时切换到 Agent 模式",
        maturity: CapabilityMaturity::Beta,
        entrypoints: E_CHAT_MODE_ENTRYPOINTS,
        required_effects: &[ToolEffectClass::Execute],
        advanced: false,
        dependencies: &[],
    },
];

const E_CHAT_MODE_ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint {
        kind: "ui",
        target: "输入区 · 模式 chip / Ctrl+Shift+M",
        label: "切换 Agent / 纯对话",
    },
    Entrypoint {
        kind: "http",
        target: "POST /session/{id}/mode",
        label: "设置会话模式",
    },
    Entrypoint {
        kind: "ui",
        target: "输入区 · 提示词 chip",
        label: "选择或自定义系统提示词",
    },
];

const E_MEDIA_IMAGE_ENTRYPOINTS: &[Entrypoint] = &[Entrypoint {
    kind: "tool",
    target: "generate_image",
    label: "生成图片",
}];

const E_MEDIA_SPEECH_ENTRYPOINTS: &[Entrypoint] = &[Entrypoint {
    kind: "tool",
    target: "text_to_speech",
    label: "合成语音",
}];

/// GET /capabilities：功能目录（"我现在能做什么"页的数据面）。
/// Configuration evidence only, using the exact resolver used by media execution.
#[cfg(test)]
fn media_connection_views() -> (Value, Value) {
    use owo_agent_core::gateway::media_config::{media_connection_status, MediaService};
    (
        media_connection_status(MediaService::ImageGeneration),
        media_connection_status(MediaService::TextToSpeech),
    )
}

pub async fn capabilities_list(State(state): State<Arc<AppState>>) -> Json<Value> {
    use owo_agent_core::gateway::{
        media_config::MediaService,
        media_profiles::{checked_status, MediaProfileStore},
    };
    let store = MediaProfileStore::new(&state.data_root);
    let snapshot = tokio::task::spawn_blocking(move || {
        let catalog = store.load()?;
        let checks = store.checks(catalog.revision)?;
        Ok::<_, String>((catalog, checks))
    })
    .await
    .unwrap_or_else(|_| Err("media_profiles/storage_error: 连接读取任务失败".into()));
    let image_connection = snapshot
        .as_ref()
        .map(|(c, checks)| checked_status(c, MediaService::ImageGeneration, checks))
        .unwrap_or_else(|reason| {
            owo_agent_core::gateway::media_profiles::invalid_status(
                MediaService::ImageGeneration,
                reason.clone(),
            )
        });
    let speech_connection = snapshot
        .as_ref()
        .map(|(c, checks)| checked_status(c, MediaService::TextToSpeech, checks))
        .unwrap_or_else(|reason| {
            owo_agent_core::gateway::media_profiles::invalid_status(
                MediaService::TextToSpeech,
                reason.clone(),
            )
        });
    let items: Vec<Value> = CAPABILITIES
        .iter()
        .map(|capability| {
            json!({
                "id": capability.id.as_str(),
                "user_name": capability.user_name,
                "summary": capability.summary,
                "maturity": capability.maturity.as_str(),
                "entrypoints": capability.entrypoints.iter().map(|entrypoint| json!({
                    "kind": entrypoint.kind,
                    "target": entrypoint.target,
                    "label": entrypoint.label,
                })).collect::<Vec<Value>>(),
                "required_effects": capability.required_effects.iter()
                    .map(|class| class.label()).collect::<Vec<&'static str>>(),
                "advanced": capability.advanced,
                "dependencies": capability.dependencies.iter()
                    .map(|id| id.as_str()).collect::<Vec<&'static str>>(),
            })
        })
        .collect();
    let stable = CAPABILITIES
        .iter()
        .filter(|capability| capability.maturity == CapabilityMaturity::Stable)
        .count();
    let beta = CAPABILITIES
        .iter()
        .filter(|capability| capability.maturity == CapabilityMaturity::Beta)
        .count();
    let experimental = CAPABILITIES
        .iter()
        .filter(|capability| capability.maturity == CapabilityMaturity::Experimental)
        .count();
    Json(json!({
        "media_connections": {"image_generation": image_connection, "text_to_speech": speech_connection},
        "count": CAPABILITIES.len(),
        "maturity": { "stable": stable, "beta": beta, "experimental": experimental },
        "capabilities": items,
        "constraints": { "request_read_only": true, "scoped_turn_cancellation": true, "owobot_scoped_model": true, "owobot_project_roles": true, "structured_input_parts": true, "custom_model_connection": true, "model_input_capability_admission": true, "model_input_preflight": true, "custom_model_env_credentials": true, "durable_session_facts": true, "media_image_generation": image_connection["configured"] == true, "media_speech": speech_connection["configured"] == true, "chat_mode": true },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 配置投影不得把工具注册、端点或凭据存在误报为连接/服务验证。
    #[test]
    fn media_configuration_views_do_not_claim_connection_or_service_verification() {
        let (image, speech) = media_connection_views();
        for view in [image, speech] {
            assert!(view["configured"].is_boolean());
            assert!(view["connected"].is_null());
            assert!(view["verified"].is_null());
            assert!(view["last_verified_at"].is_null());
            assert!(view.get("api_key").is_none());
        }
    }

    /// constraints 里必须同时给出图片与语音两个能力位（源码级契约，避免路由响应漏字段）。
    #[test]
    fn constraints_declare_media_flags() {
        let source = include_str!("capabilities.rs");
        for key in ["media_image_generation", "media_speech"] {
            assert!(source.contains(&format!("\"{key}\":")), "缺少能力位 {key}");
        }
        // 用拼接构造前缀，避免断言本身命中"密钥样式"。
        let key_prefix = format!("{}-", "sk");
        assert!(!source.contains(&key_prefix), "源码不得出现密钥样式字符串");
    }

    /// 不变量：能力 id 全局唯一。
    #[test]
    fn capability_ids_are_unique() {
        let mut seen: Vec<&'static str> = Vec::new();
        for capability in CAPABILITIES {
            assert!(
                !seen.contains(&capability.id.as_str()),
                "重复能力 id：{}",
                capability.id.as_str()
            );
            seen.push(capability.id.as_str());
        }
    }

    /// 不变量：依赖必须指向已声明能力（目录自洽）。
    #[test]
    fn dependencies_reference_declared_capabilities() {
        for capability in CAPABILITIES {
            for dependency in capability.dependencies {
                assert!(
                    CAPABILITIES.iter().any(|other| other.id == *dependency),
                    "{} 依赖未声明能力 {}",
                    capability.id.as_str(),
                    dependency.as_str()
                );
            }
        }
    }

    /// 不变量（§8.3）：Stable 必须有用户入口；描述文案不得为空。
    #[test]
    fn stable_requires_entrypoints_and_user_copy() {
        for capability in CAPABILITIES {
            assert!(!capability.user_name.is_empty());
            assert!(!capability.summary.is_empty());
            if capability.maturity == CapabilityMaturity::Stable {
                assert!(
                    !capability.entrypoints.is_empty(),
                    "Stable 能力 {} 缺少用户入口",
                    capability.id.as_str()
                );
            }
        }
    }

    /// Agent V2 的四个新能力必须登记，且不能在没有真实入口时声称 Stable。
    #[test]
    fn agent_v2_capabilities_are_registered_with_entrypoints() {
        for id in [
            "tasks.runtime",
            "workspace.isolation",
            "automations.schedule",
            "bridge.external",
        ] {
            let capability = CAPABILITIES
                .iter()
                .find(|capability| capability.id == CapabilityId(id))
                .unwrap_or_else(|| panic!("缺少能力登记：{id}"));
            assert!(
                !capability.entrypoints.is_empty(),
                "{id} 必须给出真实用户入口"
            );
            assert_ne!(
                capability.maturity,
                CapabilityMaturity::Stable,
                "{id} 尚无同版本真实运行验收，不得标记 Stable"
            );
        }
        let workspace = CAPABILITIES
            .iter()
            .find(|capability| capability.id == CapabilityId("workspace.isolation"))
            .expect("工作区隔离必须登记");
        assert!(workspace.summary.contains("非 Git"));
        let bridge = CAPABILITIES
            .iter()
            .find(|capability| capability.id == CapabilityId("bridge.external"))
            .expect("桥接能力必须登记");
        assert!(bridge.summary.contains("主界面"));
    }

    #[test]
    fn chat_mode_lists_mode_route_and_prompt_entrypoint() {
        let chat = CAPABILITIES
            .iter()
            .find(|capability| capability.id == CapabilityId("chat.conversation"))
            .expect("纯对话能力必须登记");
        let targets: Vec<&str> = chat
            .entrypoints
            .iter()
            .map(|entrypoint| entrypoint.target)
            .collect();
        assert!(targets
            .iter()
            .any(|target| target.contains("POST /session/{id}/mode")));
        assert!(targets.iter().any(|target| target.contains("提示词")));
        assert!(chat.summary.contains("不开放本地文件"));
    }
}
