//! WorkerOutputV1 输出契约与定向修复原语；执行循环由 core WorkerRuntime 统一承载。
//!
//! 生产 `SubagentRunner`、Team `ProfileSubagentRunner` 与 ProductEval `EvalAgentWorker`
//! 共用此处的输出契约检查；各自解析工具权限后，通过 WorkerRuntime 执行同一回合循环。
//! **最终结果必须是合法的 `WorkerOutputV1` 契约 JSON，自由文本交付路径彻底关闭**。
//!
//! 七期契约冻结口径：
//! 1. 先经 [`parse_worker_output`] 解析，再按角色规则校验（`read_only` 参数是两路的
//!    critic 角色代理：`true` == critic 角色，禁带 artifact；`false` == producer 角色，
//!    done 必须携带 artifact）；
//! 2. 不合规只做**至多一次**定向修复（一次直接 provider 调用），修复后 producer/critic
//!    **双复验**才接受（旧 EvalAgentWorker 只复验 critic，共享执行器更严格）；
//! 3. 仍不合规 → `Err`，错误前缀冻结为 `output_contract_invalid:`（小写下划线失败码，
//!    与 `failure_code` 字段同一口径，UI 对 `error` 字段前缀双容错），禁止无限重试。
//!
//! 成功时返回**契约合规的 JSON 本体**（引擎重解析后按 Artifact/Handoff 版本化登记，
//! Producer 正文只取自 `artifact.content`）；连「无最终文本」的兜底文本也走契约执行，
//! 不豁免。

use crate::agent::{AgentConfig, TurnEvent};
use crate::gateway::{ChatMessage, ModelOutput, ModelProvider};
use crate::permissions::{Approver, Policy};
use crate::subagent::TurnEventSink;
use crate::tools::ToolRegistry;
use crate::workswarm_output::{
    contract_repair_prompt, contract_system_prompt, parse_worker_output, strip_code_fences,
    WorkerOutputParse, WorkerOutputV1,
};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// 子代理回合预算（按任务复杂度自适应，而非固定 12 轮）。
///
/// 依据：委派提示词的长度/结构是"任务复杂度"的可用代理指标——一句话委派
/// （"看看 X"）给基础轮数即可；带验收标准、多步骤、多文件的委派需要更多轮。
/// 读角色（探索）天然比写角色（实现+验证）轮数少。
///
/// 计算：基础值（只读 10 / 通用 16）+ 每 200 字符 +1，上限（只读 24 / 通用 40），
/// 最终再与调用方配置的 `max_turns` 取小（配置是硬上限）。
pub fn adaptive_subagent_turns(prompt: &str, read_only: bool) -> usize {
    let base: usize = if read_only { 10 } else { 16 };
    let cap: usize = if read_only { 24 } else { 40 };
    let extra = prompt.chars().count() / 200;
    base.saturating_add(extra).min(cap)
}

/// 子代理自动复核开关（默认开；`OWO_SUBAGENT_REVIEW=0|false` 关闭）。
///
/// 关掉的原因通常是成本/延迟（复核 = 一次额外子代理回合）；质量优先时保持默认开。
pub fn subagent_review_enabled() -> bool {
    std::env::var("OWO_SUBAGENT_REVIEW")
        .map(|value| !(value == "0" || value.eq_ignore_ascii_case("false")))
        .unwrap_or(true)
}

/// 父侧自动复核提示词：独立 critic 逐条核对验收标准与证据（只读）。
///
/// 这是"子代理草草了事"的质量门：委派返回后由**另一个**子代理独立核对，
/// 而不是让交付者自己宣布完成。
pub fn review_prompt(task: &str, output: &str) -> String {
    format!(
        "你是独立复核子代理（critic）：只读核对上游交付是否**真正**满足委派任务的验收标准。\n\
         【委派任务与验收标准】\n{task}\n\n\
         【上游交付（契约 JSON）】\n{output}\n\n\
         复核要求：\n\
         1. 逐条核对任务里的验收标准（缺标准时按「任务字面要求是否被逐字满足」判定）；\n\
         2. 核对证据是否支撑结论（无证据的完成声明一律视为未完成）；\n\
         3. 不通过时给出**可执行**的返工清单（缺什么、改哪里、怎么验证）。\n\
         输出按你的输出契约：summary 里放 {{\"approved\":true|false,\"score\":0-100,\"comments\":[\"…\"],\"rework\":[\"…\"]}}。"
    )
}

