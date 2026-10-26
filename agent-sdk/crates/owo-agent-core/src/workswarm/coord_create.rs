//! Team creation, role topology and resolved strategy metadata.
use super::task_graph::should_enable_parallel_assignment;
use super::task_profile::derive_task_profile;
use super::*;

use super::coord_strategy::{
    align_dynamic_role_specs, bind_auto_reviewer_strategy_budget, bind_missing_strategy_budgets,
    build_auto_reviewer_for_model_writers, resolve_strategy_selection, should_add_auto_reviewer,
    should_trim_to_single, validate_team_budget_config,
};
#[cfg(test)]
use super::coord_strategy::{build_auto_reviewer_role, is_auto_reviewer_role};
#[cfg(test)]
use super::task_graph::is_independent_reviewer_role;

fn validate_parent_context_snapshot(snapshot: &str) -> WorkSwarmResult<()> {
    if snapshot.len() > super::coord_artifacts::MAX_PARENT_CONTEXT_SNAPSHOT_BYTES {
        return Err(WorkSwarmError::Validation(format!(
            "parent session context snapshot exceeds {} bytes",
            super::coord_artifacts::MAX_PARENT_CONTEXT_SNAPSHOT_BYTES
        )));
    }
    let value: Value = serde_json::from_str(snapshot).map_err(|error| {
        WorkSwarmError::Validation(format!("invalid parent session context JSON: {error}"))
    })?;
    if value.get("kind").and_then(Value::as_str) != Some("source_session_context_v1")
        || !value.get("core_spec").is_some_and(Value::is_object)
    {
        return Err(WorkSwarmError::Validation(
            "parent context must be source_session_context_v1 with an object core_spec".to_string(),
        ));
    }
    Ok(())
}

impl TeamCoordinator {
    /// 创建团队运行：成员/角色/assignee 绑定 + 任务图 + ProjectSpace + TeamRun。
    pub async fn create_team_run(&self, req: &CreateTeamRequest) -> WorkSwarmResult<TeamRun> {
        self.create_team_run_with_preflight(req, |_, _, _| Ok(()))
            .await
    }

