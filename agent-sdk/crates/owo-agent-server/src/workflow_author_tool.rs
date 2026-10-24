//! Agent 可调用的团队工作流编排工具。
//!
//! 对话主导的关键不是再造一套表单，而是让 Agent 在 Work 模式里把用户的一句话
//! 直接落成可校验、可编辑、可拖拽的团队工作流。工具只写 `<data_root>/workswarm/templates`，
//! 不执行团队、不碰工作区文件；保存前后都走与 `/workflows/user` 相同的校验/评审。
//!
//! 权限边界：工具声明 `Write` effect（保存的是配置，不是工作区文件），仍由统一
//! ToolHost/Policy 决定是否需要审批，不在工具内部绕过。

use async_trait::async_trait;
use owo_agent_core::tool_effects::{EffectClass, ToolEffect};
use owo_agent_core::tools::{Tool, ToolContext, ToolSpec};
use owo_agent_core::user_workflow::{
    review_workflow_template, validate_workflow_template, workflow_review_blocked,
};
use owo_agent_core::workswarm::TeamTemplateRegistry;
use owo_agent_protocol::TeamTemplate;
use serde_json::{json, Value};
use std::path::PathBuf;

const TOOL_NAME: &str = "workflow_author";

/// 一句话 → 团队工作流（角色 / 依赖 / 工具与 MCP / 交付契约）。
pub struct WorkflowAuthorTool {
    workswarm_root: PathBuf,
    workspace: PathBuf,
}

impl WorkflowAuthorTool {
    pub fn new(workswarm_root: PathBuf, workspace: PathBuf) -> Self {
        Self {
            workswarm_root,
            workspace,
        }
    }

    fn parse_template(&self, args: &Value) -> Result<TeamTemplate, String> {
        let raw = args
            .get("template")
            .cloned()
            .ok_or_else(|| "缺少 template（完整 TeamTemplate JSON）".to_string())?;
        let mut template: TeamTemplate = if let Some(text) = raw.as_str() {
            serde_json::from_str(text)
                .map_err(|error| format!("template JSON 解析失败：{error}"))?
        } else {
            serde_json::from_value(raw)
                .map_err(|error| format!("template JSON 解析失败：{error}"))?
        };
        // 宿主补齐来源/时间/版本，避免模型把内部字段写错或漏写。
        template.origin = Some("user".to_string());
        if template.created_at.trim().is_empty() {
            template.created_at = chrono::Utc::now().to_rfc3339();
        }
        if template
            .version
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            template.version = Some("1.0.0".to_string());
        }
        if template
            .description
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            let description = args
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            template.description = description.map(str::to_string);
        }
        Ok(template)
    }
}

