//! Read-only projection of the actual WorkerProfile assembler and live parent tools.
use super::Agent;
use crate::tools::ToolSpec;
use crate::worker_profile::WorkerProfile;
use crate::workswarm::RoleSpec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowToolCapability {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub effect: Option<crate::ToolEffect>,
    pub source: String,
    pub worker_supported: bool,
    pub profiles: Vec<String>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    /// registered describes assembly, never service verification or permission approval.
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowToolIssue {
    pub role: String,
    pub tool: String,
    pub code: String,
    pub message: String,
}

impl Agent {
    pub fn workflow_tool_capabilities(&self) -> Result<Vec<WorkflowToolCapability>, String> {
        let parent = self
            .registry
            .read()
            .map_err(|_| "tool registry unavailable")?
            .specs();
        let mut native = WorkerProfile::supported_native_tool_specs()
            .into_iter()
            .map(|spec| (spec.name.clone(), spec))
            .collect::<BTreeMap<_, _>>();
        let native_names = native
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        for spec in parent {
            native.entry(spec.name.clone()).or_insert(spec);
        }
        // Both media tools use one catalog revision for this preflight snapshot.
        let media_catalog = self
            .media_connections_root
            .as_ref()
            .map(|root| crate::gateway::media_profiles::MediaProfileStore::new(root).load());
        let default_profiles = [
            ("reader", "reader"),
            ("implementer", "builder"),
            ("researcher", "researcher"),
        ]
        .into_iter()
        .map(|(label, role)| {
            (
                label,
                WorkerProfile::for_role(role, 1)
                    .build_registry(Vec::new())
                    .specs()
                    .into_iter()
                    .map(|spec| spec.name)
                    .collect::<std::collections::BTreeSet<_>>(),
            )
        })
        .collect::<Vec<_>>();
        Ok(native
            .into_values()
            .map(|spec: ToolSpec| {
                let is_native = native_names.contains(&spec.name);
                let supported =
                    is_native || super::workflow_tools::is_parent_workflow_tool(&spec.name);
                let mut profiles = Vec::new();
                if supported {
                    profiles.push("declared".into());
                }
                if is_native {
                    for (label, names) in &default_profiles {
                        if names.contains(&spec.name) {
                            profiles.push((*label).into());
                        }
                    }
                }
                let (status, reason) = if !supported {
                    (
                        "unsupported",
                        Some("此主会话工具尚未接入工作流 Worker".into()),
                    )
                } else if self.tool_disabled(&spec.name) {
                    ("disabled", Some("宿主已禁用此工具".into()))
                } else if matches!(spec.name.as_str(), "generate_image" | "text_to_speech") {
                    let configured = match &media_catalog {
                        Some(Ok(catalog)) => catalog
                            .resolve(
                                crate::gateway::media_config::MediaService::for_tool(&spec.name)
                                    .expect("media service"),
                            )
                            .and_then(|connection| connection.require_configured()),
                        Some(Err(reason)) => Err(reason.clone()),
                        None => crate::tools::media_configuration_status(&spec.name),
                    };
                    match configured {
                        Ok(()) => ("registered", None),
                        Err(reason) => ("not_configured", Some(reason)),
                    }
                } else {
                    ("registered", None)
                };
                let (input, output) = match spec.name.as_str() {
                    "generate_image" => ("text", "image"),
                    "text_to_speech" => ("text", "audio"),
                    _ if is_native || matches!(spec.name.as_str(), "web_fetch" | "web_search") => {
                        ("json", "json")
                    }
                    _ => ("unknown", "unknown"),
                };
                WorkflowToolCapability {
                    name: spec.name,
                    description: spec.description,
                    input_schema: spec.input_schema,
                    effect: spec.effect,
                    source: if is_native {
                        "worker_native"
                    } else if supported {
                        "parent_proxy"
                    } else {
                        "main_session"
                    }
                    .into(),
                    worker_supported: supported,
                    profiles,
                    input_modalities: vec![input.into()],
                    output_modalities: vec![output.into()],
                    status: status.into(),
                    reason,
                }
            })
            .collect())
    }

    /// Validate resolved roles before persistence and again when starting an attempt.
    /// ToolHost remains responsible for argument, approval and task/path checks.
    pub fn preflight_workflow_roles(
        &self,
        roles: &[RoleSpec],
        parallel: bool,
        template_id: Option<&str>,
    ) -> Result<Vec<WorkflowToolIssue>, String> {
        let catalog = self
            .workflow_tool_capabilities()?
            .into_iter()
            .map(|tool| (tool.name.clone(), tool))
            .collect::<BTreeMap<_, _>>();
        let mut issues = Vec::new();
        for role in roles {
            if role.assignee == "human" {
                continue;
            }
            if let Some(worker) = role
                .worker
                .as_deref()
                .filter(|name| !matches!(*name, "" | "agent"))
            {
                if !matches!(worker, "echo" | "sleep" | "fail") {
                    issues.push(WorkflowToolIssue {
                        role: role.role.clone(),
                        tool: worker.into(),
                        code: "worker_not_registered".into(),
                        message: "执行器未注册；需要显式适配器".into(),
                    });
                }
                continue;
            }
            let profile = WorkerProfile::for_team_role_spec(role, 1, parallel, template_id);
            for tool in &role.tools {
                if !profile.visible_tools.contains(tool) {
                    issues.push(WorkflowToolIssue {
                        role: role.role.clone(),
                        tool: tool.clone(),
                        code: "profile_restricted".into(),
                        message: "角色画像不允许此工具".into(),
                    });
                }
            }
            issues.extend(profile_issues(&role.role, &profile, &catalog));
        }
        Ok(issues)
    }
    pub fn preflight_workflow_profile(
        &self,
        profile: &WorkerProfile,
    ) -> Result<Vec<WorkflowToolIssue>, String> {
        let catalog = self
            .workflow_tool_capabilities()?
            .into_iter()
            .map(|tool| (tool.name.clone(), tool))
            .collect::<BTreeMap<_, _>>();
        Ok(profile_issues("worker", profile, &catalog))
    }
}

fn profile_issues(
    role: &str,
    profile: &WorkerProfile,
    catalog: &BTreeMap<String, WorkflowToolCapability>,
) -> Vec<WorkflowToolIssue> {
    profile
        .visible_tools
        .iter()
        .filter(|name| !name.starts_with("__owo_"))
        .filter_map(|name| {
            let tool = catalog.get(name);
            if tool.is_some_and(|tool| tool.worker_supported && tool.status == "registered") {
                return None;
            }
            Some(WorkflowToolIssue {
                role: role.into(),
                tool: name.clone(),
                code: tool
                    .map(|tool| tool.status.clone())
                    .unwrap_or_else(|| "not_connected".into()),
                message: tool
                    .and_then(|tool| tool.reason.clone())
                    .unwrap_or_else(|| "工具未连接或未接入 Worker".into()),
            })
        })
        .collect()
}
