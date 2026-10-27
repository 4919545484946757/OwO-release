//! Workflow-node execution contracts. Fixture models/tools never call media services.
//! Exercise the production profile, ToolHost, approval, live registry and model override.
use async_trait::async_trait;
use owo_agent_core::gateway::{ChatMessage, ModelOutput, ModelProvider, ToolCall};
use owo_agent_core::permissions::{Approver, Decision, PermissionRequest, Policy};
use owo_agent_core::tool_effects::{EffectClass, ToolEffect};
use owo_agent_core::tools::{Tool, ToolContext, ToolRegistry, ToolSpec};
use owo_agent_core::worker_profile::{ProfileSubagentRunner, WorkerProfile};
use owo_agent_core::workswarm::RoleSpec;
use owo_agent_core::{Agent, TurnEvent};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const MEDIA_TOOL: &str = "mcp__media_fixture__generate";
const MODEL: &str = "fixture-visual-director";
const OUTPUT: &str = r#"{"status":"done","summary":"fixture contract","artifact":{"kind":"document","format":"markdown","content":"Controlled fixture; no media generated."},"evidence":[],"open_issues":[]}"#;

struct MediaFixture {
    spec: Arc<Mutex<ToolSpec>>,
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for MediaFixture {
    fn spec(&self) -> ToolSpec {
        self.spec.lock().unwrap().clone()
    }
    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        assert_eq!(
            ctx.session.model, MODEL,
            "node model reaches the tool context"
        );
        assert_eq!(args["prompt"], "fixture scene");
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"fixture":true,"request_received":true}))
    }
}

struct Consent {
    allow: bool,
    requests: AtomicUsize,
}
#[async_trait]
impl Approver for Consent {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        assert_eq!(request.tool, MEDIA_TOOL);
        self.requests.fetch_add(1, Ordering::SeqCst);
        if self.allow {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}

struct FixtureModel {
    outputs: Mutex<VecDeque<ModelOutput>>,
    models: Mutex<Vec<String>>,
    before_tool: Option<Box<dyn Fn() + Send + Sync>>,
    calls: AtomicUsize,
}
impl FixtureModel {
    fn next(&self, model: Option<&str>) -> Result<ModelOutput, String> {
        self.models
            .lock()
            .unwrap()
            .push(model.unwrap_or("").to_string());
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            if let Some(hook) = &self.before_tool {
                hook();
            }
        }
        self.outputs
            .lock()
            .unwrap()
            .pop_front()
            .ok_or("fixture exhausted".into())
    }
}
#[async_trait]
impl ModelProvider for FixtureModel {
    async fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ModelOutput, String> {
        self.next(None)
    }
    async fn complete_with_model(
        &self,
        model: Option<&str>,
        _: &[ChatMessage],
        _: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.next(model)
    }
    async fn complete_stream_with_model(
        &self,
        model: Option<&str>,
        _: &[ChatMessage],
        _: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        let output = self.next(model)?;
        if let ModelOutput::Text(text) = &output {
            on_delta(text.clone());
        }
        Ok(output)
    }
}

#[derive(Clone, Copy, Debug)]
enum Change {
    None,
    Disabled,
    Disconnected,
    Schema,
    Effect,
    ParentReadOnly,
    Undeclared,
}