#[async_trait]
impl Tool for WorkflowAuthorTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::with_effect(
            TOOL_NAME,
            "把一个团队工作流保存为可编辑模板。用户用一句话描述目标（例如视频复刻、\n\
             代码评审、资料研究）时，由你拆成角色、依赖、工具/MCP 与交付契约，然后调用本工具。\n\
             \n\
             template 必须包含 template_id（ASCII，字母开头）、name、applicability、roles；\n\
             roles 每项含 role、assignee(agent/human/worker)、depends_on、tools、write_paths、\n\
             verify。工具名可直接引用当前已连接 MCP 的 `mcp__server__tool`。保存前宿主会\n\
             重新校验并返回问题；保存成功后可让用户在「团队协作 / 自定义工作流」里拖拽画布。"
                .to_string(),
            json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "用户原话或一句话目标（用于补全模板描述与审计）"
                    },
                    "template": {
                        "type": "object",
                        "description": "完整 TeamTemplate JSON；角色/依赖/工具/写范围/验证断言都要显式给出",
                        "properties": {
                            "template_id": {
                                "type": "string",
                                "description": "ASCII 标识，字母开头，只含字母数字与 -_."
                            },
                            "name": { "type": "string" },
                            "mode": {
                                "type": "string",
                                "enum": ["single", "team", "swarmflow"]
                            },
                            "applicability": {
                                "type": "string",
                                "description": "用于自动匹配的适用条件关键词"
                            },
                            "description": { "type": "string" },
                            "roles": {
                                "type": "array",
                                "minItems": 1,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "role": { "type": "string" },
                                        "assignee": {
                                            "type": "string",
                                            "enum": ["agent", "human", "worker"]
                                        },
                                        "worker": { "type": "string" },
                                        "model": { "type": "string" },
                                        "depends_on": {
                                            "type": "array",
                                            "items": { "type": "string" }
                                        },
                                        "tools": {
                                            "type": "array",
                                            "items": { "type": "string" }
                                        },
                                        "write_paths": {
                                            "type": "array",
                                            "items": { "type": "string" }
                                        },
                                        "capabilities": {
                                            "type": "array",
                                            "items": { "type": "string" }
                                        },
                                        "verify": {
                                            "type": "string",
                                            "description": "例如 non_empty / contains:<text> / equals:<text>"
                                        },
                                        "handoff_contract": { "type": "string" }
                                    },
                                    "required": ["role", "assignee"]
                                }
                            }
                        },
                        "required": ["template_id", "name", "applicability", "roles"]
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "同 id 已存在时是否覆盖；缺省 false，避免静默改掉用户模板"
                    }
                },
                "required": ["template"]
            }),
            Some(ToolEffect {
                tool: TOOL_NAME.to_string(),
                class: EffectClass::Write,
                source: "builtin".to_string(),
                risk_note: Some(
                    "保存团队工作流配置到本机 data_root（不执行团队、不写工作区文件）".to_string(),
                ),
                annotations: None,
                host_verified_readonly: false,
            }),
        )
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let template = self.parse_template(&args)?;
        let errors = validate_workflow_template(&template);
        if !errors.is_empty() {
            return Err(format!(
                "工作流校验未通过（修正后重试）：\n- {}",
                errors.join("\n- ")
            ));
        }
        let findings = review_workflow_template(&template);
        if workflow_review_blocked(&findings) {
            let blocked: Vec<String> = findings
                .iter()
                .filter(|finding| finding.level == "block")
                .map(|finding| format!("{}：{}", finding.category, finding.detail))
                .collect();
            return Err(format!(
                "工作流被评审门禁阻止（修正后重试）：\n- {}",
                blocked.join("\n- ")
            ));
        }

        let registry = TeamTemplateRegistry::for_workswarm_root(&self.workswarm_root);
        let overwrite = args
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let existing = registry.get_template(&template.template_id);
        if existing.as_ref().is_some_and(|saved| {
            !matches!(saved.origin.as_deref(), Some("user") | Some("community"))
        }) {
            return Err("内置工作流不可覆盖；请使用新的 template_id 保存副本".to_string());
        }
        let exists = existing.is_some();
        if exists && !overwrite {
            return Err(format!(
                "工作流 {} 已存在；如确认覆盖，请传 overwrite=true",
                template.template_id
            ));
        }
        registry
            .save_template(&template)
            .map_err(|error| format!("保存工作流失败：{error}"))?;

        let warnings: Vec<String> = findings
            .iter()
            .filter(|finding| finding.level != "info")
            .map(|finding| format!("{}：{}", finding.category, finding.detail))
            .collect();
        if let Ok(mut audit) = ctx.audit.lock() {
            audit.record(
                "workflows",
                if exists {
                    "workflow_author_overwrite"
                } else {
                    "workflow_author_create"
                },
                Some(TOOL_NAME.to_string()),
                Some(true),
                format!(
                    "template_id={} roles={} workspace={} warnings={}",
                    template.template_id,
                    template.roles.len(),
                    self.workspace.display(),
                    warnings.len()
                ),
            );
        }

        Ok(json!({
            "ok": true,
            "template_id": template.template_id,
            "name": template.name,
            "roles": template.roles.len(),
            "overwritten": exists,
            "findings": warnings,
            "message": format!(
                "已保存团队工作流「{}」。打开「团队协作 → 自定义 / AI 编排」可拖动节点、采纳或建队。",
                template.name
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> WorkflowAuthorTool {
        WorkflowAuthorTool::new(
            std::env::temp_dir().join("owo-workflow-author-test"),
            std::env::temp_dir(),
        )
    }

    fn valid_template() -> Value {
        json!({
            "description": "把研究结论写成可执行方案",
            "template": {
                "template_id": "research-plan",
                "name": "研究到方案",
                "mode": "team",
                "applicability": "研究 方案",
                "roles": [{
                    "role": "researcher",
                    "assignee": "agent",
                    "depends_on": [],
                    "tools": ["read_file"],
                    "write_paths": [],
                    "verify": "non_empty"
                }]
            }
        })
    }

    #[test]
    fn parse_template_fills_host_owned_fields_and_validates() {
        let parsed = tool()
            .parse_template(&valid_template())
            .expect("模板应可解析");
        assert_eq!(parsed.origin.as_deref(), Some("user"));
        assert_eq!(parsed.version.as_deref(), Some("1.0.0"));
        assert!(!parsed.created_at.trim().is_empty());
        assert_eq!(
            parsed.description.as_deref(),
            Some("把研究结论写成可执行方案")
        );
        assert!(validate_workflow_template(&parsed).is_empty());
    }

    #[test]
    fn parse_template_rejects_missing_template() {
        let error = tool()
            .parse_template(&json!({"description": "只有一句话"}))
            .expect_err("缺少 template 必须报错");
        assert!(error.contains("缺少 template"));
    }

    #[test]
    fn spec_is_a_write_effect_builtin_tool() {
        let spec = tool().spec();
        assert_eq!(spec.name, TOOL_NAME);
        let effect = spec.effect.expect("必须声明 effect");
        assert!(matches!(effect.class, EffectClass::Write));
    }

    #[tokio::test]
    async fn run_saves_validated_template_and_requires_overwrite() {
        use owo_agent_core::{AuditLog, ElementRegistry, Policy, Session, SkillRegistry};
        use std::sync::{Arc, Mutex};

        let temp = tempfile::tempdir().unwrap();
        let workswarm_root = temp.path().join("workswarm");
        let tool = WorkflowAuthorTool::new(workswarm_root.clone(), temp.path().to_path_buf());
        let mut session = Session::new(temp.path(), "mock", None);
        let policy = Policy::new(temp.path());
        let audit = Arc::new(Mutex::new(AuditLog::default()));
        let elements = Arc::new(Mutex::new(ElementRegistry::new()));
        let skills = SkillRegistry::default();
        let mut ctx = ToolContext {
            workspace: temp.path(),
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        let saved = tool.run(&mut ctx, valid_template()).await.unwrap();
        assert_eq!(saved["template_id"], "research-plan");
        assert!(workswarm_root
            .join("templates")
            .join("templates")
            .join("research-plan.json")
            .exists());

        let error = tool
            .run(&mut ctx, valid_template())
            .await
            .expect_err("同 id 缺省不得覆盖");
        assert!(error.contains("overwrite=true"));

        let mut overwrite = valid_template();
        overwrite["overwrite"] = json!(true);
        let saved = tool.run(&mut ctx, overwrite).await.unwrap();
        assert_eq!(saved["overwritten"], true);
    }

    #[tokio::test]
    async fn author_template_is_visible_to_http_team_creation_and_reopened_registry() {
        use owo_agent_core::{Agent, AuditLog, ElementRegistry, Policy, Session, SkillRegistry};
        use owo_agent_core::tools::ToolRegistry;
        use owo_agent_core::sqlite_store::SqliteSessionStore;
        use std::sync::{Arc, Mutex};
        use tower::ServiceExt;

        struct NoModel;
        #[async_trait]
        impl owo_agent_core::gateway::ModelProvider for NoModel {
            async fn complete(
                &self,
                _: &[owo_agent_core::ChatMessage],
                _: &[ToolSpec],
            ) -> Result<owo_agent_core::ModelOutput, String> {
                Err("human-node persistence regression must not call a model".into())
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let data_root = temp.path().join("data");
        std::fs::create_dir_all(&workspace).unwrap();
        let workswarm_root = data_root.join("workswarm");
        let author = WorkflowAuthorTool::new(workswarm_root.clone(), workspace.clone());
        let policy = Policy::new(&workspace);
        let mut session = Session::new(&workspace, "test", None);
        let audit = Arc::new(Mutex::new(AuditLog::default()));
        let elements = Arc::new(Mutex::new(ElementRegistry::new()));
        let skills = SkillRegistry::default();
        let mut ctx = ToolContext {
            workspace: &workspace, policy: &policy, session: &mut session,
            audit: &audit, subagent: None, skills: &skills, elements: &elements,
            fanout: None, abort: None, questioner: None,
        };
        let mut args = valid_template();
        args["template"]["roles"][0]["assignee"] = json!("human");
        args["template"]["roles"][0]["tools"] = json!([]);
        author.run(&mut ctx, args).await.unwrap();
        let registry = TeamTemplateRegistry::for_workswarm_root(&workswarm_root);
        assert_eq!(registry.list_templates().len(), 1);
        let mut builtin = registry.get_template("research-plan").unwrap();
        builtin.template_id = "protected-builtin".into();
        builtin.origin = None;
        registry.save_template(&builtin).unwrap();
        let mut overwrite = valid_template();
        overwrite["template"]["template_id"] = json!("protected-builtin");
        overwrite["overwrite"] = json!(true);
        assert!(author.run(&mut ctx, overwrite).await.unwrap_err().contains("内置"));
        assert!(registry.get_template("protected-builtin").unwrap().origin.is_none());

        let agent = Agent::new(Arc::new(NoModel), ToolRegistry::empty(), Policy::new(&workspace), Default::default());
        let store = SqliteSessionStore::open(&workspace.join("sessions.db")).unwrap();
        let state = Arc::new(crate::AppState::new(agent, store, workspace.join("traces"), data_root, workspace));
        let app = crate::build_router(Arc::clone(&state));
        let response = app.clone().oneshot(axum::http::Request::builder()
            .uri("/workflows/user")
            .header("Authorization", format!("Bearer {}", state.auth_token.token()))
            .body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let listing: Value = serde_json::from_slice(&body).unwrap();
        assert!(listing["templates"].as_array().unwrap().iter().any(|item| item["template"]["template_id"] == "research-plan"));
        let response = app.oneshot(axum::http::Request::builder()
            .method("POST").uri("/teams")
            .header("Authorization", format!("Bearer {}", state.auth_token.token()))
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(json!({
                "objective": "check authored human workflow", "mode": "team", "strategy": "team", "template_id": "research-plan"
            }).to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let created: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status, axum::http::StatusCode::ACCEPTED, "{created}");
        assert_eq!(created["template_id"], "research-plan");
        assert!(TeamTemplateRegistry::for_workswarm_root(&workswarm_root).get_template("research-plan").is_some());
    }

}
