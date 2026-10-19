//! §4.5.3 结构化权限 profile → 策略规则展开（权限中心的唯一口径来源）。
//!
//! 为什么必须在服务端做：指南 §4.5.1 要页面显示"文件系统、命令和网络三个维度的
//! **实际范围**"，而服务端此前只有一个五值枚举 [`PermissionProfile`]。
//! 如果让前端从档位名去猜维度（"workspace 档 = 文件可写 + 命令询问"），
//! 那是在权限页面上编造范围——用户会照着它做授权决定，比不显示更危险。
//! 所以：
//! - [`PermissionSpec::from_profile`]：把档位**如实**投影成三维度（投影不出来的
//!   一律 `custom`，并允许上层补真实 scopes）；
//! - [`PermissionSpec::expand`]：展开成人可读、可断言的规则清单（页面/审计都用它）；
//! - [`PermissionSpec::extra_denial`]：**只收紧不放宽**的叠加判定——维度说 deny 时
//!   覆盖为 Deny；维度永远不能把 Deny 变成 Allow（放宽走既有权衡：档位切换 + grant，
//!   那两条路径都有审计与审批链约束）。
//!
//! 与 `permissions.rs` 的分工：策略与档位留在 `permissions.rs`（M1 契约不变），
//! 本模块只是它的结构化视图 + 收紧层，`Policy::set_spec` 是唯一接入点。
use serde::{Deserialize, Serialize};

use crate::permissions::{Decision, Level, PermissionProfile, PermissionRequest};

/// 文件系统维度（§4.5.3 字面量，改名即破坏前端映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemScope {
    /// 不开放文件系统（连只读也不行）。
    None,
    /// 工作区内只读。
    WorkspaceRead,
    /// 工作区内可写（越界路径仍由 `resolve_within` 拒绝）。
    WorkspaceWrite,
    /// 无文件范围限制（越界路径放行；高风险，仅 `unrestricted` 档位投影）。
    Unrestricted,
    /// 自定义：范围由 `scopes` 说，不假装等于上面三档。
    Custom,
}

/// 命令 / 网络维度三态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    /// 一律拒绝。
    Deny,
    /// 白名单/审批制。
    Allowlisted,
    /// 不设名单限制（只有完全访问才允许出现）。
    Unrestricted,
}

impl FilesystemScope {
    /// 线上字面量（与 serde 表一致；`wire_literals_match_guide` 测试锁死）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::WorkspaceRead => "workspace_read",
            Self::WorkspaceWrite => "workspace_write",
            Self::Unrestricted => "unrestricted",
            Self::Custom => "custom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "不开放",
            Self::WorkspaceRead => "工作区只读",
            Self::WorkspaceWrite => "工作区可写",
            Self::Unrestricted => "无范围限制",
            Self::Custom => "自定义范围",
        }
    }
}

impl RuleScope {
    /// 线上字面量（与 serde 表一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Allowlisted => "allowlisted",
            Self::Unrestricted => "unrestricted",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Deny => "拒绝",
            Self::Allowlisted => "白名单/审批",
            Self::Unrestricted => "不限",
        }
    }
}

/// 授权持久度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistenceScope {
    /// 仅本次。
    Once,
    /// 本任务（进程内会话级）。
    Task,
    /// 工作区长期（必须可撤销）。
    Workspace,
}

impl PersistenceScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Task => "task",
            Self::Workspace => "workspace",
        }
    }

    /// 人类可读标签（审批卡与权限中心共用；§4.5.2 四动作口径）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Once => "仅本次",
            Self::Task => "本任务",
            Self::Workspace => "工作区长期",
        }
    }

    /// 兼容读取既有审批卡 scope 字面量（`once/session/one_hour/always_readonly`）：
    /// 旧客户端仍在发这些值，不能因为换词表就把它们的授权意图丢掉。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "once" => Some(Self::Once),
            "task" | "session" => Some(Self::Task),
            "workspace" | "one_hour" | "always_readonly" => Some(Self::Workspace),
            _ => None,
        }
    }
}