/// 返工提示词：把复核意见变成下一轮交付的硬要求（一次返工，不无限重试）。
pub fn rework_prompt(task: &str, previous: &str, review: &str) -> String {
    format!(
        "你上一轮的交付**未通过独立复核**，现在按复核意见返工一次（只有这一次机会）。\n\
         【原始任务与验收标准】\n{task}\n\n\
         【上一轮交付】\n{previous}\n\n\
         【复核意见（必须逐条落实）】\n{review}\n\n\
         返工要求：只做复核指出的缺口；完成后按输出契约提交（artifact.content 放最终交付正文，\
         evidence 里写清每条验收标准的证据）。"
    )
}

/// 复核结论解析：从 critic 契约输出里取 `"approved"` 布尔。
///
/// 兼容模型把结论 JSON 放进 `summary` 字符串的写法（契约规定如此），也兼容
/// 裸 JSON 片段；解析不到返回 `None`（调用方按"无法判定"处理，不误判为通过）。
pub fn critic_approved(review_text: &str) -> Option<bool> {
    let text = strip_code_fences(review_text);
    let haystack = match parse_worker_output(&text) {
        WorkerOutputParse::Parsed(output) => output.summary,
        _ => text.clone(),
    };
    let mut search_from = 0usize;
    while let Some(index) = haystack[search_from..].find("\"approved\"") {
        let after = &haystack[search_from + index + "\"approved\"".len()..];
        let after = after.trim_start();
        let after = after.strip_prefix(':').unwrap_or(after).trim_start();
        if after.starts_with("true") {
            return Some(true);
        }
        if after.starts_with("false") {
            return Some(false);
        }
        search_from += index + "\"approved\"".len();
    }
    None
}

#[cfg(test)]
mod review_tests {
    use super::{critic_approved, review_prompt, rework_prompt};

    #[test]
    fn critic_verdict_parsed_from_summary_and_bare_json() {
        // 契约写法：结论 JSON 放在 summary 字符串里。
        let wrapped = r#"{"status":"done","summary":"{\"approved\":false,\"score\":40,\"comments\":[\"缺证据\"]}","evidence":[],"open_issues":[]}"#;
        assert_eq!(critic_approved(wrapped), Some(false));
        let ok = r#"{"status":"done","summary":"{\"approved\":true,\"score\":92}","evidence":[],"open_issues":[]}"#;
        assert_eq!(critic_approved(ok), Some(true));
        // 裸 JSON / 围栏包裹也兼容。
        assert_eq!(
            critic_approved("```json\n{\"approved\": true}\n```"),
            Some(true)
        );
        // 解析不到 → None（调用方不得当成通过）。
        assert_eq!(critic_approved("看起来没问题"), None);
        assert_eq!(
            critic_approved(r#"{"status":"done","summary":"完成"}"#),
            None
        );
    }

    #[test]
    fn review_and_rework_prompts_carry_task_and_findings() {
        let review = review_prompt("写 hello.txt（验收：内容为 hi）", "{\"status\":\"done\"}");
        assert!(review.contains("写 hello.txt（验收：内容为 hi）"));
        assert!(review.contains("approved"));
        let rework = rework_prompt("写 hello.txt", "上一轮交付", "缺证据：没有 read_file 回读");
        assert!(rework.contains("未通过独立复核"));
        assert!(rework.contains("缺证据：没有 read_file 回读"));
        assert!(rework.contains("只有这一次机会"));
    }
}
#[cfg(test)]
mod budget_tests {
    use super::adaptive_subagent_turns;

