use super::*;

#[test]
fn an_uncapped_parent_still_gives_nested_workers_a_finite_round_budget() {
    assert_eq!(
        super::nested_turn_cap(0),
        crate::subagent::MAX_SUBAGENT_TURNS
    );
    assert_eq!(super::nested_turn_cap(5), 5);
    assert_eq!(
        super::nested_turn_cap(usize::MAX),
        crate::subagent::MAX_SUBAGENT_TURNS
    );
}

#[tokio::test]
async fn default_user_turn_can_run_more_than_sixty_five_model_tool_rounds() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    registry.register(ProbeTool {
        label: "probe_a",
        delay_ms: 0,
        class: EffectClass::Read,
        host_verified: true,
        state: Arc::clone(&state),
    });
    let mut outputs = VecDeque::new();
    for index in 0..65 {
        outputs.push_back(ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
            id: format!("call-{index}"),
            name: "probe_a".to_string(),
            arguments: serde_json::json!({ "path": format!("item-{index}.txt") }),
        }]));
    }
    outputs.push_back(ModelOutput::Text("全部完成".to_string()));
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider {
            outputs: Mutex::new(outputs),
        }),
        registry,
        Policy::new("."),
        AgentConfig::default(),
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "连续执行较长任务",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("默认用户回合不应在第 60 轮或第 64 个工具调用处提前停止");

    assert_eq!(outcome.final_text.as_deref(), Some("全部完成"));
    assert!(!outcome.reached_model_turn_limit);
    assert_eq!(state.completed.lock().unwrap().len(), 65);
}

#[tokio::test]
async fn failed_host_validation_is_fed_back_and_repaired_before_final() {
    let workspace = tempfile::tempdir().unwrap();
    let call = |id: &str, name: &str, arguments: serde_json::Value| crate::gateway::ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
    };
    let outputs = Mutex::new(VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![call(
            "plan",
            "verification_plan",
            serde_json::json!({
                "plan": {
                    "plan_id": "readme-acceptance",
                    "requirements": [{
                        "requirement_id": "readme-has-acceptance",
                        "covers_requirement_ids": ["user-request:包含验收通过"],
                        "validator_id": "workspace-file-contains-v1",
                        "validator_version": "1",
                        "scope": {
                            "kind": "workspace_paths",
                            "relative_paths": ["README.md"]
                        },
                        "arguments": {"text": "验收通过"},
                        "required": true,
                        "resources": {
                            "cpu_slots": 1,
                            "memory_mb": 16,
                            "exclusive_workspace": false,
                            "timeout_ms": 10000
                        }
                    }]
                }
            }),
        )]),
        ModelOutput::ToolCalls(vec![call(
            "write-first",
            "write_file",
            serde_json::json!({"path":"README.md","content":"第一版"}),
        )]),
        ModelOutput::Text("第一版已经完成。".to_string()),
        ModelOutput::ToolCalls(vec![call(
            "write-repair",
            "write_file",
            serde_json::json!({"path":"README.md","content":"验收通过"}),
        )]),
        ModelOutput::Text("修复后的版本已通过宿主验收。".to_string()),
    ]));
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider { outputs }),
        ToolRegistry::new(),
        Policy::new(workspace.path()),
        AgentConfig::default(),
    );
    let mut session = Session::new(workspace.path(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "修改 README 并包含验收通过",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("宿主失败反馈后应允许模型在同一回合修复");

    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Accepted
    );
    assert_eq!(
        outcome.final_text.as_deref(),
        Some("修复后的版本已通过宿主验收。")
    );
    let final_events = outcome
        .events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::Final { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(final_events, vec!["修复后的版本已通过宿主验收。"]);
    assert!(session.messages.iter().any(|message| {
        message.role == "system"
            && message.content.as_deref().is_some_and(|content| {
                content.contains("宿主已按本回合冻结的 VerificationPlan")
                    && content.contains("Failed")
            })
    }));
    assert!(session
        .validation_receipts
        .iter()
        .any(|receipt| receipt.verdict == crate::plan::ValidationVerdictV1::Failed));
    let epochs = session
        .validation_receipts
        .iter()
        .map(|receipt| receipt.epoch)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        epochs,
        std::collections::BTreeSet::from([1, 2]),
        "每次宿主重验必须推进验证 epoch，且同一轮的回执共享 epoch"
    );
    assert_eq!(session.execution_receipts[0].status, "stale");
    assert_eq!(session.execution_receipts[1].status, "accepted");
}

#[tokio::test]
async fn host_validation_repair_rounds_are_bounded_even_when_fingerprints_change() {
    // 每次写入内容不同 → 验收失败的 subject hash / 指纹都不同，模拟"修一次、败一次"
    // 永不重复的修复循环；回合必须在 MAX_HOST_VALIDATION_REPAIR_ROUNDS 后收敛。
    let workspace = tempfile::tempdir().unwrap();
    let call = |id: &str, name: &str, arguments: serde_json::Value| crate::gateway::ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
    };
    let mut script: Vec<ModelOutput> = vec![ModelOutput::ToolCalls(vec![call(
        "plan",
        "verification_plan",
        serde_json::json!({
            "plan": {
                "plan_id": "bounded-repair",
                "requirements": [{
                    "requirement_id": "readme-ok",
                    "covers_requirement_ids": ["user-request:包含验收通过"],
                    "validator_id": "workspace-file-contains-v1",
                    "validator_version": "1",
                    "scope": {"kind": "workspace_paths", "relative_paths": ["README.md"]},
                    "arguments": {"text": "验收通过"},
                    "required": true,
                    "resources": {
                        "cpu_slots": 1,
                        "memory_mb": 16,
                        "exclusive_workspace": false,
                        "timeout_ms": 10000
                    }
                }]
            }
        }),
    )])];
    for round in 0..=MAX_HOST_VALIDATION_REPAIR_ROUNDS {
        script.push(ModelOutput::ToolCalls(vec![call(
            &format!("write-{round}"),
            "write_file",
            serde_json::json!({"path":"README.md","content":format!("第{round}版")}),
        )]));
        script.push(ModelOutput::Text(format!("第{round}版完成")));
    }
    // 若上限失效，队列耗尽会以 Provider 错误结束，断言就不会看到收敛的 final。
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider {
            outputs: Mutex::new(VecDeque::from(script)),
        }),
        ToolRegistry::new(),
        Policy::new(workspace.path()),
        AgentConfig::default(),
    );
    let mut session = Session::new(workspace.path(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "修改 README 并包含验收通过",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("修复轮次耗尽后必须收敛为可见结果，而不是继续请求模型");

    assert!(
        matches!(
            outcome.completion_status,
            owo_agent_protocol::CompletionStatusV1::Unverified
                | owo_agent_protocol::CompletionStatusV1::Blocked
        ),
        "未通过宿主验收的候选必须如实标为未验证/阻断：{:?}",
        outcome.completion_status
    );
    assert_eq!(
        outcome.final_text.as_deref().map(|text| text.to_string()),
        Some(format!("第{MAX_HOST_VALIDATION_REPAIR_ROUNDS}版完成")),
        "达到修复上限时应交付最后一版可见结论"
    );
    let feedback_rounds = session
        .messages
        .iter()
        .filter(|message| {
            message.role == "system"
                && message.content.as_deref().is_some_and(|content| {
                    content.contains("宿主已按本回合冻结的 VerificationPlan")
                })
        })
        .count();
    assert_eq!(
        feedback_rounds, MAX_HOST_VALIDATION_REPAIR_ROUNDS,
        "恰好经历 MAX 次宿主反馈；第 MAX+1 次验收失败直接收口，不再追加反馈"
    );
}

#[test]
fn default_turn_and_tool_call_limits_allow_the_model_to_finish_naturally() {
    let config = AgentConfig::default();

    assert_eq!(config.max_turns, 0, "zero means no hidden model-round cap");
    assert_eq!(
        config.max_tool_calls_per_turn, 0,
        "zero means no hidden tool-call cap"
    );
}

#[test]
fn stale_receipts_from_prior_turns_do_not_change_plain_answer_completion() {
    let mut session = Session::new(
        std::env::current_dir().expect("workspace path"),
        "test-model",
        None,
    );
    session
        .execution_receipts
        .push(crate::session::ExecutionReceipt {
            receipt_id: "exec-old".to_string(),
            tool: "write_file".to_string(),
            turn_id: "prior-turn".to_string(),
            changed_files: vec!["src/old.rs".to_string()],
            snapshot_keys: Default::default(),
            before_hashes: Default::default(),
            after_hashes: Default::default(),
            diff_sha256: "old-diff".to_string(),
            created_at: "2026-10-01T00:00:00Z".to_string(),
            status: "stale".to_string(),
            validation_receipt_id: Some("old-validation".to_string()),
        });

    let status = super::assess_single_turn_completion(
        &mut session,
        "解释一下这个模块",
        "current-turn",
        &[],
        false,
        Some("这个模块负责会话状态管理。"),
    );

    assert_eq!(
        status,
        owo_agent_protocol::CompletionStatusV1::ResponseComplete
    );
}

/// 子代理 / Team Worker（`subagent_depth > 0`）不进入 Single 验收门控：
/// 写源码后不应收到"调用 verification_plan"的宿主反馈（其工具面没有该工具，
/// 模型只能报 blocked 导致整步失败）；完成语义由 WorkerOutputV1 契约承担。
#[tokio::test]
async fn subagent_turns_skip_single_completion_gate() {
    let workspace = tempfile::tempdir().unwrap();
    let call = |id: &str, name: &str, arguments: serde_json::Value| crate::gateway::ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
    };
    let script = vec![
        ModelOutput::ToolCalls(vec![call(
            "write",
            "write_file",
            serde_json::json!({
                "path": "src/calc.py",
                "content": "def divide(a, b):\n    return a / b\n"
            }),
        )]),
        ModelOutput::Text("{\"status\":\"done\"}".to_string()),
    ];
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider {
            outputs: Mutex::new(VecDeque::from(script)),
        }),
        ToolRegistry::new(),
        Policy::new(workspace.path()),
        AgentConfig {
            subagent_depth: 1,
            ..Default::default()
        },
    );
    let mut session = Session::new(workspace.path(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "修复 divide",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("子代理回合应正常结束");

    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Accepted,
        "子代理不应被 Single 验收标为未验证"
    );
    assert!(
        session.messages.iter().all(|message| {
            !message
                .content
                .as_deref()
                .is_some_and(|content| content.contains("verification_plan"))
        }),
        "子代理不应收到要求登记 VerificationPlan 的宿主反馈"
    );
}