/// 结构化 profile（§4.5.3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionSpec {
    pub filesystem: FilesystemScope,
    pub command: RuleScope,
    pub network: RuleScope,
    pub persistence: PersistenceScope,
    /// 作用域条目：`path:src/**` / `host:api.openai.com` / `command:git status`。
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// 展开后的单条规则（页面与审计共用的可断言单元）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedRule {
    /// 稳定规则 id（测试与撤销定位用）。
    pub id: String,
    pub dimension: &'static str,
    /// allow | ask | deny
    pub effect: String,
    pub subject: String,
    pub reason: String,
}

impl PermissionSpec {
    /// 由既有档位如实投影。`workspace_write=false` 时把可写降为只读
    /// （设置里关掉写开关是真实状态，不能继续显示"可写"）。
    pub fn from_profile(profile: PermissionProfile, workspace_write: bool) -> Self {
        let (filesystem, command, network, persistence) = match profile {
            PermissionProfile::ReadOnly => (
                FilesystemScope::WorkspaceRead,
                RuleScope::Deny,
                RuleScope::Deny,
                PersistenceScope::Once,
            ),
            PermissionProfile::Workspace | PermissionProfile::AutoReview => (
                if workspace_write {
                    FilesystemScope::WorkspaceWrite
                } else {
                    FilesystemScope::WorkspaceRead
                },
                RuleScope::Allowlisted,
                RuleScope::Allowlisted,
                PersistenceScope::Task,
            ),
            PermissionProfile::FullAccess => (
                FilesystemScope::WorkspaceWrite,
                RuleScope::Unrestricted,
                RuleScope::Unrestricted,
                PersistenceScope::Workspace,
            ),
            // Unrestricted：越界文件访问 + 命令/网络不设名单（高风险，显式 opt-in）。
            PermissionProfile::Unrestricted => (
                FilesystemScope::Unrestricted,
                RuleScope::Unrestricted,
                RuleScope::Unrestricted,
                PersistenceScope::Workspace,
            ),
            // Custom 档位本身就说明"范围在别处"，不能替它编一个看起来整齐的词表值。
            PermissionProfile::Custom => (
                FilesystemScope::Custom,
                RuleScope::Allowlisted,
                RuleScope::Allowlisted,
                PersistenceScope::Task,
            ),
        };
        Self {
            filesystem,
            command,
            network,
            persistence,
            scopes: Vec::new(),
        }
    }

    /// 反查最接近的既有档位（`Policy::set_spec` 用它同步档位；
    /// 只降不升：任何维度是 Deny 时绝不映射到 FullAccess）。
    pub fn nearest_profile(&self) -> PermissionProfile {
        if self.filesystem == FilesystemScope::Unrestricted {
            return PermissionProfile::Unrestricted;
        }
        if self.filesystem == FilesystemScope::Custom {
            return PermissionProfile::Custom;
        }
        let read_only_filesystem = self.filesystem == FilesystemScope::WorkspaceRead
            || self.filesystem == FilesystemScope::None;
        let everything_denied = read_only_filesystem
            && self.command == RuleScope::Deny
            && self.network == RuleScope::Deny;
        if everything_denied {
            return PermissionProfile::ReadOnly;
        }
        if self.command == RuleScope::Unrestricted && self.network == RuleScope::Unrestricted {
            return PermissionProfile::FullAccess;
        }
        PermissionProfile::Workspace
    }