    #[test]
    fn adaptive_budget_scales_with_prompt_and_role() {
        // 一句话委派：基础轮数（只读 10 / 通用 16）。
        assert_eq!(adaptive_subagent_turns("看看 X", true), 10);
        assert_eq!(adaptive_subagent_turns("看看 X", false), 16);
        // 长委派（带验收标准/多步骤）：每 200 字符 +1。
        let long = "需求：".to_string() + &"x".repeat(600);
        assert_eq!(adaptive_subagent_turns(&long, false), 19);
        // 上限：只读 24 / 通用 40（防"委派一段超长文本换来无限轮"）。
        let huge = "x".repeat(20_000);
        assert_eq!(adaptive_subagent_turns(&huge, true), 24);
        assert_eq!(adaptive_subagent_turns(&huge, false), 40);
        // 调用方配置是硬上限：min 在调用点做（此处只验函数本体不超过上限）。
        assert!(adaptive_subagent_turns(&huge, false) <= 40);
    }
}
/// 契约执行结果（两路共享）。
#[derive(Debug, Clone)]
pub struct ContractEnforcementResult {
    /// 契约合规本体（`WorkerOutputV1` JSON；直接返回给引擎/调用方）。
    pub text: String,
    /// 已消耗的定向修复次数（0 = 首轮即合规；1 = 修复一次后接受）。
    /// 调用方（如 EvalAgentWorker stats）据此计 model_calls/output_repairs 计数。
    pub repairs: u32,
    /// Usage attributable to the optional contract-repair request.
    pub usage: Option<crate::gateway::TokenUsage>,
}

/// 契约执行失败（唯一的一次定向修复已消耗）。
#[derive(Debug, Clone)]
pub struct ContractEnforcementError {
    /// 失败原因；前缀冻结为 `output_contract_invalid:`。
    pub message: String,
    /// 已消耗的定向修复次数；请求前拒绝为 0，发起修复后为 1。
    pub repairs: u32,
    /// Usage attributable to the repair attempt, including invalid repaired output.
    pub usage: Option<crate::gateway::TokenUsage>,
}

/// 角色规则校验（producer / critic）。
fn validate_by_role(output: &WorkerOutputV1, is_critic: bool) -> Result<(), String> {
    if is_critic {
        output.validate_critic()
    } else {
        output.validate()
    }
}

/// 输出契约执行器——生产 Agent 与 ProductEval Worker 的**唯一**共享入口。
///
/// - `is_critic`：角色代理（两路 `read_only` 语义对齐：true == critic 角色）；
/// - 首轮不合规时把**具体违例原因**带进修复提示词（定向修复，不是重发完整指令）；
/// - 修复后 producer/critic 双复验，合法才接受；
/// - `repairs` 计数在 Ok/Err 两侧都返回（调用方按它计 stats，失败也计——与六期基线一致）。
pub async fn enforce_worker_output_contract(
    provider: &Arc<dyn ModelProvider>,
    text: &str,
    is_critic: bool,
) -> Result<ContractEnforcementResult, ContractEnforcementError> {
    enforce_worker_output_contract_with_model(provider, None, text, is_critic).await
}

/// Same output-contract path with an explicit request model, used by task-scoped workers
/// so a repair request stays on the same model route as the work it repairs.
pub async fn enforce_worker_output_contract_with_model(
    provider: &Arc<dyn ModelProvider>,
    model: Option<&str>,
    text: &str,
    is_critic: bool,
) -> Result<ContractEnforcementResult, ContractEnforcementError> {
    enforce_worker_output_contract_controlled(provider, model, text, is_critic, None, None, true)
        .await
}

/// Runtime-owned repair boundary. The repair shares cancellation, remaining wall-clock
/// budget and request-count budget with the worker, rather than starting an unbounded
/// second execution lane after Agent::run_turn has finished.
pub(crate) async fn enforce_worker_output_contract_controlled(
    provider: &Arc<dyn ModelProvider>,
    model: Option<&str>,
    text: &str,
    is_critic: bool,
    abort: Option<&AtomicBool>,
    timeout: Option<std::time::Duration>,
    allow_repair: bool,
) -> Result<ContractEnforcementResult, ContractEnforcementError> {
    let first_violation = match parse_worker_output(text) {
        WorkerOutputParse::Parsed(output) => match validate_by_role(&output, is_critic) {
            Ok(()) => {
                return Ok(ContractEnforcementResult {
                    text: text.to_string(),
                    repairs: 0,
                    usage: None,
                })
            }
            Err(violation) => violation,
        },
        WorkerOutputParse::Invalid { error } => error,
        WorkerOutputParse::Legacy => {
            "输出不是 WorkerOutputV1 契约 JSON（自由文本不能登记为交付物）".to_string()
        }
    };

    let messages = [ChatMessage {
        role: "user".to_string(),
        content: Some(contract_repair_prompt(is_critic, &first_violation, text)),
        tool_calls: None,
        tool_call_id: None,
        images: Vec::new(),
    }];
    let refusal = if abort.is_some_and(|signal| signal.load(std::sync::atomic::Ordering::Relaxed)) {
        Some("worker_aborted: 输出修复前任务已取消")
    } else if !allow_repair {
        Some("worker_turn_budget_exhausted: 没有剩余模型调用预算用于输出修复")
    } else if timeout.is_some_and(|remaining| remaining.is_zero()) {
        Some("worker_deadline_exhausted: 没有剩余时间用于输出修复")
    } else {
        None
    };
    if let Some(message) = refusal {
        return Err(ContractEnforcementError {
            message: message.to_string(),
            repairs: 0,
            usage: None,
        });
    }
    let wait_for_cancel = async {
        match abort {
            Some(signal) => {
                while !signal.load(std::sync::atomic::Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
            None => std::future::pending::<()>().await,
        }
    };
    let request = async {
        let request = provider.complete_with_model_observed(model, &messages, &[]);
        match timeout {
            Some(remaining) => tokio::time::timeout(remaining, request)
                .await
                .map_err(|_| "worker_deadline_exhausted: 输出修复超时".to_string())?,
            None => request.await,
        }
    };
    let observed = tokio::select! {
        biased;
        _ = wait_for_cancel => Err("worker_aborted: 输出修复已取消".to_string()),
        result = request => result,
    };
    let (repaired, repair_usage) = match observed {
        Err(message) => {
            return Err(ContractEnforcementError {
                message: format!("output_contract_invalid: {message}"),
                repairs: 1,
                usage: None,
            });
        }
        Ok(observed) => {
            let text = match observed.output {
                ModelOutput::Text(text) => strip_code_fences(&text),
                _ => String::new(),
            };
            (text, observed.metadata.usage)
        }
    };
    let violation = match parse_worker_output(&repaired) {
        WorkerOutputParse::Parsed(output) => match validate_by_role(&output, is_critic) {
            Ok(()) => {
                return Ok(ContractEnforcementResult {
                    text: repaired,
                    repairs: 1,
                    usage: repair_usage,
                })
            }
            Err(violation) => violation,
        },
        WorkerOutputParse::Invalid { error } => error,
        WorkerOutputParse::Legacy => {
            "定向修复后仍是自由文本（自由文本不能登记为交付物）".to_string()
        }
    };
    Err(ContractEnforcementError {
        message: format!("output_contract_invalid: 定向修复一次后仍不符合契约：{violation}"),
        repairs: 1,
        usage: repair_usage,
    })
}

/// 契约化子代理执行器：承载完整 subagent 回合循环 + 输出契约执行。
///
/// `SubagentRunner::run` 委托到这里（七期一路）。字段集与 `SubagentRunner` 完全一致
/// （不新增/不改名任何字段——`agent.rs`/`tools.rs`/`workswarm_api.rs` 里的结构体
/// 字面量不受影响），区别只在返回的是**契约校验后的 JSON 本体**而非自由文本。
pub struct ContractSubagentRunner<'a> {
    pub provider: Arc<dyn ModelProvider>,
    /// Host restrictions are inherited live; role constraints may only narrow them.
    pub parent_policy: &'a Policy,
    pub approver: &'a dyn Approver,
    pub abort: &'a AtomicBool,
    pub depth: usize,
    pub max_turns: usize,
    pub model: String,
    /// 嵌套回合事件出口（`None` = 不转发，后台路径）。
    pub events: Option<TurnEventSink<'a>>,
}

impl ContractSubagentRunner<'_> {
    /// 运行一个只读或通用子代理会话，返回契约校验后的 JSON 本体。
    ///
    /// Capability resolution stays in this adapter; Agent loop, task-session handling,
    /// budget exhaustion, output repair, and telemetry are shared by WorkerRuntime.
    pub async fn run(
        &self,
        workspace: &Path,
        prompt: &str,
        read_only: bool,
    ) -> Result<String, String> {
        let policy = self.parent_policy.for_workspace(workspace.to_path_buf())?;
        let policy = if read_only {
            policy.read_only_scope()
        } else {
            policy
        };
        let registry = if read_only {
            ToolRegistry::read_only()
        } else {
            ToolRegistry::new()
        };
        let config = AgentConfig {
            max_turns: if self.max_turns == 0 {
                adaptive_subagent_turns(prompt, read_only).min(crate::subagent::MAX_SUBAGENT_TURNS)
            } else {
                adaptive_subagent_turns(prompt, read_only)
                    .min(self.max_turns)
                    .min(crate::subagent::MAX_SUBAGENT_TURNS)
            },
            max_tool_calls_per_turn: crate::agent::DEFAULT_BOUNDED_TOOL_CALL_CAP,
            subagent_depth: self.depth + 1,
            ..Default::default()
        };
        let base_prompt = if read_only {
            "你是只读探索子代理：只能读取/搜索工作区文件，禁止写入或执行命令；调查完成后用简洁中文汇报发现。\n"
        } else {
            "你是通用子代理：独立完成委派任务，工具调用仍需审批，完成后汇报结果。\n\
             **验收纪律**：先明确任务里的验收标准；每一项结论都要有证据（读到的文件路径、命令输出、\
             测试结果），写进 evidence 字段；无证据的完成声明会被独立复核判为未完成。\n"
        };
        let system_prompt = format!("{base_prompt}{}", contract_system_prompt(read_only));
        let sink = self.events.clone();
        let event_sink = Arc::new(move |event: &TurnEvent| {
            let Some(sink) = sink.as_ref() else {
                return;
            };
            match event {
                TurnEvent::PermissionRequest(_) | TurnEvent::ModelCall => sink(event),
                TurnEvent::ToolStart {
                    id,
                    tool,
                    args_preview,
                } => sink(&TurnEvent::ToolStart {
                    id: id.clone(),
                    tool: format!("sub:{tool}"),
                    args_preview: args_preview.clone(),
                }),
                TurnEvent::ToolResult {
                    id,
                    tool,
                    ok,
                    error,
                    preview,
                    command_receipt,
                    outputs,
                } => sink(&TurnEvent::ToolResult {
                    id: id.clone(),
                    tool: format!("sub:{tool}"),
                    ok: *ok,
                    error: error.clone(),
                    preview: preview.clone(),
                    command_receipt: command_receipt.clone(),
                    outputs: outputs.clone(),
                }),
                _ => {}
            }
        });
        let runtime = crate::worker_runtime::WorkerRuntime {
            provider: Arc::clone(&self.provider),
            approver: self.approver,
            abort: self.abort,
            depth: self.depth,
            model: self.model.clone(),
            workspace: workspace.to_path_buf(),
            registry,
            policy,
            config,
            system_prompt: Some(system_prompt),
            is_critic: read_only,
            event_sink: Some(event_sink),
            session_store: None,
            worker_session_id: None,
            parent_session_id: None,
        };
        runtime
            .run_report(prompt)
            .await
            .map(|report| report.output)
            .map_err(|error| error.message)
    }
}

#[cfg(test)]
mod enforcement_usage_tests {
    use super::enforce_worker_output_contract_with_model;
    use crate::gateway::{
        ChatMessage, ModelCallMetadata, ModelOutput, ModelProvider, ObservedModelOutput, TokenUsage,
    };
    use crate::tools::ToolSpec;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct RepairUsageProvider {
        requested_model: Mutex<Option<String>>,
    }

