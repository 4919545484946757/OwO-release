//! Host-bound verification plan tool and coverage validation for Single turns.

use super::{Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Host-bound acceptance requirements for an ordinary Single task.
pub(super) struct SingleVerificationPlanTool;

pub(crate) fn validate_single_verification_plan(
    plan: &crate::plan::VerificationPlanV1,
) -> Result<(), String> {
    plan.validate()?;
    if plan.plan_id.len() > 128 || plan.requirements.len() > 32 {
        return Err("VerificationPlan 超过宿主的 plan_id/requirement 数量上限".to_string());
    }
    for requirement in &plan.requirements {
        let manual_acceptance = requirement.validator_id
            == crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID
            && matches!(&requirement.scope, crate::plan::VerificationScopeV1::Manual);
        if requirement.covers_requirement_ids.is_empty()
            || requirement.covers_requirement_ids.iter().any(|id| {
                id.strip_prefix("user-request:")
                    .is_none_or(|quote| quote.trim().is_empty())
            })
        {
            return Err(format!(
                "requirement {} 的 covers_requirement_ids 必须使用 user-request:<用户原文验收片段>",
                requirement.requirement_id
            ));
        }
        if requirement.validator_version.as_deref() != Some("1") {
            return Err(format!(
                "requirement {} 必须固定宿主 validator version 1",
                requirement.requirement_id
            ));
        }
        if manual_acceptance {
            if requirement.arguments != json!({}) {
                return Err(format!(
                    "requirement {} 的人工验收 arguments 必须为空对象",
                    requirement.requirement_id
                ));
            }
            continue;
        }
        if !crate::verification::is_registered_workspace_validator(&requirement.validator_id) {
            return Err(format!(
                "requirement {} 使用了未注册的宿主 validator/version",
                requirement.requirement_id
            ));
        }
        let crate::plan::VerificationScopeV1::WorkspacePaths { relative_paths } =
            &requirement.scope
        else {
            return Err(format!(
                "requirement {} 必须绑定 WorkspacePaths 或明确声明人工验收",
                requirement.requirement_id
            ));
        };
        if !(crate::verification::MIN_WORKSPACE_VALIDATION_PATHS
            ..=crate::verification::MAX_WORKSPACE_VALIDATION_PATHS)
            .contains(&relative_paths.len())
        {
            return Err(format!(
                "requirement {} 声明了 {} 个路径；WorkspacePaths 必须提供 {}..={} 个工作区相对路径",
                requirement.requirement_id,
                relative_paths.len(),
                crate::verification::MIN_WORKSPACE_VALIDATION_PATHS,
                crate::verification::MAX_WORKSPACE_VALIDATION_PATHS
            ));
        }
        if !crate::verification::workspace_validator_arguments_supported(
            &requirement.validator_id,
            &requirement.arguments,
        ) {
            let expected = crate::verification::workspace_validator_contracts()
                .iter()
                .find(|contract| contract.validator_id == requirement.validator_id)
                .map(validator_arguments_hint)
                .unwrap_or_else(|| "该 validator 未注册".to_string());
            return Err(format!(
                "requirement {} 的 validator {} 参数不符合宿主契约；arguments 应为 {}，不要添加额外字段",
                requirement.requirement_id, requirement.validator_id, expected
            ));
        }
        let resources = &requirement.resources;
        if resources.cpu_slots != 1
            || !(8..=128).contains(&resources.memory_mb)
            || resources.exclusive_workspace
            || !(1..=30_000).contains(&resources.timeout_ms)
        {
            return Err(format!(
                "requirement {} 的资源声明超出宿主验证器预算",
                requirement.requirement_id
            ));
        }
    }
    Ok(())
}

/// 新计划是否"逐字保留"了已登记计划里的全部要求（允许新增，不允许删除/改写）。
/// 用于允许首次写入后补上行为命令等**加强型**修正，同时禁止降低验收要求。
fn plan_is_superset(
    previous: &crate::plan::VerificationPlanV1,
    next: &crate::plan::VerificationPlanV1,
) -> bool {
    previous
        .requirements
        .iter()
        .all(|old| next.requirements.iter().any(|new| new == old))
}

pub(super) fn validate_single_request_coverage(
    plan: &crate::plan::VerificationPlanV1,
    request: &str,
) -> Result<(), String> {
    let mut quotes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for requirement in plan
        .requirements
        .iter()
        .filter(|requirement| requirement.required)
    {
        for coverage_id in &requirement.covers_requirement_ids {
            let quote = coverage_id
                .strip_prefix("user-request:")
                .map(str::trim)
                .filter(|quote| !quote.is_empty())
                .ok_or_else(|| format!("验收点 {coverage_id} 不是有效的用户原文引用"))?;
            if seen.insert(quote.to_string()) {
                quotes.push(quote.to_string());
            }
        }
    }
    crate::request_requirements::validate_exact_user_request_quotes(&quotes, request).map_err(
        |error| {
            // 真实模型实测：模型常引用自己改写的句子而被拒，反复重试浪费回合。
            // 错误里直接给出用户原文的可复制片段，帮助模型一次改对。
            let suggestion = request
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| line.chars().take(60).collect::<String>())
                .unwrap_or_default();
            format!(
                "{error}；请逐字复制用户原文中的连续片段（仅允许反引号差异），例如：{suggestion}"
            )
        },
    )?;
    crate::request_requirements::validate_plan_covers_explicit_acceptance(plan, request)?;
    Ok(())
}

