//! Regression scripts use the real ToolHost/filesystem/process runner. They are
//! deterministic loop tests, not substitutes for live-model product acceptance.
use owo_agent_core::agent::{Agent, AgentConfig, TurnEvent};
use owo_agent_core::gateway::ToolCall;
use owo_agent_core::gateway::{ChatMessage, ModelOutput, ModelProvider};
use owo_agent_core::permissions::Policy;
use owo_agent_core::session::Session;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::tools::ToolSpec;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

struct Script(Mutex<VecDeque<ModelOutput>>);
#[async_trait::async_trait]
impl ModelProvider for Script {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        if tools.is_empty() {
            let input = messages
                .last()
                .and_then(|message| message.content.as_deref())
                .unwrap_or_default();
            if input.contains("FILE calc.py") {
                assert!(
                    input.contains("FILE test_calc.py"),
                    "review must see unchanged tests in the verified scope"
                );
                assert!(input.contains("host_command_receipts"));
                assert!(input.contains("\"exit_code\": 1"));
                assert!(input.contains("\"exit_code\": 0"));
            }
        }
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| "script exhausted".into())
    }
}
fn call(id: &str, name: &str, arguments: serde_json::Value) -> ModelOutput {
    ModelOutput::ToolCalls(vec![ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }])
}
fn agent(workspace: &std::path::Path, outputs: Vec<ModelOutput>, config: AgentConfig) -> Agent {
    Agent::new(
        Arc::new(Script(Mutex::new(outputs.into()))),
        ToolRegistry::new(),
        Policy::new(workspace),
        config,
    )
}

#[tokio::test]
async fn report_without_plan_is_repaired_and_verified_in_the_same_turn() {
    let workspace = tempfile::tempdir().unwrap();
    let agent = agent(
        workspace.path(),
        vec![
            call(
                "write",
                "write_file",
                json!({"path":"report.md", "content":"Analysis: verified report"}),
            ),
            call(
                "plan",
                "verification_plan",
                json!({"plan":{
                    "plan_id":"report-check", "requirements":[{
                        "requirement_id":"report-content", "covers_requirement_ids":["user-request:Create report.md containing verified report"],
                        "validator_id":"workspace-file-contains-v1", "validator_version":"1",
                        "scope":{"kind":"workspace_paths", "relative_paths":["report.md"]},
                        "arguments":{"text":"verified report"}, "required":true,
                        "resources":{"cpu_slots":1,"memory_mb":16,"exclusive_workspace":false,"timeout_ms":10000}
                    }]
                }}),
            ),
            call(
                "write-retry",
                "write_file",
                json!({"path":"report.md", "content":"Analysis: verified report"}),
            ),
            ModelOutput::Text("Report verified".into()),
        ],
        AgentConfig::default(),
    );
    let mut session = Session::new(workspace.path(), "script", None);
    let outcome = agent
        .run_turn(
            &mut session,
            "Create report.md containing verified report",
            &owo_agent_core::permissions::AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    record(
        "report-recovery",
        &agent,
        &session,
        &outcome,
        workspace.path(),
        &["report.md"],
    );
    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Accepted
    );
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("report.md")).unwrap(),
        "Analysis: verified report"
    );
    assert!(session.messages.iter().any(|message| message.role == "tool"
        && message
            .content
            .as_deref()
            .is_some_and(|text| text.contains("tool/verification_plan_required"))));
    assert_eq!(
        outcome
            .events
            .iter()
            .filter(|event| matches!(event, TurnEvent::Final { .. }))
            .count(),
        1
    );
    assert!(!session.validation_receipts.is_empty());
}

#[tokio::test]
async fn ignored_preflight_stops_without_writing_or_claiming_acceptance() {
    let workspace = tempfile::tempdir().unwrap();
    let agent = agent(
        workspace.path(),
        vec![
            call(
                "write",
                "write_file",
                json!({"path":"report.txt", "content":"candidate"}),
            ),
            ModelOutput::Text("done".into()),
        ],
        AgentConfig::default(),
    );
    let mut session = Session::new(workspace.path(), "script", None);
    let outcome = agent
        .run_turn(
            &mut session,
            "Create report.txt",
            &owo_agent_core::permissions::AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    record(
        "unplanned-write-blocked",
        &agent,
        &session,
        &outcome,
        workspace.path(),
        &[],
    );
    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Blocked
    );
    assert!(outcome
        .final_text
        .as_deref()
        .unwrap()
        .contains("任务尚未完成"));
    assert_eq!(outcome.model_calls.len(), 2);
    assert!(!workspace.path().join("report.txt").exists());
    assert!(session.execution_receipts.is_empty());
}

