//! Single Agent turn execution loop.
//!
//! The public Agent API and long-lived configuration stay in `mod.rs`; this module owns
//! one turn's request/stream/tool/verification/completion lifecycle.

use super::*;

impl Agent {
    /// 回合执行主体：`run_turn` / `run_turn_with_asker` / `run_turn_with_images` 共用。
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_turn_inner(
        &self,
        session: &mut Session,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        approver: &dyn Approver,
        abort: &AtomicBool,
        on_event: &mut (dyn FnMut(&TurnEvent) + Send),
        questioner: Option<&dyn crate::question::Questioner>,
        options: &TurnExecutionOptions,
    ) -> Result<TurnOutcome, AgentError> {
        let started_at = Utc::now().to_rfc3339();
        let started = std::time::Instant::now();
        let turn_id = match options.turn_id.as_deref() {
            Some(id) => owo_agent_protocol::canonical_turn_id(id)
                .ok_or_else(|| AgentError::Session("turn/invalid_id".into()))?,
            None => uuid::Uuid::new_v4().to_string(),
        };
        let mut usage = TokenUsage::default();
        let mut model_calls = Vec::new();
        session.transient_model_calls.clear();
        session.active_task_context = Some(
            crate::task_context::ResolvedTaskContext::for_single_turn(&turn_id, prompt),
        );
        if abort.load(Ordering::Acquire) {
            return Err(AgentError::Aborted);
        }
        let turn_policy = self
            .policy
            .for_workspace(session.workspace.clone())
            .map_err(AgentError::Session)?;
        let turn_policy = if options.read_only {
            turn_policy.read_only_scope()
        } else {
            turn_policy
        };
        let mut usage_known = true;
        let mut model_requests = 0usize;
        // §9.2：turn 入口建立统一预算（None = 不限时，仅记账不强制）；
        // §9.3：阶段耗时瀑布按发生顺序累积。
        let mut budget = DeadlineBudget::new(self.config.turn_deadline, PhaseBudgets::default());
        let mut phase_timings: Vec<PhaseTiming> = Vec::new();
        // 新回合代表从当前历史继续发展，旧的 rewind/undo 分支不能再恢复。
        session.redo_stack.clear();
        session.message_redo_stack.clear();
        // 指令文件按目录层级发现（根 → 进程 cwd，Codex 同级语义）：越靠近工作目录的
        // AGENTS.md/CLAUDE.md 越具体；cwd 不在工作区内时退化为仅根目录。
        let rules = match std::env::current_dir() {
            Ok(cwd) => load_project_rules_hierarchical(&session.workspace, &cwd),
            Err(_) => load_project_rules(&session.workspace),
        };
        let mut system = build_system_prompt(session.system_prompt.as_deref(), &rules);
        if !self.skills.list_enabled().is_empty() {
            let mut catalog = vec!["可用技能（通过 use_skill 工具按名调用）：".to_string()];
            for skill in self.skills.list_enabled() {
                catalog.push(format!("- {}：{}", skill.name, skill.description));
            }
            system.push_str("\n\n");
            system.push_str(&catalog.join("\n"));
        }
        let mut messages = vec![ChatMessage::system(system)];
        messages.extend(session.messages.iter().cloned());
        if images.is_empty() {
            messages.push(ChatMessage::user(prompt.to_string()));
        } else {
            messages.push(ChatMessage::user_with_images(
                prompt.to_string(),
                images.to_vec(),
            ));
        }
        // 存量历史可能带非法序列（压缩切分、中断半截、外部导入）：发请求前归一。
        sanitize_history(&mut messages);
        // A2-1 UserPromptSubmit hook：exit 2 = 拒绝本回合（敏感词门卫/强制工单号等
        // 确定性控制），stderr 回喂模型与用户。
        let hooks = self.hooks_snapshot();
        if !hooks.is_empty() {
            let outcome = hooks
                .run(
                    crate::hooks::HookEvent::UserPromptSubmit,
                    &serde_json::json!({ "prompt": prompt, "session_id": session.id }),
                )
                .await;
            if let crate::hooks::HookOutcome::Blocked(stderr) = outcome {
                self.audit
                    .lock()
                    .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                    .record(
                        &session.id,
                        "hook_user_prompt_submit",
                        None,
                        Some(false),
                        format!("阻断：{stderr}"),
                    );
                return Err(AgentError::HookBlocked(stderr));
            }
        }
        let tools = self
            .visible_tool_specs()
            .into_iter()
            .filter(|spec| {
                !turn_policy.is_read_only()
                    || spec.effect.as_ref().is_some_and(|effect| {
                        effect.class == EffectClass::Read && effect.host_verified_readonly
                    })
            })
            .collect();
        // §9.3：schema 预算——超限时压缩描述/剥离噪声键（不删工具），
        // 并计算稳定指纹（provider schema 缓存复用的 key 基础）。
        let (tools, schema_report) =
            crate::schema_budget::enforce_budget(tools, crate::schema_budget::budget_from_env());

        let mut events = Vec::new();
        let mut final_text = None;
        let mut steps = 0usize;
        // 空回答静默重试计数（见 EMPTY_REPLY_RETRIES）。
        let mut empty_retries = 0usize;
        // 循环保护状态（本回合内）：工具调用总量 + 「同一 name/参数」重复计数。
        let mut tool_calls_seen = 0usize;
        let mut call_signatures: HashMap<String, usize> = HashMap::new();
        // 事件出口共享单元：嵌套子代理（subagent/explore）要能在父回合 await 期间
        // **即时**回传工具进度与审批请求（审批卡必须立刻到达客户端，否则子代理会
        // 一直等一个到不了的决定，直到审批超时——"卡死"的根因）。
        let event_cell: EventCell<'_> = Arc::new(Mutex::new(on_event));
        let nested_sink: crate::subagent::TurnEventSink<'_> = {
            let cell = Arc::clone(&event_cell);
            Arc::new(move |event: &TurnEvent| {
                // 中毒也继续转发（与仓库既有锁处理口径一致）：静默丢弃事件会让
                // 子代理进度/审批卡凭空消失，比"带毒继续"危险得多。
                let mut forward = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                forward(event);
            })
        };

