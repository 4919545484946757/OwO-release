//! 七期一路定向测试：生产 Worker 输出契约执行器（`contract_worker`）。
//!
//! 覆盖：`enforce_worker_output_contract` 的 producer/critic 双角色规则、
//! 定向修复至多一次、修复后双复验、围栏剥离、失败前缀冻结（`output_contract_invalid:`），
//! 以及 `SubagentRunner` 端到端（生产路径）自由文本交付路径彻底关闭。

use async_trait::async_trait;
use owo_agent_core::contract_worker::{
    enforce_worker_output_contract, ContractEnforcementError, ContractSubagentRunner,
};
use owo_agent_core::gateway::ToolCall;
use owo_agent_core::gateway::{ChatMessage, ModelOutput, ModelProvider};
use owo_agent_core::permissions::AutoApprover;
use owo_agent_core::subagent::SubagentRunner;
use owo_agent_core::tools::ToolSpec;
use owo_agent_core::workswarm_output::{parse_worker_output, strip_code_fences, WorkerOutputParse};
use owo_agent_core::TurnEvent;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// 合法的 producer 契约输出（done + 非空 artifact.content）。
const PRODUCER_OK: &str = r###"{"status":"done","summary":"交付完成","artifact":{"kind":"document","format":"markdown","content":"## 交付正文\n关键结论 A。"},"evidence":[],"open_issues":[]}"###;

/// 合法的 critic 契约输出（评审结论放 summary，禁止 artifact）。
const CRITIC_OK: &str = r#"{"status":"done","summary":"评审完成","artifact":null,"evidence":[],"open_issues":[],"handoff":null,"review_result":{"verdict":"approved","reviewed_requirement_ids":[],"findings":[]}}"#;

/// critic 越权：携带 artifact（首轮违例，应触发定向修复）。
const CRITIC_WITH_ARTIFACT: &str = r#"{"status":"done","summary":"评审完成","artifact":{"kind":"review","format":"json","content":"{\"score\":60}"},"evidence":[],"open_issues":[]}"#;

/// 自由文本（Legacy：不是契约 JSON，不能登记为交付物）。
const FREE_TEXT: &str = "任务完成：我已经修好了 bug 并通过了测试。";

/// 嵌套子代理事件转发：工具进度与审批请求必须即时到达父回合的事件出口。
///
/// 回归背景：`ContractSubagentRunner` 此前用 `|_event| {}` 丢弃全部子事件——
/// 子代理的审批卡到不了客户端，子代理阻塞在 `approver.decide()` 上直到超时
/// （表现为"任务卡死"）。本测试断言 `PermissionRequest` 与带 `sub:` 前缀的
/// ToolStart/ToolResult 都经 `events` 出口外发。
#[tokio::test]
async fn nested_subagent_forwards_permission_and_tool_events() {
    let workspace =
        std::env::temp_dir().join(format!("owo-subagent-events-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    // 脚本：先要求执行命令（Execute 级 → 触发审批请求），再交付契约输出。
    let provider = ScriptedProvider::from_outputs(vec![
        ModelOutput::ToolCalls(vec![ToolCall {
            id: "call-1".to_string(),
            name: "run_command".to_string(),
            arguments: serde_json::json!({ "command": "echo nested" }),
        }]),
        ModelOutput::Text(PRODUCER_OK.to_string()),
    ]);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };
    let seen: Arc<Mutex<Vec<TurnEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_seen = Arc::clone(&seen);
    let sink: owo_agent_core::subagent::TurnEventSink<'_> = Arc::new(move |event: &TurnEvent| {
        sink_seen.lock().expect("事件锁中毒").push(event.clone());
    });
    let parent_policy = owo_agent_core::Policy::new(&workspace);
    let runner = ContractSubagentRunner {
        parent_policy: &parent_policy,
        provider: provider_arc(&provider),
        approver: &approver,
        abort: &abort,
        depth: 0,
        max_turns: 4,
        model: "mock".to_string(),
        events: Some(sink),
    };
    runner
        .run(&workspace, "执行一条命令并交付", false)
        .await
        .expect("契约合规输出应被接受");

    let events = seen.lock().expect("事件锁中毒").clone();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TurnEvent::PermissionRequest(_))),
        "审批请求必须外发（否则子代理会等到审批超时）：{events:?}"
    );
    let tool_starts: Vec<String> = events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ToolStart { tool, .. } => Some(tool.clone()),
            _ => None,
        })
        .collect();
    assert!(
        tool_starts.iter().any(|tool| tool == "sub:run_command"),
        "子代理工具启动事件应带 sub: 前缀外发：{tool_starts:?}"
    );
    let tool_results: Vec<String> = events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ToolResult { tool, .. } => Some(tool.clone()),
            _ => None,
        })
        .collect();
    assert!(
        tool_results.iter().any(|tool| tool == "sub:run_command"),
        "子代理工具结果事件应带 sub: 前缀外发：{tool_results:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            TurnEvent::Final { .. } | TurnEvent::TokenDelta { .. }
        )),
        "子代理的 Final/TokenDelta 不得混入父回合事件流"
    );
    let _ = std::fs::remove_dir_all(&workspace);
}
/// 脚本化 provider：按序返回预设输出；耗尽后报错。
struct ScriptedProvider {
    outputs: Mutex<VecDeque<ModelOutput>>,
    calls: AtomicU32,
}

