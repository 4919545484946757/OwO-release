//! Live-model acceptance driver: uses the production Agent/ToolHost/policy/store.
//! Credentials are read from process environment only. The case file is nonsecret.
//! No trusted/unrestricted mode: each side effect is matched to the case scope.
use owo_agent_core::agent::{TurnExecutionOptions, TurnSessionCheckpoint};
use owo_agent_core::gateway::{OpenAiCompatibleConfig, OpenAiCompatibleProvider};
use owo_agent_core::permissions::{Approver, Decision, PermissionRequest, Policy};
use owo_agent_core::session::{JsonSessionStore, Session, SessionStore};
use owo_agent_core::{Agent, AgentConfig, ToolRegistry, TurnEvent};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Deserialize)]
struct Case {
    label: String,
    workspace: PathBuf,
    output: PathBuf,
    prompt: String,
    allowed_writes: Vec<String>,
    #[serde(default)]
    allowed_commands: Vec<String>,
    #[serde(default)]
    approval_delay_ms: u64,
}
struct ScopedApproval {
    writes: Vec<String>,
    commands: Vec<String>,
    delay_ms: u64,
    log: Mutex<Vec<serde_json::Value>>,
}
#[async_trait::async_trait]
impl Approver for ScopedApproval {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        let allowed = match request.tool.as_str() {
            "write_file" | "edit_file" => request
                .args
                .get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|path| self.writes.iter().any(|allowed| allowed == path)),
            "run_command" => request
                .args
                .get("command")
                .and_then(|v| v.as_str())
                .is_some_and(|command| self.commands.iter().any(|allowed| allowed == command)),
            "verification_plan" | "todo" => true,
            _ => false,
        };
        println!(
            "{}",
            json!({"event":"scoped_approval", "request_id":request.request_id,
            "tool":request.tool, "allowed":allowed, "delay_ms":self.delay_ms})
        );
        if allowed && self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms.min(5000))).await;
        }
        self.log
            .lock()
            .unwrap()
            .push(json!({"request":request, "decision":if allowed {"allow"} else {"deny"}}));
        if allowed {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = std::env::args_os()
        .nth(1)
        .ok_or("expected nonsecret case JSON path")?;
    let case: Case = serde_json::from_slice(&std::fs::read(file)?)?;
    let workspace = case.workspace.canonicalize()?;
    std::fs::create_dir_all(&case.output)?;
    let store: Arc<dyn SessionStore> =
        Arc::new(JsonSessionStore::new(case.output.join("sessions")));
    let provider = Arc::new(OpenAiCompatibleProvider::new(
        OpenAiCompatibleConfig::from_env()?,
    )?);
    let mut registry = ToolRegistry::new();
    registry.retain_names(
        &[
            "read_file",
            "list_dir",
            "search_files",
            "grep",
            "write_file",
            "edit_file",
            "git_status",
            "git_diff",
            "todo",
            "verification_plan",
            "run_command",
        ]
        .map(String::from),
    );
    let agent = Agent::new(
        provider,
        registry,
        Policy::new(&workspace),
        AgentConfig {
            max_turns: 20,
            max_tool_calls_per_turn: 40,
            turn_deadline: Some(Duration::from_secs(240)),
            max_command_timeout_ms: Some(30_000),
            compaction_enabled: false,
            ..Default::default()
        },
    );
    let approval = ScopedApproval {
        writes: case.allowed_writes,
        commands: case.allowed_commands,
        delay_ms: case.approval_delay_ms,
        log: Mutex::new(Vec::new()),
    };
    let mut session = Session::new(&workspace, "live-model", None);
    let options = TurnExecutionOptions {
        session_checkpoint: Some(TurnSessionCheckpoint::new(store.clone())),
        ..Default::default()
    };
    let abort = AtomicBool::new(false);
    let result = agent
        .run_turn_with_images_options(
            &mut session,
            &case.prompt,
            &[],
            &approval,
            None,
            &abort,
            &mut |event| match event {
                TurnEvent::ModelCall => println!("{}", json!({"event":"model_call"})),
                TurnEvent::ToolStart { id, tool, .. } => {
                    println!("{}", json!({"event":"tool_start","id":id,"tool":tool}))
                }
                TurnEvent::ToolResult {
                    id,
                    tool,
                    ok,
                    error,
                    ..
                } => println!(
                    "{}",
                    json!({"event":"tool_result","id":id,"tool":tool,"ok":ok,"error":error})
                ),
                _ => {}
            },
            &options,
        )
        .await;
    let audit = agent.audit_log();
    let audit = audit.lock().unwrap();
    let result = match result {
        Ok(outcome) => {
            println!(
                "{}",
                json!({"event":"case_finished","label":case.label,
                "completion_status":outcome.completion_status,"steps":outcome.steps,
                "model_calls":outcome.model_calls.len(),"usage":outcome.usage,"usage_known":outcome.usage_known})
            );
            let mut value = serde_json::to_value(outcome)?;
            if let Some(events) = value.get_mut("events").and_then(|v| v.as_array_mut()) {
                events.retain(|event| {
                    event.get("TokenDelta").is_none() && event.get("ReasoningDelta").is_none()
                });
            }
            json!({"ok":true,"outcome":value})
        }
        Err(error) => {
            let mut detail = error.to_string();
            if let Ok(secret) = std::env::var("OPENAI_API_KEY") {
                if !secret.is_empty() {
                    detail = detail.replace(&secret, "[redacted]");
                }
            }
            println!(
                "{}",
                json!({"event":"case_failed","label":case.label,"error":detail})
            );
            json!({"ok":false,"error":detail})
        }
    };
    let restored = store.load(&session.id)?;
    let evidence = json!({"case":case.label,"model_kind":"live provider; no scripted responses",
        "prompt":case.prompt,"result":result,"session":session,
        "approvals":approval.log.lock().unwrap().clone(),"audit":audit.entries,
        "durable_reload":{"session_id":restored.id,"messages":restored.messages.len(),
            "execution_receipts":restored.execution_receipts.len()}});
    std::fs::write(
        case.output.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    for path in &approval.writes {
        if workspace.join(path).is_file() {
            std::fs::copy(workspace.join(path), case.output.join(path))?;
        }
    }
    if evidence["result"]["ok"] == false {
        std::process::exit(1);
    }
    Ok(())
}