#[test]
fn estimate_tokens_counts_chars_and_overhead() {
    let messages = vec![
        ChatMessage::system("规则".to_string()),
        ChatMessage::user("你好，请帮我总结这段代码".to_string()),
        ChatMessage::assistant_text("好的。".to_string()),
    ];
    let total = estimate_tokens(&messages);
    // P1-1 口径（取优合并自远端 engine）：中文 ≈1 token/字（cl100k 实测区间），
    // 3 条共 17 字 + 12 开销；旧公式「字符数/2」会给出 ~20 的低估。
    assert!(
        (22..=45).contains(&total),
        "估算 token {total} 应在中文真实区间（旧公式低估）"
    );
}

/// 取优合并（远端 engine）：切点落在 tool 群组内时对齐 assistant(tool_calls)，
/// 群组前无调用则跳过孤儿群组（避免保留段以孤立 tool 开头被模型拒 400）。
#[test]
fn align_keep_start_pulls_in_tool_call_message() {
    let messages = vec![
        ChatMessage::system("系统".to_string()),
        ChatMessage::user("请求".to_string()),
        ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
            id: "c1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({ "path": "a.txt" }),
        }]),
        ChatMessage::tool("c1".to_string(), "结果一".to_string()),
        ChatMessage::user("继续".to_string()),
    ];
    // 切点在 tool 上：回退到 assistant。
    assert_eq!(align_keep_start(&messages, 3), 2);
    // 切点在群组之外：不动。
    assert_eq!(align_keep_start(&messages, 4), 4);
    // 群组前无 assistant(tool_calls)（脏历史）：跳过整个群组。
    let orphan = vec![
        ChatMessage::system("系统".to_string()),
        ChatMessage::user("请求".to_string()),
        ChatMessage::tool("孤儿".to_string(), "结果".to_string()),
        ChatMessage::user("继续".to_string()),
    ];
    assert_eq!(align_keep_start(&orphan, 2), 3);
}

/// 回归：**摘要压缩的切点也必须对齐 tool 群组**。
///
/// 线上事故：跑了 33 个工具的回合，压缩提示刚出现，下一轮请求整体失败——
/// `Messages with role 'tool' must be a response to a preceding message with
/// 'tool_calls'`。根因是 `maybe_compact` 用 `messages[head_end..]` 直接切片，
/// 而 `keep_recent` 是按条数切的，切点落进 assistant(tool_calls) 与它的 tool
/// 结果之间就切出了孤立的 tool 开头。同文件另两条裁剪路径都做了对齐，只有这条漏了。
#[test]
fn compaction_split_never_starts_tail_with_orphan_tool() {
    // 复刻现场形状：大量「user → assistant(tool_calls) → tool」交错。
    let mut messages = vec![ChatMessage::system("系统".to_string())];
    for index in 0..20 {
        messages.push(ChatMessage::user(format!("请求{index}")));
        let call_id = format!("c{index}");
        messages.push(ChatMessage::assistant_tool_calls(vec![
            crate::gateway::ToolCall {
                id: call_id.clone(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({ "path": "a.txt" }),
            },
        ]));
        messages.push(ChatMessage::tool(call_id, format!("结果{index}")));
    }
    let mut checked = 0;
    for keep_recent in 1..=messages.len() {
        let Some(tail_start) = compaction_split(&messages, keep_recent) else {
            continue;
        };
        checked += 1;
        // 1) 保留段绝不能以孤立 tool 开头（否则模型 400）。
        assert_ne!(
            messages[tail_start].role, "tool",
            "keep_recent={keep_recent} 切出了孤立 tool 开头的保留段（tail_start={tail_start}）"
        );
        // 2) head 与 tail 必须互补，不得静默丢历史。
        let head_len = tail_start - 1; // messages[1..tail_start]
        let tail_len = messages.len() - tail_start;
        assert_eq!(
            head_len + tail_len + 1,
            messages.len(),
            "keep_recent={keep_recent} 时 head/tail 不互补，历史被静默丢弃"
        );
    }
    assert!(checked > 0, "所有 keep_recent 都拒绝压缩，测试等于没跑");
}

/// 切点恰好落在 assistant(tool_calls) 自身时也不动它（群组头本来就是合法起点）。
#[test]
fn compaction_split_keeps_assistant_tool_calls_boundary() {
    let messages = vec![
        ChatMessage::system("系统".to_string()),
        ChatMessage::user("请求一".to_string()),
        ChatMessage::assistant_text("好的。".to_string()),
        ChatMessage::user("请求二".to_string()),
        ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
            id: "c1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({ "path": "a.txt" }),
        }]),
        ChatMessage::tool("c1".to_string(), "结果".to_string()),
        ChatMessage::assistant_text("完成。".to_string()),
        ChatMessage::user("谢谢".to_string()),
    ];
    // keep_recent=4 → head_end=4，正好是 assistant(tool_calls)：原样保留。
    assert_eq!(compaction_split(&messages, 4), Some(4));
    // keep_recent=3 → head_end=5 落在 tool 上：回退到群组头 4。
    assert_eq!(compaction_split(&messages, 3), Some(4));
}