    #[async_trait]
    impl ModelProvider for RepairUsageProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            Ok(ModelOutput::Text(String::new()))
        }

        async fn complete_with_model_observed(
            &self,
            model: Option<&str>,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ObservedModelOutput, String> {
            *self.requested_model.lock().unwrap() = model.map(str::to_string);
            Ok(ObservedModelOutput {
                output: ModelOutput::Text(
                    r#"{"status":"done","summary":"fixed","artifact":{"kind":"note","format":"text","content":"accepted"},"evidence":[],"open_issues":[]}"#.to_string(),
                ),
                metadata: ModelCallMetadata {
                    usage: Some(TokenUsage {
                        prompt_tokens: 11,
                        completion_tokens: 5,
                        total_tokens: 16,
                    }),
                    ..Default::default()
                },
            })
        }
    }

    #[tokio::test]
    async fn repair_usage_and_model_are_retained() {
        let provider = Arc::new(RepairUsageProvider {
            requested_model: Mutex::new(None),
        });
        let provider_dyn: Arc<dyn ModelProvider> = provider.clone();
        let result = enforce_worker_output_contract_with_model(
            &provider_dyn,
            Some("worker-model"),
            "not a contract",
            false,
        )
        .await
        .unwrap();

        assert_eq!(result.repairs, 1);
        assert_eq!(result.usage.unwrap().total_tokens, 16);
        assert_eq!(
            provider.requested_model.lock().unwrap().as_deref(),
            Some("worker-model")
        );
    }
}