    pub async fn create_team_run_with_preflight(
        &self,
        req: &CreateTeamRequest,
        preflight: impl Fn(&[RoleSpec], bool, Option<&str>) -> WorkSwarmResult<()>,
    ) -> WorkSwarmResult<TeamRun> {
        if let Some(snapshot) = req.parent_context_snapshot.as_deref() {
            validate_parent_context_snapshot(snapshot)?;
        }
        validate_team_budget_config(&req.budget).map_err(WorkSwarmError::Validation)?;
        let objective = req.objective.trim();
        if objective.is_empty() {
            return Err(WorkSwarmError::Validation("objective 不能为空".to_string()));
        }
        let selection = resolve_strategy_selection(req.mode, req.strategy)
            .map_err(WorkSwarmError::Validation)?;
        // 角色来源：显式 roles（可搭配显式模板记录来源）> 模板（指定/匹配）> 模式默认。
        // 保留模板是否由调用者显式选择；策略裁剪只能覆盖自动匹配模板，不能悄悄改写用户指定拓扑。
        let has_explicit_roles = !req.roles.is_empty();
        let has_explicit_template = req.mode != TeamMode::Single && req.template_id.is_some();
        // Read document + revision + hash together, before preflight or any async persistence.
        // Never re-read a newer head to describe roles resolved from an older revision.
        let source_revision = if req.mode == TeamMode::Single {
            None
        } else if let Some(id) = req.template_id.as_deref() {
            Some(
                self.templates
                    .get_revision(id)
                    .map_err(|error| WorkSwarmError::Run(error.to_string()))?
                    .filter(|record| record.template.is_some())
                    .ok_or_else(|| WorkSwarmError::NotFound(format!("模板 {id} 不存在")))?,
            )
        } else if has_explicit_roles {
            None
        } else {
            self.templates
                .find_match_revision(req.mode, objective)
                .map_err(|error| WorkSwarmError::Run(error.to_string()))?
        };
        if req.mode == TeamMode::Swarmflow && !has_explicit_roles && source_revision.is_none() {
            return Err(WorkSwarmError::Validation(
                "swarmflow 模式必须基于版本化模板（无匹配模板；请先提供 template_id 或 roles）"
                    .to_string(),
            ));
        }
        let template_source = source_revision
            .as_ref()
            .map(|record| {
                Ok::<_, WorkSwarmError>(RunTemplateSource {
                    template_id: record.template_id.clone(),
                    revision: record.revision,
                    content_hash: record.content_hash.clone().ok_or_else(|| {
                        WorkSwarmError::Run(format!(
                            "模板 {} 的修订缺少内容哈希",
                            record.template_id
                        ))
                    })?,
                    roles_overridden: has_explicit_roles,
                    requested_model: req.model.clone(),
                })
            })
            .transpose()?;
        let template_id = template_source
            .as_ref()
            .map(|source| source.template_id.clone());
        let mut roles: Vec<RoleSpec> = if has_explicit_roles {
            req.roles.clone()
        } else if let Some(template) = source_revision
            .as_ref()
            .and_then(|record| record.template.as_ref())
        {
            template.roles.iter().cloned().map(RoleSpec::from).collect()
        } else if req.mode == TeamMode::Single {
            vec![RoleSpec::agent("runner")]
        } else {
            default_relay_roles()
        };

        // Custom workflows are matched by the capabilities/tools they declare, not
        // by a built-in role-name allowlist. An effectful tool surface without an
        // explicit semantic capability becomes a task_worker; a display-only
        // `custom` marker is preserved but does not suppress inference.
        for role in &mut roles {
            role.capabilities =
                super::infer_role_capabilities(&role.role, &role.tools, &role.capabilities);
        }

        // 约束：角色唯一；Agent 成员 ≤ max_agent_members；写路径合法（权限默认 deny）。
        let mut seen = std::collections::HashSet::new();
        let mut agent_count = 0usize;
        for r in &roles {
            if r.role.trim().is_empty() {
                return Err(WorkSwarmError::Validation("角色名不能为空".to_string()));
            }
            if !seen.insert(r.role.clone()) {
                return Err(WorkSwarmError::Validation(format!("角色重复：{}", r.role)));
            }
            if r.assignee.is_empty() || r.assignee == "agent" {
                agent_count += 1;
            }
            super::roles::validate_role_write_paths_with_capabilities(
                &r.role,
                &r.capabilities,
                &r.write_paths,
            )
            .map_err(WorkSwarmError::Validation)?;
            super::roles::validate_role_tool_capabilities(
                &r.role,
                &r.capabilities,
                &r.tools,
                &r.write_paths,
            )
            .map_err(WorkSwarmError::Validation)?;
        }
        // §6.1: ad-hoc graphs and auto-assembled teams use the five-Agent default.
        // An explicitly selected registered template owns its declared membership
        // (the host already validated/imported that graph); callers may still lower
        // it with `max_agent_members`.
        let default_member_limit = if has_explicit_template && !has_explicit_roles {
            agent_count
        } else {
            self.max_agent_members
        };
        let member_limit = req.max_agent_members.unwrap_or(default_member_limit);
        if agent_count > member_limit {
            return Err(WorkSwarmError::Validation(format!(
                "动态团队 Agent 成员超过上限（{agent_count} > {member_limit}）"
            )));
        }
        // swarmflow 且显式给了非模板角色 → 仍允许（模板角色 + 补充），此处不额外限制。
        if roles.is_empty() {
            return Err(WorkSwarmError::Validation(
                "团队至少需要一个角色".to_string(),
            ));
        }

        let team_id = format!("team-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let project_id = format!("proj-{team_id}");
        let correlation_id = new_correlation_id();

        // 角色规格（worker 缺省：agent 角色 = "agent" 模型驱动；human = user_id）。
        let mut specs: Vec<RoleSpec> = roles
            .into_iter()
            .map(|mut r| {
                if r.assignee.is_empty() {
                    r.assignee = "agent".to_string();
                }
                if r.worker.is_none() && r.assignee == "agent" {
                    r.worker = Some("agent".to_string());
                }
                r
            })
            .collect();

        // 旧 API/模板按名称推导一次职责能力；之后授权与执行只读取 capability。
        for role in &mut specs {
            if role.capabilities.is_empty() && is_review_role_name(&role.role) {
                role.capabilities.push("review".to_string());
            }
        }

        // 五期：组队策略判定（auto 判定 / single / team 强制；缺省 auto）。
        // 默认不再盲目启用多 Agent：auto 判定为 single 时裁剪到单角色，
        // 角色数与模型调用量随之下降；判定理由随 strategy_decision 暴露给 UI。
        let engine = crate::team_strategy::TeamStrategyEngine::default();
        let profile = derive_task_profile(
            objective,
            &specs,
            req.parent_context_snapshot
                .as_deref()
                .is_some_and(|snapshot| !snapshot.trim().is_empty()),
            template_id.is_some() || !req.roles.is_empty(),
        );
        // 十期·四路：接入三路冻结的收益策略 gate——auto 判定先过
        // `gate_auto`（无证据/不达标/样本不足/过期/绑定不匹配/非预选组 → 默认
        // single，附理由）；显式 single/team 不被 gate 降级（decide_with_policy
        // 内部保证）。gate 理由随 strategy_decision 暴露给 UI。
        let (gate, gate_verdict, gate_evidence) = benefit_gate_for_runtime(
            template_id.as_deref(),
            objective,
            &self.run_dir,
            &specs,
            req.model.as_deref(),
        );
        let mut strategy_plan = engine.decide_with_policy(selection, &profile, Some(&gate));
        strategy_plan.reasons.insert(
            0,
            format!(
                "任务画像来自当前目标和角色声明：类别={}，预期交付角色={}，已知父会话上下文={}，独立评审={}，风险信号={:?}（未命中不代表安全），历史成功率=未知",
                profile.category.as_deref().unwrap_or("未分类"),
                profile.artifact_count,
                profile.input_count,
                profile.needs_independent_review,
                profile.risk,
            ),
        );
        let gate_reason = if gate.allow_team {
            format!("收益 gate 放行组队：{gate_evidence}")
        } else {
            format!("收益 gate 默认 single（{gate_evidence}）")
        };
        if !strategy_plan
            .reasons
            .iter()
            .any(|r| r.contains("收益 gate"))
        {
            strategy_plan.reasons.push(gate_reason.clone());
        }
        // 显式 ForceSingle 始终约束实际拓扑；Auto 的 single 判定裁剪默认/自动匹配角色，
        // 但保留调用方显式提交的角色图和模板编排。Swarmflow 只允许显式 ForceSingle 覆盖。
        let trim_to_single = should_trim_to_single(
            req.mode,
            selection,
            strategy_plan.is_single(),
            specs.len(),
            has_explicit_roles,
            has_explicit_template,
        );
        if trim_to_single {
            // 单 Agent 判定：**优先保留可交付的写角色**（builder/producer/writer/leader），
            // 而不是盲目保留首个角色——默认接力的首个是只读 planner，裁到它会让团队
            // "成功"却零产出（"草草了事"的根因）。研究类任务没有写角色时保留首个
            // （researcher 只读是正确语义）。
            let keep_index = specs
                .iter()
                .position(|spec| {
                    crate::worker_profile::WorkerProfile::for_role(&spec.role, 0).is_writer()
                })
                .unwrap_or(0);
            let mut kept = specs.remove(keep_index);
            // 上游角色已被裁掉：清空依赖（保持 DAG 可拓扑排序）。
            kept.depends_on.clear();
            strategy_plan.reasons.push(format!(
                "判定单 Agent：保留可交付角色 {}（已裁剪 {} 个附加角色，上游依赖解除）",
                kept.role,
                specs.len()
            ));
            specs = vec![kept];
        }

        // 八期一路：模板级自适应角色策略——简单任务自动减少 Worker（创建期裁剪）。
        // 仅作用于「角色来自模板」（req.roles 为空）且多角色团队——用户显式编排
        // 始终尊重（与 trim_to_single 同口径），其 reviewer 由运行期无变更跳过兜底；
        // 被跳过角色的依赖重定向到其上游（保持 DAG 可拓扑排序）；跳过名单与节省
        // 预算进 strategy_decision.adaptive + 审计。
        let mut budget_map: BTreeMap<String, usize> = template_id
            .as_deref()
            .and_then(crate::builtin_team_templates::descriptor)
            .map(|d| {
                d.budget_calls_per_role
                    .iter()
                    .map(|rb| (rb.role.clone(), rb.budget_calls))
                    .collect()
            })
            .unwrap_or_default();
        // Installed/user templates may carry explicit per-role budgets. The
        // template value is still only a suggestion; runtime task allocation
        // and the team-wide max_model_calls remain authoritative.
        if let Some(template) = source_revision
            .as_ref()
            .and_then(|record| record.template.as_ref())
        {
            for (role, calls) in &template.budget_calls_per_role {
                budget_map.insert(role.clone(), *calls);
            }
        }
        for spec in &specs {
            budget_map.entry(spec.role.clone()).or_insert_with(|| {
                if spec.is_orchestrator() {
                    16
                } else {
                    32
                }
            });
        }
        bind_missing_strategy_budgets(&strategy_plan.roles, &specs, &mut budget_map);
        let mut adaptive_skips: Vec<crate::team_strategy::SkippedRole> = Vec::new();
        let mut adaptive_saved_calls = 0usize;
        // 角色来自模板（包括用户显式选用内置/已安装模板）时允许宿主自适应裁剪；
        // 只有调用方显式提交的角色图必须原样尊重。未知模板不会命中具体模板策略，
        // 因此这里不会因为 template_id 存在就跳过模板级优化。
        if specs.len() > 1 && !has_explicit_roles {
            let role_names: Vec<String> = specs.iter().map(|s| s.role.clone()).collect();
            let adaptive = crate::team_strategy::plan_adaptive_roles(
                template_id.as_deref(),
                &role_names,
                &budget_map,
                &profile,
            );
            if !adaptive.skipped.is_empty() {
                for skip in &adaptive.skipped {
                    let deps_of_skip = specs
                        .iter()
                        .find(|s| s.role == skip.role)
                        .map(|s| s.depends_on.clone())
                        .unwrap_or_default();
                    // 指向被跳过角色的依赖 → 重定向到该角色的上游（去重保序）。
                    for s in &mut specs {
                        if s.role == skip.role || !s.depends_on.iter().any(|d| d == &skip.role) {
                            continue;
                        }
                        let mut rewritten: Vec<String> = Vec::new();
                        for d in &s.depends_on {
                            if d == &skip.role {
                                for up in &deps_of_skip {
                                    if !rewritten.contains(up) {
                                        rewritten.push(up.clone());
                                    }
                                }
                            } else if !rewritten.contains(d) {
                                rewritten.push(d.clone());
                            }
                        }
                        s.depends_on = rewritten;
                    }
                    specs.retain(|s| s.role != skip.role);
                    strategy_plan.reasons.push(format!(
                        "自适应裁剪：跳过角色 {}（{}）",
                        skip.role, skip.reason
                    ));
                    adaptive_skips.push(skip.clone());
                }
                adaptive_saved_calls = adaptive.saved_budget_calls;
                strategy_plan.budget_calls_total = strategy_plan
                    .budget_calls_total
                    .saturating_sub(adaptive.saved_budget_calls);
            }
        }
        if selection == crate::team_strategy::TeamSelectionMode::ForceTeam
            && req.mode == TeamMode::Team
            && req.roles.is_empty()
            && template_id.is_none()
        {
            let (dynamic_skips, dynamic_saved_calls) =
                align_dynamic_role_specs(&mut specs, &strategy_plan.roles, &mut budget_map);
            if !dynamic_skips.is_empty() {
                adaptive_skips.extend(dynamic_skips);
                adaptive_saved_calls = adaptive_saved_calls.saturating_add(dynamic_saved_calls);
                strategy_plan.reasons.push(
                    "动态 Team 默认图已收敛到策略计划角色；移除 planner/leader 等未计划串行阶段，并将依赖桥接到最近的保留上游".to_string(),
                );
            }
        }
        let mut auto_reviewer_roles = Vec::new();
        // Only model-backed writers can use the automatically assembled model reviewer.
        // Read-only teams and custom/human workers keep their declared topology; custom
        // source producers must declare their reviewer, and DeliveryGate still fails
        // closed for any source candidate without independent review evidence.
        let auto_reviewer_allowed = !has_explicit_roles
            && !has_explicit_template
            && should_add_auto_reviewer(req.mode, strategy_plan.is_single());
        if auto_reviewer_allowed {
            if let Some(reviewer) = build_auto_reviewer_for_model_writers(&specs) {
                let member_limit = req.max_agent_members.unwrap_or(self.max_agent_members);
                let agent_count = specs.iter().filter(|spec| spec.assignee == "agent").count();
                if agent_count.saturating_add(1) > member_limit {
                    return Err(WorkSwarmError::Validation(format!(
                        "源码交付需要独立评审，但 Agent 成员上限不足（需要 {}，上限 {}）",
                        agent_count.saturating_add(1),
                        member_limit
                    )));
                }
                bind_auto_reviewer_strategy_budget(
                    &mut strategy_plan,
                    &mut budget_map,
                    &reviewer.role,
                );
                strategy_plan.reasons.push(
                    "自动补入只读独立 Reviewer；仅在存在工作区变更或未解决 Issue 时运行"
                        .to_string(),
                );
                auto_reviewer_roles.push(reviewer.role.clone());
                specs.push(reviewer);
            }
        }
        let mut strategy_decision = serde_json::to_value(&strategy_plan).ok();
        if let Some(obj) = strategy_decision.as_mut().and_then(Value::as_object_mut) {
            // 八期一路 additive：自适应指标（skipped_roles/skip_reason/saved_budget_calls/
            // context_bytes/提前结束原因）。运行期事件由 note_adaptive_event 追加。
            obj.insert(
                "adaptive".to_string(),
                json!({
                    "skipped_roles": adaptive_skips
                        .iter()
                        .map(|s| json!({"role": s.role, "reason": s.reason}))
                        .collect::<Vec<_>>(),
                    "saved_budget_calls": adaptive_saved_calls,
                    "auto_independent_reviewers": auto_reviewer_roles,
                    "context_bytes_total": 0,
                    "runtime_skipped": [],
                    "events": [],
                    "early_exit": Value::Null,
                }),
            );
            // 十期·四路：收益策略 gate 判定随 strategy_decision 暴露（UI/审计可追溯
            // 为什么 auto 走了 single——无证据/不达标等逐条理由）。
            obj.insert(
                "benefit_gate".to_string(),
                json!({
                    "allow_team": gate.allow_team,
                    "mandatory_review": gate.mandatory_review,
                    "reasons": gate.reasons,
                    "evidence": gate_verdict.as_ref().map(|v| json!({
                        "task_group": v.task_group,
                        "eligible": v.eligible,
                        "sample_sufficient": v.sample_sufficient,
                        "single_n": v.single_n,
                        "multi_n": v.multi_n,
                        "generated_at": v.generated_at,
                        "bindings": v.bindings,
                    })),
                    "note": gate_reason,
                }),
            );
        }
        let strategy_mode = strategy_plan.mode.clone();

        let parallel_assignment = should_enable_parallel_assignment(req.parallel, &specs);
        if parallel_assignment {
            for role in &mut specs {
                if role.is_orchestrator()
                    && !role
                        .handoff_contract
                        .as_deref()
                        .is_some_and(|contract| contract.contains("TaskGraphV1"))
                {
                    let existing = role.handoff_contract.take().unwrap_or_default();
                    role.handoff_contract = Some(if existing.trim().is_empty() {
                        super::task_graph::TASK_GRAPH_ORCHESTRATOR_CONTRACT.to_string()
                    } else {
                        format!(
                            "{existing}\n\n{}",
                            super::task_graph::TASK_GRAPH_ORCHESTRATOR_CONTRACT
                        )
                    });
                }
            }
        }

        let node_errors = crate::user_workflow::validate_node_ids(
            specs
                .iter()
                .map(|spec| (spec.role.as_str(), spec.node_id.as_deref())),
        );
        if !node_errors.is_empty() {
            return Err(WorkSwarmError::Validation(node_errors.join("; ")));
        }
        preflight(&specs, parallel_assignment, template_id.as_deref())?;

        // 成员（agent/human/worker 运行时绑定）。
        let mut members = Vec::new();
        for spec in &specs {
            let member_id = format!("m-{}", spec.role);
            let binding = match spec.assignee.as_str() {
                "human" => RuntimeBinding::Human {
                    user_id: spec.worker.clone().unwrap_or_else(|| "user".to_string()),
                },
                "worker" => RuntimeBinding::Worker {
                    worker_name: spec.worker.clone().unwrap_or_else(|| spec.role.clone()),
                },
                _ => RuntimeBinding::Agent {
                    agent_id: format!("{team_id}:{member_id}"),
                },
            };
            let read_only = spec.is_reviewer();
            self.bus.register(member_id.clone(), 64).await;
            // 写面声明（十一期）：角色级写路径优先；未声明沿用 "artifacts" 标记
            // （legacy wire 语义，运行期写面仍按工作区级处理）。
            let write_scope = if read_only {
                Vec::new()
            } else if !spec.write_paths.is_empty() {
                spec.write_paths.clone()
            } else {
                vec!["artifacts".to_string()]
            };
            members.push(TeamMember {
                member_id: member_id.clone(),
                role: spec.role.clone(),
                runtime_binding: binding,
                capabilities: if spec.capabilities.is_empty() {
                    vec![spec.role.clone()]
                } else {
                    spec.capabilities.clone()
                },
                tool_scope: if read_only {
                    vec!["read".to_string()]
                } else {
                    vec!["read".to_string(), "write".to_string()]
                },
                read_scope: vec!["project_space".to_string(), "artifacts".to_string()],
                write_scope,
                budget: Value::Null,
                handoff_contract: spec.handoff_contract.clone(),
                health: MemberHealth::Active,
            });
        }

        // 任务图（步骤 ↔ 成员 绑定；worker 名 = member_id，由 run 注册表按名派发角色 worker）。
        let mut plan = Plan::new(format!("{team_id}-plan"), team_id.clone());
        plan.description = format!("WorkSwarm 团队 {team_id}：{objective}");
        for spec in &specs {
            let step_id = format!("s-{}", spec.role);
            let member_id = format!("m-{}", spec.role);
            let mut input = spec.extra_input.clone();
            if !input.is_object() {
                input = json!({});
            }
            if let Some(obj) = input.as_object_mut() {
                obj.insert("objective".to_string(), json!(objective));
                obj.insert(
                    "_workswarm".to_string(),
                    json!({
                        "team_id": team_id,
                        "member_id": member_id,
                        "step_id": step_id,
                        "role_capabilities": spec.capabilities,
                        "role_is_integrator": spec.is_integrator(),
                    }),
                );
                // 十一期：模型解析——角色显式 model 优先，否则团队统一模型
                // （`req.model`，来自请求或 `<workspace>/settings.json` 的 team 段）；
                // `extra_input.model` 显式值优先于两者。
                let effective_model = spec
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .or_else(|| {
                        req.model
                            .as_deref()
                            .map(str::trim)
                            .filter(|m| !m.is_empty())
                    });
                let has_model = obj
                    .get("model")
                    .and_then(Value::as_str)
                    .map(|m| !m.trim().is_empty())
                    .unwrap_or(false);
                if !has_model {
                    if let Some(model) = effective_model {
                        obj.insert("model".to_string(), json!(model));
                    }
                }
            }
            // Every new role has a host-owned output obligation, including
            // optional legacy verify fields. Explicit/custom validators remain
            // unchanged; source changes still require behavioral/review evidence.
            let verification = parse_verify(spec.verify.as_deref().unwrap_or("non_empty"));
            let verification_plan = verification_plan_for_step(&step_id, &verification);
            let step = StepSpec {
                id: step_id.clone(),
                depends_on: spec.depends_on.iter().map(|d| format!("s-{d}")).collect(),
                parallel: true,
                worker: member_id,
                input,
                verify: Some(verification),
                verification_plan: Some(verification_plan),
                retries: 0,
            };
            plan.add_step(step);
        }
        plan.validate()
            .map_err(|e| WorkSwarmError::Validation(format!("任务图非法：{e}")))?;

        // 目标（预算映射到 GoalBudget）。
        let mut goal = Goal::new(team_id.clone(), objective.to_string());
        goal.budget = parse_goal_budget(&req.budget);
        let mut state = GoalRunState::new(goal, plan);
        state.run_id = team_id.clone();
        state.team_task_events = Some(super::coord_task_events::next_outbox(
            None, &state, &specs, 0, true,
        )?);
        state.persist(&self.run_dir).map_err(WorkSwarmError::Run)?;

        // ProjectSpace（统一事实空间：任务/产物/决策/交接/活动）。
        let now = now_ts();
        let space = ProjectSpace {
            project_id: project_id.clone(),
            goal_id: Some(team_id.clone()),
            team_id: Some(team_id.clone()),
            tasks: state.plan.steps.iter().map(|s| s.id.clone()).collect(),
            artifacts: Vec::new(),
            decisions: Vec::new(),
            approvals: Vec::new(),
            discussions: Vec::new(),
            activity_stream: vec![format!(
                "{now} team.created（mode={}，{} 个成员，来源={}）",
                match req.mode {
                    TeamMode::Single => "single",
                    TeamMode::Team => "team",
                    TeamMode::Swarmflow => "swarmflow",
                },
                members.len(),
                template_id
                    .as_deref()
                    .map(|t| format!("template:{t}"))
                    .unwrap_or_else(|| "dynamic".to_string()),
            )],
            delivery_manifest_ref: None,
            rework_tasks: Vec::new(),
            status: ProjectSpaceStatus::Active,
            version: 1,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        self.store.save_project_space(&space).await?;

        // P1：父会话 CoreSpec 以不可变 CAS 快照保存；Worker 仅通过引用读取。
        let mut shared_context_refs = Vec::new();
        if let Some(snapshot) = req.parent_context_snapshot.as_deref() {
            let hash = self
                .cas
                .put(snapshot.as_bytes())
                .map_err(WorkSwarmError::Run)?;
            shared_context_refs.push(format!("cas://sha256:{hash}"));
        }
        // TeamRun（Leader 产出的结构化协作计划）。
        let team = TeamRun {
            team_id: team_id.clone(),
            goal_id: req.goal_id.clone().or_else(|| Some(team_id.clone())),
            mode: req.mode,
            members: members.clone(),
            task_graph_ref: Some(format!("{team_id}-plan")),
            project_space_id: Some(project_id),
            template_id: template_id.clone(),
            shared_context_refs,
            budget: req.budget.clone(),
            human_policy: req.human_policy.clone(),
            strategy_decision,
            status: TeamRunStatus::Created,
            created_at: now.clone(),
            updated_at: now,
        };
        self.store.save_team_run(&team).await?;
        self.audit(
            &team_id,
            "team.strategy",
            format!(
                "组队策略：{}（selection={}，角色 {} 个，预算 {} 次调用）",
                strategy_mode,
                selection.as_str(),
                strategy_plan.roles.len(),
                strategy_plan.budget_calls_total
            ),
        );

        // Parallel assignment follows the resolved role capabilities.
        // 运行元数据（correlation + 角色规格 sidecar）。
        RunMeta {
            team_id: team_id.clone(),
            correlation_id,
            roles: specs,
            template_id: template_id.clone(),
            template_source,
            budgets: budget_map,
            parallel: parallel_assignment,
        }
        .save(&self.run_dir)?;

        if let Err(error) = self.relay_task_events(&team_id, 128) {
            self.record_runtime_event(&team_id, "team.task_delivery.pending", error.to_string());
        }
        // 八期一路：创建期自适应裁剪审计（跳过角色/节省预算，best-effort 可读性）。
        if !adaptive_skips.is_empty() {
            self.audit(
                &team_id,
                "team.adaptive_skip",
                format!(
                    "自适应裁剪 {} 个角色（节省预算 {} 次调用）：{}",
                    adaptive_skips.len(),
                    adaptive_saved_calls,
                    adaptive_skips
                        .iter()
                        .map(|s| s.role.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                ),
            );
        }

        self.audit(
            &team_id,
            "team.created",
            format!(
                "组队：mode={} 成员={} 来源={}（correlation 已建立）",
                match team.mode {
                    TeamMode::Single => "single",
                    TeamMode::Team => "team",
                    TeamMode::Swarmflow => "swarmflow",
                },
                members.len(),
                template_id
                    .clone()
                    .map(|t| format!("template:{t}"))
                    .unwrap_or_else(|| "dynamic".to_string()),
            ),
        );
        Ok(team)
    }
}

#[cfg(test)]
mod strategy_topology_policy_tests {
    use super::{resolve_strategy_selection, should_add_auto_reviewer, should_trim_to_single};
    use crate::team_strategy::TeamSelectionMode;
    use crate::workswarm::TeamMode;

    #[test]
    fn single_mode_resolves_to_one_worker_and_rejects_conflicting_team_selection() {
        assert_eq!(
            resolve_strategy_selection(TeamMode::Single, None).unwrap(),
            TeamSelectionMode::ForceSingle,
        );
        assert_eq!(
            resolve_strategy_selection(TeamMode::Single, Some(TeamSelectionMode::ForceSingle),)
                .unwrap(),
            TeamSelectionMode::ForceSingle,
        );
        assert!(
            resolve_strategy_selection(TeamMode::Single, Some(TeamSelectionMode::ForceTeam),)
                .unwrap_err()
                .contains("冲突")
        );
        assert_eq!(
            resolve_strategy_selection(TeamMode::Team, None).unwrap(),
            TeamSelectionMode::Auto,
        );
    }

    #[test]
    fn forced_single_overrides_an_explicit_template_but_auto_preserves_it() {
        assert!(should_trim_to_single(
            TeamMode::Team,
            TeamSelectionMode::ForceSingle,
            true,
            4,
            false,
            true,
        ));
        assert!(!should_trim_to_single(
            TeamMode::Team,
            TeamSelectionMode::Auto,
            true,
            4,
            false,
            true,
        ));
    }

    #[test]
    fn auto_single_trims_only_automatically_selected_topology() {
        assert!(should_trim_to_single(
            TeamMode::Team,
            TeamSelectionMode::Auto,
            true,
            4,
            false,
            false,
        ));
        assert!(!should_trim_to_single(
            TeamMode::Team,
            TeamSelectionMode::Auto,
            true,
            4,
            true,
            false,
        ));
        assert!(!should_trim_to_single(
            TeamMode::Swarmflow,
            TeamSelectionMode::Auto,
            true,
            4,
            false,
            false,
        ));
    }

    #[test]
    fn single_strategy_does_not_recreate_a_hidden_reviewer_worker() {
        assert!(!should_add_auto_reviewer(TeamMode::Single, false));
        assert!(!should_add_auto_reviewer(TeamMode::Team, true));
        assert!(should_add_auto_reviewer(TeamMode::Team, false));
    }
}

#[cfg(test)]
mod strategy_budget_binding_tests {
    use super::{bind_missing_strategy_budgets, RoleSpec};
    use crate::team_strategy::RolePlan;
    use std::collections::BTreeMap;

    #[test]
    fn dynamic_workers_receive_strategy_budgets_and_template_limits_win() {
        let strategy = vec![
            RolePlan {
                role: "producer".to_string(),
                duty: String::new(),
                budget_calls: 4,
            },
            RolePlan {
                role: "critic".to_string(),
                duty: String::new(),
                budget_calls: 2,
            },
            RolePlan {
                role: "leader".to_string(),
                duty: String::new(),
                budget_calls: 3,
            },
        ];
        let mut builder = RoleSpec::agent("builder");
        builder.capabilities.push("implementation".to_string());
        let specs = vec![
            builder,
            RoleSpec::agent("critic"),
            RoleSpec::agent("leader"),
        ];
        let mut budgets = BTreeMap::from([("builder".to_string(), 7)]);
        bind_missing_strategy_budgets(&strategy, &specs, &mut budgets);

        assert_eq!(budgets.get("builder"), Some(&7));
        assert_eq!(budgets.get("critic"), Some(&2));
        assert_eq!(budgets.get("leader"), Some(&3));
    }

    #[test]
    fn unknown_dynamic_role_uses_producer_ceiling_instead_of_worker_default() {
        let strategy = vec![RolePlan {
            role: "producer".to_string(),
            duty: String::new(),
            budget_calls: 4,
        }];
        let specs = vec![RoleSpec::agent("custom_builder")];
        let mut budgets = BTreeMap::new();
        bind_missing_strategy_budgets(&strategy, &specs, &mut budgets);
        assert_eq!(budgets.get("custom_builder"), Some(&4));
    }
}

#[cfg(test)]
mod dynamic_topology_alignment_tests {
    use super::{align_dynamic_role_specs, bind_missing_strategy_budgets};
    use crate::team_strategy::{RolePlan, TaskProfile, TeamSelectionMode, TeamStrategyEngine};
    use crate::workswarm::roles::default_relay_roles;
    use std::collections::BTreeMap;

    #[test]
    fn simple_forced_team_removes_unplanned_serial_stages_and_keeps_review_chain() {
        let strategy = TeamStrategyEngine::default().decide(
            TeamSelectionMode::ForceTeam,
            &TaskProfile {
                category: Some("code".to_string()),
                ..TaskProfile::default()
            },
        );
        let mut specs = default_relay_roles();
        let mut budgets = BTreeMap::new();
        bind_missing_strategy_budgets(&strategy.roles, &specs, &mut budgets);
        let (skipped, saved) = align_dynamic_role_specs(&mut specs, &strategy.roles, &mut budgets);

        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.role.as_str())
                .collect::<Vec<_>>(),
            vec!["builder", "critic"]
        );
        assert!(specs[0].depends_on.is_empty());
        assert_eq!(specs[1].depends_on, vec!["builder"]);
        // The strategy has no leader, so the skipped leader falls back to the
        // current worker default (24); planner keeps its producer budget (4).
        assert_eq!(saved, 28);
        assert_eq!(
            skipped
                .iter()
                .map(|role| role.role.as_str())
                .collect::<Vec<_>>(),
            vec!["planner", "leader"]
        );
        assert_eq!(budgets.get("builder"), Some(&4));
        assert_eq!(budgets.get("critic"), Some(&2));
    }

    #[test]
    fn dynamic_projection_bridges_dependencies_across_a_removed_review_stage() {
        let strategy_roles = vec![
            RolePlan {
                role: "builder".to_string(),
                duty: "produce".to_string(),
                budget_calls: 4,
            },
            RolePlan {
                role: "leader".to_string(),
                duty: "integrate".to_string(),
                budget_calls: 3,
            },
        ];
        let mut specs = default_relay_roles();
        let mut budgets = BTreeMap::from([
            ("builder".to_string(), 4),
            ("critic".to_string(), 2),
            ("leader".to_string(), 3),
            ("planner".to_string(), 4),
        ]);
        let (skipped, saved) = align_dynamic_role_specs(&mut specs, &strategy_roles, &mut budgets);
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.role.as_str())
                .collect::<Vec<_>>(),
            vec!["builder", "leader"]
        );
        assert_eq!(specs[1].depends_on, vec!["builder"]);
        assert_eq!(saved, 6);
        assert_eq!(skipped.len(), 2);
    }
}