        let mut model_turns = 0usize;
        let mut reached_model_turn_limit = false;
        let mut turn_completion_status = None;
        let mut validation_feedback_fingerprints = std::collections::BTreeSet::new();
        loop {
            if self.config.max_turns > 0 && model_turns >= self.config.max_turns {
                reached_model_turn_limit = true;
                break;
            }
            model_turns = model_turns.saturating_add(1);
            if abort.load(Ordering::Relaxed) {
                commit_turn_messages(session, &messages);
                return Err(AgentError::Aborted);
            }
            // A2-1 PreCompact hook：通知性质（不阻断——压缩是保护性动作）。
            let hooks = self.hooks_snapshot();
            if !hooks.is_empty() {
                let _ = hooks
                    .run(
                        crate::hooks::HookEvent::PreCompact,
                        &serde_json::json!({
                            "session_id": session.id,
                            "messages": messages.len(),
                            "estimated_tokens": estimate_tokens(&messages),
                        }),
                    )
                    .await;
            }
            let compaction = self.maybe_compact(&mut messages, &session.id, false).await;
            let summary = match compaction {
                Ok(summary) => summary,
                Err(error) => {
                    commit_turn_messages(session, &messages);
                    return Err(error);
                }
            };
            if let Some(summary) = summary {
                // 压缩请求目前未暴露 per-request usage，因此总用量必须标为不完整。
                usage_known = false;
                emit(
                    &mut events,
                    &event_cell,
                    TurnEvent::Compaction {
                        summary: summary.clone(),
                    },
                );
            }
            if messages.len() > self.config.context_limit {
                compact_truncate(&mut messages, self.config.context_limit);
            }

            emit(&mut events, &event_cell, TurnEvent::ModelCall);
            let on_event_reborrow = &event_cell;
            let model_started = std::time::Instant::now();
            // §9.3 瀑布：首个 TokenDelta 到达时刻记为首 token 时延。
            // 哨兵必须与合法值域不相交：0ms 是真实可能（本地/mock 端点同毫秒
            // 首达、时钟粒度），不能用 0 作"未触发"，否则观测数据被静默吞掉。
            const FIRST_TOKEN_UNSET: u64 = u64::MAX;
            // 声明须先于 emit_delta 闭包（闭包捕获引用）。
            let first_token_ms = std::sync::atomic::AtomicU64::new(FIRST_TOKEN_UNSET);
            let mut emit_chunk = |chunk: StreamChunk| {
                // §9.3 瀑布：首个增量到达即记录首 token 时延（compare_exchange 保证只记首次）。
                let _ = first_token_ms.compare_exchange(
                    FIRST_TOKEN_UNSET,
                    model_started.elapsed().as_millis() as u64,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                );
                // 思考通道单独事件外发（不写入对话历史；CLI/前端可折叠展示）。
                let event = match chunk {
                    StreamChunk::Content(delta) => TurnEvent::TokenDelta { delta },
                    StreamChunk::Reasoning(delta) => TurnEvent::ReasoningDelta { delta },
                };
                emit(&mut events, on_event_reborrow, event);
            };
            // §9.2：每次模型调用（即下一回合的 retry 点）前复查剩余预算；
            // 激活时以阶段剩余预算包裹超时，超时即结构化失败。
            // M4.2 会话级路由：显式覆盖（创建会话时指定）进请求体；未覆盖时
            // Provider 自行解析（OPENAI_MODEL 热切换 → 启动配置 → 内置默认）。
            // 克隆出循环体，避免与 `commit_turn_messages(session, …)` 的再借用冲突。
            let wire_model = session.model_override.clone();
            let attempt = async {
                tokio::select! {
                    output = self.provider.complete_stream_with_reasoning_and_model_observed(
                        wire_model.as_deref(),
                        &messages,
                        &tools,
                        &mut emit_chunk,
                    ) => {
                        output.map_err(AgentError::Gateway)
                    }
                    _ = wait_for_abort(abort) => Err(AgentError::Aborted),
                }
            };
            let output = match self.config.turn_deadline {
                Some(_) => {
                    let model_budget = budget.remaining(Phase::Model).map_err(|exceeded| {
                        commit_turn_messages(session, &messages);
                        exceeded.to_agent_error()
                    })?;
                    match tokio::time::timeout(model_budget, attempt).await {
                        Ok(result) => result,
                        Err(_) => {
                            session.transient_model_calls.push(ModelCallRecord {
                                metadata: ModelCallMetadata {
                                    model: wire_model.clone(),
                                    latency_ms: Some(model_started.elapsed().as_millis() as u64),
                                    ..ModelCallMetadata::default()
                                },
                                succeeded: false,
                            });
                            commit_turn_messages(session, &messages);
                            return Err(AgentError::Gateway(format!(
                                "预算耗尽：phase=model elapsed_ms={}（§9.2 DeadlineBudget）",
                                model_started.elapsed().as_millis()
                            )));
                        }
                    }
                }
                None => attempt.await,
            };
            let observed = match output {
                Ok(observed) => observed,
                Err(error) => {
                    session.transient_model_calls.push(ModelCallRecord {
                        metadata: ModelCallMetadata {
                            model: wire_model.clone(),
                            latency_ms: Some(model_started.elapsed().as_millis() as u64),
                            ..ModelCallMetadata::default()
                        },
                        succeeded: false,
                    });
                    commit_turn_messages(session, &messages);
                    return Err(error);
                }
            };
            model_requests = model_requests.saturating_add(1);
            let model_elapsed = model_started.elapsed();
            let mut request_metadata = observed.metadata.clone();
            request_metadata
                .latency_ms
                .get_or_insert(model_elapsed.as_millis() as u64);
            let request_record = ModelCallRecord {
                metadata: request_metadata,
                succeeded: true,
            };
            session.transient_model_calls.push(request_record.clone());
            model_calls.push(request_record);
            if let Some(request_usage) = observed.metadata.usage {
                usage.add(&request_usage);
            } else {
                usage_known = false;
            }
            let output = observed.output;
            budget.record(Phase::Model, model_elapsed);
            phase_timings.push(PhaseTiming {
                phase: Phase::Model.as_str().to_string(),
                elapsed_ms: model_elapsed.as_millis() as u64,
                target: String::new(),
                first_token_ms: {
                    let seen = first_token_ms.load(std::sync::atomic::Ordering::SeqCst);
                    (seen != FIRST_TOKEN_UNSET).then_some(seen)
                },
            });

            match output {
                // 空回答（网关截断/模型超载）不再直接当正常完成：先静默重试一次，
                // 仍为空则用「本回合已执行工具动作摘要」兜底，保证用户总有可见回复。
                ModelOutput::Text(text) if text.trim().is_empty() => {
                    if empty_retries < EMPTY_REPLY_RETRIES {
                        empty_retries += 1;
                        messages.push(ChatMessage::user(EMPTY_REPLY_RETRY_PROMPT.to_string()));
                        continue;
                    }
                    let fallback = synthesize_fallback_reply(
                        &messages,
                        steps,
                        "模型连续返回空回答（可能被网关截断或超载）",
                    );
                    messages.push(ChatMessage::assistant_text(fallback.clone()));
                    final_text = Some(fallback.clone());
                    emit(
                        &mut events,
                        &event_cell,
                        TurnEvent::Final { text: fallback },
                    );
                    break;
                }
                ModelOutput::Text(mut text) => {
                    // 单回合验收（Single completion）只属于面向用户的顶层回合：宿主登记
                    // VerificationPlan、独立评审和人工验收都以用户请求为锚点。子代理 /
                    // Team Worker（subagent_depth > 0）的完成语义由各自的输出契约决定，
                    // 其工具面不含 verification_plan；若仍进入该门控，宿主要求的登记
                    // 无法执行，模型只能报 blocked 并让整步失败（长任务实测）。
                    let mut completion_status = if self.config.subagent_depth > 0 {
                        owo_agent_protocol::CompletionStatusV1::Accepted
                    } else {
                        assess_single_turn_completion(
                            session,
                            prompt,
                            &turn_id,
                            &events,
                            false,
                            Some(&text),
                        )
                    };
                    if completion_status == owo_agent_protocol::CompletionStatusV1::Unverified {
                        let current_plan = session.single_verification_plan.clone().filter(|_| {
                            single_verification_plan_matches_turn(session, prompt, &turn_id)
                        });
                        if let Some(plan) = current_plan.filter(|plan| {
                            plan.requirements.iter().any(|requirement| {
                                requirement.required
                                    && requirement.validator_id
                                        == crate::completion::SINGLE_MANUAL_ACCEPTANCE_VALIDATOR_ID
                            })
                        }) {
                            completion_status =
                                single_manual_acceptance::request_single_manual_acceptance(
                                    session, &plan, &turn_id, questioner, abort,
                                )
                                .await;
                            if abort.load(Ordering::Relaxed) {
                                messages.push(ChatMessage::assistant_text(text.clone()));
                                commit_turn_messages(session, &messages);
                                return Err(AgentError::Aborted);
                            }
                        }
                    }
                    if completion_status == owo_agent_protocol::CompletionStatusV1::Accepted
                        && self.config.subagent_depth == 0
                    {
                        let candidate_paths =
                            single_review::accepted_candidate_paths(session, &turn_id);
                        if single_review::is_required(prompt, &candidate_paths) {
                            let model_turn_available =
                                self.config.max_turns == 0 || model_turns < self.config.max_turns;
                            let review_timeout = if self.config.turn_deadline.is_some() {
                                budget.remaining(Phase::Model).ok()
                            } else {
                                None
                            };
                            let allow_review_request = model_turn_available
                                && (self.config.turn_deadline.is_none()
                                    || review_timeout.is_some());
                            let review = single_review::review_candidate(
                                &self.provider,
                                session.model_override.as_deref(),
                                session,
                                prompt,
                                &turn_id,
                                &crate::CasStore::hash_of(prompt.as_bytes()),
                                &candidate_paths,
                                allow_review_request,
                                abort,
                                review_timeout,
                            )
                            .await;
                            if let Some(request) = review.request {
                                emit(&mut events, &event_cell, TurnEvent::ModelCall);
                                let review_elapsed =
                                    std::time::Duration::from_millis(review.request_duration_ms);
                                budget.record(Phase::Model, review_elapsed);
                                phase_timings.push(PhaseTiming {
                                    phase: Phase::Model.as_str().to_string(),
                                    elapsed_ms: review.request_duration_ms,
                                    target: "single_independent_review".to_string(),
                                    first_token_ms: None,
                                });
                                model_turns = model_turns.saturating_add(1);
                                model_requests = model_requests.saturating_add(1);
                                session.transient_model_calls.push(request.clone());
                                model_calls.push(request);
                                if let Some(request_usage) = review.usage {
                                    usage.add(&request_usage);
                                }
                                usage_known &= review.usage_known;
                            }
                            let review_verdict = review.receipt.verdict;
                            let review_passed =
                                review_verdict == crate::plan::ValidationVerdictV1::Passed;
                            single_review::apply_review_issue_receipt(
                                session,
                                &turn_id,
                                &review.receipt,
                            );
                            session.validation_receipts.push(review.receipt);
                            if !review_passed {
                                let current_validation_ids = session
                                    .validation_receipts
                                    .iter()
                                    .filter(|receipt| {
                                        receipt.attempt_id == turn_id
                                            && receipt.validator_id
                                                != "workspace-independent-review-v1"
                                            && receipt.verdict
                                                == crate::plan::ValidationVerdictV1::Passed
                                    })
                                    .map(|receipt| receipt.receipt_id.clone())
                                    .collect::<std::collections::HashSet<_>>();
                                for execution in &mut session.execution_receipts {
                                    if execution.status == "accepted"
                                        && execution
                                            .validation_receipt_id
                                            .as_ref()
                                            .is_some_and(|id| current_validation_ids.contains(id))
                                    {
                                        execution.status = "executed".to_string();
                                        execution.validation_receipt_id = None;
                                    }
                                }
                            }
                            completion_status = crate::completion::apply_required_review(
                                completion_status,
                                review_verdict,
                            );
                            if abort.load(Ordering::Relaxed) {
                                messages.push(ChatMessage::assistant_text(text.clone()));
                                commit_turn_messages(session, &messages);
                                return Err(AgentError::Aborted);
                            }
                        }
                    }
                    if completion_status == owo_agent_protocol::CompletionStatusV1::Accepted {
                        if let Some(notice) =
                            single_manual_acceptance::completion_notice(session, &turn_id)
                        {
                            text.push_str(&notice);
                        }
                    }
                    let can_retry_after_validation = (self.config.max_turns == 0
                        || model_turns < self.config.max_turns)
                        && (self.config.turn_deadline.is_none()
                            || budget.remaining(Phase::Model).is_ok());
                    let plan_is_current =
                        single_verification_plan_matches_turn(session, prompt, &turn_id);
                    let has_turn_source_candidate =
                        session.execution_receipts.iter().any(|receipt| {
                            receipt.turn_id == turn_id
                                && receipt.status == "executed"
                                && receipt
                                    .changed_files
                                    .iter()
                                    .any(|path| single_path_is_source_code(path))
                        });
                    let retry_feedback = if can_retry_after_validation
                        && plan_is_current
                        && matches!(
                            completion_status,
                            owo_agent_protocol::CompletionStatusV1::Unverified
                                | owo_agent_protocol::CompletionStatusV1::Blocked
                        ) {
                        single_validation_retry_feedback(session, &turn_id)
                    } else if can_retry_after_validation
                        && !plan_is_current
                        && has_turn_source_candidate
                        && matches!(
                            completion_status,
                            owo_agent_protocol::CompletionStatusV1::Candidate
                                | owo_agent_protocol::CompletionStatusV1::Unverified
                        )
                    {
                        single_missing_verification_plan_feedback(session, &turn_id)
                    } else {
                        None
                    };
                    if let Some((fingerprint, feedback)) = retry_feedback {
                        let repeated_failure =
                            !validation_feedback_fingerprints.insert(fingerprint.clone());
                        turn_completion_status = Some(completion_status);
                        messages.push(ChatMessage::assistant_text(text.clone()));
                        if repeated_failure
                            || validation_feedback_fingerprints.len()
                                > MAX_HOST_VALIDATION_REPAIR_ROUNDS
                        {
                            // 指纹重复或修复轮次耗尽：带真实完成状态收口，绝不无限修复。
                            final_text = Some(text.clone());
                            emit(&mut events, &event_cell, TurnEvent::Final { text });
                            break;
                        }
                        single_review::mark_review_issue_repair_dispatched(session, &turn_id);
                        messages.push(ChatMessage::system(feedback));
                        final_text = None;
                        continue;
                    }
                    turn_completion_status = Some(completion_status);
                    messages.push(ChatMessage::assistant_text(text.clone()));
                    final_text = Some(text.clone());
                    emit(&mut events, &event_cell, TurnEvent::Final { text });
                    break;
                }
                ModelOutput::ToolCalls(calls) => {
                    // 所有 Provider（含自定义/测试 Provider）统一校验结构不变量
                    // （id/name/arguments 形态）。「工具不在本轮清单/已热卸载」不在此处
                    // 终止回合：它属于工具级错误，必须回喂模型自修复（见下方 guard 分支）。
                    if let Err(error) = crate::gateway::validate_tool_call_shapes(&calls) {
                        commit_turn_messages(session, &messages);
                        return Err(AgentError::Gateway(error));
                    }
                    // 循环保护（对标 Codex/OpenCode）：先查总量上限，再逐调用查重复。
                    if self.config.max_tool_calls_per_turn > 0
                        && tool_calls_seen.saturating_add(calls.len())
                            > self.config.max_tool_calls_per_turn
                    {
                        let limit = self.config.max_tool_calls_per_turn;
                        commit_turn_messages(session, &messages);
                        return Err(AgentError::Gateway(format!(
                            "循环保护：单回合工具调用达到上限 {limit}（已请求 {tool_calls_seen} + 本批 {}）。已停止执行以避免失控循环；请缩小任务或分步重试。",
                            calls.len()
                        )));
                    }
                    tool_calls_seen = tool_calls_seen.saturating_add(calls.len());
                    messages.push(ChatMessage::assistant_tool_calls(calls.clone()));
                    // §9.1 阶段一——权限判定保持原始 tool-call 顺序：Ask 的独立审批
                    // 与用户审批仍按原序逐个交互，先得到每个调用的 Allow/Deny。
                    let mut prepared: Vec<PreparedCall> = Vec::with_capacity(calls.len());
                    for call in &calls {
                        if abort.load(Ordering::Relaxed) {
                            commit_turn_messages(session, &messages);
                            return Err(AgentError::Aborted);
                        }
                        // 循环保护：同一 name + 规范化参数重复超过上限 → 拦截，不审批不执行。
                        let signature = tool_call_signature(call);
                        let repeats = call_signatures.entry(signature).or_insert(0);
                        *repeats += 1;
                        if *repeats > self.config.max_repeated_tool_calls {
                            let reason = format!(
                                "循环保护：工具 `{}` 携相同参数已请求 {} 次（上限 {}），本次不再执行。请改变策略或直接给出结论。",
                                call.name, *repeats, self.config.max_repeated_tool_calls
                            );
                            prepared.push(PreparedCall {
                                approval: None,
                                reason: reason.clone(),
                                guard_error: Some(reason),
                            });
                            continue;
                        }
                        // 热卸载/未下发的工具：不终止回合，作为工具错误回喂模型换策略。
                        if self.tool_disabled(&call.name) {
                            let error = format!("工具已被禁用（插件热卸载）：{}", call.name);
                            prepared.push(PreparedCall {
                                approval: None,
                                reason: error.clone(),
                                guard_error: Some(error),
                            });
                            continue;
                        }
                        // Check against the exact schema sent in this model turn before
                        // hooks, approval, or execution; feed failures back for model repair.
                        let Some(schema_check) = tools.iter().find(|spec| spec.name == call.name)
                        else {
                            let error = format!(
                                "工具 {} 不在本轮工具列表中（可能已禁用或名称错误）；本次未执行。",
                                call.name
                            );
                            prepared.push(PreparedCall {
                                approval: None,
                                reason: error.clone(),
                                guard_error: Some(error),
                            });
                            continue;
                        };
                        if let Err(error) =
                            crate::gateway::validate_tool_arguments(&call.arguments, schema_check)
                        {
                            self.audit
                                .lock()
                                .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                                .record(
                                    &session.id,
                                    "tool_schema_rejected",
                                    Some(call.name.clone()),
                                    Some(false),
                                    error.clone(),
                                );
                            prepared.push(PreparedCall {
                                approval: None,
                                reason: error.clone(),
                                guard_error: Some(error),
                            });
                            continue;
                        }
                        // A2-1 PreToolUse hook：exit 2 = 阻断该次调用，stderr 原样作为
                        // 拒绝原因回喂模型（模型可据此换策略），不终止回合。
                        let hooks = self.hooks_snapshot();
                        if !hooks.is_empty() {
                            let outcome = hooks
                                .run(
                                    crate::hooks::HookEvent::PreToolUse,
                                    &serde_json::json!({
                                        "tool": call.name,
                                        "args": call.arguments,
                                        "session_id": session.id,
                                    }),
                                )
                                .await;
                            if let crate::hooks::HookOutcome::Blocked(stderr) = outcome {
                                self.audit
                                    .lock()
                                    .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                                    .record(
                                        &session.id,
                                        "hook_pre_tool_use",
                                        Some(call.name.clone()),
                                        Some(false),
                                        format!("阻断：{stderr}"),
                                    );
                                prepared.push(PreparedCall {
                                    approval: None,
                                    reason: format!("hook 阻断：{stderr}"),
                                    guard_error: Some(format!("hook 阻断：{stderr}")),
                                });
                                continue;
                            }
                        }
                        // §5.1：从注册表取出完整的 ToolSpec（含 effect 唯一事实源），
                        // 交给 Policy 判定，避免全局名字再查询。
                        let call_spec = self
                            .registry
                            .read()
                            .map_err(|_| AgentError::Session("工具注册表锁中毒".into()))?
                            .get(&call.name)
                            .map(|tool| tool.spec());
                        let request = turn_policy.evaluate_with_effect(
                            &call.name,
                            call_spec.as_ref().and_then(|spec| spec.effect.as_ref()),
                            &call.arguments,
                        );
                        let mut decision_reason = request.reason.clone();
                        let decision = match turn_policy.decision(&request) {
                            Decision::Ask => {
                                // 独立审批模型先于打扰用户（Auto-review）。
                                let approval_started = std::time::Instant::now();
                                let verdict = if let Some(reviewer) = &self.reviewer {
                                    // 审批上下文必须是**本回合用户原始指令**（审查 N9：
                                    // 此前取 `session.messages.last()`，那是上一轮历史，
                                    // 审批模型看不到本轮意图；长度由 reviewer 侧截断）。
                                    let context = Some(prompt.to_string());
                                    reviewer.review(&request, context.as_deref()).await
                                } else {
                                    ReviewVerdict::Unknown
                                };
                                if self.reviewer.is_some() {
                                    phase_timings.push(PhaseTiming {
                                        phase: Phase::Approval.as_str().to_string(),
                                        elapsed_ms: approval_started.elapsed().as_millis() as u64,
                                        target: format!("{}:review", call.name),
                                        first_token_ms: None,
                                    });
                                }
                                match verdict {
                                    ReviewVerdict::Deny => {
                                        self.audit
                                            .lock()
                                            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                                            .record(
                                                &session.id,
                                                "auto_review",
                                                Some(call.name.clone()),
                                                Some(false),
                                                format!("独立审批模型拒绝：{}", request.reason),
                                            );
                                        Decision::Deny
                                    }
                                    ReviewVerdict::Allow => {
                                        self.audit
                                            .lock()
                                            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                                            .record(
                                                &session.id,
                                                "auto_review",
                                                Some(call.name.clone()),
                                                Some(true),
                                                "独立审批模型放行".to_string(),
                                            );
                                        Decision::Allow
                                    }
                                    ReviewVerdict::Unknown => {
                                        let decide_started = std::time::Instant::now();
                                        let outcome = {
                                            let mut notify = || {
                                                emit(
                                                    &mut events,
                                                    &event_cell,
                                                    TurnEvent::PermissionRequest(request.clone()),
                                                )
                                            };
                                            approver
                                                .decide_with_notification(&request, &mut notify)
                                                .await
                                        };
                                        if let Some(reason) = outcome.reason {
                                            decision_reason =
                                                format!("{}; {reason}", request.reason);
                                        }
                                        let decided = outcome.decision;
                                        phase_timings.push(PhaseTiming {
                                            phase: Phase::Approval.as_str().to_string(),
                                            elapsed_ms: decide_started.elapsed().as_millis() as u64,
                                            target: call.name.clone(),
                                            first_token_ms: None,
                                        });
                                        decided
                                    }
                                }
                            }
                            other => other,
                        };
                        let approval = ToolApprovalGrant::from_decision(&request, decision).ok();
                        let approved = approval.is_some();
                        self.audit
                            .lock()
                            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                            .record(
                                &session.id,
                                "permission",
                                Some(call.name.clone()),
                                Some(approved),
                                decision_reason.clone(),
                            );
                        prepared.push(PreparedCall {
                            approval,
                            reason: decision_reason.clone(),
                            guard_error: None,
                        });
                    }

                    // §9.1 阶段二——执行：仅「已放行 + 宿主验证只读（EffectClass::Read
                    // 且 host_verified_readonly）+ 未禁用」的连续调用组成有界并发组
                    //（默认 4）；写/执行/注入、MCP 自报只读未验证、未知工具与被拒
                    // 调用一律串行。tool 消息按原 tool-call 顺序回填。
                    let group_events: Arc<Mutex<Vec<TurnEvent>>> = Arc::new(Mutex::new(Vec::new()));
                    let mut results: Vec<Result<serde_json::Value, String>> =
                        Vec::with_capacity(calls.len());
                    let mut index = 0;
                    while index < calls.len() {
                        if abort.load(Ordering::Relaxed) {
                            commit_turn_messages(session, &messages);
                            return Err(AgentError::Aborted);
                        }
                        let eligible_here = prepared[index].approval.is_some()
                            && !self.tool_disabled(&calls[index].name)
                            && self.call_is_concurrent_eligible(&calls[index]);
                        if eligible_here {
                            // 连续可并发段（上限 tool_concurrency）；遇任何不满足条件
                            // 的调用立即断组——写/执行紧邻只读时绝不进同一组。
                            let start = index;
                            let mut end = start + 1;
                            while end < calls.len()
                                && end - start < self.config.tool_concurrency.max(1)
                                && prepared[end].approval.is_some()
                                && !self.tool_disabled(&calls[end].name)
                                && self.call_is_concurrent_eligible(&calls[end])
                            {
                                end += 1;
                            }
                            let mut futures = Vec::with_capacity(end - start);
                            for (offset, call) in calls[start..end].iter().enumerate() {
                                let approval = prepared[start + offset]
                                    .approval
                                    .clone()
                                    .expect("eligible tool call must have approval grant");
                                let workspace = session.workspace.clone();
                                let max_command_timeout_ms = self.config.max_command_timeout_ms;
                                let capability_context = ToolCapabilityContext::for_workspace(
                                    &workspace,
                                    session.id.clone(),
                                    turn_id.clone(),
                                )
                                .with_command_timeout(max_command_timeout_ms);
                                // 并发组内全部为宿主验证只读工具（经审计不改变会话
                                // 状态）；Session 按值克隆以满足 ToolContext 的 &mut
                                // 签名，克隆上的任何变更被有意丢弃（读取语义不变）。
                                let mut session_view = session.clone();
                                let subagent = SubagentRunner {
                                    provider: Arc::clone(&self.provider),
                                    parent_policy: &turn_policy,
                                    approver,
                                    abort,
                                    depth: self.config.subagent_depth,
                                    max_turns: nested_turn_cap(self.config.max_turns),
                                    model: session.model_override.clone().unwrap_or_default(),
                                    events: Some(Arc::clone(&nested_sink)),
                                };
                                // A5-1：fan-out 通道（owned，'static 闭包约束）。
                                let fanout = crate::subagent::FanOutRunner {
                                    provider: Arc::clone(&self.provider),
                                    parent_policy: Arc::new(
                                        turn_policy
                                            .for_workspace(workspace.clone())
                                            .map_err(AgentError::Session)?,
                                    ),
                                    workspace: workspace.clone(),
                                    model: session_view.model_override.clone().unwrap_or_default(),
                                    depth: self.config.subagent_depth,
                                    max_turns: nested_turn_cap(self.config.max_turns),
                                };
                                let sink = Arc::clone(&group_events);
                                let call_id = call.id.clone();
                                let tool_name = call.name.clone();
                                let arguments = call.arguments.clone();
                                let tool_host = self.tool_host.clone();
                                // 实时状态：ToolStart 立即外发（不再等整组结束），
                                // 让长任务/多工具链在 CLI 上可见"正在跑哪些工具、什么参数"。
                                emit(
                                    &mut events,
                                    &event_cell,
                                    TurnEvent::ToolStart {
                                        id: call_id.clone(),
                                        tool: tool_name.clone(),
                                        args_preview: tool_args_preview(&call.arguments),
                                    },
                                );
                                let call_policy = &turn_policy;
                                futures.push(async move {
                                    let mut ctx = ToolContext {
                                        workspace: &workspace,
                                        policy: call_policy,
                                        session: &mut session_view,
                                        audit: &self.audit,
                                        subagent: Some(subagent),
                                        skills: &self.skills,
                                        elements: &self.elements,
                                        fanout: Some(fanout),
                                        abort: Some(abort),
                                        // 并行只读组不参与提问（无 mut session/UI 通道）。
                                        questioner: None,
                                    };
                                    let command_before = crate::command_evidence::capture_for_tool(
                                        ctx.session,
                                        &tool_name,
                                        &arguments,
                                    );
                                    let outcome = match tool_host.issue(
                                        &tool_name,
                                        arguments,
                                        approval,
                                        capability_context,
                                    ) {
                                        Ok(capability) => {
                                            tool_host.execute(capability, &mut ctx).await
                                        }
                                        Err(error) => Err(error),
                                    };
                                    if let Ok(mut buffer) = sink.lock() {
                                        let command_receipt = command_execution_receipt(
                                            &tool_name,
                                            &outcome,
                                            ctx.session,
                                            command_before,
                                        );
                                        buffer.push(TurnEvent::ToolResult {
                                            id: call_id,
                                            tool: tool_name,
                                            ok: outcome.is_ok(),
                                            error: outcome.as_ref().err().cloned(),
                                            preview: tool_preview(&outcome),
                                            command_receipt,
                                        });
                                    }
                                    outcome
                                });
                            }
                            // 有界并发轮询；abort → 组合 future 被 drop，所有未完成
                            // 工具在 await 点被取消，不残留后台任务（§9.1.6）。
                            let group_started = std::time::Instant::now();
                            let outcomes = if self.config.turn_deadline.is_some() {
                                // §9.2：组级预算包裹——超时 drop 组合 future，
                                // 组内所有未完成工具随 await 点取消。
                                let tool_budget =
                                    budget.remaining(Phase::Tool).map_err(|exceeded| {
                                        commit_turn_messages(session, &messages);
                                        exceeded.to_agent_error()
                                    })?;
                                match tokio::time::timeout(tool_budget, async {
                                    tokio::select! {
                                        outcomes = futures::future::join_all(futures) => Ok(outcomes),
                                        _ = wait_for_abort(abort) => Err(AgentError::Aborted),
                                    }
                                })
                                .await
                                {
                                    Ok(Ok(outcomes)) => outcomes,
                                    Ok(Err(AgentError::Aborted)) => {
                                        commit_turn_messages(session, &messages);
                                        return Err(AgentError::Aborted);
                                    }
                                    Ok(Err(other)) => return Err(other),
                                    Err(_) => {
                                        commit_turn_messages(session, &messages);
                                        return Err(AgentError::Gateway(format!(
                                            "预算耗尽：phase=tool elapsed_ms={}（§9.2 并发组）",
                                            group_started.elapsed().as_millis()
                                        )));
                                    }
                                }
                            } else {
                                // 实时回放：等待期间每 100ms drain 一次组事件缓冲，
                                // 工具一完成即可在 CLI 看到 ToolResult（不再等整组结束）。
                                let join = futures::future::join_all(futures);
                                tokio::pin!(join);
                                loop {
                                    tokio::select! {
                                        outcomes = &mut join => break outcomes,
                                        _ = wait_for_abort(abort) => {
                                            commit_turn_messages(session, &messages);
                                            return Err(AgentError::Aborted);
                                        }
                                        _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                                            if let Ok(mut buffer) = group_events.lock() {
                                                for event in buffer.drain(..) {
                                                    emit(&mut events, &event_cell, event);
                                                }
                                            }
                                        }
                                    }
                                }
                            };
                            let group_elapsed = group_started.elapsed();
                            budget.record(Phase::Tool, group_elapsed);
                            phase_timings.push(PhaseTiming {
                                phase: Phase::Tool.as_str().to_string(),
                                elapsed_ms: group_elapsed.as_millis() as u64,
                                target: format!("group:{}", end - start),
                                first_token_ms: None,
                            });
                            results.extend(outcomes);
                            // 兜底 drain：把剩余 ToolResult 按插入序回放
                            // （ToolStart 已实时外发；未完成工具的 ToolResult 在此补齐）。
                            if let Ok(mut buffer) = group_events.lock() {
                                for event in buffer.drain(..) {
                                    emit(&mut events, &event_cell, event);
                                }
                            }
                            index = end;
                        } else {
                            // —— 串行执行单个调用（与原实现逐字等价）——
                            let call = &calls[index];
                            let result = if let Some(guard) = prepared[index].guard_error.clone() {
                                // 宿主执行前拦截：不执行，回灌 schema/hook/循环保护原因供模型修复。
                                emit(
                                    &mut events,
                                    &event_cell,
                                    TurnEvent::ToolResult {
                                        id: call.id.clone(),
                                        tool: call.name.clone(),
                                        ok: false,
                                        error: Some(guard.clone()),
                                        // 未执行（宿主拦截）没有结果正文可预览。
                                        preview: None,
                                        command_receipt: None,
                                    },
                                );
                                Err(guard)
                            } else if self.tool_disabled(&call.name) {
                                Err(format!("工具已被禁用（插件热卸载）：{}", call.name))
                            } else if prepared[index].approval.is_some() {
                                let workspace = session.workspace.clone();
                                let capability_context = ToolCapabilityContext::for_workspace(
                                    &workspace,
                                    session.id.clone(),
                                    turn_id.clone(),
                                )
                                .with_command_timeout(self.config.max_command_timeout_ms);
                                emit(
                                    &mut events,
                                    &event_cell,
                                    TurnEvent::ToolStart {
                                        id: call.id.clone(),
                                        tool: call.name.clone(),
                                        args_preview: tool_args_preview(&call.arguments),
                                    },
                                );
                                let subagent = SubagentRunner {
                                    provider: Arc::clone(&self.provider),
                                    parent_policy: &turn_policy,
                                    approver,
                                    abort,
                                    depth: self.config.subagent_depth,
                                    max_turns: nested_turn_cap(self.config.max_turns),
                                    model: session.model_override.clone().unwrap_or_default(),
                                    events: Some(Arc::clone(&nested_sink)),
                                };
                                let fanout = crate::subagent::FanOutRunner {
                                    provider: Arc::clone(&self.provider),
                                    parent_policy: Arc::new(
                                        turn_policy
                                            .for_workspace(workspace.clone())
                                            .map_err(AgentError::Session)?,
                                    ),
                                    workspace: workspace.clone(),
                                    model: session.model_override.clone().unwrap_or_default(),
                                    depth: self.config.subagent_depth,
                                    max_turns: nested_turn_cap(self.config.max_turns),
                                };
                                // P2-5：计划快照——工具执行后清单变化即发 PlanUpdate
                                //（前端渲染步骤进度；todo 工具为整表替换语义）。
                                let plan_before = session.todos.clone();
                                let mut ctx = ToolContext {
                                    workspace: &workspace,
                                    policy: &turn_policy,
                                    session,
                                    audit: &self.audit,
                                    subagent: Some(subagent),
                                    skills: &self.skills,
                                    elements: &self.elements,
                                    fanout: Some(fanout),
                                    abort: Some(abort),
                                    questioner,
                                };
                                let tool_started = std::time::Instant::now();
                                let command_before = crate::command_evidence::capture_for_tool(
                                    ctx.session,
                                    &call.name,
                                    &call.arguments,
                                );
                                let outcome = match self.tool_host.issue(
                                    &call.name,
                                    call.arguments.clone(),
                                    prepared[index]
                                        .approval
                                        .clone()
                                        .expect("approved tool call must have approval grant"),
                                    capability_context,
                                ) {
                                    Ok(capability) => {
                                        let run = self.tool_host.execute(capability, &mut ctx);
                                        // §9.2：激活预算时以阶段剩余包裹工具执行，
                                        // 超时转工具级错误（回合继续，模型可见）。
                                        if self.config.turn_deadline.is_some() {
                                            match budget.remaining(Phase::Tool) {
                                                Ok(tool_budget) => {
                                                    match tokio::time::timeout(tool_budget, run).await
                                                    {
                                                        Ok(outcome) => outcome,
                                                        Err(_) => Err(format!(
                                                            "工具预算耗尽（§9.2）：{} elapsed_ms={}",
                                                            call.name,
                                                            tool_started.elapsed().as_millis()
                                                        )),
                                                    }
                                                }
                                                Err(exceeded) => Err(format!(
                                                    "工具预算耗尽（§9.2）：{} {exceeded}",
                                                    call.name
                                                )),
                                            }
                                        } else {
                                            run.await
                                        }
                                    }
                                    Err(error) => Err(error),
                                };
                                let tool_elapsed = tool_started.elapsed();
                                budget.record(Phase::Tool, tool_elapsed);
                                phase_timings.push(PhaseTiming {
                                    phase: Phase::Tool.as_str().to_string(),
                                    elapsed_ms: tool_elapsed.as_millis() as u64,
                                    target: call.name.clone(),
                                    first_token_ms: None,
                                });
                                let command_receipt = command_execution_receipt(
                                    &call.name,
                                    &outcome,
                                    ctx.session,
                                    command_before,
                                );
                                emit(
                                    &mut events,
                                    &event_cell,
                                    TurnEvent::ToolResult {
                                        id: call.id.clone(),
                                        tool: call.name.clone(),
                                        ok: outcome.is_ok(),
                                        error: outcome.as_ref().err().cloned(),
                                        preview: tool_preview(&outcome),
                                        command_receipt,
                                    },
                                );
                                if ctx.session.todos != plan_before {
                                    if let Ok(steps) = serde_json::to_value(&ctx.session.todos) {
                                        emit(
                                            &mut events,
                                            &event_cell,
                                            TurnEvent::PlanUpdate { steps },
                                        );
                                    }
                                }
                                outcome
                            } else {
                                Err(format!("permission denied: {}", prepared[index].reason))
                            };
                            results.push(result);
                            index += 1;
                        }
                    }
                    // —— 回填：按原 tool-call 顺序生成 tool 消息 + 审计 + 计步 ——
                    for (call, result) in calls.iter().zip(results) {
                        if result.is_ok() && !self.call_is_concurrent_eligible(call) {
                            reset_loop_guard_after_progress(
                                &mut call_signatures,
                                &tool_call_signature(call),
                            );
                        }
                        let raw_content = match &result {
                            Ok(value) => value.to_string(),
                            Err(error) => format!("工具错误：{error}"),
                        };
                        let truncated = truncate_tool_result(&raw_content, MAX_TOOL_RESULT_CHARS);
                        let content = match (&self.artifact_store, raw_content != truncated) {
                            // §9.3：完整结果入 CAS，模型拿「指针+预览」而非盲截断；
                            // MIME/大小/截断原因/ref 一并携带（hash 即 CAS ref）。
                            // 预览同样必须过注入净化（审查 N8：此前大结果分支绕过
                            // sanitize_tool_result，>50k 字符即可夹带注入文本）。
                            (Some(store), true) => match store.put(raw_content.as_bytes()) {
                                Ok(hash) => {
                                    let safe_preview = sanitize_tool_result(&call.name, &truncated);
                                    serde_json::json!({
                                        "artifact": {
                                            "ref": hash,
                                            "mime": "text/plain",
                                            "size_bytes": raw_content.len(),
                                            "truncation_reason": format!(
                                                "工具结果超过 {} 字符上限，完整内容已存 artifact：用 read_artifact(ref=<下面 artifact.ref>) 分页读取",
                                                MAX_TOOL_RESULT_CHARS
                                            ),
                                            "preview": safe_preview,
                                        }
                                    })
                                    .to_string()
                                }
                                Err(_) => sanitize_tool_result(&call.name, &truncated),
                            },
                            _ => sanitize_tool_result(&call.name, &truncated),
                        };
                        messages.push(ChatMessage::tool(call.id.clone(), content.clone()));
                        self.audit
                            .lock()
                            .map_err(|_| AgentError::Session("审计锁中毒".into()))?
                            .record(
                                &session.id,
                                "tool_call",
                                Some(call.name.clone()),
                                None,
                                content.clone(),
                            );
                        steps += 1;
                    }
                }
            }
        }

        if final_text.is_none() {
            reached_model_turn_limit = true;
            // 步数耗尽不能只甩一句「达到最大回合数」：再补一次不带工具的收尾总结，
            // 保证回合一定有可见结论（审查/分析类任务据此产出报告），
            // 而不是让用户看到「思考完就停住」。
            if abort.load(Ordering::Relaxed) {
                commit_turn_messages(session, &messages);
                return Err(AgentError::Aborted);
            }
            emit(&mut events, &event_cell, TurnEvent::ModelCall);
            let mut wrap_messages = messages.clone();
            wrap_messages.push(ChatMessage::user(WRAP_UP_PROMPT.to_string()));
            let wrap_model = session.model_override.clone();
            let wrap_up_started = std::time::Instant::now();
            let wrap_request_started = std::sync::atomic::AtomicBool::new(false);
            let wrap_up = {
                let mut emit_wrap_delta = |delta: String| {
                    emit(&mut events, &event_cell, TurnEvent::TokenDelta { delta });
                };
                let mut wrap_chunks = |chunk: StreamChunk| {
                    if let StreamChunk::Content(delta) = chunk {
                        emit_wrap_delta(delta);
                    }
                };
                let wrap_request = async {
                    tokio::select! {
                        biased;
                        _ = wait_for_abort(abort) => Err(AgentError::Aborted),
                        result = async {
                            wrap_request_started.store(true, Ordering::Relaxed);
                            self.provider
                                .complete_stream_with_reasoning_and_model_observed(
                                    wrap_model.as_deref(),
                                    &wrap_messages,
                                    &[],
                                    &mut wrap_chunks,
                                )
                                .await
                                .map_err(AgentError::Gateway)
                        } => result,
                    }
                };
                match self.config.turn_deadline {
                    Some(_) => match budget.remaining(Phase::Model) {
                        Ok(remaining) => {
                            match tokio::time::timeout(remaining, wrap_request).await {
                                Ok(result) => result,
                                Err(_) => Err(AgentError::Gateway(format!(
                                    "预算耗尽：phase=model elapsed_ms={}（§9.2 wrap-up）",
                                    budget.turn_elapsed().as_millis()
                                ))),
                            }
                        }
                        Err(exceeded) => Err(exceeded.to_agent_error()),
                    },
                    None => wrap_request.await,
                }
            };
            let wrap_up_elapsed = wrap_up_started.elapsed();
            let wrap_request_was_started = wrap_request_started.load(Ordering::Relaxed);
            if wrap_request_was_started {
                model_requests = model_requests.saturating_add(1);
                budget.record(Phase::Model, wrap_up_elapsed);
                phase_timings.push(PhaseTiming {
                    phase: Phase::Model.as_str().to_string(),
                    elapsed_ms: wrap_up_elapsed.as_millis() as u64,
                    target: "turn_limit_wrap_up".to_string(),
                    first_token_ms: None,
                });
            }
            if matches!(&wrap_up, Err(AgentError::Aborted)) {
                if wrap_request_was_started {
                    let request_record = ModelCallRecord {
                        metadata: ModelCallMetadata {
                            model: wrap_model.clone(),
                            latency_ms: Some(wrap_up_elapsed.as_millis() as u64),
                            ..ModelCallMetadata::default()
                        },
                        succeeded: false,
                    };
                    session.transient_model_calls.push(request_record.clone());
                    model_calls.push(request_record);
                }
                commit_turn_messages(session, &messages);
                return Err(AgentError::Aborted);
            }
            let wrap_up = match wrap_up {
                Ok(observed) => {
                    let mut request_metadata = observed.metadata.clone();
                    request_metadata
                        .latency_ms
                        .get_or_insert(wrap_up_elapsed.as_millis() as u64);
                    let request_record = ModelCallRecord {
                        metadata: request_metadata,
                        succeeded: true,
                    };
                    session.transient_model_calls.push(request_record.clone());
                    model_calls.push(request_record);
                    if let Some(request_usage) = observed.metadata.usage {
                        usage.add(&request_usage);
                    } else {
                        usage_known = false;
                    }
                    Ok(observed.output)
                }
                Err(error) => {
                    if wrap_request_was_started {
                        usage_known = false;
                        let request_record = ModelCallRecord {
                            metadata: ModelCallMetadata {
                                model: wrap_model.clone(),
                                latency_ms: Some(wrap_up_elapsed.as_millis() as u64),
                                ..ModelCallMetadata::default()
                            },
                            succeeded: false,
                        };
                        session.transient_model_calls.push(request_record.clone());
                        model_calls.push(request_record);
                    }
                    Err(error.to_string())
                }
            };
            // 收尾总结同样不允许「空手而归」：模型没产出内容（或调用失败）时，
            // 用本回合已执行的工具动作摘要兜底——回合必须以可见结论结束。
            let text = match wrap_up {
                Ok(ModelOutput::Text(text)) if !text.trim().is_empty() => text,
                Ok(_) => synthesize_fallback_reply(
                    &messages,
                    steps,
                    &format!(
                        "达到最大回合数（{}）且收尾总结未产出内容",
                        self.config.max_turns
                    ),
                ),
                Err(error) => synthesize_fallback_reply(
                    &messages,
                    steps,
                    &format!(
                        "达到最大回合数（{}）且收尾总结调用失败：{error}",
                        self.config.max_turns
                    ),
                ),
            };
            messages.push(ChatMessage::assistant_text(text.clone()));
            final_text = Some(text.clone());
            emit(&mut events, &event_cell, TurnEvent::Final { text });
        }
        let persist_started = std::time::Instant::now();
        commit_turn_messages(session, &messages);
        // A2-1 Stop hook：回合结束通知（finally 类动作如测试/通知）。
        let hooks = self.hooks_snapshot();
        if !hooks.is_empty() {
            let _ = hooks
                .run(
                    crate::hooks::HookEvent::Stop,
                    &serde_json::json!({
                        "session_id": session.id,
                        "stop_reason": final_text
                            .as_deref()
                            .map(|text| text.chars().take(120).collect::<String>()),
                    }),
                )
                .await;
        }
        let persist_elapsed = persist_started.elapsed();
        budget.record(Phase::Persistence, persist_elapsed);
        phase_timings.push(PhaseTiming {
            phase: Phase::Persistence.as_str().to_string(),
            elapsed_ms: persist_elapsed.as_millis() as u64,
            target: String::new(),
            first_token_ms: None,
        });
        usage_known &= model_requests > 0;
        let completion_status = if self.config.subagent_depth > 0 {
            // 子代理 / Team Worker 不进入 Single 验收门控（见 Text 分支说明）。
            turn_completion_status.unwrap_or(owo_agent_protocol::CompletionStatusV1::Accepted)
        } else if reached_model_turn_limit {
            assess_single_turn_completion(
                session,
                prompt,
                &turn_id,
                &events,
                true,
                final_text.as_deref(),
            )
        } else if let Some(status) = turn_completion_status {
            status
        } else {
            assess_single_turn_completion(
                session,
                prompt,
                &turn_id,
                &events,
                false,
                final_text.as_deref(),
            )
        };
        session.completion_record =
            crate::trace::single_completion_record(session, completion_status);
        Ok(TurnOutcome {
            model_calls,
            final_text,
            completion_status,
            reached_model_turn_limit,
            steps,
            events,
            prompt: prompt.to_string(),
            started_at,
            duration_ms: started.elapsed().as_millis() as u64,
            usage,
            usage_known,
            phase_timings,
            tools_fingerprint: schema_report.fingerprint,
        })
    }
}