/// Render a concise model-facing hint from the same host contract schema used by
/// argument validation. This keeps tool docs from drifting as validators evolve.
fn validator_arguments_hint(contract: &crate::verification::WorkspaceValidatorContract) -> String {
    let schema = contract.arguments_schema();
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return "{}".to_string();
    };
    if properties.is_empty() {
        return "{}".to_string();
    }
    let required: std::collections::HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let fields = properties
        .iter()
        .map(|(name, field)| {
            let kind = field.get("type").and_then(Value::as_str).unwrap_or("JSON");
            let requirement = if required.contains(name.as_str()) {
                "required"
            } else {
                "optional"
            };
            let description = field
                .get("description")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(|value| format!(", {value}"))
                .unwrap_or_default();
            format!("{name}: {kind} ({requirement}{description})")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{fields}}}")
}

fn validator_contracts_hint() -> String {
    let mut entries = crate::verification::workspace_validator_contracts()
        .iter()
        .map(|contract| {
            format!(
                "{} arguments={}",
                contract.validator_id,
                validator_arguments_hint(contract)
            )
        })
        .collect::<Vec<_>>();
    entries.push(format!(
        "{} arguments={{}} (人工验收)",
        crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID
    ));
    entries.join("; ")
}

fn normalize_model_verification_plan(value: &Value) -> Result<Value, String> {
    let requirements = value
        .get("requirements")
        .and_then(Value::as_array)
        .ok_or_else(|| "VerificationPlan.requirements 必须是数组".to_string())?;
    for requirement in requirements {
        let object = requirement
            .as_object()
            .ok_or_else(|| "VerificationPlan requirement 必须是对象".to_string())?;
        if object.get("validator_id").and_then(Value::as_str).is_none() {
            return Err("每项 requirement 必须包含 validator_id 字符串".to_string());
        }
        if object
            .get("arguments")
            .is_none_or(|arguments| !arguments.is_object())
        {
            return Err("每项 requirement 的 arguments 必须是 JSON 对象".to_string());
        }
        let scope = object
            .get("scope")
            .and_then(Value::as_object)
            .ok_or_else(|| "每项 requirement 的 scope 必须是对象".to_string())?;
        match scope.get("kind").and_then(Value::as_str) {
            Some("workspace_paths") if !scope.contains_key("relative_paths") => {
                return Err("workspace_paths scope 必须提供 relative_paths".to_string());
            }
            Some("manual") if scope.contains_key("relative_paths") => {
                return Err("manual scope 不接受 relative_paths".to_string());
            }
            Some("workspace_paths" | "manual") => {}
            _ => return Err("scope.kind 必须是 workspace_paths 或 manual".to_string()),
        }
    }
    Ok(value.clone())
}