async fn exercise(change: Change, approve: bool) -> (usize, usize, Vec<TurnEvent>) {
    let temp = tempfile::tempdir().unwrap();
    let spec = Arc::new(Mutex::new(ToolSpec::with_effect(
        MEDIA_TOOL,
        "Controlled media fixture".into(),
        json!({"type":"object","properties":{"prompt":{"type":"string"}},"required":["prompt"]}),
        Some(ToolEffect {
            tool: MEDIA_TOOL.into(),
            class: EffectClass::Execute,
            source: "mcp:media_fixture".into(),
            risk_note: Some("Fixture requires consent".into()),
            annotations: None,
            host_verified_readonly: false,
        }),
    )));
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(MediaFixture {
        spec: Arc::clone(&spec),
        executions: Arc::clone(&executions),
    });
    // The parent's provider is irrelevant: nodes receive their own provider/model.
    let idle: Arc<dyn ModelProvider> = Arc::new(FixtureModel {
        outputs: Mutex::new(VecDeque::new()),
        models: Mutex::new(Vec::new()),
        before_tool: None,
        calls: AtomicUsize::new(0),
    });
    let parent = Arc::new(Agent::new(
        idle,
        registry,
        Policy::new(temp.path()),
        Default::default(),
    ));
    let role = RoleSpec {
        node_id: None,
        role: "Custom visual coordinator".into(),
        assignee: "agent".into(),
        model: Some(MODEL.into()),
        tools: vec![if matches!(change, Change::Undeclared) {
            "read_file"
        } else {
            MEDIA_TOOL
        }
        .into()],
        ..Default::default()
    };
    let profile = WorkerProfile::for_team_role_spec(&role, 4, false, None);
    assert!(
        profile.read_only,
        "remote media nodes do not need a workspace writer lease"
    );
    let extra_tools = parent
        .declared_workflow_tools(&profile.visible_tools)
        .unwrap();
    let hook_parent = Arc::clone(&parent);
    let hook_spec = Arc::clone(&spec);
    let provider = Arc::new(FixtureModel {
        outputs: Mutex::new(VecDeque::from([
            ModelOutput::ToolCalls(vec![ToolCall {
                id: "media-1".into(),
                name: MEDIA_TOOL.into(),
                arguments: json!({"prompt":"fixture scene"}),
            }]),
            ModelOutput::Text(OUTPUT.into()),
        ])),
        models: Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
        before_tool: Some(Box::new(move || match change {
            Change::Disabled => hook_parent.set_tool_prefix_enabled("mcp__media_fixture__", false),
            Change::Disconnected => {
                assert_eq!(hook_parent.remove_tools_prefix("mcp__media_fixture__"), 1);
            }
            Change::Schema => {
                hook_spec.lock().unwrap().input_schema =
                    json!({"type":"object","required":["new_field"]})
            }
            Change::Effect => {
                hook_spec.lock().unwrap().effect.as_mut().unwrap().class = EffectClass::Inject
            }
            Change::ParentReadOnly => hook_parent.policy().set_read_only_runtime(true),
            _ => {}
        })),
    });
    let consent = Consent {
        allow: approve,
        requests: AtomicUsize::new(0),
    };
    let abort = AtomicBool::new(false);
    let events = Arc::new(Mutex::new(Vec::<TurnEvent>::new()));
    let captured = Arc::clone(&events);
    let runner = ProfileSubagentRunner {
        provider: provider.clone(),
        parent_policy: parent.policy(),
        parent_agent: Some(Arc::clone(&parent)),
        approver: &consent,
        abort: &abort,
        depth: 0,
        model: MODEL.into(),
        is_critic: false,
        write_allowed: Vec::new(),
        profile,
        agent_config: None,
        budget_note_override: None,
        extra_tools,
        extra_system_prompt: None,
        event_sink: Some(Arc::new(move |event| {
            captured.lock().unwrap().push(event.clone())
        })),
        session_store: None,
        worker_session_id: None,
        parent_session_id: None,
    };
    let result = runner
        .run(
            temp.path(),
            "Call the declared media tool once, then return the fixture contract",
        )
        .await;
    if matches!(change, Change::None) && approve {
        assert!(result.is_ok(), "{result:?}");
    }
    let models = provider.models.lock().unwrap();
    assert!(!models.is_empty());
    assert!(
        models.iter().all(|name| name == MODEL),
        "every node request must use its selected model: {models:?}"
    );
    let observed = events.lock().unwrap().clone();
    (
        executions.load(Ordering::SeqCst),
        consent.requests.load(Ordering::SeqCst),
        observed,
    )
}

#[tokio::test]
async fn custom_media_node_executes_only_after_consent_and_uses_its_model() {
    for allow in [false, true] {
        let (executions, requests, events) = exercise(Change::None, allow).await;
        assert_eq!(requests, 1, "the remote API call must request consent");
        assert_eq!(executions, usize::from(allow));
        if !allow {
            assert!(!events
                .iter()
                .any(|event| matches!(event, TurnEvent::ToolStart { .. })));
            assert!(events.iter().any(|event| matches!(event,
                TurnEvent::ToolResult { ok: false, command_receipt: None, error: Some(reason), .. }
                    if reason.contains("permission denied"))));
        }
        assert!(events
            .iter()
            .any(|event| matches!(event, TurnEvent::PermissionRequest(_))));
        assert!(events.iter().any(|event| matches!(event, TurnEvent::ToolResult { tool, ok, .. } if tool == MEDIA_TOOL && *ok == allow)), "{events:?}");
    }
}