impl ScriptedProvider {
    fn new(outputs: &[&str]) -> Arc<Self> {
        Self::from_outputs(
            outputs
                .iter()
                .map(|s| ModelOutput::Text(s.to_string()))
                .collect(),
        )
    }

    fn from_outputs(outputs: Vec<ModelOutput>) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(outputs.into()),
            calls: AtomicU32::new(0),
        })
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let next = self
            .outputs
            .lock()
            .expect("provider 锁中毒")
            .pop_front()
            .ok_or_else(|| "脚本输出耗尽".to_string())?;
        Ok(next)
    }
}

fn provider_arc(provider: &Arc<ScriptedProvider>) -> Arc<dyn ModelProvider> {
    // 显式绑定以触发 Arc<具体类型> → Arc<dyn ModelProvider> 的 unsizing 强制转换。
    let arc: Arc<ScriptedProvider> = Arc::clone(provider);
    let dyn_arc: Arc<dyn ModelProvider> = arc;
    dyn_arc
}

#[tokio::test]
async fn producer_valid_json_passes_without_repair() {
    let provider = ScriptedProvider::new(&[]);
    let enforced = enforce_worker_output_contract(&provider_arc(&provider), PRODUCER_OK, false)
        .await
        .expect("合法 producer 输出应直接通过");
    assert_eq!(enforced.repairs, 0, "首轮即合规不应消耗修复");
    assert_eq!(enforced.text, PRODUCER_OK, "合规输出应原样返回（不重生成）");
    assert_eq!(provider.calls(), 0, "合规路径不应发起 provider 调用");
}

#[tokio::test]
async fn producer_free_text_repaired_once() {
    // 首轮文本经参数进入执行器；队列里只有「定向修复」的 provider 应答。
    let provider = ScriptedProvider::new(&[PRODUCER_OK]);
    let enforced = enforce_worker_output_contract(&provider_arc(&provider), FREE_TEXT, false)
        .await
        .expect("自由文本应经一次定向修复转为契约 JSON");
    assert_eq!(enforced.repairs, 1, "恰好一次修复");
    assert_eq!(enforced.text, PRODUCER_OK, "修复结果应被接受");
    assert_eq!(provider.calls(), 1, "修复 = 恰好一次 provider 调用");
    match parse_worker_output(&enforced.text) {
        WorkerOutputParse::Parsed(output) => {
            output
                .validate()
                .expect("修复后 producer 输出应通过 validate");
        }
        other => panic!("修复结果不是 Parsed：{other:?}"),
    }
}