/// 存量脏历史归一：丢孤立 tool、补缺失结果且紧跟调用。
#[test]
fn sanitize_history_drops_orphan_tool_and_fills_missing_results() {
    let mut messages = vec![
        ChatMessage::system("系统".to_string()),
        ChatMessage::tool("orphan-1".to_string(), "孤儿结果".to_string()),
        ChatMessage::user("请求".to_string()),
        ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
            id: "keep-1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({ "path": "a.txt" }),
        }]),
        ChatMessage::tool("keep-1".to_string(), "正常结果".to_string()),
        ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
            id: "lost-1".to_string(),
            name: "write_file".to_string(),
            arguments: serde_json::json!({ "path": "b.txt" }),
        }]),
        ChatMessage::user("新回合".to_string()),
    ];
    sanitize_history(&mut messages);
    assert_eq!(messages[0].role, "system");
    assert!(!messages
        .iter()
        .any(|message| message.tool_call_id.as_deref() == Some("orphan-1")));
    assert!(messages
        .iter()
        .any(|message| message.tool_call_id.as_deref() == Some("keep-1")));
    let lost_index = messages
        .iter()
        .position(|message| {
            message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| calls.iter().any(|call| call.id == "lost-1"))
        })
        .expect("lost-1 的 assistant 消息应保留");
    assert_eq!(messages[lost_index + 1].role, "tool");
    assert_eq!(
        messages[lost_index + 1].tool_call_id.as_deref(),
        Some("lost-1")
    );
}

/// 取优合并（远端 engine）：token 预算硬裁剪保留 system + 最近 tail。
#[test]
fn compact_truncate_to_budget_keeps_system_and_tail() {
    let mut messages = vec![ChatMessage::system("系统".to_string())];
    for index in 0..40 {
        messages.push(ChatMessage::user(format!(
            "历史消息 {index} {}",
            "内容".repeat(200)
        )));
    }
    compact_truncate_to_budget(&mut messages, 2_000);
    assert_eq!(messages[0].role, "system");
    assert!(estimate_tokens(&messages) <= 2_000, "应压回预算内");
    assert!(messages.len() < 41, "必须裁掉多数历史");
    assert!(
        messages
            .last()
            .unwrap()
            .content
            .as_deref()
            .unwrap_or("")
            .contains("历史消息 39"),
        "必须保留最新消息"
    );
}

#[test]
fn empty_messages_cost_zero() {
    assert_eq!(estimate_tokens(&[]), 0);
}

#[test]
fn compact_truncate_keeps_system_and_recent_tail() {
    let mut messages = vec![ChatMessage::system("系统".to_string())];
    for index in 0..10 {
        messages.push(ChatMessage::user(format!("消息{index}")));
    }
    compact_truncate(&mut messages, 4);
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, "system");
    assert!(messages
        .iter()
        .any(|message| message.content.as_deref() == Some("消息9")));
    assert!(messages
        .iter()
        .any(|message| message.content.as_deref() == Some("消息7")));
}

#[test]
fn compact_truncate_keeps_tool_call_and_results_together() {
    let mut messages = vec![
        ChatMessage::system("系统".to_string()),
        ChatMessage::user("旧请求".to_string()),
        ChatMessage::assistant_tool_calls(vec![crate::gateway::ToolCall {
            id: "call-1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({ "path": "a.txt" }),
        }]),
        ChatMessage::tool("call-1".to_string(), "结果".to_string()),
        ChatMessage::user("继续".to_string()),
        ChatMessage::assistant_text("好的".to_string()),
    ];

    compact_truncate(&mut messages, 4);

    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "assistant");
    assert!(messages[1].tool_calls.is_some());
    assert_eq!(messages[2].role, "tool");
    assert_eq!(messages[2].tool_call_id.as_deref(), Some("call-1"));
}

#[test]
fn tool_result_is_bounded_without_splitting_unicode() {
    let result = truncate_tool_result(&"中".repeat(10), 3);
    assert!(result.starts_with("中中中"));
    assert!(result.contains("工具输出已截断"));
}

// ---------- §9.1 安全并发只读工具 ----------

use crate::tools::{Tool, ToolSpec};
use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;

/// §9.1 共享探针状态：活跃数峰值（并发观测）+ 完成名单（顺序/取消观测）。
struct ProbeState {
    active: AtomicUsize,
    peak: Mutex<usize>,
    completed: Mutex<Vec<String>>,
}

impl ProbeState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            active: AtomicUsize::new(0),
            peak: Mutex::new(0),
            completed: Mutex::new(Vec::new()),
        })
    }

    fn begin(&self) {
        let current = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        let mut peak = self.peak.lock().unwrap();
        if current > *peak {
            *peak = current;
        }
    }

    fn end(&self, label: &str) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.completed.lock().unwrap().push(label.to_string());
    }
}

/// effect 可配置的探针工具（验证只读 / 自报只读 / 写），供分组断言。
struct ProbeTool {
    label: &'static str,
    delay_ms: u64,
    class: EffectClass,
    host_verified: bool,
    state: Arc<ProbeState>,
}

#[async_trait::async_trait]
impl Tool for ProbeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::with_effect(
            self.label,
            "并发探针".to_string(),
            serde_json::json!({ "type": "object" }),
            Some(crate::tool_effects::ToolEffect {
                tool: self.label.to_string(),
                class: self.class,
                source: "builtin".to_string(),
                risk_note: None,
                annotations: None,
                host_verified_readonly: self.host_verified,
            }),
        )
    }

    async fn run(
        &self,
        _ctx: &mut ToolContext<'_>,
        _args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.state.begin();
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        self.state.end(self.label);
        Ok(serde_json::json!({ "tool": self.label, "delay_ms": self.delay_ms }))
    }
}

/// 固定脚本 provider：按序吐出预置输出（工具调用轮 + 文本轮）。
struct ScriptedTestProvider {
    outputs: Mutex<VecDeque<ModelOutput>>,
}

#[async_trait::async_trait]
impl ModelProvider for ScriptedTestProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.outputs
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| "脚本输出已耗尽".to_string())
    }
}

/// 两个探针调用的脚本：第一轮按给定顺序发两个 tool-call，随后文本收尾。
fn two_call_then_text(first: &str, second: &str) -> Mutex<VecDeque<ModelOutput>> {
    Mutex::new(VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![
            crate::gateway::ToolCall {
                id: format!("call-{first}"),
                name: first.to_string(),
                arguments: serde_json::json!({}),
            },
            crate::gateway::ToolCall {
                id: format!("call-{second}"),
                name: second.to_string(),
                arguments: serde_json::json!({}),
            },
        ]),
        ModelOutput::Text("完成".to_string()),
    ]))
}