#[cfg(windows)]
#[tokio::test]
async fn alternating_failed_processes_do_not_reset_loop_guard_or_audit_success() {
    let workspace = tempfile::tempdir().unwrap();
    let mut script = Vec::new();
    for index in 0..6 {
        script.push(call(
            &format!("cmd-{index}"),
            "run_command",
            json!({"command":format!("exit /b {}", 7 + index % 2)}),
        ));
    }
    script.push(ModelOutput::Text("Commands failed; work remains".into()));
    let agent = agent(
        workspace.path(),
        script,
        AgentConfig {
            max_repeated_tool_calls: 2,
            ..Default::default()
        },
    );
    let mut session = Session::new(workspace.path(), "script", None);
    let outcome = agent
        .run_turn(
            &mut session,
            "Check command failures",
            &owo_agent_core::permissions::AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    record(
        "failed-processes",
        &agent,
        &session,
        &outcome,
        workspace.path(),
        &[],
    );
    let process_results = outcome
        .events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ToolResult {
                ok,
                error,
                command_receipt: Some(receipt),
                ..
            } => Some((ok, error, receipt)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        process_results.len(),
        4,
        "only the first two attempts per command may execute"
    );
    for (ok, error, receipt) in process_results {
        assert!(!ok);
        assert!(matches!(receipt.exit_code, 7 | 8));
        assert!(error.as_deref().unwrap().contains("executor/process_exit"));
    }
    let denied = outcome
        .events
        .iter()
        .filter(|event| {
            matches!(
                event,
                TurnEvent::ToolResult {
                    ok: false,
                    command_receipt: None,
                    error: Some(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        denied, 2,
        "both alternating calls must eventually be guarded"
    );
    let log = agent.audit_log();
    let audit = log.lock().unwrap();
    let receipts = audit
        .entries
        .iter()
        .filter(|entry| entry.event == "tool_receipt")
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 4);
    for receipt in receipts {
        let detail: serde_json::Value = serde_json::from_str(&receipt.detail).unwrap();
        assert_eq!(detail["ok"], false);
        assert!(detail["result_sha256"].is_string());
    }
    assert!(session
        .messages
        .iter()
        .filter_map(|message| message.content.as_deref())
        .any(|content| content.contains("exit_code") && content.contains('7')));
}

/// Optional test-run evidence output, never a production execution path.
fn record(
    label: &str,
    agent: &Agent,
    session: &Session,
    outcome: &owo_agent_core::TurnOutcome,
    workspace: &std::path::Path,
    files: &[&str],
) {
    let Some(root) = std::env::var_os("OWO_EXECUTION_EVIDENCE_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(root).join(label);
    std::fs::create_dir_all(&dir).unwrap();
    let audit = agent.audit_log();
    let audit = audit.lock().unwrap();
    let value = json!({
        "model": "scripted regression driver; not live-model acceptance",
        "tools": "real ToolHost, real filesystem and Windows processes",
        "outcome": outcome,
        "messages": session.messages,
        "execution_receipts": session.execution_receipts,
        "validation_receipts": session.validation_receipts,
        "audit": audit.entries,
    });
    std::fs::write(
        dir.join("evidence.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    for file in files {
        std::fs::copy(workspace.join(file), dir.join(file)).unwrap();
    }
}

#[cfg(windows)]
#[tokio::test]
async fn real_code_test_failure_repair_and_diff_are_delivered() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("calc.py"),
        "def add(a, b):\n    return a - b\n",
    )
    .unwrap();
    std::fs::write(workspace.path().join("test_calc.py"),
        "import sys\nsys.dont_write_bytecode = True\nimport unittest\nfrom calc import add\nclass Tests(unittest.TestCase):\n    def test_add(self):\n        self.assertEqual(add(2, 3), 5)\n").unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "calc.py", "test_calc.py"],
    ] {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(workspace.path())
            .status()
            .unwrap()
            .success());
    }
    let request = "修复加法并运行测试";
    let command = "python -m unittest test_calc.py";
    let agent = agent(workspace.path(), vec![
        call("read", "read_file", json!({"path":"calc.py"})),
        call("plan", "verification_plan", json!({"plan": {
            "plan_id":"calc-check", "requirements":[{
                "requirement_id":"calc-tests", "covers_requirement_ids":[format!("user-request:{request}")],
                "validator_id":"workspace-command-success-v1", "validator_version":"1",
                "scope":{"kind":"workspace_paths", "relative_paths":["calc.py","test_calc.py"]},
                "arguments":{"command":command}, "required":true,
                "resources":{"cpu_slots":1,"memory_mb":64,"exclusive_workspace":false,"timeout_ms":30000}
            }]
        }})),
        call("test-fail", "run_command", json!({"command":command})),
        call("fix", "write_file", json!({"path":"calc.py","content":"def add(a, b):\n    return a + b\n"})),
        call("test-pass", "run_command", json!({"command":command})),
        call("diff", "git_diff", json!({})),
        ModelOutput::Text("Tests passed; diff attached".into()),
        ModelOutput::Text(json!({
            "status":"done","summary":"Scripted review fixture",
            "review_result":{"verdict":"approved","reviewed_requirement_ids":["calc-tests",format!("user-request:{request}")],"findings":[]},
            "evidence":[{"source":"calc.py","note":"actual host command evidence"},{"source":"test_calc.py","note":"unchanged test snapshot and host command proof"}],"open_issues":[]
        }).to_string()),
    ],AgentConfig{max_turns:12,..Default::default()});
    let mut session = Session::new(workspace.path(), "script", None);
    let outcome = agent
        .run_turn(
            &mut session,
            request,
            &owo_agent_core::permissions::AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    record(
        "code-repair",
        &agent,
        &session,
        &outcome,
        workspace.path(),
        &["calc.py", "test_calc.py"],
    );
    let codes = outcome
        .events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ToolResult {
                command_receipt: Some(receipt),
                ..
            } => Some(receipt.exit_code),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(codes, vec![1, 0]);
    assert!(session
        .validation_receipts
        .iter()
        .any(
            |receipt| receipt.validator_id == "workspace-independent-review-v1"
                && receipt
                    .subject_sha256
                    .contains_key("workspace-path:test_calc.py")
        ));
    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Accepted
    );
    assert!(outcome.events.iter().any(
        |event| matches!(event,TurnEvent::ToolResult{tool,ok:true,preview:Some(preview),..}
        if tool=="git_diff" && preview.contains("return a + b"))
    ));
}

fn contains_plan(path: &str, request: &str, text: &str) -> ModelOutput {
    call(
        "plan",
        "verification_plan",
        json!({"plan": {
            "plan_id":"fixture-plan", "requirements":[{
                "requirement_id":"file-content", "covers_requirement_ids":[format!("user-request:{request}")],
                "validator_id":"workspace-file-contains-v1", "validator_version":"1",
                "scope":{"kind":"workspace_paths","relative_paths":[path]},
                "arguments":{"text":text}, "required":true,
                "resources":{"cpu_slots":1,"memory_mb":16,"exclusive_workspace":false,"timeout_ms":10000}
            }]
        }}),
    )
}

#[tokio::test]
async fn host_repairs_stop_after_two_distinct_failed_candidates() {
    let workspace = tempfile::tempdir().unwrap();
    let request = "Create report containing required";
    let mut script = vec![contains_plan("report.md", request, "required")];
    for round in 0..3 {
        script.push(call(
            &format!("write-{round}"),
            "write_file",
            json!({"path":"report.md","content":format!("candidate {round}")}),
        ));
        script.push(ModelOutput::Text(format!("round {round}")));
    }
    let agent = agent(workspace.path(), script, AgentConfig::default());
    let mut session = Session::new(workspace.path(), "script", None);
    let outcome = agent
        .run_turn(
            &mut session,
            request,
            &owo_agent_core::permissions::AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    record(
        "bounded-repair",
        &agent,
        &session,
        &outcome,
        workspace.path(),
        &["report.md"],
    );
    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Blocked
    );
    assert_eq!(outcome.final_text.as_deref(), Some("round 2"));
    assert_eq!(outcome.model_calls.len(), 7);
    assert_eq!(
        session
            .messages
            .iter()
            .filter(|message| message.role == "system")
            .count(),
        2
    );
}

#[tokio::test]
async fn all_builtin_file_edit_paths_wait_for_a_current_plan() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("original.txt"), "original").unwrap();
    let script = vec![
        call(
            "write",
            "write_file",
            json!({"path":"new.txt","content":"new"}),
        ),
        call(
            "edit",
            "edit_file",
            json!({"path":"original.txt","old_string":"original","new_string":"edited"}),
        ),
        call(
            "multi",
            "multi_edit",
            json!({"path":"original.txt","edits":[{"old_string":"original","new_string":"edited"}]}),
        ),
        call(
            "patch",
            "apply_patch",
            json!({"patch":"*** Begin Patch\n*** Delete File: original.txt\n*** End Patch"}),
        ),
        ModelOutput::Text("cannot proceed without plan".into()),
    ];
    let agent = agent(workspace.path(), script, AgentConfig::default());
    let mut session = Session::new(workspace.path(), "script", None);
    let outcome = agent
        .run_turn(
            &mut session,
            "Edit these files",
            &owo_agent_core::permissions::AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    record(
        "edit-preflight",
        &agent,
        &session,
        &outcome,
        workspace.path(),
        &["original.txt"],
    );
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("original.txt")).unwrap(),
        "original"
    );
    assert!(!workspace.path().join("new.txt").exists());
    assert!(session.execution_receipts.is_empty());
    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Blocked
    );
    assert_eq!(
        outcome
            .events
            .iter()
            .filter(
                |event| matches!(event,TurnEvent::ToolResult{error:Some(error),..}
        if error.starts_with("tool/verification_plan_required:"))
            )
            .count(),
        4
    );
    assert!(!outcome
        .events
        .iter()
        .any(|event| matches!(event, TurnEvent::ToolStart { .. })));
}