fn verification_plan_requirement_schema() -> Value {
    let validator_ids = crate::verification::workspace_validator_contracts()
        .iter()
        .map(|contract| contract.validator_id)
        .chain(std::iter::once(
            crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID,
        ))
        .collect::<Vec<_>>();
    let common_properties = json!({
        "requirement_id": {"type": "string", "minLength": 1},
        "covers_requirement_ids": {
            "type": "array",
            "minItems": 1,
            "items": {"type": "string", "minLength": 1, "description": "格式：user-request:<用户原文中的精确验收片段>。"}
        },
        "validator_id": {"type": "string", "enum": validator_ids},
        "arguments": {
            "type": "object",
            "description": format!("参数按 validator_id 匹配：{}", validator_contracts_hint())
        },
        "validator_version": {"type": "string", "enum": ["1"]},
        "required": {"type": "boolean"},
        "resources": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "cpu_slots": {"type": "integer", "enum": [1]},
                "memory_mb": {"type": "integer", "minimum": 8, "maximum": 128},
                "exclusive_workspace": {"type": "boolean", "enum": [false]},
                "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 30000}
            },
            "required": ["cpu_slots", "memory_mb", "exclusive_workspace", "timeout_ms"]
        },
        "scope": {
            "type": "object",
            "additionalProperties": false,
            "description": format!(
                "自动检查使用 kind=workspace_paths，并提供{}至{}个相对路径；人工验收使用 kind=manual 且不提供路径。",
                crate::verification::MIN_WORKSPACE_VALIDATION_PATHS,
                crate::verification::MAX_WORKSPACE_VALIDATION_PATHS
            ),
            "properties": {
                "kind": {"type": "string", "enum": ["workspace_paths", "manual"]},
                "relative_paths": {
                    "type": "array",
                    "minItems": crate::verification::MIN_WORKSPACE_VALIDATION_PATHS,
                    "maxItems": crate::verification::MAX_WORKSPACE_VALIDATION_PATHS,
                    "items": {"type": "string", "minLength": 1, "description": "工作区根目录下的相对文件路径，不得使用绝对路径或 ..。"}
                }
            },
            "required": ["kind"]
        }
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": common_properties,
        "required": [
            "requirement_id",
            "covers_requirement_ids",
            "validator_id",
            "arguments",
            "validator_version",
            "scope",
            "required",
            "resources"
        ]
    })
}

