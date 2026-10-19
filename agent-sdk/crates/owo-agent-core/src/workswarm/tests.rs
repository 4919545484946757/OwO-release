use super::util::*;
use super::*;
use crate::plan::VerificationSpec;
use owo_agent_protocol::TeamTemplate;

#[test]
fn team_benefit_model_binding_requires_one_effective_agent_model() {
    let mut writer = RoleSpec::agent("writer");
    let provider_model = "glm-5.3-flash";
    assert_eq!(
        resolve_team_model_binding(std::slice::from_ref(&writer), None, provider_model).as_deref(),
        Some(provider_model)
    );

    writer.model = Some("writer-model".to_string());
    writer.extra_input = serde_json::json!({ "model": "input-model" });
    assert_eq!(
        resolve_team_model_binding(
            std::slice::from_ref(&writer),
            Some("team-model"),
            provider_model
        )
        .as_deref(),
        Some("input-model")
    );
    writer.extra_input = Value::Null;
    assert_eq!(
        resolve_team_model_binding(
            std::slice::from_ref(&writer),
            Some("team-model"),
            provider_model
        )
        .as_deref(),
        Some("writer-model")
    );

    let mut reviewer = RoleSpec::agent("reviewer");
    reviewer.model = Some("review-model".to_string());
    assert_eq!(
        resolve_team_model_binding(&[writer, reviewer], None, provider_model),
        None
    );
}

#[test]
fn default_relay_roles_form_valid_dag() {
    let roles = default_relay_roles();
    assert_eq!(roles.len(), 4);
    let mut plan = Plan::new("p", "g");
    for r in &roles {
        let verification = r.verify.as_ref().map(|value| parse_verify(value));
        let step = StepSpec {
            id: format!("s-{}", r.role),
            depends_on: r.depends_on.iter().map(|d| format!("s-{d}")).collect(),
            parallel: true,
            worker: format!("m-{}", r.role),
            input: Value::Null,
            verification_plan: verification
                .as_ref()
                .map(|value| verification_plan_for_step(&format!("s-{}", r.role), value)),
            verify: verification,
            retries: 0,
        };
        plan.add_step(step);
    }
    plan.validate().unwrap();
}

#[test]
fn parse_verify_kinds() {
    assert!(matches!(
        parse_verify("non_empty"),
        VerificationSpec::OutputNonEmpty
    ));
    assert!(matches!(
        parse_verify("contains:ok"),
        VerificationSpec::OutputContains(_)
    ));
    assert!(matches!(
        parse_verify("equals:x"),
        VerificationSpec::OutputEquals(_)
    ));
    assert!(matches!(
        parse_verify("whatever"),
        VerificationSpec::Custom(_)
    ));

    let supported = verification_plan_for_step("task-1", &parse_verify("contains:accepted"));
    assert!(supported.validate().is_ok());
    assert_eq!(
        supported.requirements[0].validator_id,
        "artifact-output-contains-v1"
    );
    assert_eq!(supported.requirements[0].arguments["value"], "accepted");

    let unsupported = verification_plan_for_step("task-2", &parse_verify("custom-check"));
    assert_eq!(
        unsupported.requirements[0].validator_id, "custom-check",
        "Custom must stay unresolved for the host registry and cannot become a passing fallback"
    );
}

#[test]
fn role_kind_mapping() {
    assert_eq!(role_kind("planner"), "plan");
    assert_eq!(role_kind("builder"), "document");
    assert_eq!(role_kind("critic"), "review");
    assert!(is_review_role_name("reviewer"));
    assert!(is_review_role_name("content_reviewer"));
    assert!(is_review_role_name("security-review"));
    assert!(!is_review_role_name("project_integrator"));
    assert!(is_review_role("audit", &["review".to_string()]));
    assert!(!is_review_role("reviewer", &["implement".to_string()]));
    let mut custom_reviewer = RoleSpec::agent("quality_gate");
    custom_reviewer.capabilities = vec!["review".to_string()];
    assert!(custom_reviewer.is_reviewer());
    assert!(validate_role_write_paths_with_capabilities(
        &custom_reviewer.role,
        &custom_reviewer.capabilities,
        &["src/lib.rs".to_string()],
    )
    .is_err());
    assert!(validate_role_write_paths("content_reviewer", &[]).is_ok());
    assert!(validate_role_write_paths("content_reviewer", &["notes.md".into()]).is_err());
    assert_eq!(role_kind("leader"), "final");
    assert_eq!(role_kind("custom-role"), "custom-role");
}