async fn run_with(
    registry: ToolRegistry,
    outputs: Mutex<VecDeque<ModelOutput>>,
    abort: &AtomicBool,
) -> Result<(TurnOutcome, Session), AgentError> {
    let provider = Arc::new(ScriptedTestProvider { outputs });
    let agent = Agent::new(
        provider,
        registry,
        Policy::new("."),
        AgentConfig {
            max_turns: 4,
            ..Default::default()
        },
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(&mut session, "跑探针", &approver, abort, &mut |_| {})
        .await?;
    Ok((outcome, session))
}

#[tokio::test]
async fn two_verified_read_tools_execute_concurrently() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    for label in ["probe_a", "probe_b"] {
        registry.register(ProbeTool {
            label,
            delay_ms: 120,
            class: EffectClass::Read,
            host_verified: true,
            state: Arc::clone(&state),
        });
    }
    let (outcome, _) = run_with(
        registry,
        two_call_then_text("probe_a", "probe_b"),
        &AtomicBool::new(false),
    )
    .await
    .expect("回合应成功");
    assert_eq!(outcome.final_text.as_deref(), Some("完成"));
    assert_eq!(state.completed.lock().unwrap().len(), 2, "两个工具都应完成");
    assert!(
        *state.peak.lock().unwrap() >= 2,
        "两个宿主验证只读工具必须并发（活跃峰值 ≥2）"
    );
}

/// 远端 agent.rs 取优：`ToolResult` 事件带结果预览（截断，供步骤时间线 chip 展开）。
#[tokio::test]
async fn tool_result_events_carry_preview() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    for label in ["probe_a", "probe_b"] {
        registry.register(ProbeTool {
            label,
            delay_ms: 0,
            class: EffectClass::Read,
            host_verified: true,
            state: Arc::clone(&state),
        });
    }
    let (outcome, _) = run_with(
        registry,
        two_call_then_text("probe_a", "probe_b"),
        &AtomicBool::new(false),
    )
    .await
    .expect("回合应成功");
    let previews: Vec<(String, String)> = outcome
        .events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ToolResult {
                tool,
                preview: Some(preview),
                ..
            } => Some((tool.clone(), preview.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(previews.len(), 2, "两个工具结果都应带预览：{previews:?}");
    assert!(
        previews
            .iter()
            .any(|(_, preview)| preview.contains("probe_a")),
        "预览应含工具结果正文：{previews:?}"
    );
}

/// M4.2：`session.model_override` 是请求级路由真相——显式覆盖进 wire，
/// `"default"` 哨兵清除后透传 None（Provider 解析链）；展示模型与 wire 解耦。
#[tokio::test]
async fn session_model_override_reaches_wire_model() {
    struct WireRecorder {
        seen: Mutex<Vec<Option<String>>>,
    }
    #[async_trait::async_trait]
    impl ModelProvider for WireRecorder {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            Err("本测试只走流式路径".to_string())
        }
        async fn complete_stream_with_model(
            &self,
            model: Option<&str>,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
            on_delta: &mut (dyn FnMut(String) + Send),
        ) -> Result<ModelOutput, String> {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(model.map(str::to_string));
            on_delta("完成".to_string());
            Ok(ModelOutput::Text("完成".to_string()))
        }
    }
    let recorder = Arc::new(WireRecorder {
        seen: Mutex::new(Vec::new()),
    });
    let agent = Agent::new(
        recorder.clone() as Arc<dyn ModelProvider>,
        ToolRegistry::new(),
        Policy::new("."),
        AgentConfig {
            max_turns: 4,
            ..Default::default()
        },
    );
    let approver = crate::permissions::AutoApprover { allow: true };
    let mut session = Session::new(std::env::temp_dir(), "display-model", None)
        .with_model_override(Some("wire-x".to_string()));
    agent
        .run_turn(
            &mut session,
            "你好",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("回合应成功");
    // 展示值不受路由覆盖影响。
    assert_eq!(session.model, "display-model");
    // "default" 哨兵 = 清除覆盖 → 下一回合透传 None（回退 Provider 链）。
    session.set_model_override(Some("default".to_string()));
    agent
        .run_turn(
            &mut session,
            "再来",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("第二回合应成功");
    let seen = recorder
        .seen
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        seen.as_slice(),
        &[Some("wire-x".to_string()), None],
        "覆盖必须进 wire；清除后必须回退 Provider 解析链"
    );
}

/// §9.3 瀑布：model 阶段记录首 token 时延（流式增量首达时刻 ≤ 总耗时）。
#[tokio::test]
async fn model_phase_timing_carries_first_token_latency() {
    let (outcome, _) = run_with(
        ToolRegistry::new(),
        Mutex::new(VecDeque::from(vec![ModelOutput::Text("完成".to_string())])),
        &AtomicBool::new(false),
    )
    .await
    .expect("回合应成功");
    let model = outcome
        .phase_timings
        .iter()
        .find(|timing| timing.phase == "model")
        .expect("应有 model 阶段瀑布记录");
    assert!(
        model.first_token_ms.is_some(),
        "§9.3：model 阶段应记录首 token 时延（默认流式包装会发首个增量）"
    );
    assert!(
        model.first_token_ms.unwrap() <= model.elapsed_ms,
        "首 token 时延不得超过该阶段总耗时"
    );
}

#[tokio::test]
async fn concurrent_group_results_backfill_in_original_call_order() {
    let mut registry = ToolRegistry::new();
    registry.register(ProbeTool {
        label: "probe_slow",
        delay_ms: 150,
        class: EffectClass::Read,
        host_verified: true,
        state: ProbeState::new(),
    });
    registry.register(ProbeTool {
        label: "probe_fast",
        delay_ms: 10,
        class: EffectClass::Read,
        host_verified: true,
        state: ProbeState::new(),
    });
    let (_, session) = run_with(
        registry,
        two_call_then_text("probe_slow", "probe_fast"),
        &AtomicBool::new(false),
    )
    .await
    .expect("回合应成功");
    // 快工具先完成，但 tool 消息必须仍按原始 tool-call 顺序回填。
    let tool_ids: Vec<&str> = session
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .filter_map(|message| message.tool_call_id.as_deref())
        .collect();
    assert_eq!(
        tool_ids,
        vec!["call-probe_slow", "call-probe_fast"],
        "结果必须按原 tool-call 顺序回填"
    );
    let slow_content = session
        .messages
        .iter()
        .find(|message| {
            message.role == "tool" && message.tool_call_id.as_deref() == Some("call-probe_slow")
        })
        .and_then(|message| message.content.as_deref())
        .unwrap_or_default();
    assert!(slow_content.contains("probe_slow"), "慢工具结果不得串位");
}

#[test]
fn write_unverified_and_unknown_tools_are_not_concurrent_eligible() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    for (label, class, verified) in [
        ("probe_read_verified", EffectClass::Read, true),
        ("probe_read_self_reported", EffectClass::Read, false),
        ("probe_write", EffectClass::Write, true),
    ] {
        registry.register(ProbeTool {
            label,
            delay_ms: 0,
            class,
            host_verified: verified,
            state: Arc::clone(&state),
        });
    }
    let provider = Arc::new(ScriptedTestProvider {
        outputs: Mutex::new(VecDeque::new()),
    });
    let agent = Agent::new(provider, registry, Policy::new("."), AgentConfig::default());
    let call = |name: &str| crate::gateway::ToolCall {
        id: "x".to_string(),
        name: name.to_string(),
        arguments: serde_json::json!({}),
    };
    assert!(agent.call_is_concurrent_eligible(&call("probe_read_verified")));
    // MCP 自报 readOnlyHint 但宿主未验证 → 不得并发（保持串行）。
    assert!(!agent.call_is_concurrent_eligible(&call("probe_read_self_reported")));
    // 写工具绝不可能进并发组（同路径读写不并发由分组排除保证）。
    assert!(!agent.call_is_concurrent_eligible(&call("probe_write")));
    // 未知工具 → 串行错误路径。
    assert!(!agent.call_is_concurrent_eligible(&call("__missing__")));
}