    /// 展开为规则清单（§4.5.1"实际范围"就是它渲染出来的）。
    pub fn expand(&self) -> Vec<ExpandedRule> {
        let mut rules = Vec::new();
        rules.push(ExpandedRule {
            id: "filesystem".to_string(),
            dimension: "filesystem",
            effect: match self.filesystem {
                FilesystemScope::None => "deny",
                FilesystemScope::WorkspaceRead => "allow",
                FilesystemScope::WorkspaceWrite => "allow",
                FilesystemScope::Unrestricted => "allow",
                FilesystemScope::Custom => "ask",
            }
            .to_string(),
            subject: match self.filesystem {
                FilesystemScope::None => "全部路径".to_string(),
                FilesystemScope::WorkspaceRead | FilesystemScope::WorkspaceWrite => {
                    "工作区内路径".to_string()
                }
                FilesystemScope::Unrestricted => "全部路径（含工作区外）".to_string(),
                FilesystemScope::Custom => self
                    .scopes
                    .iter()
                    .filter(|scope| scope.starts_with("path:"))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("、"),
            },
            reason: match self.filesystem {
                FilesystemScope::None => "未开放文件系统".to_string(),
                FilesystemScope::WorkspaceRead => "只读；写操作一律拒绝".to_string(),
                FilesystemScope::WorkspaceWrite => "工作区内可写，越界路径拒绝".to_string(),
                FilesystemScope::Unrestricted => "无范围限制：越界路径放行（高风险）".to_string(),
                FilesystemScope::Custom => "自定义范围（未列出的路径按询问处理）".to_string(),
            },
        });
        for (dimension, scope, subjects) in [
            (
                "command",
                self.command,
                self.scopes
                    .iter()
                    .filter(|scope| scope.starts_with("command:"))
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            (
                "network",
                self.network,
                self.scopes
                    .iter()
                    .filter(|scope| scope.starts_with("host:"))
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
        ] {
            rules.push(ExpandedRule {
                id: dimension.to_string(),
                dimension,
                effect: match scope {
                    RuleScope::Deny => "deny",
                    RuleScope::Allowlisted => "ask",
                    RuleScope::Unrestricted => "allow",
                }
                .to_string(),
                subject: if subjects.is_empty() {
                    match scope {
                        RuleScope::Unrestricted => "不设限制".to_string(),
                        RuleScope::Deny => "全部".to_string(),
                        RuleScope::Allowlisted => "未声明白名单".to_string(),
                    }
                } else {
                    subjects.join("、")
                },
                reason: match scope {
                    RuleScope::Deny => format!("{dimension} 一律拒绝（含审批也不能放行）"),
                    RuleScope::Allowlisted => format!("{dimension} 按白名单/审批放行"),
                    RuleScope::Unrestricted => format!("{dimension} 无名单限制：完全访问特征"),
                },
            });
        }
        rules.push(ExpandedRule {
            id: "persistence".to_string(),
            dimension: "persistence",
            effect: "allow".to_string(),
            subject: self.persistence.as_str().to_string(),
            reason: match self.persistence {
                PersistenceScope::Once => "授权仅本次有效".to_string(),
                PersistenceScope::Task => "授权在本任务内有效，任务结束即失效".to_string(),
                PersistenceScope::Workspace => "工作区长期授权：可在权限中心逐条撤销".to_string(),
            },
        });
        rules
    }

    /// 只收紧的叠加判定：返回 `Some(Decision::Deny)` 表示维度直接拒绝。
    /// 永不返回 `Some(Allow)`——放宽只能靠切档位或 grant（两条路径都有审计）。
    pub fn extra_denial(&self, request: &PermissionRequest) -> Option<Decision> {
        match dimension_of(request) {
            Some("filesystem") => match self.filesystem {
                // 只读档拒绝写；None 档连读也拒绝（Read 级里只有文件读取会归到这一维）。
                FilesystemScope::None => Some(Decision::Deny),
                FilesystemScope::WorkspaceRead if request.level == Level::Write => {
                    Some(Decision::Deny)
                }
                _ => None,
            },
            Some("command") if self.command == RuleScope::Deny => Some(Decision::Deny),
            Some("network") if self.network == RuleScope::Deny => Some(Decision::Deny),
            _ => None,
        }
    }

    /// 完全访问风险拆解（§4.5.2：选择后必须列明范围与不可逆性）。
    pub fn risk_notes(&self) -> Vec<String> {
        let mut notes = vec![match self.filesystem {
            FilesystemScope::None => "文件：不开放".to_string(),
            FilesystemScope::WorkspaceRead => "文件：只读，写入一律拒".to_string(),
            FilesystemScope::WorkspaceWrite => {
                "文件：工作区内可写；覆盖/删除不可自动撤销（依赖变更集回滚）".to_string()
            }
            FilesystemScope::Unrestricted => {
                "文件：无范围限制，可读写工作区外任意路径（高风险，不可回滚）".to_string()
            }
            FilesystemScope::Custom => format!("文件：自定义 {} 条范围", self.scopes.len()),
        }];
        notes.push(match self.command {
            RuleScope::Deny => "命令：拒绝".to_string(),
            RuleScope::Allowlisted => "命令：白名单/审批制".to_string(),
            RuleScope::Unrestricted => "命令：不设限制，外部程序与系统副作用无法回滚".to_string(),
        });
        notes.push(match self.network {
            RuleScope::Deny => "网络：拒绝（数据不出境）".to_string(),
            RuleScope::Allowlisted => "网络：按端点与审批放行".to_string(),
            RuleScope::Unrestricted => "网络：不设限制，出境数据无法召回".to_string(),
        });
        notes.push(match self.persistence {
            PersistenceScope::Once => "有效期：仅本次".to_string(),
            PersistenceScope::Task => "有效期：本任务".to_string(),
            PersistenceScope::Workspace => "有效期：工作区长期（必须显式撤销才失效）".to_string(),
        });
        notes
    }
}

/// 把一次权限请求归到 §4.5.3 的维度；归不进去（UI 注入 / 编排 / 提问类）返回
/// `None`，由既有档位/审批链处理——本模块不接管第四类动作。
///
/// 覆盖范围必须与内置工具注册表同步：所有会碰文件、命令、网络的内置工具都要
/// 归到对应维度，否则 `spec{filesystem=workspace_read}` 这类"只读承诺"会被
/// `edit_file` / `apply_patch` / `web_fetch` 绕过（审查 N7）。
/// 编排类（`explore` / `subagent` / `fan_out_subagents`）与交互类（`todo` /
/// `ask_user` / `use_skill` / `verification_plan`）不在这里收敛：它们各自有
/// 子会话策略/审批链，且拆分子会话维度会重复计权。
pub fn dimension_of(request: &PermissionRequest) -> Option<&'static str> {
    let tool = request.tool.as_str();
    if tool.starts_with("browser_")
        || tool.starts_with("http_")
        || matches!(
            tool,
            "web_fetch" | "web_search" | "http_get" | "http_post" | "fetch_url" | "download_file"
        )
    {
        return Some("network");
    }
    if matches!(tool, "run_command" | "shell_output" | "kill_shell")
        || tool.starts_with("shell.")
        || tool.starts_with("command:")
    {
        return Some("command");
    }
    if matches!(
        tool,
        "read_file"
            | "write_file"
            | "edit_file"
            | "multi_edit"
            | "apply_patch"
            | "list_dir"
            | "search_files"
            | "grep"
            | "find_symbol"
            | "read_artifact"
            | "read_image"
            | "delete_file"
            | "move_file"
    ) || tool.starts_with("git_")
    {
        return Some("filesystem");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(tool: &str, level: Level) -> PermissionRequest {
        PermissionRequest::new(tool, json!({ "path": "a.txt" }), level, "测试")
    }

    fn spec(filesystem: FilesystemScope, command: RuleScope, network: RuleScope) -> PermissionSpec {
        PermissionSpec {
            filesystem,
            command,
            network,
            persistence: PersistenceScope::Task,
            scopes: Vec::new(),
        }
    }

    #[test]
    fn profile_projection_is_faithful() {
        let read_only = PermissionSpec::from_profile(PermissionProfile::ReadOnly, true);
        assert_eq!(read_only.command, RuleScope::Deny);
        assert_eq!(read_only.filesystem, FilesystemScope::WorkspaceRead);

        // 写开关被关掉时，工作区档不得继续显示"可写"。
        let no_write = PermissionSpec::from_profile(PermissionProfile::Workspace, false);
        assert_eq!(no_write.filesystem, FilesystemScope::WorkspaceRead);
        let with_write = PermissionSpec::from_profile(PermissionProfile::Workspace, true);
        assert_eq!(with_write.filesystem, FilesystemScope::WorkspaceWrite);

        // custom 档位不替它编词表。
        let custom = PermissionSpec::from_profile(PermissionProfile::Custom, true);
        assert_eq!(custom.filesystem, FilesystemScope::Custom);
        assert_eq!(custom.nearest_profile(), PermissionProfile::Custom);
    }

    #[test]
    fn expansion_covers_all_four_dimensions() {
        let rules = spec(
            FilesystemScope::WorkspaceWrite,
            RuleScope::Deny,
            RuleScope::Allowlisted,
        )
        .expand();
        let ids: Vec<&str> = rules.iter().map(|rule| rule.id.as_str()).collect();
        assert_eq!(ids, vec!["filesystem", "command", "network", "persistence"]);
        let command = rules.iter().find(|rule| rule.id == "command").unwrap();
        assert_eq!(command.effect, "deny");
        assert!(command.reason.contains("一律拒绝"));
        let fs = rules.iter().find(|rule| rule.id == "filesystem").unwrap();
        assert!(
            fs.subject.contains("工作区"),
            "路径只说必要范围：{}",
            fs.subject
        );
        assert!(
            !fs.subject.contains(':') || fs.subject == "工作区内路径",
            "不得回显绝对路径：{}",
            fs.subject
        );
    }

    #[test]
    fn extra_denial_only_tightens_never_loosens() {
        let strict = spec(FilesystemScope::None, RuleScope::Deny, RuleScope::Deny);
        assert_eq!(
            strict.extra_denial(&request("read_file", Level::Read)),
            Some(Decision::Deny)
        );
        assert_eq!(
            strict.extra_denial(&request("run_command", Level::Execute)),
            Some(Decision::Deny)
        );
        assert_eq!(
            strict.extra_denial(&request("browser_navigate", Level::Execute)),
            Some(Decision::Deny)
        );
        // 宽松档位也永远拿不到 Some(Allow)：放宽只能走档位切换/grant。
        let loose = spec(
            FilesystemScope::WorkspaceWrite,
            RuleScope::Unrestricted,
            RuleScope::Unrestricted,
        );
        for tool in [
            "write_file",
            "run_command",
            "browser_navigate",
            "desktop_click",
        ] {
            assert_eq!(
                loose.extra_denial(&request(tool, Level::Write)),
                None,
                "{tool} 不该被本模块判成 Allow"
            );
        }
        // 只读文件维度只拒写，不误伤读。
        let readonly_fs = spec(
            FilesystemScope::WorkspaceRead,
            RuleScope::Allowlisted,
            RuleScope::Allowlisted,
        );
        assert_eq!(
            readonly_fs.extra_denial(&request("read_file", Level::Read)),
            None
        );
        assert_eq!(
            readonly_fs.extra_denial(&request("write_file", Level::Write)),
            Some(Decision::Deny)
        );
    }

    #[test]
    fn extra_denial_covers_edit_patch_network_and_command_surface() {
        // 审查 N7：edit_file / multi_edit / apply_patch 必须归入 filesystem，
        // web_fetch 必须归入 network；否则"只读/数据不出境"承诺会被绕过。
        let readonly_fs = spec(
            FilesystemScope::WorkspaceRead,
            RuleScope::Unrestricted,
            RuleScope::Unrestricted,
        );
        for tool in ["edit_file", "multi_edit", "apply_patch"] {
            assert_eq!(
                readonly_fs.extra_denial(&request(tool, Level::Write)),
                Some(Decision::Deny),
                "{tool} 属于 filesystem 写，只读档必须拒绝"
            );
        }
        let denied_network = spec(
            FilesystemScope::WorkspaceWrite,
            RuleScope::Unrestricted,
            RuleScope::Deny,
        );
        assert_eq!(
            denied_network.extra_denial(&request("web_fetch", Level::Execute)),
            Some(Decision::Deny),
            "web_fetch 属于 network，网络 Deny 必须拒绝"
        );
        let denied_command = spec(
            FilesystemScope::WorkspaceWrite,
            RuleScope::Deny,
            RuleScope::Unrestricted,
        );
        for tool in ["kill_shell", "shell_output"] {
            assert_eq!(
                denied_command.extra_denial(&request(tool, Level::Read)),
                Some(Decision::Deny),
                "{tool} 属于 command，命令 Deny 必须拒绝"
            );
        }
        assert_eq!(
            dimension_of(&request("grep", Level::Read)),
            Some("filesystem")
        );
        assert_eq!(
            dimension_of(&request("git_status", Level::Read)),
            Some("filesystem")
        );
        assert_eq!(
            dimension_of(&request("read_image", Level::Read)),
            Some("filesystem")
        );
    }

    #[test]
    fn nearest_profile_never_escalates_to_full_access_by_accident() {
        let mixed = spec(
            FilesystemScope::WorkspaceWrite,
            RuleScope::Deny,
            RuleScope::Deny,
        );
        assert_eq!(mixed.nearest_profile(), PermissionProfile::Workspace);
        let wide = spec(
            FilesystemScope::WorkspaceWrite,
            RuleScope::Unrestricted,
            RuleScope::Unrestricted,
        );
        assert_eq!(wide.nearest_profile(), PermissionProfile::FullAccess);
        // 任一维度还在拒绝，就不许升成完全访问。
        let half_wide = spec(
            FilesystemScope::WorkspaceWrite,
            RuleScope::Unrestricted,
            RuleScope::Allowlisted,
        );
        assert_eq!(half_wide.nearest_profile(), PermissionProfile::Workspace);
    }

    #[test]
    fn wire_literals_match_guide() {
        assert_eq!(
            serde_json::to_string(&FilesystemScope::WorkspaceRead).unwrap(),
            "\"workspace_read\""
        );
        assert_eq!(
            serde_json::to_string(&FilesystemScope::WorkspaceWrite).unwrap(),
            "\"workspace_write\""
        );
        assert_eq!(
            serde_json::to_string(&RuleScope::Allowlisted).unwrap(),
            "\"allowlisted\""
        );
        assert_eq!(
            serde_json::to_string(&PersistenceScope::Workspace).unwrap(),
            "\"workspace\""
        );
        assert_eq!(
            PersistenceScope::parse("task"),
            Some(PersistenceScope::Task)
        );
        assert_eq!(
            PersistenceScope::parse("always_readonly"),
            Some(PersistenceScope::Workspace),
            "旧审批卡 scope 必须还能读进来"
        );
        assert_eq!(PersistenceScope::parse("forever"), None);
    }

    #[test]
    fn full_access_risk_notes_are_specific() {
        let wide = PermissionSpec {
            filesystem: FilesystemScope::Custom,
            command: RuleScope::Unrestricted,
            network: RuleScope::Unrestricted,
            persistence: PersistenceScope::Workspace,
            scopes: vec![
                "path:src/**".to_string(),
                "host:api.example.com".to_string(),
            ],
        };
        let notes = wide.risk_notes();
        assert_eq!(notes.len(), 4, "目录/命令/网络/时长四项齐备");
        assert!(notes[0].contains("自定义 2 条范围"), "{}", notes[0]);
        assert!(notes[1].contains("无法回滚"));
        assert!(notes[2].contains("无法召回"));
        assert!(notes[3].contains("撤销"));
    }
}