#[cfg(test)]
mod automatic_reviewer_role_tests {
    use super::{
        build_auto_reviewer_for_model_writers, build_auto_reviewer_role, is_auto_reviewer_role,
    };
    use crate::workswarm::RoleSpec;

    #[test]
    fn read_only_model_team_keeps_declared_topology() {
        let mut researcher = RoleSpec::agent("researcher");
        researcher.worker = Some("agent".to_string());
        assert!(build_auto_reviewer_for_model_writers(&[researcher]).is_none());
    }

    #[test]
    fn custom_writer_does_not_silently_add_an_unavailable_model_worker() {
        let mut writer = RoleSpec::agent("builder");
        writer.worker = Some("custom-worker".to_string());
        writer.write_paths = vec!["src/main.rs".to_string()];
        assert!(build_auto_reviewer_for_model_writers(&[writer]).is_none());
    }

    #[test]
    fn model_writer_gets_independent_review_without_duplication() {
        let mut writer = RoleSpec::agent("builder");
        writer.worker = Some("agent".to_string());
        let reviewer = build_auto_reviewer_for_model_writers(&[writer.clone()]).unwrap();
        assert!(reviewer.is_reviewer());
        assert_eq!(reviewer.depends_on, vec!["builder"]);
        assert!(build_auto_reviewer_for_model_writers(&[writer, reviewer]).is_none());
    }

