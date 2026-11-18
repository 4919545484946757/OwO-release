//! 真实单 Agent 执行器（V1-R1 第二天 · 第一路）集成测试：
//! - 审批器：write_file 只放行 allow_write、run_command 只放行任务命令白名单；
//! - 范围工具：allow_read / allow_write 逐调用强制（直接构造 ToolContext）；
//! - 端到端（MatrixRunner + 脚本 Provider）：真实 run_turn 产出产物与工具轨迹、
//!   越界写被拒并登记、取消后零模型调用、预算超限中止、真实套件 10 任务 dry 自检。

use async_trait::async_trait;
use owo_agent_core::audit::AuditLog;
use owo_agent_core::element_registry::ElementRegistry;
use owo_agent_core::gateway::{
    ChatMessage, ModelCallMetadata, ModelOutput, ModelProvider, ObservedModelOutput, StreamChunk,
    TokenUsage, ToolCall,
};
use owo_agent_core::permissions::{Approver, Decision, Policy};
use owo_agent_core::session::Session;
use owo_agent_core::skill::SkillRegistry;
use owo_agent_core::tools::{Tool, ToolContext, ToolSpec};
use owo_agent_product_eval::product_eval::single_agent::{
    ProductEvalApprover, ProductEvalScope, ScopeListDir, ScopeReadFile, ScopeRunCommand,
    ScopeSearchFiles, ScopeWriteFile, SingleAgentExecutor,
};
use owo_agent_product_eval::product_eval::{
    load_suite, validate_suite, AgentMode, ArtifactChecker, CaseExecutor, EvalCategory,
    InputFixture, MatrixRunner, ProductEvalCase, ProductEvalReport, ProductEvalSuite,
    ReferenceDryExecutor, RunOptions, RunStatus, SuiteBundle, PRODUCT_EVAL_SCHEMA_VERSION,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

struct FailingProvider;

#[async_trait]
impl ModelProvider for FailingProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        Err("gateway error: stream decoder detail".to_string())
    }
}

// ---------------------------------------------------------------------------
// 构造工具
// ---------------------------------------------------------------------------

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pe-sa-tests-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn agent_case(id: &str, category: EvalCategory) -> ProductEvalCase {
    ProductEvalCase {
        schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
        id: id.to_string(),
        category,
        title: format!("任务 {id}"),
        instruction: "阅读输入材料，把结论写入报告。".to_string(),
        inputs: vec![InputFixture {
            path: "inputs/a.txt".to_string(),
            content: "材料：alpha；结论：alpha 通过。".to_string(),
        }],
        allow_read: vec!["inputs/**".to_string()],
        allow_write: vec!["out/**".to_string()],
        expected_artifacts: vec!["out/report.md".to_string()],
        checkers: vec![ArtifactChecker::Contains {
            path: "out/report.md".to_string(),
            text: "alpha".to_string(),
        }],
        reference_outputs: BTreeMap::new(),
        timeout_secs: Some(120),
        max_model_calls: Some(6),
        repetitions: None,
        allow_commands: Vec::new(),
        failure_classes: Vec::new(),
        expected_side_effects: Vec::new(),
        fault_injection: None,
    }
}

fn tool_call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{}", uuid::Uuid::new_v4().simple()),
        name: name.to_string(),
        arguments: args,
    }
}

/// 脚本 Provider：按队列回放模型输出；可选逐调用累计 token 用量。
struct ScriptedProvider {
    outputs: Mutex<VecDeque<ModelOutput>>,
    calls: AtomicU32,
    tokens_per_call: u64,
    usage: Mutex<TokenUsage>,
}

impl ScriptedProvider {
    fn new(outputs: Vec<ModelOutput>) -> Arc<Self> {
        Self::with_usage(outputs, 0)
    }

