//! 工作流版本对比（社区包更新/本地定制 vs 上游）：纯函数，无 IO。
//!
//! 社区共享场景：别人发来 v1.1，我要在导入前看清"角色/工具/写范围/依赖"改了什么，
//! 尤其是**权限面变大**的部分（新增执行/联网工具、新增 MCP 依赖、写范围从受限变成
//! 工作区任意写、角色数增加）。这里不做语义好坏判断，只做结构化差异 + 风险提示。

use owo_agent_protocol::TeamTemplate;

/// 单字段变化。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FieldChange {
    pub field: String,
    pub before: String,
    pub after: String,
}

/// 角色级变化。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RoleChange {
    pub role: String,
    pub changes: Vec<FieldChange>,
    /// **新版**该角色的风险标记（与健康摘要同一口径；无风险为空）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<String>,
}

/// 两个模板的结构化差异。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct WorkflowDiff {
    /// 模板级字段变化（name/version/description/mode/applicability/tags）。
    pub metadata: Vec<FieldChange>,
    pub roles_added: Vec<String>,
    pub roles_removed: Vec<String>,
    pub roles_changed: Vec<RoleChange>,
    /// 权限面变大的风险提示（新增执行/联网工具、MCP、写范围放宽、角色增加）。
    pub risks: Vec<String>,
    /// **新增角色**的风险行（带工具/写范围/风险标记）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_role_risks: Vec<crate::user_workflow::RoleHealth>,
}

impl WorkflowDiff {
    /// 是否没有任何差异。
    pub fn is_empty(&self) -> bool {
        self.metadata.is_empty()
            && self.roles_added.is_empty()
            && self.roles_removed.is_empty()
            && self.roles_changed.is_empty()
            && self.risks.is_empty()
            && self.added_role_risks.is_empty()
    }
}

/// 执行/联网类工具（新增即提示）。
const ELEVATED_TOOLS: &[&str] = &[
    "run_command",
    "web_fetch",
    "web_search",
    "browser_click",
    "browser_type",
    "computer_use",
];

fn field(field: &str, before: impl Into<String>, after: impl Into<String>) -> FieldChange {
    FieldChange {
        field: field.to_string(),
        before: before.into(),
        after: after.into(),
    }
}

fn join_list(items: &[String]) -> String {
    items.join(", ")
}

fn added<'a>(before: &'a [String], after: &'a [String]) -> Vec<String> {
    after
        .iter()
        .filter(|item| !before.iter().any(|existing| existing == *item))
        .cloned()
        .collect()
}

fn removed<'a>(before: &'a [String], after: &'a [String]) -> Vec<String> {
    before
        .iter()
        .filter(|item| !after.iter().any(|existing| existing == *item))
        .cloned()
        .collect()
}