#[tokio::test]
async fn producer_broken_json_repaired_once() {
    let provider = ScriptedProvider::new(&[PRODUCER_OK]);
    let enforced =
        enforce_worker_output_contract(&provider_arc(&provider), r#"{"status":"done"#, false)
            .await
            .expect("坏 JSON 应经一次定向修复转为契约 JSON");
    assert_eq!(enforced.repairs, 1);
    assert_eq!(enforced.text, PRODUCER_OK);
    assert_eq!(provider.calls(), 1);
}

#[tokio::test]
async fn critic_with_artifact_repaired_and_revalidated() {
    let provider = ScriptedProvider::new(&[CRITIC_OK]);
    let enforced =
        enforce_worker_output_contract(&provider_arc(&provider), CRITIC_WITH_ARTIFACT, true)
            .await
            .expect("critic 越权 artifact 应经一次定向修复");
    assert_eq!(enforced.repairs, 1);
    assert_eq!(enforced.text, CRITIC_OK);
    // 双复验：修复结果仍按 critic 角色规则校验（禁带 artifact + 结论非空）。
    match parse_worker_output(&enforced.text) {
        WorkerOutputParse::Parsed(output) => {
            assert!(
                output.artifact.is_none(),
                "critic 修复结果不得携带 artifact"
            );
            output
                .validate_critic()
                .expect("critic 修复结果应通过 validate_critic");
        }
        other => panic!("修复结果不是 Parsed：{other:?}"),
    }
}

#[tokio::test]
async fn critic_repair_output_fences_are_stripped() {
    let fenced = format!("```json\n{CRITIC_OK}\n```");
    let provider = ScriptedProvider::new(&[&fenced]);
    let enforced =
        enforce_worker_output_contract(&provider_arc(&provider), CRITIC_WITH_ARTIFACT, true)
            .await
            .expect("带围栏的修复输出应剥离围栏后接受");
    assert_eq!(enforced.repairs, 1);
    assert_eq!(enforced.text, CRITIC_OK, "围栏应被剥离为 JSON 本体");
    // 独立验证 strip_code_fences 的围栏语义（语言标记行 + 成对围栏）。
    assert_eq!(strip_code_fences(&fenced), CRITIC_OK);
}

#[tokio::test]
async fn enforce_failure_carries_frozen_prefix_and_repairs_count() {
    let provider = ScriptedProvider::new(&["还是自由文本"]);
    // 四路集成微修：clippy err_expect（.err().expect(..) → expect_err(..)）。
    let error = enforce_worker_output_contract(&provider_arc(&provider), FREE_TEXT, false)
        .await
        .expect_err("修复后仍不合规应失败");
    assert!(
        error.message.starts_with("output_contract_invalid:"),
        "失败前缀必须冻结为 output_contract_invalid:（UI failure_code 口径），实际：{}",
        error.message
    );
    assert!(error.message.contains("定向修复一次后仍不符合契约"));
    assert_eq!(
        error.repairs, 1,
        "失败路径也应计 1 次修复（与六期 stats 基线一致）"
    );
    assert_eq!(provider.calls(), 1, "修复上限 1 次：不应有第二次修复调用");
}

#[tokio::test]
async fn subagent_end_to_end_producer() {
    let workspace = std::env::temp_dir();
    let provider = ScriptedProvider::new(&[PRODUCER_OK]);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };
    let parent_policy = owo_agent_core::Policy::new(&workspace);
    let runner = SubagentRunner {
        parent_policy: &parent_policy,
        provider: provider_arc(&provider),
        approver: &approver,
        abort: &abort,
        depth: 0,
        max_turns: 3,
        model: "mock".to_string(),
        events: None,
    };
    let text = runner
        .run(&workspace, "调查并交付修复", false)
        .await
        .expect("producer 子代理端到端应成功");
    assert_eq!(text, PRODUCER_OK, "返回值应为契约合规 JSON 本体");
    match parse_worker_output(&text) {
        WorkerOutputParse::Parsed(output) => {
            output.validate().expect("端到端产物应通过 producer 校验");
            assert!(
                output
                    .artifact
                    .as_ref()
                    .expect("producer 必须携带 artifact")
                    .content
                    .contains("交付正文"),
                "Producer 正文只取自 artifact.content"
            );
        }
        other => panic!("端到端产物不是 Parsed：{other:?}"),
    }
    assert_eq!(
        provider.calls(),
        1,
        "合规路径：1 次 Agent 回合调用，0 次修复调用"
    );
}