#[tokio::test]
async fn abort_during_concurrent_group_cancels_pending_tools() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    for label in ["probe_a", "probe_b"] {
        registry.register(ProbeTool {
            label,
            delay_ms: 400,
            class: EffectClass::Read,
            host_verified: true,
            state: Arc::clone(&state),
        });
    }
    let abort = Arc::new(AtomicBool::new(false));
    {
        let abort = Arc::clone(&abort);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            abort.store(true, Ordering::SeqCst);
        });
    }
    let result = run_with(registry, two_call_then_text("probe_a", "probe_b"), &abort).await;
    assert!(
        matches!(result, Err(AgentError::Aborted)),
        "组执行中途 abort 应返回 Aborted：{result:?}"
    );
    assert!(
        state.completed.lock().unwrap().is_empty(),
        "取消后不得有任何工具完成（无后台残留）"
    );
}

// ---------- §9.3 超大工具结果 artifact 落盘 ----------

/// §9.3：输出超大结果的探针工具（宿主验证只读）。
struct BigOutputTool;

#[async_trait::async_trait]
impl Tool for BigOutputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::with_effect(
            "probe_big_output",
            "超大输出探针".to_string(),
            serde_json::json!({ "type": "object" }),
            Some(crate::tool_effects::ToolEffect {
                tool: "probe_big_output".to_string(),
                class: EffectClass::Read,
                source: "builtin".to_string(),
                risk_note: None,
                annotations: None,
                host_verified_readonly: true,
            }),
        )
    }

    async fn run(
        &self,
        _ctx: &mut ToolContext<'_>,
        _args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "blob": "x".repeat(60_000) }))
    }
}

fn big_output_script() -> Mutex<VecDeque<ModelOutput>> {
    Mutex::new(VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
            id: "call-big".to_string(),
            name: "probe_big_output".to_string(),
            arguments: serde_json::json!({}),
        }]),
        ModelOutput::Text("完成".to_string()),
    ]))
}

fn first_tool_content(session: &Session) -> String {
    session
        .messages
        .iter()
        .find(|message| message.role == "tool")
        .and_then(|message| message.content.clone())
        .expect("应有 tool 消息")
}

#[tokio::test]
async fn oversized_tool_result_stored_as_artifact_pointer() {
    let mut registry = ToolRegistry::new();
    registry.register(BigOutputTool);
    let provider = Arc::new(ScriptedTestProvider {
        outputs: big_output_script(),
    });
    let artifact_dir =
        std::env::temp_dir().join(format!("owo-artifact-test-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(crate::cas_store::CasStore::new(artifact_dir).expect("CAS 初始化应成功"));
    let agent = Agent::new(provider, registry, Policy::new("."), AgentConfig::default())
        .with_artifact_store(Arc::clone(&store));
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "大输出",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("回合应成功");
    assert_eq!(outcome.final_text.as_deref(), Some("完成"));

    let tool_message = first_tool_content(&session);
    let envelope: serde_json::Value =
        serde_json::from_str(&tool_message).expect("应为 artifact JSON 信封");
    let artifact = &envelope["artifact"];
    let artifact_ref = artifact["ref"].as_str().expect("应有 ref").to_string();
    assert_eq!(artifact["mime"].as_str(), Some("text/plain"));
    assert!(
        artifact["size_bytes"].as_u64().unwrap_or(0) > 50_000,
        "应记录原始大小"
    );
    assert!(
        artifact["truncation_reason"]
            .as_str()
            .unwrap_or_default()
            .contains("artifact"),
        "应记录截断原因"
    );
    assert!(
        tool_message.chars().count() < 55_000,
        "回填内容应为指针+预览，而非全量原文"
    );
    // CAS 往返：按 ref 可取回完整原文。
    let restored = store.get_text(&artifact_ref).expect("CAS 应含完整结果");
    assert!(restored.len() > 50_000, "完整原文应落盘");
    assert!(restored.contains("xxxxxxxxxx"));
}

#[tokio::test]
async fn without_artifact_store_truncation_keeps_legacy_text() {
    let mut registry = ToolRegistry::new();
    registry.register(BigOutputTool);
    let provider = Arc::new(ScriptedTestProvider {
        outputs: big_output_script(),
    });
    let agent = Agent::new(provider, registry, Policy::new("."), AgentConfig::default());
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    agent
        .run_turn(
            &mut session,
            "大输出",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("回合应成功");
    let tool_message = first_tool_content(&session);
    assert!(
        tool_message.contains("工具输出已截断"),
        "未挂载 store 时维持盲截断旧行为"
    );
    assert!(
        !tool_message.contains("\"artifact\""),
        "未挂载 store 时不得产生 artifact 信封"
    );
}

/// 循环保护契约（对标 Codex/OpenCode）：弱模型反复发起**完全相同**的工具调用时，
/// 宿主必须拦截（不执行），并最终以可读原因结束回合，而不是放大成几百次执行。
#[tokio::test]
async fn repeated_identical_tool_call_is_loop_guarded() {
    struct RepeatingProvider {
        call: crate::gateway::ToolCall,
    }
    #[async_trait::async_trait]
    impl ModelProvider for RepeatingProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            Ok(ModelOutput::ToolCalls(vec![self.call.clone()]))
        }
    }

    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    registry.register(ProbeTool {
        label: "probe_a",
        delay_ms: 0,
        class: EffectClass::Read,
        host_verified: true,
        state: Arc::clone(&state),
    });
    let provider = Arc::new(RepeatingProvider {
        call: crate::gateway::ToolCall {
            id: "call-fixed".to_string(),
            name: "probe_a".to_string(),
            arguments: serde_json::json!({ "path": "same.txt" }),
        },
    });
    let agent = Agent::new(
        provider,
        registry,
        Policy::new("."),
        AgentConfig {
            max_turns: 30,
            max_repeated_tool_calls: 2,
            max_tool_calls_per_turn: 5,
            ..Default::default()
        },
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let error = agent
        .run_turn(
            &mut session,
            "死循环",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect_err("应触发循环保护");
    let message = error.to_string();
    assert!(
        message.contains("循环保护"),
        "错误应说明循环保护：{message}"
    );
    assert_eq!(
        state.completed.lock().unwrap().len(),
        2,
        "重复调用只允许执行 max_repeated_tool_calls 次，其余必须被拦截"
    );
}

#[tokio::test]
async fn successful_write_allows_repeating_the_same_validation_command() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    registry.register(ProbeTool {
        label: "probe_test",
        delay_ms: 0,
        class: EffectClass::Read,
        host_verified: true,
        state: Arc::clone(&state),
    });
    registry.register(ProbeTool {
        label: "probe_write",
        delay_ms: 0,
        class: EffectClass::Write,
        host_verified: true,
        state: Arc::clone(&state),
    });
    let repeated_test = |id: &str| crate::gateway::ToolCall {
        id: id.to_string(),
        name: "probe_test".to_string(),
        arguments: serde_json::json!({"command":"cargo test --test feature"}),
    };
    let outputs = Mutex::new(VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![repeated_test("test-1")]),
        ModelOutput::ToolCalls(vec![repeated_test("test-2")]),
        ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
            id: "write".to_string(),
            name: "probe_write".to_string(),
            arguments: serde_json::json!({"path":"src/lib.rs","content":"fixed"}),
        }]),
        ModelOutput::ToolCalls(vec![repeated_test("test-3")]),
        ModelOutput::ToolCalls(vec![repeated_test("test-4")]),
        ModelOutput::Text("修复后测试通过".to_string()),
    ]));
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider { outputs }),
        registry,
        Policy::new("."),
        AgentConfig {
            max_turns: 10,
            max_tool_calls_per_turn: 10,
            max_repeated_tool_calls: 2,
            ..Default::default()
        },
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "修改后重新运行相同测试",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("成功写入后应允许重新运行相同验收命令");

    assert_eq!(outcome.final_text.as_deref(), Some("修复后测试通过"));
    assert_eq!(state.completed.lock().unwrap().len(), 5);
}