    #[test]
    fn creates_read_only_tail_reviewer_for_all_existing_roles() {
        let roles = vec![RoleSpec::agent("builder"), RoleSpec::agent("tester")];
        let reviewer = build_auto_reviewer_role(&roles).unwrap();
        assert_eq!(reviewer.role, "independent_reviewer");
        assert_eq!(reviewer.depends_on, vec!["builder", "tester"]);
        assert!(reviewer.is_reviewer());
        assert!(reviewer.write_paths.is_empty());
        assert!(is_auto_reviewer_role(&reviewer));
        assert_eq!(reviewer.verify.as_deref(), Some("non_empty"));
    }

    #[test]
    fn does_not_duplicate_existing_independent_reviewer() {
        let roles = vec![RoleSpec::agent("builder"), RoleSpec::agent("reviewer")];
        assert!(build_auto_reviewer_role(&roles).is_none());
    }

    #[test]
    fn lead_and_parallel_writer_cannot_count_as_independent_reviewers() {
        let mut lead = RoleSpec::agent("lead");
        lead.capabilities.push("review".to_string());
        let mut writer = RoleSpec::agent("w1");
        writer.capabilities.push("review".to_string());
        let reviewer = RoleSpec::agent("independent_reviewer");
        assert!(!super::is_independent_reviewer_role(&lead));
        assert!(!super::is_independent_reviewer_role(&writer));
        assert!(super::is_independent_reviewer_role(&reviewer));
    }