#[tokio::test]
async fn media_nodes_follow_live_tool_revocation_schema_effect_and_parent_policy() {
    for change in [
        Change::Disabled,
        Change::Disconnected,
        Change::Schema,
        Change::Effect,
        Change::ParentReadOnly,
    ] {
        let (executions, requests, events) = exercise(change, true).await;
        assert_eq!(executions, 0, "{change:?} must prevent the external call");
        if matches!(change, Change::ParentReadOnly) {
            assert_eq!(
                requests, 0,
                "parent policy denial cannot be overridden by consent"
            );
        }
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TurnEvent::ToolResult { ok: false, .. })),
            "{change:?}: {events:?}"
        );
    }
}

#[tokio::test]
async fn unselected_media_tools_are_not_executable_by_a_workflow_node() {
    let (executions, requests, _) = exercise(Change::Undeclared, true).await;
    assert_eq!(executions, 0);
    assert_eq!(
        requests, 0,
        "an undeclared tool is rejected before approval"
    );
}

struct NativeConsent;
#[async_trait]
impl Approver for NativeConsent {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        assert_eq!(request.tool, "write_file");
        Decision::Allow
    }
}

#[tokio::test]
async fn workflow_catalog_native_tools_obey_live_host_disable_before_write() {
    for disable_before_start in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let idle: Arc<dyn ModelProvider> = Arc::new(FixtureModel {
            outputs: Mutex::new(VecDeque::new()),
            models: Mutex::new(Vec::new()),
            before_tool: None,
            calls: AtomicUsize::new(0),
        });
        let parent = Arc::new(Agent::new(
            idle,
            ToolRegistry::new(),
            Policy::new(temp.path()),
            Default::default(),
        ));
        let hook_parent = Arc::clone(&parent);
        let provider = Arc::new(FixtureModel {
            outputs: Mutex::new(VecDeque::from([
                ModelOutput::ToolCalls(vec![ToolCall {
                    id: "write-1".into(),
                    name: "write_file".into(),
                    arguments: json!({"path":"should-not-exist.txt","content":"unwanted"}),
                }]),
                ModelOutput::Text(OUTPUT.into()),
            ])),
            models: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            before_tool: Some(Box::new(move || {
                hook_parent.set_tool_prefix_enabled("write_file", false)
            })),
        });
        if disable_before_start {
            parent.set_tool_prefix_enabled("write_file", false);
        }
        let abort = AtomicBool::new(false);
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let runner = ProfileSubagentRunner {
            provider: provider.clone(),
            parent_policy: parent.policy(),
            parent_agent: Some(Arc::clone(&parent)),
            approver: &NativeConsent,
            abort: &abort,
            depth: 0,
            model: MODEL.into(),
            is_critic: false,
            write_allowed: vec![temp.path().into()],
            profile: WorkerProfile::for_declared_tools(&["write_file".into()], 3),
            agent_config: None,
            budget_note_override: None,
            extra_tools: Vec::new(),
            extra_system_prompt: None,
            event_sink: Some(Arc::new(move |event| {
                captured.lock().unwrap().push(event.clone())
            })),
            session_store: None,
            worker_session_id: None,
            parent_session_id: None,
        };
        let result = runner
            .run(
                temp.path(),
                "Attempt a scoped write, then report the result",
            )
            .await;
        assert!(!temp.path().join("should-not-exist.txt").exists());
        if disable_before_start {
            assert!(result.unwrap_err().contains("disabled"));
            assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        } else {
            assert!(events.lock().unwrap().iter().any(|event| matches!(event,TurnEvent::ToolResult{tool,ok:false,error:Some(reason),..} if tool=="write_file" && reason.contains("disabled"))));
        }
    }
}