/// 对比两个工作流模板。
pub fn diff_templates(before: &TeamTemplate, after: &TeamTemplate) -> WorkflowDiff {
    let mut diff = WorkflowDiff::default();

    let mut metadata = Vec::new();
    if before.name != after.name {
        metadata.push(field("name", &before.name, &after.name));
    }
    if before.mode != after.mode {
        metadata.push(field(
            "mode",
            format!("{:?}", before.mode),
            format!("{:?}", after.mode),
        ));
    }
    if before.applicability != after.applicability {
        metadata.push(field(
            "applicability",
            &before.applicability,
            &after.applicability,
        ));
    }
    if before.description != after.description {
        metadata.push(field(
            "description",
            before.description.clone().unwrap_or_default(),
            after.description.clone().unwrap_or_default(),
        ));
    }
    if before.version != after.version {
        metadata.push(field(
            "version",
            before.version.clone().unwrap_or_default(),
            after.version.clone().unwrap_or_default(),
        ));
    }
    if before.tags != after.tags {
        metadata.push(field(
            "tags",
            join_list(&before.tags),
            join_list(&after.tags),
        ));
    }
    diff.metadata = metadata;

    let before_roles: Vec<String> = before.roles.iter().map(|role| role.role.clone()).collect();
    let after_roles: Vec<String> = after.roles.iter().map(|role| role.role.clone()).collect();
    diff.roles_added = added(&before_roles, &after_roles);
    diff.roles_removed = removed(&before_roles, &after_roles);
    if diff.roles_added.len() > diff.roles_removed.len() {
        diff.risks.push(format!(
            "角色数增加（{} → {}）：新增 {}",
            before.roles.len(),
            after.roles.len(),
            diff.roles_added.join(", ")
        ));
    }

    for role in &after.roles {
        let Some(previous) = before.roles.iter().find(|item| item.role == role.role) else {
            // 新增角色：把它的权限面也点出来（执行/联网工具、MCP 依赖）。
            let elevated: Vec<String> = role
                .tools
                .iter()
                .filter(|tool| ELEVATED_TOOLS.contains(&tool.as_str()))
                .cloned()
                .collect();
            if !elevated.is_empty() {
                diff.risks.push(format!(
                    "新增角色 {} 带执行/联网工具：{}",
                    role.role,
                    elevated.join(", ")
                ));
            }
            let mcp: Vec<String> = role
                .tools
                .iter()
                .filter(|tool| tool.starts_with("mcp__"))
                .cloned()
                .collect();
            if !mcp.is_empty() {
                diff.risks.push(format!(
                    "新增角色 {} 带 MCP 依赖：{}",
                    role.role,
                    mcp.join(", ")
                ));
            }
            let health = crate::user_workflow::role_health(role);
            if !health.risks.is_empty() {
                diff.added_role_risks.push(health);
            }
            continue;
        };
        let mut changes = Vec::new();
        if previous.assignee != role.assignee {
            changes.push(field("assignee", &previous.assignee, &role.assignee));
        }
        if previous.worker != role.worker {
            changes.push(field(
                "worker",
                previous.worker.clone().unwrap_or_default(),
                role.worker.clone().unwrap_or_default(),
            ));
        }
        if previous.depends_on != role.depends_on {
            changes.push(field(
                "depends_on",
                join_list(&previous.depends_on),
                join_list(&role.depends_on),
            ));
        }
        if previous.verify != role.verify {
            changes.push(field(
                "verify",
                previous.verify.clone().unwrap_or_default(),
                role.verify.clone().unwrap_or_default(),
            ));
        }
        if previous.handoff_contract != role.handoff_contract {
            changes.push(field(
                "handoff_contract",
                previous.handoff_contract.clone().unwrap_or_default(),
                role.handoff_contract.clone().unwrap_or_default(),
            ));
        }
        if previous.tools != role.tools {
            let tools_added = added(&previous.tools, &role.tools);
            let tools_removed = removed(&previous.tools, &role.tools);
            changes.push(field(
                "tools",
                join_list(&previous.tools),
                join_list(&role.tools),
            ));
            let elevated_added: Vec<String> = tools_added
                .iter()
                .filter(|tool| ELEVATED_TOOLS.contains(&tool.as_str()))
                .cloned()
                .collect();
            if !elevated_added.is_empty() {
                diff.risks.push(format!(
                    "角色 {} 新增执行/联网工具：{}",
                    role.role,
                    elevated_added.join(", ")
                ));
            }
            let mcp_added: Vec<String> = tools_added
                .iter()
                .filter(|tool| tool.starts_with("mcp__"))
                .cloned()
                .collect();
            if !mcp_added.is_empty() {
                diff.risks.push(format!(
                    "角色 {} 新增 MCP 依赖：{}",
                    role.role,
                    mcp_added.join(", ")
                ));
            }
            let _ = tools_removed;
        }
        if previous.write_paths != role.write_paths {
            changes.push(field(
                "write_paths",
                join_list(&previous.write_paths),
                join_list(&role.write_paths),
            ));
            // 空 = 工作区内任意写：从"有范围"变成"无范围"是明显放大。
            if !previous.write_paths.is_empty() && role.write_paths.is_empty() {
                diff.risks
                    .push(format!("角色 {} 的写范围从受限变为工作区任意写", role.role));
            } else {
                let widened = added(&previous.write_paths, &role.write_paths);
                if !widened.is_empty() {
                    diff.risks.push(format!(
                        "角色 {} 新增写路径：{}",
                        role.role,
                        widened.join(", ")
                    ));
                }
            }
        }
        if !changes.is_empty() {
            diff.roles_changed.push(RoleChange {
                role: role.role.clone(),
                changes,
                // 只带"新版仍然有风险"的标记：变化引起的能力面前移一目了然。
                risks: crate::user_workflow::role_health(role).risks,
            });
        }
    }

    diff
}