    #[test]
    fn generated_role_name_avoids_collision() {
        // Explicit non-review capability overrides the legacy name inference.
        // A genuine reviewer with this name is already covered by the no-duplicate test.
        let mut named_producer = RoleSpec::agent("independent_reviewer");
        named_producer.capabilities = vec!["implementation".to_string()];
        assert!(!named_producer.is_reviewer());
        let roles = vec![RoleSpec::agent("builder"), named_producer];
        let reviewer = build_auto_reviewer_role(&roles).unwrap();
        assert_eq!(reviewer.role, "independent_2_reviewer");
    }
}

#[cfg(test)]
mod team_budget_validation_tests {
    use super::validate_team_budget_config;
    use serde_json::json;

    #[test]
    fn validates_known_budget_fields_and_preserves_unknown_extensions() {
        assert!(validate_team_budget_config(&json!({
            "max_model_calls": 100,
            "max_cost_usd": 1.25,
            "max_wall_secs": 60,
            "future_budget_field": {"opaque": true}
        }))
        .is_ok());
        assert!(validate_team_budget_config(&serde_json::Value::Null).is_ok());
        assert!(validate_team_budget_config(&json!({
            "max_model_calls": 2.5
        }))
        .unwrap_err()
        .contains("max_model_calls"));
        assert!(validate_team_budget_config(&json!({
            "max_cost_usd": -0.01
        }))
        .unwrap_err()
        .contains("max_cost_usd"));
        assert!(validate_team_budget_config(&json!({
            "max_wall_secs": "60"
        }))
        .unwrap_err()
        .contains("max_wall_secs"));
        assert!(validate_team_budget_config(&json!(false)).is_err());
    }
}