fn verification_plan_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "plan": {
                "type": "object",
                "additionalProperties": false,
                "description": "首次工作区写入前登记验收要求；登记后不可替换。",
                "properties": {
                    "plan_id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "requirements": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 32,
                        "description": format!("每项覆盖用户原文验收点。自动检查仅列相关工作区相对文件路径，每项{}至{}个；人工验收使用 manual。", crate::verification::MIN_WORKSPACE_VALIDATION_PATHS, crate::verification::MAX_WORKSPACE_VALIDATION_PATHS),
                        "items": verification_plan_requirement_schema()
                    }
                },
                "required": ["plan_id", "requirements"]
            }
        },
        "required": ["plan"]
    })
}
#[async_trait]
impl Tool for SingleVerificationPlanTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "verification_plan".into(),
            description: "开始任何工作区写入前，先登记本次任务的宿主验收要求。计划首次写入后只能追加/加强（例如补上行为命令），不能删除或改写既有要求。每个必需 covers_requirement_ids 都必须写成 user-request:<用户原文中的精确验收片段>，宿主会核对它确实出现在本回合输入中（允许用户原文与引用之间有 Markdown 反引号差异）。选验证器按产物类型：源码/可运行工程用 workspace-command-success-v1（只接受宿主登记的行为命令，如 cargo test/npm test/python -m pytest，禁止 dir/echo 等普通命令）；文本或文档产物用静态校验 workspace-file-exists-v1 / workspace-file-non-empty-v1 / workspace-file-contains-v1 / workspace-json-field-equals-v1，不要为普通文本文件登记命令校验；确实没有可运行自动验收时，才可声明 single-human-acceptance-v1 + manual scope，宿主会展示当前候选快照并等待用户明确验收。计划本身不是通过证据。arguments 只允许契约列出的字段，多一个额外字段都会被拒。resources 四个字段按 schema 显式填写；人工验收不消耗该资源配额。可照抄的最小示例（单条行为命令）：{\"plan\":{\"plan_id\":\"p1\",\"requirements\":[{\"requirement_id\":\"req-tests\",\"covers_requirement_ids\":[\"user-request:<原文逐字片段>\"],\"validator_id\":\"workspace-command-success-v1\",\"validator_version\":\"1\",\"arguments\":{\"command\":\"npm test\"},\"required\":true,\"scope\":{\"kind\":\"workspace_paths\",\"relative_paths\":[\"tests/game.test.mjs\"]},\"resources\":{\"cpu_slots\":1,\"memory_mb\":64,\"exclusive_workspace\":false,\"timeout_ms\":30000}}]}}".into(),
            input_schema: verification_plan_input_schema(),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let model_plan = args.get("plan").ok_or("缺少 plan 对象")?;
        let normalized_plan = normalize_model_verification_plan(model_plan)?;
        let plan: crate::plan::VerificationPlanV1 = serde_json::from_value(normalized_plan)
            .map_err(|error| format!("VerificationPlan 结构非法：{error}"))?;
        validate_single_verification_plan(&plan)?;
        let task_context = ctx
            .session
            .active_task_context
            .as_ref()
            .ok_or("当前 Agent 回合没有宿主解析的任务上下文")?;
        let request = task_context
            .objective
            .as_deref()
            .ok_or("当前 Agent 回合没有可核对的原始用户输入")?;
        let input_sha256 = crate::CasStore::hash_of(request.as_bytes());
        let turn_id = task_context
            .attempt_id
            .clone()
            .ok_or("当前 Agent 回合没有绑定的 attempt_id")?;
        validate_single_request_coverage(&plan, request)?;
        let has_current_turn_writes = ctx
            .session
            .execution_receipts
            .iter()
            .any(|receipt| receipt.turn_id == turn_id && receipt.status != "reverted");
        let registered_same_turn =
            ctx.session.single_verification_plan_turn_id.as_deref() == Some(turn_id.as_str());
        // 真实模型实测：模型常在首次登记时写错 validator 参数，随后才想补上行为命令。
        // 允许「追加/加强」型替换（逐字保留全部既有要求），仍禁止删除或改写既有要求，
        // 从而既解开死锁，又保持"不能看到结果后降低验收"的防作弊语义。
        let weakens = match ctx.session.single_verification_plan.as_ref() {
            Some(previous) => previous != &plan && !plan_is_superset(previous, &plan),
            None => true,
        };
        if has_current_turn_writes && !(registered_same_turn && !weakens) {
            return Err(
                "VerificationPlan 必须在首次工作区写入前登记；首次写入后只能追加/加强验收要求，不能替换或降低".to_string(),
            );
        }
        if registered_same_turn && weakens {
            return Err(
                "本回合 VerificationPlan 已冻结，不能在看到验证结果后降低验收要求；只能追加/加强"
                    .to_string(),
            );
        }
        let mut resolved_context = task_context.clone();
        resolved_context.bind_single_verification_plan(&plan)?;
        ctx.session.single_verification_plan = Some(plan.clone());
        ctx.session.single_verification_plan_input_sha256 = Some(input_sha256);
        ctx.session.single_verification_plan_turn_id = Some(turn_id);
        ctx.session.active_task_context = Some(resolved_context);
        Ok(json!({"plan": plan, "status": "registered_pending_host_validation"}))
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    #[test]
    fn verification_plan_schema_is_derived_from_registered_validator_contracts() {
        let schema = verification_plan_input_schema();
        assert_eq!(schema["type"], "object");
        let requirement = &schema["properties"]["plan"]["properties"]["requirements"]["items"];
        let validator_ids = requirement["properties"]["validator_id"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        let expected_ids = crate::verification::workspace_validator_contracts()
            .iter()
            .map(|contract| contract.validator_id)
            .chain(std::iter::once(
                crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID,
            ))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(validator_ids, expected_ids);
        assert_eq!(requirement["properties"]["arguments"]["type"], "object");
        assert_eq!(
            requirement["properties"]["scope"]["properties"]["kind"]["enum"],
            json!(["workspace_paths", "manual"])
        );
        assert_eq!(
            requirement["properties"]["scope"]["properties"]["relative_paths"]["minItems"],
            crate::verification::MIN_WORKSPACE_VALIDATION_PATHS
        );
        assert_eq!(
            requirement["properties"]["scope"]["properties"]["relative_paths"]["maxItems"],
            crate::verification::MAX_WORKSPACE_VALIDATION_PATHS
        );
        assert!(requirement.get("anyOf").is_none());
        assert!(requirement.get("oneOf").is_none());
        for contract in crate::verification::workspace_validator_contracts() {
            assert!(validator_contracts_hint().contains(contract.validator_id));
        }
    }

    #[test]
    fn model_plan_normalizer_accepts_generic_contract_and_rejects_invalid_scope_pairing() {
        let input = json!({
            "plan_id": "p",
            "requirements": [{
                "requirement_id": "r",
                "validator_id": "workspace-file-contains-v1",
                "arguments": {"text": "hello"},
                "scope": {"kind": "workspace_paths", "relative_paths": ["README.md"]}
            }]
        });
        let normalized = normalize_model_verification_plan(&input).unwrap();
        assert_eq!(
            normalized["requirements"][0]["validator_id"],
            "workspace-file-contains-v1"
        );
        assert_eq!(
            normalized["requirements"][0]["arguments"],
            json!({"text": "hello"})
        );

        let missing_paths = json!({"requirements": [{"validator_id": "workspace-file-exists-v1", "arguments": {}, "scope": {"kind": "workspace_paths"}}]});
        assert!(normalize_model_verification_plan(&missing_paths)
            .unwrap_err()
            .contains("relative_paths"));
        let manual_with_paths = json!({"requirements": [{"validator_id": "single-human-acceptance-v1", "arguments": {}, "scope": {"kind": "manual", "relative_paths": ["README.md"]}}]});
        assert!(normalize_model_verification_plan(&manual_with_paths)
            .unwrap_err()
            .contains("manual scope"));
    }

    #[test]
    fn generic_model_schema_defers_validator_specific_arguments_to_host_contract() {
        let schema = verification_plan_input_schema();
        let requirement_schema =
            &schema["properties"]["plan"]["properties"]["requirements"]["items"];
        let make_requirement = |validator: &str, arguments: Value| {
            json!({
                "requirement_id": "r1",
                "covers_requirement_ids": ["user-request:must work"],
                "validator_id": validator,
                "arguments": arguments,
                "validator_version": "1",
                "scope": {"kind":"workspace_paths","relative_paths":["README.md"]},
                "required": true,
                "resources": {
                    "cpu_slots": 1,
                    "memory_mb": 32,
                    "exclusive_workspace": false,
                    "timeout_ms": 5000
                }
            })
        };
        assert!(crate::json_schema::validate(
            &make_requirement("workspace-file-contains-v1", json!({"text":"hello"})),
            requirement_schema,
            "requirement"
        )
        .is_ok());
        // Provider-facing schema remains a portable generic function contract.
        // Validator-specific shapes are enforced by the host's canonical registry.
        assert!(crate::json_schema::validate(
            &make_requirement("workspace-command-success-v1", json!({"text":"hello"})),
            requirement_schema,
            "requirement"
        )
        .is_ok());
        assert!(crate::json_schema::validate(
            &make_requirement("workspace-file-contains-v1", json!({})),
            requirement_schema,
            "requirement"
        )
        .is_ok());
        assert!(
            !crate::verification::workspace_validator_arguments_supported(
                "workspace-command-success-v1",
                &json!({"text":"hello"})
            )
        );
        assert!(
            !crate::verification::workspace_validator_arguments_supported(
                "workspace-file-contains-v1",
                &json!({})
            )
        );
        assert!(
            crate::verification::workspace_validator_arguments_supported(
                "workspace-file-contains-v1",
                &json!({"text":"hello"})
            )
        );
        assert!(
            !crate::verification::workspace_validator_arguments_supported(
                "workspace-file-contains-v1",
                &json!({"text":"hello","extra":true})
            )
        );
        assert!(
            !crate::verification::workspace_validator_arguments_supported(
                "not-registered",
                &json!({"text":"hello"})
            )
        );
    }

    #[test]
    fn plan_superset_allows_appending_but_not_rewriting_registered_requirements() {
        let requirement = |id: &str, validator: &str, arguments: Value| {
            json!({
                "requirement_id": id,
                "covers_requirement_ids": ["user-request:must work"],
                "validator_id": validator,
                "validator_version": "1",
                "arguments": arguments,
                "required": true,
                "scope": {"kind": "workspace_paths", "relative_paths": ["README.md"]},
                "resources": {
                    "cpu_slots": 1,
                    "memory_mb": 32,
                    "exclusive_workspace": false,
                    "timeout_ms": 5000
                }
            })
        };
        let plan = |requirements: Vec<Value>| {
            serde_json::from_value::<crate::plan::VerificationPlanV1>(json!({
                "plan_id": "p1",
                "requirements": requirements
            }))
            .expect("测试计划应可反序列化")
        };
        let base = plan(vec![requirement(
            "req-static",
            "workspace-file-exists-v1",
            json!({}),
        )]);
        // 追加行为命令属于"加强"，允许（真实模型实测：先登记静态校验，写入后想补命令）。
        let mut appended = base.clone();
        appended.requirements.push(
            serde_json::from_value(requirement(
                "req-tests",
                "workspace-command-success-v1",
                json!({"command": "npm test"}),
            ))
            .unwrap(),
        );
        assert!(plan_is_superset(&base, &appended));
        // 反向删减既有要求属于"降低"，拒绝。
        assert!(!plan_is_superset(&appended, &base));
        // 改写既有要求的参数属于"降低"，拒绝。
        let mut rewritten = base.clone();
        rewritten.requirements[0].arguments = json!({"text": "改掉"});
        assert!(!plan_is_superset(&base, &rewritten));
        // 完全一致（幂等重登记）视为不降低。
        assert!(plan_is_superset(&base, &base));
    }

    #[test]
    fn coverage_error_suggests_a_copyable_user_excerpt() {
        let plan = serde_json::from_value::<crate::plan::VerificationPlanV1>(json!({
            "plan_id": "p",
            "requirements": [{
                "requirement_id": "r",
                "covers_requirement_ids": ["user-request:并不存在的原文"],
                "validator_id": "workspace-file-exists-v1",
                "validator_version": "1",
                "arguments": {},
                "required": true,
                "scope": {"kind": "workspace_paths", "relative_paths": ["README.md"]},
                "resources": {
                    "cpu_slots": 1,
                    "memory_mb": 8,
                    "exclusive_workspace": false,
                    "timeout_ms": 1000
                }
            }]
        }))
        .unwrap();
        let error =
            validate_single_request_coverage(&plan, "构建坦克大战：支持本地双人并训练 AI 对手")
                .unwrap_err();
        assert!(
            error.contains("例如：构建坦克大战"),
            "覆盖率错误应附带可复制的用户原文片段：{error}"
        );
    }
}