#[test]
fn template_adopt_is_idempotent_and_reject_blocks() {
    let dir = std::env::temp_dir().join(format!("owo-workswarm-test-{}", now_ms()));
    let reg = TeamTemplateRegistry::new(dir.clone());
    let template = TeamTemplate {
        template_id: "tpl-x".into(),
        name: "test".into(),
        mode: TeamMode::Team,
        roles: Vec::new(),
        applicability: "浏览器 任务".into(),
        source_team_id: Some("team-x".into()),
        created_at: now_ts(),
        origin: None,
        description: None,
        version: None,
        tags: Vec::new(),
    };
    let proposal = TeamTemplateProposal {
        proposal_id: "prop-x".into(),
        template: template.clone(),
        source_team_id: "team-x".into(),
        evidence: vec!["art-1".into()],
        status: TeamTemplateProposalStatus::Proposed,
        created_at: now_ts(),
    };
    reg.save_proposal(&proposal).unwrap();
    // 提案不影响注册表（只提案，不自动启用）。
    assert!(reg.get_template("tpl-x").is_none());
    // 采纳 → 进注册表。
    let adopted = reg.adopt_proposal("prop-x").unwrap();
    assert_eq!(adopted.template_id, "tpl-x");
    assert!(reg.get_template("tpl-x").is_some());
    // 幂等。
    let again = reg.adopt_proposal("prop-x").unwrap();
    assert_eq!(again.template_id, "tpl-x");
    // 已采纳不可拒绝。
    assert!(reg.reject_proposal("prop-x").is_err());
    // 匹配：applicability 关键词命中。
    assert!(reg
        .find_match(TeamMode::Team, "完成浏览器表单任务")
        .is_some());
    assert!(reg
        .find_match(TeamMode::Single, "完成浏览器表单任务")
        .is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn existing_agent_prompt_receives_the_host_resolved_task_contract() {
    let task = crate::task_context::ResolvedTaskContext::from_worker_input(&json!({
        "assigned_task_id": "task-api",
        "assigned_task": "实现 posts API 分页",
        "assigned_acceptance": "覆盖默认值和上限",
        "assigned_verification": {"requirements": ["page boundary"]},
        "assigned_write_paths": ["apps/api"],
        "required_capabilities": ["write_file", "run_command"]
    }))
    .unwrap();
    let ctx = json!({
        "role": "implementer",
        "capabilities": ["implement"],
        "_resolved_task_context": task.to_value().unwrap()
    });
    let input = json!({ "prompt": "已存在的角色提示", "_workswarm": {} });
    let enriched = TeamCoordinator::build_enriched_input(&ctx, &input, "agent");
    assert!(enriched["prompt"].is_null());
    let prompt_context = &enriched["team_prompt_context"];
    let handoff = prompt_context["handoff_contract"].as_str().unwrap();
    assert!(handoff.contains("已存在的角色提示"));
    assert!(handoff.contains("实现 posts API 分页"));
    assert!(handoff.contains("覆盖默认值和上限"));
    assert!(handoff.contains("apps/api"));
    assert_eq!(enriched["resolved_task_context"]["task_id"], "task-api");
    assert_eq!(prompt_context["task_scoped"], true);
}

#[test]
fn existing_template_reviewer_prompt_keeps_host_review_manifest() {
    let task = crate::task_context::ResolvedTaskContext::from_worker_input(&json!({
        "assigned_task_id": "task-review",
        "assigned_task": "审查已实现的 API",
        "assigned_acceptance": "确认错误路径和边界行为"
    }))
    .unwrap();
    let ctx = json!({
        "role": "reviewer",
        "capabilities": ["review"],
        "handoff_contract": "host review manifest: step-api:behavior",
        "_resolved_task_context": task.to_value().unwrap()
    });
    let input = json!({"prompt": "已有模板评审提示"});
    let enriched = TeamCoordinator::build_enriched_input(&ctx, &input, "agent");
    let prompt_context = &enriched["team_prompt_context"];
    let handoff = prompt_context["handoff_contract"].as_str().unwrap();

    assert!(handoff.contains("已有模板评审提示"));
    assert!(handoff.contains("host review manifest: step-api:behavior"));
    assert!(handoff.contains("审查已实现的 API"));
    assert!(handoff.contains("确认错误路径和边界行为"));
    assert_eq!(enriched["read_only"], true);
}

#[test]
fn final_runtime_profile_controls_team_prompt_permissions_and_budget() {
    let context = json!({
        "objective_text": "检查当前实现",
        "role": "implementer",
        "handoff_contract": "只读检查并提交发现",
        "template_id": null,
        "budget_calls": 12,
        "upstream": [],
        "shared_facts": [],
        "shared_context_revision": 0,
    });
    let effective_profile = crate::worker_profile::WorkerProfile::for_role("reviewer", 3);
    let (prompt, _) =
        TeamCoordinator::compile_role_prompt_with_profile(&context, &effective_profile);
    let tool_line = prompt
        .lines()
        .find(|line| line.contains("可见工具仅限："))
        .unwrap();
    assert!(tool_line.contains("read_file"));
    assert!(!tool_line.contains("write_file"));
    assert!(!tool_line.contains("run_command"));
    assert!(prompt.contains("禁止写入工作区文件"));
    assert!(prompt.contains("你的回合预算为 3 回合"));
}

#[test]
fn enriched_input_agent_and_echo_paths() {
    let ctx = json!({
        "team_id": "t", "objective_text": "O", "role": "critic",
        "handoff_contract": "C", "shared_context_revision": 4, "_workswarm": {},
        "shared_facts": [{"key":"contract","value":"FACT","truncated":true}],
        "upstream": [{"role":"builder","artifact_id":"a1","version":1,"content":"BODY"}]
    });
    // Agent 入口只传结构化 Prompt 上下文；Server 收窄出最终 profile 后再生成模型提示。
    let input = json!({"_workswarm": {}});
    let v = TeamCoordinator::build_enriched_input(&ctx, &input, "agent");
    let owned = TeamCoordinator::build_enriched_input_owned(ctx.clone(), &input, "agent");
    assert_eq!(v, owned);
    assert!(v["prompt"].is_null());
    assert_eq!(v["team_prompt_context"]["role"], "critic");
    assert_eq!(v["read_only"], true);
    let (prompt, meta) = TeamCoordinator::compile_role_prompt_with_meta(&v["team_prompt_context"]);
    assert!(prompt.contains("# 角色：critic"));
    assert!(prompt.contains("team_context_read"));
    let fact_bytes = ctx["shared_facts"].to_string().len();
    assert_eq!(meta["shared_fact_bytes"].as_u64(), Some(fact_bytes as u64));
    assert_eq!(meta["shared_fact_truncated_count"].as_u64(), Some(1));
    assert_eq!(
        meta["context_bytes"].as_u64(),
        Some(meta["upstream_context_bytes"].as_u64().unwrap() + fact_bytes as u64)
    );

    // echo：text 承载上下文切片。
    let v = TeamCoordinator::build_enriched_input(&ctx, &json!({}), "echo");
    assert!(v["text"].as_str().unwrap().contains("BODY"));
}