#[cfg(test)]
mod auto_reviewer_budget_binding_tests {
    #[test]
    fn auto_reviewer_uses_the_planned_critic_budget_without_double_counting() {
        let mut plan = crate::team_strategy::TeamPlan {
            mode: "team".to_string(),
            requested: "auto".to_string(),
            roles: vec![
                crate::team_strategy::RolePlan {
                    role: "builder".to_string(),
                    duty: "produce".to_string(),
                    budget_calls: 4,
                },
                crate::team_strategy::RolePlan {
                    role: "critic".to_string(),
                    duty: "review".to_string(),
                    budget_calls: 2,
                },
            ],
            parallelism: 1,
            budget_calls_total: 6,
            json_repair: false,
            reasons: Vec::new(),
        };
        let mut budgets = std::collections::BTreeMap::new();
        super::bind_auto_reviewer_strategy_budget(&mut plan, &mut budgets, "independent_reviewer");

        assert_eq!(budgets.get("independent_reviewer"), Some(&2));
        assert_eq!(plan.budget_calls_total, 6);
        assert_eq!(plan.roles.len(), 2);
        assert!(plan
            .roles
            .iter()
            .any(|role| role.role == "independent_reviewer"));
    }

    #[test]
    fn auto_reviewer_without_planned_critic_adds_one_default_budget() {
        let mut plan = crate::team_strategy::TeamPlan {
            mode: "single".to_string(),
            requested: "auto".to_string(),
            roles: vec![crate::team_strategy::RolePlan {
                role: "builder".to_string(),
                duty: "produce".to_string(),
                budget_calls: 4,
            }],
            parallelism: 1,
            budget_calls_total: 4,
            json_repair: false,
            reasons: Vec::new(),
        };
        let mut budgets = std::collections::BTreeMap::new();
        super::bind_auto_reviewer_strategy_budget(&mut plan, &mut budgets, "independent_reviewer");

        assert_eq!(plan.budget_calls_total, 7);
        assert_eq!(plan.roles.len(), 2);
        assert_eq!(budgets.get("independent_reviewer"), Some(&3));
    }
}