/// 取优合并（远端 engine）：思考通道增量以 ReasoningDelta 事件外发。
struct ReasoningProvider;

#[async_trait::async_trait]
impl crate::gateway::ModelProvider for ReasoningProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        Ok(ModelOutput::Text("答复".to_string()))
    }

    async fn complete_stream_with_reasoning_and_model(
        &self,
        _model: Option<&str>,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        on_chunk(StreamChunk::Reasoning("先想一步。".to_string()));
        on_chunk(StreamChunk::Content("答复".to_string()));
        Ok(ModelOutput::Text("答复".to_string()))
    }
}

struct RequestUsageProvider;

#[async_trait::async_trait]
impl crate::gateway::ModelProvider for RequestUsageProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        unreachable!("Agent should use the observed streaming interface")
    }

    async fn complete_stream_with_reasoning_and_model_observed(
        &self,
        _model: Option<&str>,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
        _on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<crate::gateway::ObservedModelOutput, String> {
        let user_text = messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .and_then(|message| message.content.as_deref())
            .unwrap_or_default();
        let tokens = if user_text == "worker-a" { 11 } else { 101 };
        Ok(crate::gateway::ObservedModelOutput {
            output: ModelOutput::Text("ok".to_string()),
            metadata: crate::gateway::ModelCallMetadata {
                usage: Some(crate::gateway::TokenUsage {
                    prompt_tokens: tokens,
                    completion_tokens: 1,
                    total_tokens: tokens + 1,
                }),
                ..Default::default()
            },
        })
    }
}

#[tokio::test]
async fn concurrent_turns_keep_request_usage_separate() {
    let provider = Arc::new(RequestUsageProvider);
    let make_agent = || {
        Agent::new(
            provider.clone(),
            ToolRegistry::new(),
            Policy::new("."),
            AgentConfig::default(),
        )
    };
    let agent_a = make_agent();
    let agent_b = make_agent();
    let mut session_a = Session::new(std::env::temp_dir(), "test-model", None);
    let mut session_b = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let abort = AtomicBool::new(false);
    let mut sink_a = |_event: &TurnEvent| {};
    let mut sink_b = |_event: &TurnEvent| {};
    let (turn_a, turn_b) = tokio::join!(
        agent_a.run_turn(&mut session_a, "worker-a", &approver, &abort, &mut sink_a),
        agent_b.run_turn(&mut session_b, "worker-b", &approver, &abort, &mut sink_b),
    );
    let turn_a = turn_a.expect("worker A should finish");
    let turn_b = turn_b.expect("worker B should finish");
    assert_eq!(turn_a.usage.total_tokens, 12);
    assert_eq!(turn_b.usage.total_tokens, 102);
    assert!(turn_a.usage_known && turn_b.usage_known);
}

#[tokio::test]
async fn reasoning_chunks_are_emitted_as_events() {
    let agent = Agent::new(
        Arc::new(ReasoningProvider),
        ToolRegistry::new(),
        Policy::new("."),
        AgentConfig::default(),
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "问",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("回合应成功");
    let reasoning: Vec<&str> = outcome
        .events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ReasoningDelta { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning, vec!["先想一步。"]);
    assert_eq!(outcome.final_text.as_deref(), Some("答复"));
}

/// 写计划清单的测试工具（模拟 todo 工具的整表替换语义）。
struct PlanWriteTool;

#[async_trait::async_trait]
impl Tool for PlanWriteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "plan_write".into(),
            description: "写计划".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            effect: None,
        }
    }

    async fn run(
        &self,
        ctx: &mut ToolContext<'_>,
        _args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        ctx.session.todos = vec![crate::session::TodoItem {
            content: "步骤A".to_string(),
            status: "in_progress".to_string(),
        }];
        Ok(serde_json::json!({ "ok": true }))
    }
}

#[tokio::test]
async fn todo_change_emits_plan_update() {
    let mut registry = ToolRegistry::new();
    registry.register(PlanWriteTool);
    let outputs = Mutex::new(VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
            id: "plan-1".to_string(),
            name: "plan_write".to_string(),
            arguments: serde_json::json!({}),
        }]),
        ModelOutput::Text("好了".to_string()),
    ]));
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider { outputs }),
        registry,
        Policy::new("."),
        AgentConfig::default(),
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "写计划",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("回合应成功");
    let plan = outcome
        .events
        .iter()
        .find_map(|event| match event {
            TurnEvent::PlanUpdate { steps } => Some(steps.clone()),
            _ => None,
        })
        .expect("todo 变化应外发 PlanUpdate");
    assert_eq!(plan[0]["content"], serde_json::json!("步骤A"));
    assert_eq!(plan[0]["status"], serde_json::json!("in_progress"));
}

fn blocking_hook(event: &str, matcher: Option<&str>, reason: &str) -> crate::hooks::HookConfig {
    let command = if cfg!(windows) {
        format!("echo {reason} 1>&2 & exit /b 2")
    } else {
        format!("echo {reason} 1>&2; exit 2")
    };
    crate::hooks::HookConfig {
        event: event.to_string(),
        matcher: matcher.map(str::to_string),
        command,
        fail_mode: None,
    }
}

/// A2-1 hooks：UserPromptSubmit exit 2 = 拒绝本回合（stderr 回喂）。
#[tokio::test]
async fn user_prompt_submit_hook_blocks_turn() {
    let outputs = Mutex::new(VecDeque::from(vec![ModelOutput::Text(
        "不应到达".to_string(),
    )]));
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider { outputs }),
        ToolRegistry::new(),
        Policy::new("."),
        AgentConfig::default(),
    );
    agent.set_hooks(crate::hooks::HookManager::from_configs(&[blocking_hook(
        "user_prompt_submit",
        None,
        "敏感词门卫",
    )]));
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let error = agent
        .run_turn(
            &mut session,
            "hi",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap_err();
    match error {
        AgentError::HookBlocked(message) => assert!(message.contains("敏感词门卫"), "{message}"),
        other => panic!("应为 HookBlocked：{other:?}"),
    }
    assert!(session.messages.is_empty(), "被 hook 拒绝的回合不落历史");
}

/// A2-1 hooks：PreToolUse exit 2 = 阻断该次调用但回合继续（stderr 回喂模型）。
#[tokio::test]
async fn pre_tool_use_hook_blocks_call_but_keeps_turn() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    registry.register(ProbeTool {
        label: "probe_a",
        delay_ms: 0,
        class: EffectClass::Read,
        host_verified: true,
        state: Arc::clone(&state),
    });
    let outputs = Mutex::new(VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
            id: "call-a".to_string(),
            name: "probe_a".to_string(),
            arguments: serde_json::json!({}),
        }]),
        ModelOutput::Text("已按 hook 提示调整".to_string()),
    ]));
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider { outputs }),
        registry,
        Policy::new("."),
        AgentConfig::default(),
    );
    agent.set_hooks(crate::hooks::HookManager::from_configs(&[blocking_hook(
        "pre_tool_use",
        Some("probe_a"),
        "该工具被策略组禁用",
    )]));
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "跑探针",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("hook 阻断单次调用不应终止回合");
    assert_eq!(outcome.final_text.as_deref(), Some("已按 hook 提示调整"));
    let blocked = outcome
        .events
        .iter()
        .find_map(|event| match event {
            TurnEvent::ToolResult {
                error: Some(error), ..
            } if error.contains("hook 阻断") => Some(error.clone()),
            _ => None,
        })
        .expect("应留下 hook 阻断的工具错误");
    assert!(blocked.contains("该工具被策略组禁用"), "{blocked}");
    assert_eq!(
        state.completed.lock().unwrap().len(),
        0,
        "被 hook 拦截的工具不得执行"
    );
}