    fn with_usage(outputs: Vec<ModelOutput>, tokens_per_call: u64) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(outputs.into_iter().collect()),
            calls: AtomicU32::new(0),
            tokens_per_call,
            usage: Mutex::new(TokenUsage::default()),
        })
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.tokens_per_call > 0 {
            let mut usage = self.usage.lock().unwrap();
            usage.prompt_tokens += self.tokens_per_call / 2;
            usage.completion_tokens += self.tokens_per_call / 2;
            usage.total_tokens += self.tokens_per_call;
        }
        let mut queue = self.outputs.lock().unwrap();
        queue.pop_front().ok_or_else(|| "脚本输出耗尽".to_string())
    }

    async fn complete_stream_with_reasoning_and_model_observed(
        &self,
        _model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ObservedModelOutput, String> {
        let output = self.complete(messages, tools).await?;
        if let ModelOutput::Text(text) = &output {
            on_chunk(StreamChunk::Content(text.clone()));
        }
        Ok(ObservedModelOutput {
            reasoning_replay: None,
            output,
            metadata: ModelCallMetadata {
                usage: Some(TokenUsage {
                    prompt_tokens: self.tokens_per_call / 2,
                    completion_tokens: self.tokens_per_call / 2,
                    total_tokens: self.tokens_per_call,
                }),
                ..ModelCallMetadata::default()
            },
        })
    }

    fn usage_snapshot(&self) -> TokenUsage {
        *self.usage.lock().unwrap()
    }
}

fn test_bundle(cases: Vec<ProductEvalCase>) -> SuiteBundle {
    SuiteBundle {
        dir: std::env::temp_dir(),
        suite: ProductEvalSuite {
            schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
            name: "single-agent-test-suite".to_string(),
            description: String::new(),
            defaults: Default::default(),
            tasks: cases
                .iter()
                .map(|c| format!("tasks/{}.json", c.id))
                .collect(),
        },
        cases,
    }
}

fn no_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

async fn run_matrix(
    executor: Arc<dyn CaseExecutor>,
    cases: Vec<ProductEvalCase>,
    cancel: Arc<AtomicBool>,
) -> (ProductEvalReport, PathBuf) {
    let out = temp_dir("matrix");
    let runner = MatrixRunner::new(test_bundle(cases), &out);
    let opts = RunOptions {
        modes: vec![AgentMode::Single],
        reps_override: Some(1),
        only: None,
        category: None,
        fresh: true,
        batch_label: None,
        provider_endpoint_sha256: None,
        tags: Vec::new(),
        repair_attempts: 0,
        desktop_driver: None,
        fault_driver: None,
        driver_config: None,
    };
    let report = runner
        .run(
            executor,
            "live-agent-test",
            Some("stub-model".to_string()),
            &opts,
            cancel,
        )
        .await
        .expect("矩阵执行失败");
    (report, out)
}