struct ApprovalGate(
    tokio::sync::mpsc::UnboundedSender<(
        owo_agent_core::permissions::PermissionRequest,
        tokio::sync::oneshot::Sender<owo_agent_core::permissions::Decision>,
    )>,
);
#[async_trait::async_trait]
impl owo_agent_core::permissions::Approver for ApprovalGate {
    async fn decide(
        &self,
        request: &owo_agent_core::permissions::PermissionRequest,
    ) -> owo_agent_core::permissions::Decision {
        use owo_agent_core::permissions::Decision;
        if request.tool != "write_file" {
            return Decision::Allow;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.0.send((request.clone(), tx)).unwrap();
        rx.await.unwrap_or(Decision::Deny)
    }
}

#[tokio::test]
async fn real_write_waits_for_bound_approval_then_continues() {
    let workspace = tempfile::tempdir().unwrap();
    let request = "Create approved.txt";
    let agent = agent(
        workspace.path(),
        vec![
            contains_plan("approved.txt", request, "approved"),
            call(
                "write",
                "write_file",
                json!({"path":"approved.txt","content":"approved"}),
            ),
            ModelOutput::Text("approved file delivered".into()),
        ],
        AgentConfig::default(),
    );
    let mut session = Session::new(workspace.path(), "script", None);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let approver = ApprovalGate(tx);
    let abort = AtomicBool::new(false);
    let mut callback = |_: &TurnEvent| {};
    let mut run = Box::pin(agent.run_turn(&mut session, request, &approver, &abort, &mut callback));
    let (permission, reply) = tokio::select! {
        permission=rx.recv()=>permission.unwrap(),
        result=&mut run=>panic!("write completed before approval: {result:?}"),
        _=tokio::time::sleep(std::time::Duration::from_secs(5))=>panic!("approval was not requested"),
    };
    assert_eq!(permission.tool, "write_file");
    assert!(!workspace.path().join("approved.txt").exists());
    reply
        .send(owo_agent_core::permissions::Decision::Allow)
        .unwrap();
    let outcome = run.await.unwrap();
    record(
        "approval",
        &agent,
        &session,
        &outcome,
        workspace.path(),
        &["approved.txt"],
    );
    assert_eq!(
        outcome.completion_status,
        owo_agent_protocol::CompletionStatusV1::Accepted
    );
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("approved.txt")).unwrap(),
        "approved"
    );
}