/// 远端 agent.rs 取优：步数耗尽补一次不带工具的收尾总结，回合以可见结论结束
/// （旧行为直接返回「达到最大回合数」错误，用户只看到思考过程后什么都没有）。
#[tokio::test]
async fn max_turns_exhaustion_runs_wrap_up_and_returns_final_text() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    registry.register(ProbeTool {
        label: "probe_a",
        delay_ms: 0,
        class: EffectClass::Read,
        host_verified: true,
        state: Arc::clone(&state),
    });
    let call = |index: &str| crate::gateway::ToolCall {
        id: format!("call-{index}"),
        name: "probe_a".to_string(),
        arguments: serde_json::json!({ "path": format!("{index}.txt") }),
    };
    let outputs = Mutex::new(VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![call("a")]),
        ModelOutput::ToolCalls(vec![call("b")]),
        ModelOutput::Text("收尾总结报告".to_string()),
    ]));
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider { outputs }),
        registry,
        Policy::new("."),
        AgentConfig {
            max_turns: 2,
            ..Default::default()
        },
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "跑两个探针",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("收尾总结应让回合成功");
    assert_eq!(outcome.final_text.as_deref(), Some("收尾总结报告"));
    assert!(outcome.reached_model_turn_limit);
    assert!(
        outcome
            .events
            .iter()
            .any(|event| matches!(event, TurnEvent::Final { text } if text == "收尾总结报告")),
        "收尾总结必须以 Final 事件外发"
    );
}

struct WrapUpHangingProvider {
    requests: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl crate::gateway::ModelProvider for WrapUpHangingProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        if self
            .requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            Ok(ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
                id: "call-before-wrap-up".to_string(),
                name: "probe_a".to_string(),
                arguments: serde_json::json!({}),
            }]))
        } else {
            std::future::pending().await
        }
    }
}

#[tokio::test]
async fn turn_limit_wrap_up_obeys_deadline_and_records_the_timed_out_request() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    registry.register(ProbeTool {
        label: "probe_a",
        delay_ms: 0,
        class: EffectClass::Read,
        host_verified: true,
        state,
    });
    let provider = Arc::new(WrapUpHangingProvider {
        requests: std::sync::atomic::AtomicUsize::new(0),
    });
    let agent = Agent::new(
        provider,
        registry,
        Policy::new("."),
        AgentConfig {
            max_turns: 1,
            turn_deadline: Some(std::time::Duration::from_millis(100)),
            ..Default::default()
        },
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let abort = AtomicBool::new(false);
    let started = std::time::Instant::now();
    let outcome = agent
        .run_turn(
            &mut session,
            "跑探针后生成总结",
            &crate::permissions::AutoApprover { allow: true },
            &abort,
            &mut |_| {},
        )
        .await
        .expect("收尾请求超时应退回可见摘要，而不是挂住回合");

    assert!(outcome.reached_model_turn_limit);
    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Unverified
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(outcome.model_calls.len(), 2);
    assert!(!outcome.model_calls[1].succeeded);
    assert!(outcome
        .phase_timings
        .iter()
        .any(|timing| timing.target == "turn_limit_wrap_up"));
}

/// 远端 agent.rs 取优：连续空回答走兜底摘要（含本回合工具动作），不静默失败。
#[tokio::test]
async fn empty_reply_falls_back_to_tool_action_summary() {
    let state = ProbeState::new();
    let mut registry = ToolRegistry::new();
    registry.register(ProbeTool {
        label: "probe_a",
        delay_ms: 0,
        class: EffectClass::Read,
        host_verified: true,
        state: Arc::clone(&state),
    });
    let outputs = Mutex::new(VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
            id: "call-a".to_string(),
            name: "probe_a".to_string(),
            arguments: serde_json::json!({ "path": "fallback.txt" }),
        }]),
        ModelOutput::Text("   ".to_string()),
        ModelOutput::Text(String::new()),
    ]));
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider { outputs }),
        registry,
        Policy::new("."),
        AgentConfig {
            max_turns: 4,
            ..Default::default()
        },
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let outcome = agent
        .run_turn(
            &mut session,
            "跑探针",
            &approver,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("兜底摘要应让回合成功");
    let text = outcome.final_text.expect("必须有可见回复");
    assert!(
        text.contains("本回合模型没有产出正式回答"),
        "空回答应走兜底摘要：{text}"
    );
    assert!(
        text.contains("fallback.txt"),
        "兜底摘要应列出工具动作参数：{text}"
    );
}

/// 签名稳定性：参数键序不同但语义相同 → 视为同一调用（否则弱模型换键序即可绕过）。
#[test]
fn tool_call_signature_ignores_key_order() {
    let a = crate::gateway::ToolCall {
        id: "a".into(),
        name: "write_file".into(),
        arguments: serde_json::json!({ "path": "x.txt", "content": "hi" }),
    };
    let b = crate::gateway::ToolCall {
        id: "b".into(),
        name: "write_file".into(),
        arguments: serde_json::json!({ "content": "hi", "path": "x.txt" }),
    };
    assert_eq!(tool_call_signature(&a), tool_call_signature(&b));
}

/// A1-2 多模态（取优合并自远端 engine）：`run_turn_with_images` 把图片以
/// `MessageImage` 附到用户消息——provider 实际收到 data URL，而非仅路径文本。
struct RecordingProvider {
    seen: Mutex<Vec<ChatMessage>>,
}

#[async_trait::async_trait]
impl ModelProvider for RecordingProvider {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.seen.lock().unwrap().extend_from_slice(messages);
        Ok(ModelOutput::Text("ok".to_string()))
    }
}

#[tokio::test]
async fn run_turn_with_images_feeds_vision_message_to_provider() {
    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let agent = Agent::new(
        provider.clone(),
        ToolRegistry::new(),
        Policy::new("."),
        AgentConfig::default(),
    );
    let mut session = Session::new(std::env::temp_dir(), "test-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let images = vec![crate::gateway::MessageImage::from_url(
        "data:image/png;base64,AAAA",
    )];
    let outcome = agent
        .run_turn_with_images(
            &mut session,
            "看图",
            &images,
            &approver,
            None,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(outcome.final_text.as_deref(), Some("ok"));
    let seen = provider.seen.lock().unwrap();
    let user = seen
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .expect("provider 应收到用户消息");
    assert_eq!(user.images.len(), 1);
    assert_eq!(user.images[0].url, "data:image/png;base64,AAAA");
}

