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

/// 功能目录注册表（§8.3 单一来源）。新增能力：在此追加条目 + 同步路由契约测试。
pub static CAPABILITIES: &[CapabilityDescriptor] = &[
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
];

/// GET /capabilities：功能目录（"我现在能做什么"页的数据面）。
pub async fn capabilities_list(State(_state): State<Arc<AppState>>) -> Json<Value> {
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
        "count": CAPABILITIES.len(),
        "maturity": { "stable": stable, "beta": beta, "experimental": experimental },
        "capabilities": items,
        "constraints": { "request_read_only": true, "scoped_turn_cancellation": true, "custom_model_connection": true, "custom_model_env_credentials": true, "durable_session_facts": true },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