/// 直接调用工具的测试脚手架：作用域内构造 ToolContext（局部借用，无别名）。
macro_rules! call_tool {
    ($sandbox:expr, $tool:expr, $args:expr) => {{
        let mut session = Session::new($sandbox, "stub-model", None);
        let policy = Policy::new($sandbox);
        let audit = Arc::new(Mutex::new(AuditLog::default()));
        let skills = SkillRegistry::default();
        let elements = Arc::new(Mutex::new(ElementRegistry::new()));
        let mut ctx = ToolContext {
            workspace: $sandbox,
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
        $tool.run(&mut ctx, $args).await
    }};
}

fn seeded_sandbox(tag: &str) -> PathBuf {
    let sandbox = temp_dir(tag);
    std::fs::create_dir_all(sandbox.join("inputs")).unwrap();
    std::fs::create_dir_all(sandbox.join("notes")).unwrap();
    std::fs::write(sandbox.join("inputs/a.txt"), "alpha 材料").unwrap();
    std::fs::write(sandbox.join("notes/secret.txt"), "机密").unwrap();
    sandbox
}

// ---------------------------------------------------------------------------
// 1) 审批器：范围写 + 命令白名单 + 未知工具
// ---------------------------------------------------------------------------

async fn approver_verdict(
    approver: &ProductEvalApprover,
    policy: &Policy,
    tool: &str,
    args: Value,
) -> Decision {
    let request = policy.evaluate(tool, &args);
    if policy.decision(&request) == Decision::Deny {
        return Decision::Deny;
    }
    approver.decide(&request).await
}

#[tokio::test]
async fn approver_allows_only_task_scoped_writes_and_commands() {
    let sandbox = temp_dir("approver");
    let mut scope = ProductEvalScope::from_case(&agent_case("appr", EvalCategory::Code));
    scope.allow_commands = vec!["cargo test".to_string()];
    let approver = ProductEvalApprover::new(Arc::new(scope), Arc::new(Mutex::new(Vec::new())));
    let policy = Policy::new(&sandbox);

    // 范围内写放行；越界写拒绝。
    assert_eq!(
        approver_verdict(
            &approver,
            &policy,
            "write_file",
            json!({"path": "out/report.md"})
        )
        .await,
        Decision::Allow
    );
    assert_eq!(
        approver_verdict(&approver, &policy, "write_file", json!({"path": "evil.md"})).await,
        Decision::Deny
    );

    // 白名单命令放行（精确 + 前缀带参）；其余命令拒绝；链式元字符拒绝。
    assert_eq!(
        approver_verdict(
            &approver,
            &policy,
            "run_command",
            json!({"command": "cargo test"})
        )
        .await,
        Decision::Allow
    );
    assert_eq!(
        approver_verdict(
            &approver,
            &policy,
            "run_command",
            json!({"command": "cargo test --lib format"})
        )
        .await,
        Decision::Allow
    );
    assert_eq!(
        approver_verdict(
            &approver,
            &policy,
            "run_command",
            json!({"command": "cargo publish"})
        )
        .await,
        Decision::Deny
    );
    assert_eq!(
        approver_verdict(
            &approver,
            &policy,
            "run_command",
            json!({"command": "cargo test && echo hacked"})
        )
        .await,
        Decision::Deny
    );

    // 未声明的工具（桌面/浏览器等）一律拒绝：评测不继承仓库级权限。
    assert_eq!(
        approver_verdict(
            &approver,
            &policy,
            "browser_navigate",
            json!({"url": "https://example.com"})
        )
        .await,
        Decision::Deny
    );

    // 工作区越界路径在 Policy 层即被拒（拒绝不依赖审批器）。
    let request = policy.evaluate("write_file", &json!({"path": "../escape.md"}));
    assert_eq!(policy.decision(&request), Decision::Deny);

    let _ = std::fs::remove_dir_all(&sandbox);
}

#[test]
fn empty_command_whitelist_denies_everything() {
    let scope = ProductEvalScope::from_case(&agent_case("appr2", EvalCategory::Document));
    assert!(scope.allow_commands.is_empty(), "默认白名单必须为空");
    assert!(!scope.command_ok("cargo test"));
    assert!(!scope.command_ok("echo ok"));
    assert!(!scope.command_ok(""));
    // 前缀语义：白名单条目整条或带后续参数才放行。
    let mut scoped = ProductEvalScope::from_case(&agent_case("appr3", EvalCategory::Code));
    scoped.allow_commands = vec!["cargo test".to_string()];
    assert!(scoped.command_ok("cargo test"));
    assert!(scoped.command_ok("cargo test --lib format"));
    assert!(!scoped.command_ok("cargo testx"));
    assert!(!scoped.command_ok("cargo publish"));
    assert!(!scoped.command_ok("echo ok | cargo test"));
}

// ---------------------------------------------------------------------------
// 2) 范围工具：allow_read / allow_write 逐调用强制
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scope_read_tool_enforces_allow_read() {
    let sandbox = seeded_sandbox("scope-read");
    let scope = Arc::new(ProductEvalScope::from_case(&agent_case(
        "tools",
        EvalCategory::Code,
    )));
    let log = Arc::new(Mutex::new(Vec::new()));
    let tool = ScopeReadFile::new(Arc::clone(&scope), log);

    let ok = call_tool!(&sandbox, tool, json!({"path": "inputs/a.txt"}));
    assert!(ok.is_ok(), "{ok:?}");
    assert_eq!(ok.unwrap()["content"].as_str().unwrap(), "alpha 材料");

    let denied = call_tool!(&sandbox, tool, json!({"path": "notes/secret.txt"}));
    assert!(denied.is_err());
    assert!(denied.unwrap_err().starts_with("read_denied:"));

    let _ = std::fs::remove_dir_all(&sandbox);
}

#[tokio::test]
async fn scope_write_tool_enforces_allow_write() {
    let sandbox = seeded_sandbox("scope-write");
    let scope = Arc::new(ProductEvalScope::from_case(&agent_case(
        "tools",
        EvalCategory::Code,
    )));
    let log = Arc::new(Mutex::new(Vec::new()));
    let tool = ScopeWriteFile::new(Arc::clone(&scope), log);

    let ok = call_tool!(
        &sandbox,
        tool,
        json!({"path": "out/report.md", "content": "# 报告\nalpha"})
    );
    assert!(ok.is_ok(), "{ok:?}");
    assert!(sandbox.join("out/report.md").exists());

    let denied = call_tool!(&sandbox, tool, json!({"path": "evil.md", "content": "x"}));
    assert!(denied.is_err());
    assert!(denied.unwrap_err().starts_with("write_denied:"));
    assert!(!sandbox.join("evil.md").exists(), "越界写不得产生文件");

    let _ = std::fs::remove_dir_all(&sandbox);
}

#[tokio::test]
async fn scope_list_and_search_respect_allow_read() {
    let sandbox = seeded_sandbox("scope-list");
    let scope = Arc::new(ProductEvalScope::from_case(&agent_case(
        "tools",
        EvalCategory::Code,
    )));
    let log = Arc::new(Mutex::new(Vec::new()));
    let list = ScopeListDir::new(Arc::clone(&scope), Arc::clone(&log));
    let search = ScopeSearchFiles::new(Arc::clone(&scope), Arc::clone(&log));

    // 根目录与范围内目录可列；范围外目录拒绝。
    assert!(call_tool!(&sandbox, list, json!({"path": "."})).is_ok());
    assert!(call_tool!(&sandbox, list, json!({"path": "inputs"})).is_ok());
    let denied = call_tool!(&sandbox, list, json!({"path": "notes"}));
    assert!(denied.is_err());

    // 搜索结果仅保留 allow_read 内的路径。
    let found = call_tool!(&sandbox, search, json!({"pattern": "txt"})).unwrap();
    let matches = found.get("matches").and_then(Value::as_array).unwrap();
    assert_eq!(matches.len(), 1, "{found}");
    assert_eq!(matches[0].as_str().unwrap(), "inputs/a.txt");

    let _ = std::fs::remove_dir_all(&sandbox);
}

#[tokio::test]
async fn scope_run_command_denies_empty_whitelist_and_runs_whitelisted() {
    let sandbox = seeded_sandbox("scope-cmd");
    let scope = Arc::new(ProductEvalScope::from_case(&agent_case(
        "tools",
        EvalCategory::Code,
    )));
    let log = Arc::new(Mutex::new(Vec::new()));
    let tool = ScopeRunCommand::new(Arc::clone(&scope), Arc::clone(&log));

    // 空白名单：一切命令拒绝。
    let denied = call_tool!(&sandbox, tool, json!({"command": "echo ok"}));
    assert!(denied.is_err());
    assert!(denied.unwrap_err().starts_with("command_denied:"));

    // 白名单内：真实执行（受控测试命令路径）。
    let mut allowed_scope = ProductEvalScope::from_case(&agent_case("tools", EvalCategory::Code));
    allowed_scope.allow_commands = vec!["echo ok".to_string()];
    let allowed_tool = ScopeRunCommand::new(Arc::new(allowed_scope), Arc::clone(&log));
    let ok = call_tool!(&sandbox, allowed_tool, json!({"command": "echo ok"}));
    assert!(ok.is_ok(), "{ok:?}");
    assert_eq!(ok.unwrap()["exit_code"], json!(0));

    // 链式元字符：即使前缀命中也拒绝。
    let chained = call_tool!(
        &sandbox,
        allowed_tool,
        json!({"command": "echo ok & echo hacked"})
    );
    assert!(chained.is_err());

    let _ = std::fs::remove_dir_all(&sandbox);
}

// ---------------------------------------------------------------------------
// 3) 端到端：真实 run_turn 产出产物 + 工具轨迹 + 用量
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_agent_turn_produces_artifacts_and_records_tool_log() {
    let case = {
        let mut case = agent_case("sa-live", EvalCategory::Code);
        case.allow_commands = vec!["echo ok".to_string()];
        case
    };
    let provider = ScriptedProvider::with_usage(
        vec![
            ModelOutput::ToolCalls(vec![
                tool_call("run_command", json!({"command": "echo ok"})),
                tool_call(
                    "write_file",
                    json!({"path": "out/report.md", "content": "# 报告\n结论：alpha 通过\n"}),
                ),
            ]),
            ModelOutput::Text("完成：已写 out/report.md。".to_string()),
        ],
        100,
    );
    let executor: Arc<SingleAgentExecutor> =
        Arc::new(SingleAgentExecutor::new(provider.clone(), "stub-model"));

    let (report, out) = run_matrix(executor.clone(), vec![case], no_cancel()).await;
    let run = &report.runs[0];
    assert_eq!(run.status, RunStatus::Passed, "{run:?}");
    assert_eq!(run.model_calls, 2, "{run:?}");
    assert_eq!(run.total_tokens, Some(200), "{run:?}");
    assert!(
        run.artifact_refs.iter().any(|rel| rel == "out/report.md"),
        "{run:?}"
    );
    assert!(
        run.tool_log
            .iter()
            .any(|entry| entry.starts_with("write_file out/report.md")),
        "journal 应记录真实工具调用轨迹：{:?}",
        run.tool_log
    );
    assert!(run.failed_steps.is_empty(), "{:?}", run.failed_steps);
    assert_eq!(provider.calls(), 2);

    let telemetry = executor.take_last_telemetry().expect("遥测快照缺失");
    assert_eq!(telemetry.model_calls, 2);
    assert!(telemetry.denials.is_empty(), "{:?}", telemetry.denials);
    assert_eq!(telemetry.approval_requests, 2, "{:?}", telemetry.tool_log);
    assert!(
        telemetry
            .tool_log
            .iter()
            .any(|entry| entry.starts_with("write_file out/report.md")),
        "{:?}",
        telemetry.tool_log
    );
    assert!(
        telemetry
            .tool_log
            .iter()
            .any(|entry| entry.starts_with("run_command echo ok") && entry.contains("exit 0")),
        "{:?}",
        telemetry.tool_log
    );
    assert!(telemetry.final_text.is_some());

    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------------
// 4) 端到端：越界写被拒 + 缺产物判错
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_agent_denies_out_of_scope_write_and_flags_missing_artifact() {
    let case = agent_case("sa-deny", EvalCategory::Document);
    let provider = ScriptedProvider::new(vec![
        ModelOutput::ToolCalls(vec![tool_call(
            "write_file",
            json!({"path": "evil.md", "content": "越界内容"}),
        )]),
        ModelOutput::Text("写完了（其实什么都没写成）。".to_string()),
    ]);
    let executor: Arc<SingleAgentExecutor> =
        Arc::new(SingleAgentExecutor::new(provider, "stub-model"));

    let (report, out) = run_matrix(executor.clone(), vec![case], no_cancel()).await;
    let run = &report.runs[0];
    assert_eq!(run.status, RunStatus::Error, "{run:?}");
    assert!(
        run.failed_steps
            .iter()
            .any(|step| step.starts_with("denied:")),
        "{:?}",
        run.failed_steps
    );
    assert!(
        run.failed_steps
            .iter()
            .any(|step| step.starts_with("missing_artifact:")),
        "{:?}",
        run.failed_steps
    );
    assert!(run
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("预期产物缺失"));
    assert!(run.artifact_refs.is_empty(), "{run:?}");

    // 失败沙盒留档：越界文件不存在（审批拒绝 → 写入未执行）。
    let kept = out.join("failures").join("sa-deny__single__0");
    if kept.exists() {
        assert!(!kept.join("evil.md").exists());
    }
    let telemetry = executor.take_last_telemetry().expect("遥测快照缺失");
    assert!(telemetry.approval_requests >= 1);
    assert!(!telemetry.denials.is_empty(), "{:?}", telemetry.denials);

    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn provider_failure_is_not_masked_by_missing_artifacts() {
    let case = agent_case("sa-preserve-provider-error", EvalCategory::Code);
    let executor: Arc<SingleAgentExecutor> = Arc::new(SingleAgentExecutor::new(
        Arc::new(FailingProvider),
        "stub-model",
    ));
    let (report, out) = run_matrix(executor, vec![case], no_cancel()).await;
    let run = &report.runs[0];
    assert_eq!(run.status, RunStatus::Error, "{run:?}");
    let error = run.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("gateway error: stream decoder detail"),
        "{run:?}"
    );
    assert!(error.contains("预期产物缺失"), "{run:?}");
    assert!(run
        .failed_steps
        .iter()
        .any(|step| step.starts_with("missing_artifact:")));
    let _ = std::fs::remove_dir_all(out);
}

// ---------------------------------------------------------------------------
// 5) 取消：矩阵级预置取消 = 零单元格；运行中取消 = 不再调用模型/执行工具
// ---------------------------------------------------------------------------

/// 第一次调用后置位取消令牌的 Provider（模拟 Ctrl-C 在回合中途命中）。
struct CancelOnFirstCall {
    calls: AtomicU32,
    cancel: Arc<AtomicBool>,
}

#[async_trait]
impl ModelProvider for CancelOnFirstCall {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        let nth = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if nth == 1 {
            self.cancel.store(true, Ordering::SeqCst);
            Ok(ModelOutput::ToolCalls(vec![tool_call(
                "write_file",
                json!({"path": "out/report.md", "content": "# 报告\nalpha"}),
            )]))
        } else {
            Ok(ModelOutput::Text("取消后的调用（不应发生）".to_string()))
        }
    }

    fn usage_snapshot(&self) -> TokenUsage {
        TokenUsage::default()
    }
}

#[tokio::test]
async fn preset_cancel_schedules_no_cells() {
    let case = agent_case("sa-cancel0", EvalCategory::Research);
    let provider = ScriptedProvider::new(vec![ModelOutput::Text("不应被调用".to_string())]);
    let executor: Arc<dyn CaseExecutor> =
        Arc::new(SingleAgentExecutor::new(provider.clone(), "stub-model"));

    let (report, out) = run_matrix(executor, vec![case], Arc::new(AtomicBool::new(true))).await;
    assert!(report.runs.is_empty(), "预置取消 = 不调度任何单元格");
    assert_eq!(report.pending.len(), 1, "未完成单元格保留在 pending");
    assert_eq!(provider.calls(), 0, "取消命中后不得有任何模型调用");

    let _ = std::fs::remove_dir_all(&out);
}

#[tokio::test]
async fn mid_run_cancel_stops_model_and_tool_calls() {
    let case = agent_case("sa-cancel1", EvalCategory::Research);
    let cancel = Arc::new(AtomicBool::new(false));
    let provider = Arc::new(CancelOnFirstCall {
        calls: AtomicU32::new(0),
        cancel: Arc::clone(&cancel),
    });
    let executor: Arc<dyn CaseExecutor> =
        Arc::new(SingleAgentExecutor::new(provider.clone(), "stub-model"));

    let (report, out) = run_matrix(executor, vec![case], cancel).await;
    let run = &report.runs[0];
    assert_eq!(run.status, RunStatus::Cancelled, "{run:?}");
    assert_eq!(run.cancellations, 1);
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "取消后不得再调用模型（第二次调用不应发生）"
    );
    assert!(
        run.failed_steps
            .iter()
            .any(|step| step.starts_with("cancelled:")),
        "{:?}",
        run.failed_steps
    );
    // 工具调用同样被拦下：取消后的 write_file 未执行，产物不存在。
    let kept = out.join("failures").join("sa-cancel1__single__0");
    assert!(kept.exists(), "取消沙盒应留档");
    assert!(
        !kept.join("out/report.md").exists(),
        "取消后工具不得继续执行"
    );

    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------------
// 6) 预算超限：同一 abort 令牌收口，不再继续调用模型
// ---------------------------------------------------------------------------

#[tokio::test]
async fn budget_exceeded_is_error_not_cancel_and_does_not_poison_matrix() {
    let mut case = agent_case("sa-budget", EvalCategory::Code);
    case.max_model_calls = Some(1);
    let provider = ScriptedProvider::new(vec![
        ModelOutput::ToolCalls(vec![tool_call(
            "write_file",
            json!({"path": "out/report.md", "content": "# 报告\nalpha"}),
        )]),
        ModelOutput::Text("第一次总结".to_string()),
        ModelOutput::Text("第二次总结（不应发生）".to_string()),
    ]);
    let executor: Arc<SingleAgentExecutor> =
        Arc::new(SingleAgentExecutor::new(provider.clone(), "stub-model"));

    let (report, out) = run_matrix(executor.clone(), vec![case], no_cancel()).await;
    let run = &report.runs[0];
    // 预算耗尽 = Error（保留 budget 失败步骤），不得冒充 cancelled；
    // 回合上限 = 预算是精确收口，绝不触碰共享取消令牌毒化后续单元格。
    assert_eq!(run.status, RunStatus::Error, "{run:?}");
    assert_eq!(run.cancellations, 0, "{run:?}");
    assert!(
        provider.calls() <= 1,
        "预算后不得继续调用模型（实际 {} 次）",
        provider.calls()
    );
    assert_eq!(
        run.model_calls,
        provider.calls(),
        "报表只统计预算闸门放行的真实 Provider 调用，不计入被拒绝的尝试"
    );
    assert!(
        run.failed_steps
            .iter()
            .any(|step| step.starts_with("budget:")),
        "{:?}",
        run.failed_steps
    );
    let telemetry = executor.take_last_telemetry().expect("遥测快照缺失");
    assert!(telemetry.budget_exceeded);
    assert!(!telemetry.aborted, "预算不等于取消：aborted 必须为 false");

    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------------
// 7) dry 自检：真实 v1 套件 10 任务全绿（框架 + 参考输出 + 检查器可达）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_ten_tasks_pass_dry_reference_self_check() {
    let suite_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/v1/suite.json");
    let bundle = load_suite(&suite_path).expect("加载真实套件失败");
    assert_eq!(bundle.cases.len(), 10, "套件应有 10 个任务");
    let validation = validate_suite(&bundle);
    let issues: Vec<String> = validation
        .tasks
        .iter()
        .flat_map(|task| task.issues.iter().cloned())
        .collect();
    assert!(validation.all_ok, "{issues:?}");

    let out = temp_dir("dry-selfcheck");
    let runner = MatrixRunner::new(bundle, &out);
    let opts = RunOptions {
        modes: vec![AgentMode::Single, AgentMode::Multi],
        reps_override: Some(1),
        only: None,
        category: None,
        fresh: true,
        batch_label: None,
        provider_endpoint_sha256: None,
        tags: Vec::new(),
        repair_attempts: 0,
        desktop_driver: None,
        fault_driver: None,
        driver_config: None,
    };
    let report = runner
        .run(
            Arc::new(ReferenceDryExecutor),
            "dry-reference",
            Some("self-check".to_string()),
            &opts,
            no_cancel(),
        )
        .await
        .expect("dry 自检失败");
    assert_eq!(report.runs.len(), 20, "10 任务 × 2 拓扑");
    let failures: Vec<String> = report
        .runs
        .iter()
        .filter(|run| run.status != RunStatus::Passed)
        .map(|run| format!("{:?}: {:?}", run.key, run.failed_steps))
        .collect();
    assert!(
        failures.is_empty(),
        "存在失败单元格：{}",
        failures.join("\n")
    );
    assert_eq!(report.metrics.success_rate, 1.0);

    let _ = std::fs::remove_dir_all(&out);
}