#[test]
fn single_workspace_receipt_can_bind_a_deleted_file_and_detect_recreation() {
    let root = std::env::temp_dir().join(format!("owo-single-absence-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("src")).expect("create test workspace");
    let relative = "src/deleted.rs";
    let absent = crate::verification::workspace_path_absence_sha256();
    assert!(super::single_workspace_path_matches(
        &root, relative, &absent
    ));

    let path = root.join(relative);
    std::fs::write(&path, b"recreated source").expect("recreate changed file");
    assert!(!super::single_workspace_path_matches(
        &root, relative, &absent
    ));
    let actual = crate::CasStore::hash_of(b"recreated source");
    assert!(super::single_workspace_path_matches(
        &root, relative, &actual
    ));
    std::fs::remove_dir_all(root).expect("remove test workspace");
}

#[test]
fn successful_progress_resets_other_tool_repeat_counts_but_keeps_current_count() {
    let mut repeats = std::collections::HashMap::from([
        ("run_test:{}".to_string(), 2),
        ("write_file:{}".to_string(), 1),
    ]);

    reset_loop_guard_after_progress(&mut repeats, "write_file:{}");

    assert_eq!(repeats.len(), 1);
    assert_eq!(repeats.get("write_file:{}"), Some(&1));
    assert!(!repeats.contains_key("run_test:{}"));
}

/// This opt-in test uses the real file/command ToolHost with a deterministic model,
/// not a cloud-model benchmark. Python and pytest must exist on the host.
#[tokio::test]
#[ignore = "requires host Python and pytest; run explicitly for tool-chain acceptance"]
async fn real_command_receipt_closes_single_write_test_review_chain() {
    let workspace = tempfile::tempdir().unwrap();
    let command = "python -m pytest test_check.py -q";
    let request = "创建并验证测试";
    let call = |id: &str, name: &str, arguments: serde_json::Value| crate::gateway::ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
    };
    let source = ["def test_ready():", "    assert 2 + 2 == 4", ""].join("\n");
    let outputs = VecDeque::from(vec![
        ModelOutput::ToolCalls(vec![call(
            "plan",
            "verification_plan",
            serde_json::json!({
                "plan": {
                    "plan_id": "real-tool-chain",
                    "requirements": [{
                        "requirement_id": "check",
                        "covers_requirement_ids": ["user-request:创建并验证测试"],
                        "validator_id": "workspace-command-success-v1",
                        "validator_version": "1",
                        "scope": {"kind":"workspace_paths","relative_paths":["test_check.py"]},
                        "arguments": {"command":command},
                        "required":true,
                        "resources": {"cpu_slots":1,"memory_mb":16,"exclusive_workspace":false,"timeout_ms":10000}
                    }]
                }
            }),
        )]),
        ModelOutput::ToolCalls(vec![call(
            "write",
            "write_file",
            serde_json::json!({"path":"test_check.py","content":source}),
        )]),
        ModelOutput::ToolCalls(vec![call(
            "test",
            "run_command",
            serde_json::json!({"command":command}),
        )]),
        ModelOutput::Text("测试通过，提交候选版本。".to_string()),
        ModelOutput::Text(
            serde_json::json!({
                "status":"done","summary":"独立评审通过",
                "review_result":{
                    "verdict":"approved",
                    "reviewed_requirement_ids":["check","user-request:创建并验证测试"],
                    "findings":[]
                },
                "evidence":[{"source":"test_check.py","note":"检查完整源码快照及宿主行为验证证据"}],
                "open_issues":[]
            })
            .to_string(),
        ),
    ]);
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider {
            outputs: Mutex::new(outputs),
        }),
        ToolRegistry::new(),
        Policy::new(workspace.path()),
        AgentConfig {
            max_turns: 8,
            ..Default::default()
        },
    );
    let mut session = Session::new(workspace.path(), "scripted-tool-chain", None);
    let outcome = agent
        .run_turn(
            &mut session,
            request,
            &crate::permissions::AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .expect("real tool chain should complete");
    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Accepted
    );
    let receipt = outcome
        .events
        .iter()
        .find_map(|event| match event {
            TurnEvent::ToolResult {
                tool,
                command_receipt: Some(receipt),
                ..
            } if tool == "run_command" => Some(receipt),
            _ => None,
        })
        .expect("real command must emit a host receipt");
    assert_eq!(receipt.exit_code, 0);
    assert!(receipt.workspace_hashes_complete);
    assert_eq!(receipt.workspace_hashes_before, receipt.workspace_hashes);
    assert_eq!(
        receipt.workspace_hashes.get("test_check.py"),
        Some(&Some(crate::CasStore::hash_of(source.as_bytes())))
    );
    assert!(session.validation_receipts.iter().any(|receipt| {
        receipt.validator_id == "workspace-independent-review-v1"
            && receipt.verdict == crate::plan::ValidationVerdictV1::Passed
    }));
    assert!(session
        .execution_receipts
        .iter()
        .all(|receipt| receipt.status == "accepted"));
}

#[test]
fn navigation_command_receipt_preserves_metadata_without_source_test_claim() {
    let workspace = tempfile::tempdir().unwrap();
    let session = Session::new(workspace.path(), "test", None);
    let outcome = Ok(serde_json::json!({
        "command": "git status", "exit_code": 0, "duration_ms": 12, "stdout": "clean"
    }));
    let receipt = command_execution_receipt("run_command", &outcome, &session, None).unwrap();
    assert_eq!(receipt.exit_code, 0);
    assert_eq!(receipt.duration_ms, Some(12));
    assert_eq!(
        receipt.command_sha256,
        crate::CasStore::hash_of(b"git status")
    );
    assert!(receipt.validator_id.is_none());
    assert!(!receipt.workspace_hashes_complete);
    assert!(receipt.workspace_hashes_before.is_empty());
    assert!(receipt.workspace_hashes.is_empty());
}

#[test]
fn behavior_command_without_before_snapshot_cannot_invent_verified_source() {
    let workspace = tempfile::tempdir().unwrap();
    let session = Session::new(workspace.path(), "test", None);
    let outcome = Ok(serde_json::json!({
        "command": "cargo test", "exit_code": 0, "duration_ms": 12
    }));
    let receipt = command_execution_receipt("run_command", &outcome, &session, None).unwrap();
    assert_eq!(
        receipt.validator_id.as_deref(),
        Some("workspace-command-success-v1")
    );
    assert!(!receipt.workspace_hashes_complete);
    assert!(receipt.workspace_hashes_before.is_empty());
    assert!(receipt.workspace_hashes.is_empty());
}

#[tokio::test]
async fn readonly_request_cannot_write_or_relax_a_parent_and_does_not_affect_next_turn() {
    let workspace = tempfile::tempdir().unwrap();
    let write = |id: &str| {
        ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
            id: id.into(),
            name: "write_file".into(),
            arguments: serde_json::json!({ "path": "scope-fixture.txt", "content": "fixture" }),
        }])
    };
    let outputs = VecDeque::from(vec![
        write("readonly"),
        ModelOutput::Text("readonly finished".into()),
        write("normal"),
        ModelOutput::Text("normal finished".into()),
        write("parent-readonly"),
        ModelOutput::Text("parent restriction retained".into()),
    ]);
    let agent = Agent::new(
        Arc::new(ScriptedTestProvider {
            outputs: Mutex::new(outputs),
        }),
        ToolRegistry::new(),
        Policy::new(workspace.path()),
        AgentConfig::default(),
    );
    let mut session = Session::new(workspace.path(), "fixture-model", None);
    let approver = crate::permissions::AutoApprover { allow: true };
    let abort = AtomicBool::new(false);
    agent
        .run_turn_with_images_constrained(
            &mut session,
            "read only",
            &[],
            &approver,
            None,
            &abort,
            &mut |_| {},
            true,
        )
        .await
        .unwrap();
    assert!(!workspace.path().join("scope-fixture.txt").exists());
    assert_eq!(
        agent.policy().profile(),
        crate::permissions::PermissionProfile::Workspace
    );
    agent
        .run_turn_with_images_constrained(
            &mut session,
            "normal write",
            &[],
            &approver,
            None,
            &abort,
            &mut |_| {},
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("scope-fixture.txt")).unwrap(),
        "fixture"
    );
    std::fs::remove_file(workspace.path().join("scope-fixture.txt")).unwrap();
    agent
        .policy()
        .set_profile(crate::permissions::PermissionProfile::ReadOnly);
    agent
        .run_turn_with_images_constrained(
            &mut session,
            "false must not widen the parent",
            &[],
            &approver,
            None,
            &abort,
            &mut |_| {},
            false,
        )
        .await
        .unwrap();
    assert!(!workspace.path().join("scope-fixture.txt").exists());
}