#[tokio::test]
async fn subagent_end_to_end_critic() {
    let workspace = std::env::temp_dir();
    let provider = ScriptedProvider::new(&[CRITIC_OK]);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };
    let parent_policy = owo_agent_core::Policy::new(&workspace);
    let runner = SubagentRunner {
        parent_policy: &parent_policy,
        provider: provider_arc(&provider),
        approver: &approver,
        abort: &abort,
        depth: 0,
        max_turns: 3,
        model: "mock".to_string(),
        events: None,
    };
    let text = runner
        .run(&workspace, "评审上游交付物", true)
        .await
        .expect("critic 子代理端到端应成功");
    assert_eq!(text, CRITIC_OK);
    match parse_worker_output(&text) {
        WorkerOutputParse::Parsed(output) => {
            output
                .validate_critic()
                .expect("端到端 critic 产物应通过 validate_critic");
        }
        other => panic!("端到端 critic 产物不是 Parsed：{other:?}"),
    }
}

#[tokio::test]
async fn subagent_free_text_path_is_closed() {
    let workspace = std::env::temp_dir();
    let provider = ScriptedProvider::new(&[FREE_TEXT, "还是自由文本"]);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };
    let parent_policy = owo_agent_core::Policy::new(&workspace);
    let runner = SubagentRunner {
        parent_policy: &parent_policy,
        provider: provider_arc(&provider),
        approver: &approver,
        abort: &abort,
        depth: 0,
        max_turns: 3,
        model: "mock".to_string(),
        events: None,
    };
    let error = runner
        .run(&workspace, "调查并交付修复", false)
        .await
        .expect_err("自由文本交付路径必须关闭（不得登记为交付物）");
    assert!(
        error.starts_with("output_contract_invalid:"),
        "生产路径失败前缀冻结：{error}"
    );
    assert_eq!(
        provider.calls(),
        2,
        "1 次 Agent 回合 + 1 次定向修复，且无第二次修复"
    );
}

// ContractSubagentRunner 的字段构造形态回归（与 SubagentRunner 字段集一致，
// 确保共享执行器可直接承载生产路径的全部字段）。
#[tokio::test]
async fn contract_runner_accepts_explicit_fields() {
    let workspace = std::env::temp_dir();
    let provider = ScriptedProvider::new(&[CRITIC_OK]);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };
    let parent_policy = owo_agent_core::Policy::new(&workspace);
    let runner = ContractSubagentRunner {
        parent_policy: &parent_policy,
        provider: provider_arc(&provider),
        approver: &approver,
        abort: &abort,
        depth: 0,
        max_turns: 3,
        model: "mock".to_string(),
        events: None,
    };
    let text = runner
        .run(&workspace, "评审", true)
        .await
        .expect("ContractSubagentRunner 直接构造应可用");
    assert_eq!(text, CRITIC_OK);
}

// ContractEnforcementError 的 Debug 派生回归（日志/断言路径不 panic）。
#[test]
fn enforcement_error_debug_is_stable() {
    let error = ContractEnforcementError {
        message: "output_contract_invalid: 定向修复一次后仍不符合契约：示例违例".to_string(),
        repairs: 1,
        usage: None,
    };
    let debug = format!("{error:?}");
    assert!(debug.contains("output_contract_invalid"));
    assert!(debug.contains("1"));
}