/// 人类可读摘要行（CLI/面板共用）。无差异时返回单行说明。
pub fn summarize_diff(diff: &WorkflowDiff) -> Vec<String> {
    if diff.is_empty() {
        return vec!["两个工作流没有差异".to_string()];
    }
    let mut lines = Vec::new();
    for change in &diff.metadata {
        lines.push(format!(
            "元数据 {}：{} → {}",
            change.field, change.before, change.after
        ));
    }
    if !diff.roles_added.is_empty() {
        lines.push(format!("新增角色：{}", diff.roles_added.join(", ")));
    }
    if !diff.roles_removed.is_empty() {
        lines.push(format!("移除角色：{}", diff.roles_removed.join(", ")));
    }
    for role in &diff.roles_changed {
        for change in &role.changes {
            lines.push(format!(
                "角色 {} 的 {}：{} → {}",
                role.role, change.field, change.before, change.after
            ));
        }
    }
    for row in &diff.added_role_risks {
        lines.push(format!(
            "新增角色 {} 的风险：{}",
            row.role,
            row.risks.join("；")
        ));
    }
    for role in &diff.roles_changed {
        if !role.risks.is_empty() {
            lines.push(format!(
                "角色 {} 当前风险：{}",
                role.role,
                role.risks.join("；")
            ));
        }
    }
    for risk in &diff.risks {
        lines.push(format!("[风险] {risk}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use owo_agent_protocol::{TeamMode, TeamTemplateRole};

    fn role(name: &str, tools: &[&str], write_paths: &[&str]) -> TeamTemplateRole {
        TeamTemplateRole {
            role: name.to_string(),
            assignee: "agent".to_string(),
            tools: tools.iter().map(|item| item.to_string()).collect(),
            write_paths: write_paths.iter().map(|item| item.to_string()).collect(),
            ..Default::default()
        }
    }

    fn template(version: &str, roles: Vec<TeamTemplateRole>) -> TeamTemplate {
        TeamTemplate {
            template_id: "video-recreate".to_string(),
            name: "视频复刻".to_string(),
            mode: TeamMode::Team,
            roles,
            applicability: "视频".to_string(),
            version: Some(version.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn identical_templates_have_no_diff() {
        let a = template("1.0.0", vec![role("a", &["read_file"], &["src/"])]);
        let diff = diff_templates(&a, &a.clone());
        assert!(diff.is_empty());
        assert_eq!(
            summarize_diff(&diff),
            vec!["两个工作流没有差异".to_string()]
        );
    }

    #[test]
    fn detects_metadata_role_and_tool_changes_with_risks() {
        let before = template(
            "1.0.0",
            vec![
                role("a", &["read_file"], &["src/"]),
                role("b", &["read_file"], &["out/"]),
            ],
        );
        let mut after = template(
            "1.1.0",
            vec![
                role("a", &["read_file", "run_command"], &["src/"]),
                role("c", &["read_file"], &[]),
            ],
        );
        after.name = "视频复刻 v2".to_string();
        let diff = diff_templates(&before, &after);

        assert!(diff.metadata.iter().any(|change| change.field == "version"));
        assert!(diff.metadata.iter().any(|change| change.field == "name"));
        assert_eq!(diff.roles_added, vec!["c".to_string()]);
        assert_eq!(diff.roles_removed, vec!["b".to_string()]);
        let role_a = diff
            .roles_changed
            .iter()
            .find(|change| change.role == "a")
            .expect("角色 a 应有变化");
        assert!(role_a.changes.iter().any(|change| change.field == "tools"));
        let risks = diff.risks.join("\n");
        assert!(risks.contains("新增执行/联网工具"), "{risks}");
        assert!(risks.contains("run_command"), "{risks}");

        let lines = summarize_diff(&diff);
        assert!(lines.iter().any(|line| line.contains("元数据 version")));
        assert!(lines.iter().any(|line| line.contains("新增角色：c")));
        // 角色级风险：impl 新版带 run_command；新增角色 c 未声明写范围（有写能力）。
        let impl_change = diff
            .roles_changed
            .iter()
            .find(|change| change.role == "a")
            .unwrap();
        assert!(
            impl_change
                .risks
                .iter()
                .any(|risk| risk == "执行/联网工具:run_command"),
            "{:?}",
            impl_change.risks
        );
        assert!(lines
            .iter()
            .any(|line| line.contains("角色 a 当前风险：执行/联网工具:run_command")));
    }

    #[test]
    fn added_risky_role_is_reported_with_its_own_health_row() {
        let before = template("1.0.0", vec![role("a", &["read_file"], &["src/"])]);
        let after = template(
            "1.1.0",
            vec![
                role("a", &["read_file"], &["src/"]),
                role("writer", &["write_file"], &[]),
            ],
        );
        let diff = diff_templates(&before, &after);
        assert_eq!(
            diff.added_role_risks.len(),
            1,
            "{:?}",
            diff.added_role_risks
        );
        let row = &diff.added_role_risks[0];
        assert_eq!(row.role, "writer");
        assert!(
            row.risks.iter().any(|risk| risk.contains("工作区任意写")),
            "{:?}",
            row.risks
        );
        let lines = summarize_diff(&diff);
        assert!(lines
            .iter()
            .any(|line| line.contains("新增角色 writer 的风险：")));
        assert!(!diff.is_empty());
    }

    #[test]
    fn widening_write_scope_to_workspace_is_flagged() {
        let before = template("1.0.0", vec![role("impl", &["write_file"], &["src/"])]);
        let after = template("1.0.1", vec![role("impl", &["write_file"], &[])]);
        let diff = diff_templates(&before, &after);
        assert!(
            diff.risks.iter().any(|risk| risk.contains("工作区任意写")),
            "{:?}",
            diff.risks
        );

        let before = template("1.0.0", vec![role("impl", &["write_file"], &["src/"])]);
        let after = template(
            "1.0.1",
            vec![role("impl", &["write_file"], &["src/", "docs/"])],
        );
        let diff = diff_templates(&before, &after);
        assert!(
            diff.risks
                .iter()
                .any(|risk| risk.contains("新增写路径：docs/")),
            "{:?}",
            diff.risks
        );
    }

    #[test]
    fn newly_added_mcp_role_is_reported() {
        let before = template("1.0.0", vec![role("a", &["read_file"], &["src/"])]);
        let after = template(
            "1.1.0",
            vec![
                role("a", &["read_file"], &["src/"]),
                role("translate", &["mcp__owo_plugin_translate__translate"], &[]),
            ],
        );
        let diff = diff_templates(&before, &after);
        assert_eq!(diff.roles_added, vec!["translate".to_string()]);
        assert!(diff.risks.join("\n").contains("MCP"), "{:?}", diff.risks);
    }

    #[test]
    fn role_depends_and_assignee_changes_are_reported() {
        let mut before = template("1.0.0", vec![role("a", &[], &[]), role("b", &[], &[])]);
        let mut after = template("1.0.0", vec![role("a", &[], &[]), role("b", &[], &[])]);
        after.roles[1].depends_on = vec!["a".to_string()];
        after.roles[1].assignee = "human".to_string();
        before.roles[0].verify = Some("non_empty".to_string());
        after.roles[0].verify = Some("contains:完成".to_string());
        let diff = diff_templates(&before, &after);
        let role_b = diff
            .roles_changed
            .iter()
            .find(|change| change.role == "b")
            .expect("角色 b 应有变化");
        assert!(role_b
            .changes
            .iter()
            .any(|change| change.field == "depends_on"));
        assert!(role_b
            .changes
            .iter()
            .any(|change| change.field == "assignee"));
        let role_a = diff
            .roles_changed
            .iter()
            .find(|change| change.role == "a")
            .expect("角色 a 应有变化");
        assert!(role_a.changes.iter().any(|change| change.field == "verify"));
    }
}