#[cfg(test)]
mod controlled_repair_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct HangingRepair {
        requests: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
    }

    struct RequestGuard(Arc<AtomicBool>);
    impl Drop for RequestGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl ModelProvider for HangingRepair {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[crate::tools::ToolSpec],
        ) -> Result<ModelOutput, String> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            let _guard = RequestGuard(Arc::clone(&self.dropped));
            std::future::pending().await
        }
    }

    fn provider() -> (Arc<dyn ModelProvider>, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let requests = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        (
            Arc::new(HangingRepair {
                requests: Arc::clone(&requests),
                dropped: Arc::clone(&dropped),
            }),
            requests,
            dropped,
        )
    }

    #[tokio::test]
    async fn exhausted_or_cancelled_repair_never_calls_provider() {
        for (cancelled, timeout, allow) in [
            (true, None, true),
            (false, Some(Duration::ZERO), true),
            (false, None, false),
        ] {
            let (provider, requests, _) = provider();
            let abort = AtomicBool::new(cancelled);
            let result = enforce_worker_output_contract_controlled(
                &provider,
                Some("worker"),
                "invalid",
                false,
                Some(&abort),
                timeout,
                allow,
            )
            .await
            .unwrap_err();
            assert_eq!(result.repairs, 0);
            assert_eq!(requests.load(Ordering::SeqCst), 0);
            assert!(result.usage.is_none());
        }
    }

    #[tokio::test]
    async fn repair_timeout_drops_inflight_request_and_counts_unknown_usage() {
        let (provider, requests, dropped) = provider();
        let error = enforce_worker_output_contract_controlled(
            &provider,
            None,
            "invalid",
            false,
            None,
            Some(Duration::from_millis(20)),
            true,
        )
        .await
        .unwrap_err();
        assert!(error.message.contains("worker_deadline_exhausted"));
        assert_eq!(error.repairs, 1);
        assert!(error.usage.is_none());
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancelling_repair_drops_inflight_request_without_waiting_for_provider() {
        let (provider, requests, dropped) = provider();
        let abort = AtomicBool::new(false);
        let cancel = async {
            while requests.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            abort.store(true, Ordering::SeqCst);
        };
        let repair = enforce_worker_output_contract_controlled(
            &provider,
            None,
            "invalid",
            false,
            Some(&abort),
            None,
            true,
        );
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(repair, cancel)
        })
        .await
        .expect("cancel must not wait on the provider");
        let error = result.unwrap_err();
        assert!(error.message.contains("worker_aborted"));
        assert_eq!(error.repairs, 1);
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn valid_contract_needs_no_repair_even_without_remaining_request_budget() {
        let (provider, requests, _) = provider();
        let result = enforce_worker_output_contract_controlled(
            &provider, None,
            r#"{"status":"done","summary":"fixed","artifact":{"kind":"note","format":"text","content":"accepted"},"evidence":[],"open_issues":[]}"#,
            false, None, Some(Duration::ZERO), false,
        ).await.unwrap();
        assert_eq!(result.repairs, 0);
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }
}
