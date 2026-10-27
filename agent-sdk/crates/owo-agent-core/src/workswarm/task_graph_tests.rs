//! Contracts for graph validation, scope, budgets and dynamic dependency binding.
use super::*;

#[cfg(test)]
mod task_graph_parse_tests {
    use super::parse_parallel_subtasks;

    #[test]
    fn version_one_accepts_number_and_string() {
        for version in [serde_json::json!(1), serde_json::json!("1")] {
            let graph = serde_json::json!({
                "version": version,
                "tasks": [{"task_id": "world"}],
            });
            let (tasks, versioned) = parse_parallel_subtasks(&graph.to_string())
                .expect("version 1 in either JSON type must parse");
            assert!(versioned);
            assert_eq!(tasks.len(), 1);
        }
    }

    #[test]
    fn schema_marker_taskgraph_v1_is_accepted() {
        let graph = serde_json::json!({
            "schema": "TaskGraphV1",
            "tasks": [{"task_id": "world"}],
        });
        let (tasks, versioned) =
            parse_parallel_subtasks(&graph.to_string()).expect("schema marker must parse");
        assert!(versioned);
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn graph_version_marker_is_accepted_as_taskgraph_v1() {
        let graph = serde_json::json!({
            "graph_version": "TaskGraphV1",
            "tasks": [{"task_id": "world"}],
        });
        let (tasks, versioned) =
            parse_parallel_subtasks(&graph.to_string()).expect("graph_version marker must parse");
        assert!(versioned);
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn unknown_or_missing_version_is_rejected() {
        for version in [
            serde_json::json!("2"),
            serde_json::json!(2),
            serde_json::json!(null),
        ] {
            let graph = serde_json::json!({
                "version": version,
                "tasks": [{"task_id": "world"}],
            });
            assert!(
                parse_parallel_subtasks(&graph.to_string()).is_none(),
                "version {version} must not be treated as TaskGraphV1"
            );
        }
        let missing = serde_json::json!({"tasks": [{"task_id": "world"}]});
        assert!(parse_parallel_subtasks(&missing.to_string()).is_none());
        let wrong_schema = serde_json::json!({
            "schema": "TaskGraphV2",
            "tasks": [{"task_id": "world"}],
        });
        assert!(parse_parallel_subtasks(&wrong_schema.to_string()).is_none());
    }
}

#[cfg(test)]
mod task_requirement_coverage_tests {
    use super::validate_task_requirement_coverage;

    #[test]
    fn each_user_quote_must_be_present_in_task_or_acceptance() {
        let quotes = vec!["return the saved record".to_string()];
        assert!(super::validate_task_quote_scope(
            &quotes,
            "implement the API",
            "Acceptance: return the saved record"
        )
        .is_ok());
        assert!(super::validate_task_quote_scope(
            &quotes,
            "implement an API",
            "Acceptance: return a saved record"
        )
        .unwrap_err()
        .contains("未逐字体现在"));
    }

    #[test]
    fn versioned_task_quotes_must_match_user_goal_and_cover_checklist() {
        let request = "请完成接口。\n验收标准：\n- 默认页码为 1\n- 越界返回空列表";
        let covered = vec![
            vec!["默认页码为 1".to_string()],
            vec!["越界返回空列表".to_string()],
        ];
        assert!(validate_task_requirement_coverage(&covered, request).is_ok());

        let missing = vec![vec!["默认页码为 1".to_string()]];
        assert!(validate_task_requirement_coverage(&missing, request)
            .unwrap_err()
            .contains("越界返回空列表"));

        let invented = vec![
            vec!["默认页码为 2".to_string()],
            vec!["越界返回空列表".to_string()],
        ];
        assert!(validate_task_requirement_coverage(&invented, request)
            .unwrap_err()
            .contains("不属于当前用户目标原文"));
    }

    #[test]
    fn empty_quotes_are_allowed_only_without_explicit_acceptance() {
        let no_checklist = "实现一个 3D 跑酷游戏";
        assert!(validate_task_requirement_coverage(&[vec![]], no_checklist).is_ok());

        let checklist = "实现接口。\n验收标准：\n- 默认页码为 1";
        let error = validate_task_requirement_coverage(&[vec![]], checklist).unwrap_err();
        assert!(error.contains("没有引用任何用户原文要求"), "{error}");
    }
}

#[cfg(test)]
mod parallel_assignment_validation_tests {
    use super::{
        assign_task_model_call_budgets, bind_dynamic_follow_up_dependencies,
        host_manifest_can_replace_integration, parallel_tasks_require_integration,
        should_enable_parallel_assignment, validate_parallel_subtasks, RoleSpec,
    };
    use owo_agent_contracts::plan::StepSpec;

    fn roles() -> Vec<RoleSpec> {
        vec![
            RoleSpec::agent("lead"),
            RoleSpec::agent("w1"),
            RoleSpec::agent("w2"),
            RoleSpec::agent("leader"),
        ]
    }

    fn assigned_step(id: &str, task_id: &str, input: serde_json::Value) -> StepSpec {
        let mut step = StepSpec::new(id, "writer");
        step.input = input;
        step.input["assigned_task_id"] = serde_json::json!(task_id);
        step
    }

    #[test]
    fn task_model_call_budget_grows_with_effort_but_stays_within_role_ceiling() {
        let input = [
            serde_json::json!({"task_id":"small", "worker":"w1", "task":"small", "acceptance":"done", "write_paths":[], "estimated_effort":1}),
            serde_json::json!({"task_id":"large", "worker":"w1", "task":"large", "acceptance":"done", "write_paths":[], "estimated_effort":3}),
        ];
        let mut assignments = validate_parallel_subtasks(&roles(), &input, false)
            .unwrap()
            .tasks;
        let budgets = std::collections::BTreeMap::from([("w1".to_string(), 14)]);
        assign_task_model_call_budgets(&mut assignments, &budgets).unwrap();
        let assigned = assignments
            .iter()
            .map(|task| (task.task_id.as_str(), task.model_calls_per_attempt))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(assigned["small"], 8);
        assert_eq!(assigned["large"], 10);
        assert!(assigned.values().all(|budget| *budget <= 14));
    }

    #[test]
    fn low_template_budget_is_raised_to_the_task_minimum() {
        let input = [serde_json::json!({
            "task_id":"small", "worker":"w1", "task":"small", "acceptance":"done",
            "write_paths":[], "estimated_effort":1
        })];
        let mut assignments = validate_parallel_subtasks(&roles(), &input, false)
            .unwrap()
            .tasks;
        let budgets = std::collections::BTreeMap::from([("w1".to_string(), 3)]);
        assign_task_model_call_budgets(&mut assignments, &budgets)
            .expect("a low template suggestion must be raised, not fail dispatch");
        assert_eq!(assignments[0].model_calls_per_attempt, 8);
    }

    #[test]
    fn acceptance_arrays_are_normalized_to_verifiable_text() {
        let input = [serde_json::json!({
            "task_id": "world",
            "worker": "w1",
            "task": "implement world",
            "acceptance": [
                {"id": "a1", "criterion": "node --test passes"},
                "world renders"
            ],
            "write_paths": [],
            "estimated_effort": 1,
            "verification": "non_empty",
            "risk": "normal",
            "priority": 80,
        })];
        let graph = validate_parallel_subtasks(&roles(), &input, false)
            .expect("string/object acceptance arrays must validate");
        assert!(graph.tasks[0].acceptance.contains("node --test passes"));
        assert!(graph.tasks[0].acceptance.contains("world renders"));
    }

    #[test]
    fn task_description_aliases_and_optional_refs_are_accepted() {
        let input = [serde_json::json!({
            "task_id": "world",
            "worker": "w1",
            "deliverable": "implement world",
            "acceptance": "node --test passes",
            "depends_on": [],
            "write_paths": [],
            "required_capabilities": [],
            "estimated_effort": 1,
            "verification": "non_empty",
            "risk": "normal",
            "priority": 80,
        })];
        let graph = validate_parallel_subtasks(&roles(), &input, true)
            .expect("deliverable alias and omitted optional refs must validate");
        assert_eq!(graph.tasks[0].task, "implement world");
        assert!(graph.tasks[0].read_refs.is_empty());
        assert!(graph.tasks[0].contract_refs.is_empty());
    }

    #[test]
    fn host_manifest_replaces_only_optional_parallel_integration_roles() {
        for role in ["leader", "project_integrator"] {
            assert!(host_manifest_can_replace_integration(true, role, false));
            assert!(!host_manifest_can_replace_integration(true, role, true));
            assert!(!host_manifest_can_replace_integration(false, role, false));
        }
        for role in ["reviewer", "implementer"] {
            assert!(!host_manifest_can_replace_integration(true, role, false));
        }
    }

    #[test]
    fn host_manifest_skips_leader_when_dynamic_tasks_need_no_model_integration() {
        let independent = vec![
            assigned_step(
                "step-a",
                "task-a",
                serde_json::json!({"assigned_write_paths":["src/a.rs"]}),
            ),
            assigned_step(
                "step-b",
                "task-b",
                serde_json::json!({"assigned_write_paths":["src/b.rs"]}),
            ),
        ];
        assert!(!parallel_tasks_require_integration(&independent));

        let dependent = vec![assigned_step("step-a", "task-a", serde_json::json!({})), {
            let mut step = assigned_step("step-b", "task-b", serde_json::json!({}));
            step.depends_on.push("step-a".to_string());
            step
        }];
        assert!(parallel_tasks_require_integration(&dependent));

        let scoped_dependency = vec![
            assigned_step(
                "step-a",
                "task-a",
                serde_json::json!({"assigned_write_paths":["src/a.rs"]}),
            ),
            {
                let mut step = assigned_step(
                    "step-b",
                    "task-b",
                    serde_json::json!({"assigned_write_paths":["src/b.rs"]}),
                );
                step.depends_on.push("step-a".to_string());
                step
            },
        ];
        assert!(
            !parallel_tasks_require_integration(&scoped_dependency),
            "依赖产物不应单独触发串行集成步骤；最终源码仍由宿主门禁验收"
        );

        let overlapping_writes = vec![
            assigned_step(
                "step-a",
                "task-a",
                serde_json::json!({"assigned_write_paths":["src"]}),
            ),
            assigned_step(
                "step-b",
                "task-b",
                serde_json::json!({"assigned_write_paths":["src/lib.rs"]}),
            ),
        ];
        assert!(parallel_tasks_require_integration(&overlapping_writes));

        let shared_contract = vec![
            assigned_step(
                "step-a",
                "task-a",
                serde_json::json!({"assigned_contract_refs":["api-v1"]}),
            ),
            assigned_step(
                "step-b",
                "task-b",
                serde_json::json!({"assigned_contract_refs":["api-v1"]}),
            ),
        ];
        assert!(parallel_tasks_require_integration(&shared_contract));
        assert!(parallel_tasks_require_integration(&[]));
    }

    #[test]
    fn resolved_template_lead_enables_dynamic_assignment_without_request_flag() {
        assert!(should_enable_parallel_assignment(
            false,
            &[RoleSpec::agent("lead"), RoleSpec::agent("w1")]
        ));
        assert!(should_enable_parallel_assignment(
            true,
            &[RoleSpec::agent("implementer")]
        ));
        assert!(!should_enable_parallel_assignment(
            false,
            &[RoleSpec::agent("implementer")]
        ));
    }

    #[test]
    fn custom_named_task_workers_can_receive_declared_media_tools() {
        const MEDIA_TOOL: &str = "mcp__media__video_generate";
        let mut director = RoleSpec::agent("短片总导演");
        director.capabilities = vec!["orchestrator".to_string()];
        let mut video_worker = RoleSpec::agent("镜头生成师");
        video_worker.capabilities = vec!["task_worker".to_string()];
        video_worker.tools = vec![MEDIA_TOOL.to_string()];
        let mut caption_worker = RoleSpec::agent("字幕制作人");
        caption_worker.capabilities = vec!["task_worker".to_string()];
        caption_worker.tools = vec!["read_file".to_string()];
        let roles = vec![director, video_worker, caption_worker];

        assert!(should_enable_parallel_assignment(false, &roles));
        let graph = validate_parallel_subtasks(
            &roles,
            &[serde_json::json!({
                "task_id": "generate-shot",
                "task": "按分镜生成视频片段",
                "acceptance": "视频片段已生成",
                "write_paths": [],
                "required_capabilities": [MEDIA_TOOL]
            })],
            false,
        )
        .expect("custom media tool is declared by a user-named task worker");
        assert_eq!(graph.tasks[0].worker, "镜头生成师");

        let wrong_worker = validate_parallel_subtasks(
            &roles,
            &[serde_json::json!({
                "task_id": "misrouted-shot",
                "worker": "字幕制作人",
                "task": "生成视频片段",
                "acceptance": "视频片段已生成",
                "write_paths": [],
                "required_capabilities": [MEDIA_TOOL]
            })],
            false,
        )
        .expect_err("an explicit worker without the declared tool must be rejected");
        assert!(wrong_worker.contains("outside that node's declared tool surface"));
    }

    #[test]
    fn implicit_worker_assignment_matches_disjoint_declared_write_paths() {
        let mut orchestrator = RoleSpec::agent("workflow-director");
        orchestrator.capabilities = vec!["orchestrator".into()];
        let mut docs_a = RoleSpec::agent("文档分支");
        docs_a.capabilities = vec!["task_worker".into()];
        docs_a.tools = vec!["write_file".into()];
        docs_a.write_paths = vec!["docs/a".into()];
        let mut docs_b = RoleSpec::agent("游戏分支");
        docs_b.capabilities = vec!["task_worker".into()];
        docs_b.tools = vec!["write_file".into()];
        docs_b.write_paths = vec!["docs/b".into()];
        let roles = vec![orchestrator, docs_a, docs_b];

        let graph = validate_parallel_subtasks(
            &roles,
            &[serde_json::json!({
                "task_id":"write-game-notes",
                "task":"写游戏分支说明",
                "acceptance":"docs/b 已写入",
                "write_paths":["docs/b"],
                "required_capabilities":["write_file"]
            })],
            false,
        )
        .expect("implicit assignment must skip a same-tool worker with the wrong path scope");
        assert_eq!(graph.tasks[0].worker, "游戏分支");
    }

    #[test]
    fn builtin_media_task_requires_its_outputs_write_scope() {
        let mut director = RoleSpec::agent("视觉总控");
        director.capabilities = vec!["orchestrator".into()];
        let mut artist = RoleSpec::agent("图像生成");
        artist.capabilities = vec!["task_worker".into()];
        artist.tools = vec!["generate_image".into()];
        artist.write_paths = vec!["outputs".into()];
        let roles = vec![director, artist];
        let base = serde_json::json!({
            "task_id":"cover",
            "worker":"图像生成",
            "task":"生成封面图",
            "acceptance":"封面图已生成",
            "required_capabilities":["generate_image"]
        });

        let mut missing_scope = base.clone();
        missing_scope["write_paths"] = serde_json::json!([]);
        assert!(validate_parallel_subtasks(&roles, &[missing_scope], false)
            .unwrap_err()
            .contains("writes workspace/outputs"));

        let mut scoped = base;
        scoped["write_paths"] = serde_json::json!(["outputs"]);
        let graph = validate_parallel_subtasks(&roles, &[scoped], false)
            .expect("media task may write only after declaring outputs");
        assert_eq!(graph.tasks[0].effective_paths, vec!["outputs"]);
    }

    #[test]
    fn integration_waits_for_every_task_even_when_one_worker_has_multiple_tasks() {
        let contract_prep = StepSpec::new("s-contract-prep", "m-contract-prep");
        let coordinator = StepSpec::new("s-coordinator", "m-coordinator");
        let review_brief = StepSpec::new("s-review-brief", "m-review-brief");
        let mut integrator = StepSpec::new("s-project_integrator", "m-project_integrator");
        integrator.depends_on.push("s-contract-prep".to_string());
        integrator.depends_on.push("s-w3".to_string());
        let mut leader = StepSpec::new("s-leader", "m-leader");
        leader.depends_on.push("s-coordinator".to_string());
        let mut reviewer = StepSpec::new("s-reviewer", "m-reviewer");
        reviewer.depends_on.push("s-review-brief".to_string());
        let mut steps = vec![
            contract_prep,
            coordinator,
            review_brief,
            integrator,
            leader,
            reviewer,
        ];
        let task_ids = vec![
            "s-w1".to_string(),
            "s-task-c".to_string(),
            "s-w2".to_string(),
        ];
        let reviewer_ids = vec!["s-reviewer".to_string()];
        bind_dynamic_follow_up_dependencies(&mut steps, &task_ids, &reviewer_ids);

        assert_eq!(
            steps[3].depends_on,
            vec![
                "s-contract-prep".to_string(),
                "s-task-c".to_string(),
                "s-w1".to_string(),
                "s-w2".to_string(),
            ]
        );
        assert!(
            !steps[3]
                .depends_on
                .iter()
                .any(|dependency| dependency == "s-w3"),
            "removed, unassigned writer slots must not remain as dangling prerequisites"
        );
        assert_eq!(
            steps[4].depends_on,
            vec![
                "s-coordinator".to_string(),
                "s-task-c".to_string(),
                "s-w1".to_string(),
                "s-w2".to_string(),
            ]
        );
        assert_eq!(
            steps[5].depends_on,
            vec![
                "s-leader".to_string(),
                "s-project_integrator".to_string(),
                "s-review-brief".to_string(),
                "s-task-c".to_string(),
                "s-w1".to_string(),
                "s-w2".to_string(),
            ]
        );
    }

    #[test]
    fn taskgraph_accepts_only_registered_workspace_plans_within_write_scope() {
        let make_task = |scope_path: &str, validator_id: &str| {
            serde_json::json!({
                "task_id":"verified-file",
                "worker":"w1",
                "task":"write the module",
                "acceptance":"the module contains the expected marker",
                "write_paths":["src"],
                "verification": {
                    "plan_id":"model-controlled-id-is-replaced",
                    "requirements":[{
                        "requirement_id":"model-controlled-id-is-replaced",
                        "validator_id":validator_id,
                        "validator_version":"1",
                        "scope":{"kind":"workspace_paths","relative_paths":[scope_path]},
                        "arguments":{"text":"pub fn ready"},
                        "required":true,
                        "resources":{"cpu_slots":1,"memory_mb":16,"exclusive_workspace":false,"timeout_ms":3000}
                    }]
                }
            })
        };
        let valid = validate_parallel_subtasks(
            &roles(),
            &[make_task("src/lib.rs", "workspace-file-contains-v1")],
            false,
        )
        .unwrap();
        let plan = valid.tasks[0].verification_plan.as_ref().unwrap();
        assert_eq!(plan.plan_id, "verify-verified-file");
        assert_eq!(
            plan.requirements[0].requirement_id,
            "verified-file:requirement:0"
        );
        assert!(validate_parallel_subtasks(
            &roles(),
            &[make_task("../outside.rs", "workspace-file-contains-v1")],
            false,
        )
        .is_err());
        assert!(validate_parallel_subtasks(
            &roles(),
            &[make_task("src/lib.rs", "shell-command-v1")],
            false,
        )
        .is_err());
    }

    #[test]
    fn declared_source_code_scope_requires_registered_behavior_command() {
        let mut task = serde_json::json!({
            "task_id":"source-file",
            "worker":"w1",
            "task":"implement the module",
            "acceptance":"module behavior passes its tests",
            "write_paths":["src/lib.rs"],
            "required_capabilities":["write_file"],
            "verification": {
                "plan_id":"source-plan",
                "requirements":[{
                    "requirement_id":"source-file-ready",
                    "validator_id":"workspace-file-non-empty-v1",
                    "validator_version":"1",
                    "scope":{"kind":"workspace_paths","relative_paths":["src/lib.rs"]},
                    "arguments":{},
                    "required":true,
                    "resources":{"cpu_slots":1,"memory_mb":8,"exclusive_workspace":false,"timeout_ms":5000}
                }]
            }
        });
        let error = validate_parallel_subtasks(&roles(), &[task.clone()], false).unwrap_err();
        assert!(error.contains("source-code write scope"), "{error}");

        task["required_capabilities"] = serde_json::json!(["write_file", "run_command"]);
        task["verification"]["requirements"].as_array_mut().unwrap().push(serde_json::json!({
            "requirement_id":"source-behavior",
            "validator_id":"workspace-command-success-v1",
            "validator_version":"1",
            "scope":{"kind":"workspace_paths","relative_paths":["src/lib.rs"]},
            "arguments":{"command":"cargo test -p owo-agent-core"},
            "required":true,
            "resources":{"cpu_slots":1,"memory_mb":8,"exclusive_workspace":false,"timeout_ms":30000}
        }));
        assert!(validate_parallel_subtasks(&roles(), &[task], false).is_ok());
    }

    #[test]
    fn supports_more_tasks_than_workers_and_validates_identity_and_acceptance() {
        let tasks = vec![
            serde_json::json!({"task_id":"a", "worker":"w1", "task":"one", "acceptance":"done", "write_paths":["src/a"]}),
            serde_json::json!({"task_id":"b", "worker":"w2", "task":"two", "acceptance":"done", "write_paths":["src/b"]}),
            serde_json::json!({"task_id":"c", "worker":"w1", "task":"three", "depends_on":["a"], "acceptance":"done", "write_paths":["src/c"]}),
        ];
        let validated = validate_parallel_subtasks(&roles(), &tasks, false).unwrap();
        assert_eq!(validated.tasks.len(), 3);
        let task_c = validated
            .tasks
            .iter()
            .find(|task| task.task_id == "c")
            .expect("dependent task c is retained");
        assert_eq!(task_c.worker, "w1");

        let duplicate_id = vec![
            serde_json::json!({"task_id":"a", "worker":"w1", "task":"one", "acceptance":"done", "write_paths":[]}),
            serde_json::json!({"task_id":"a", "worker":"w2", "task":"two", "acceptance":"done", "write_paths":[]}),
        ];
        assert!(validate_parallel_subtasks(&roles(), &duplicate_id, false)
            .unwrap_err()
            .contains("duplicate"));
        let empty_acceptance = vec![serde_json::json!({
            "worker":"w1", "task":"one", "acceptance":" ", "write_paths":[]
        })];
        assert!(
            validate_parallel_subtasks(&roles(), &empty_acceptance, false)
                .unwrap_err()
                .contains("acceptance")
        );
    }

    #[test]
    fn implicit_worker_assignment_balances_estimated_effort() {
        let tasks = vec![
            serde_json::json!({"task_id":"a", "task":"large", "acceptance":"done", "write_paths":[], "estimated_effort":5}),
            serde_json::json!({"task_id":"b", "task":"small", "acceptance":"done", "write_paths":[], "estimated_effort":1}),
            serde_json::json!({"task_id":"c", "task":"large", "acceptance":"done", "write_paths":[], "estimated_effort":4}),
            serde_json::json!({"task_id":"d", "task":"small", "acceptance":"done", "write_paths":[], "estimated_effort":2}),
        ];
        let validated = validate_parallel_subtasks(&roles(), &tasks, false).unwrap();
        let worker_for = |task_id: &str| {
            validated
                .tasks
                .iter()
                .find(|task| task.task_id == task_id)
                .unwrap()
                .worker
                .as_str()
        };
        assert_eq!(worker_for("a"), "w1");
        assert_eq!(worker_for("b"), "w1");
        assert_eq!(worker_for("c"), "w2");
        assert_eq!(worker_for("d"), "w2");
    }

    #[test]
    fn implicit_assignment_accounts_for_explicit_load_before_input_order() {
        let tasks = vec![
            serde_json::json!({"task_id":"implicit-large", "task":"large", "acceptance":"done", "write_paths":[], "estimated_effort":6}),
            serde_json::json!({"task_id":"explicit", "worker":"w1", "task":"reserved", "acceptance":"done", "write_paths":[], "estimated_effort":8}),
            serde_json::json!({"task_id":"implicit-small", "task":"small", "acceptance":"done", "write_paths":[], "estimated_effort":5}),
        ];
        let validated = validate_parallel_subtasks(&roles(), &tasks, false).unwrap();
        let worker_for = |task_id: &str| {
            validated
                .tasks
                .iter()
                .find(|task| task.task_id == task_id)
                .unwrap()
                .worker
                .as_str()
        };
        assert_eq!(worker_for("explicit"), "w1");
        assert_eq!(worker_for("implicit-large"), "w2");
        assert_eq!(worker_for("implicit-small"), "w2");
    }

    #[test]
    fn ready_task_order_prioritizes_estimated_critical_path() {
        let tasks = vec![
            serde_json::json!({"task_id":"a", "worker":"w1", "task":"path start", "acceptance":"done", "write_paths":[], "estimated_effort":2, "priority":10}),
            serde_json::json!({"task_id":"a2", "worker":"w1", "task":"path continuation", "depends_on":["a"], "acceptance":"done", "write_paths":[], "estimated_effort":10, "priority":10}),
            serde_json::json!({"task_id":"b", "worker":"w2", "task":"urgent short path", "acceptance":"done", "write_paths":[], "estimated_effort":9, "priority":100}),
        ];
        let validated = validate_parallel_subtasks(&roles(), &tasks, false).unwrap();
        assert_eq!(validated.tasks[0].task_id, "a");
        assert_eq!(validated.tasks[1].task_id, "a2");
        assert_eq!(validated.tasks[2].task_id, "b");
    }

    #[test]
    fn rejects_missing_dependencies_cycles_and_unordered_write_conflicts() {
        let missing_dep = vec![serde_json::json!({
            "task_id":"a", "worker":"w1", "task":"one", "depends_on":["ghost"], "acceptance":"done", "write_paths":[]
        })];
        assert!(validate_parallel_subtasks(&roles(), &missing_dep, false)
            .unwrap_err()
            .contains("dependency"));
        let cycle = vec![
            serde_json::json!({"task_id":"a", "worker":"w1", "task":"one", "depends_on":["b"], "acceptance":"done", "write_paths":[]}),
            serde_json::json!({"task_id":"b", "worker":"w2", "task":"two", "depends_on":["a"], "acceptance":"done", "write_paths":[]}),
        ];
        assert!(validate_parallel_subtasks(&roles(), &cycle, false)
            .unwrap_err()
            .contains("cycle"));
        let conflicting = vec![
            serde_json::json!({"task_id":"a", "worker":"w1", "task":"one", "acceptance":"done", "write_paths":["src"]}),
            serde_json::json!({"task_id":"b", "worker":"w2", "task":"two", "acceptance":"done", "write_paths":["src/b"]}),
        ];
        assert!(validate_parallel_subtasks(&roles(), &conflicting, false)
            .unwrap_err()
            .contains("overlap"));
    }

    #[test]
    fn preauthorized_task_paths_normalize_separators_without_prefix_escape() {
        let mut roles = roles();
        roles[1].write_paths = vec![r"docs\user".to_string()];
        let valid = vec![serde_json::json!({
            "task_id":"nested", "worker":"w1", "task":"edit nested file",
            "acceptance":"done", "write_paths":["docs/user/file.md"]
        })];
        let valid_result = validate_parallel_subtasks(&roles, &valid, false);
        assert!(
            valid_result.is_ok(),
            "valid pre-authorized path rejected: {valid_result:?}"
        );

        let prefix_escape = vec![serde_json::json!({
            "task_id":"outside", "worker":"w1", "task":"edit sibling",
            "acceptance":"done", "write_paths":["docs/user-old/file.md"]
        })];
        assert!(validate_parallel_subtasks(&roles, &prefix_escape, false)
            .unwrap_err()
            .contains("pre-authorized"));

        let root_write = vec![serde_json::json!({
            "task_id":"root", "worker":"w1", "task":"edit repository",
            "acceptance":"done", "write_paths":["."]
        })];
        assert!(validate_parallel_subtasks(&roles, &root_write, false)
            .unwrap_err()
            .contains("workspace root"));
    }

    #[test]
    fn write_scope_comparison_normalizes_windows_paths_and_workspace_root() {
        assert!(super::write_paths_overlap(r"src\a", "src/a/b"));
        assert!(super::write_paths_overlap(".", "src/a"));
        assert!(super::write_path_is_within("src/a.rs", r"src\"));
        assert!(!super::write_path_is_within("src-old/a.rs", "src"));
    }

    #[test]
    fn high_risk_tasks_require_a_separate_reviewer_role() {
        let task = serde_json::json!({
            "task_id":"sensitive-change",
            "worker":"w1",
            "task":"change authentication boundary",
            "depends_on":[],
            "read_refs":[],
            "write_paths":["src/auth"],
            "contract_refs":[],
            "required_capabilities":["write_file"],
            "estimated_effort":4,
            "verification":"non_empty",
            "risk":"critical",
            "priority":90,
            "acceptance":"authentication behavior is verified"
        });
        let error =
            validate_parallel_subtasks(&roles(), std::slice::from_ref(&task), true).unwrap_err();
        assert!(error.contains("independent reviewer"));

        let mut roles_with_reviewer = roles();
        let mut reviewer = RoleSpec::agent("reviewer");
        reviewer.depends_on = vec!["lead".to_string()];
        roles_with_reviewer.push(reviewer);
        assert!(validate_parallel_subtasks(&roles_with_reviewer, &[task], true).is_ok());
    }

    #[test]
    fn versioned_graph_requires_stable_task_ids_and_metadata() {
        let task = serde_json::json!({
            "worker":"w1", "task":"implement", "acceptance":"done", "depends_on":[],
            "read_refs":[], "write_paths":[], "contract_refs":[], "required_capabilities":[],
            "estimated_effort":1, "verification":"non_empty", "risk":"normal", "priority":50
        });
        let error =
            validate_parallel_subtasks(&roles(), std::slice::from_ref(&task), true).unwrap_err();
        assert!(error.contains("task_id"));
        let mut task = task;
        task["task_id"] = serde_json::json!("readonly-task");
        let mut missing_dependencies = task.clone();
        missing_dependencies["write_paths"] = serde_json::json!(["src/a"]);
        missing_dependencies
            .as_object_mut()
            .unwrap()
            .remove("depends_on");
        let error =
            validate_parallel_subtasks(&roles(), &[missing_dependencies], true).unwrap_err();
        assert!(error.contains("depends_on"));

        for capability in ["write_file", "apply_patch"] {
            task["required_capabilities"] = serde_json::json!([capability]);
            let error = validate_parallel_subtasks(&roles(), &[task.clone()], true).unwrap_err();
            assert!(error.contains("no write_paths"));
        }

        task["write_paths"] = serde_json::json!(["src/a"]);
        task["verification"] = serde_json::json!("run:cargo test");
        let error = validate_parallel_subtasks(&roles(), &[task], true).unwrap_err();
        assert!(error.contains("verification"));
    }

    #[test]
    fn rejects_overlapping_writer_paths_including_parent_child() {
        let overlapping = vec![
            serde_json::json!({"worker": "w1", "task": "one", "acceptance": "done", "write_paths": ["src"]}),
            serde_json::json!({"worker": "w2", "task": "two", "acceptance": "done", "write_paths": ["src/b"]}),
        ];
        let error = validate_parallel_subtasks(&roles(), &overlapping, false).unwrap_err();
        assert!(error.contains("overlap"), "{error}");
    }
}
