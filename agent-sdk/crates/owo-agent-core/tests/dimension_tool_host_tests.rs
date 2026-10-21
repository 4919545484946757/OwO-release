//! Synthetic production Agent -> ToolHost -> policy test; no shell/network executed.
use async_trait::async_trait;
use owo_agent_core::permissions::{Approver, Decision, PermissionRequest, Policy};
use owo_agent_core::tools::{Tool, ToolContext, ToolRegistry};
use owo_agent_core::{
    Agent, ChatMessage, ModelOutput, ModelProvider, Session, ToolCall, ToolSpec, TurnEvent,
};
use owo_agent_policy::{FilesystemScope, PermissionSpec, PersistenceScope, RuleScope};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
const NAME: &str = "mcp__synthetic_dimension__probe";
struct Probe(Arc<AtomicUsize>);
#[async_trait]
impl Tool for Probe {
    fn spec(&self) -> ToolSpec {
        ToolSpec::with_effect(
            NAME,
            "Synthetic counter only, no actual file/network/command access".into(),
            json!({"type":"object","required":["synthetic"],"properties":{"synthetic":{"const":true}},"additionalProperties":false}),
            None,
        )
    }
    async fn run(&self, _: &mut ToolContext<'_>, _: Value) -> Result<Value, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"synthetic":true}))
    }
}
struct Model(AtomicUsize);
#[async_trait]
impl ModelProvider for Model {
    async fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ModelOutput, String> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(ModelOutput::ToolCalls(vec![ToolCall {
                id: "synthetic-dimension".into(),
                name: NAME.into(),
                arguments: json!({"synthetic":true}),
            }]))
        } else {
            Ok(ModelOutput::Text("synthetic policy check complete".into()))
        }
    }
}
struct ExactApproval(AtomicUsize);
#[async_trait]
impl Approver for ExactApproval {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        self.0.fetch_add(1, Ordering::SeqCst);
        if request.tool == NAME && request.args == json!({"synthetic":true}) {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}
#[tokio::test]
async fn explicit_dimensions_deny_before_approval_and_execution_with_positive_control() {
    for (filesystem, command, network, allowed) in [
        (
            FilesystemScope::Unrestricted,
            RuleScope::Unrestricted,
            RuleScope::Deny,
            false,
        ),
        (
            FilesystemScope::Unrestricted,
            RuleScope::Deny,
            RuleScope::Unrestricted,
            false,
        ),
        (
            FilesystemScope::WorkspaceRead,
            RuleScope::Unrestricted,
            RuleScope::Unrestricted,
            false,
        ),
        (
            FilesystemScope::Unrestricted,
            RuleScope::Unrestricted,
            RuleScope::Unrestricted,
            true,
        ),
    ] {
        let parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.work-logs/dimension-tool-host");
        std::fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let workspace = parent.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&workspace).unwrap();
        let policy = Policy::new(&workspace);
        policy.set_spec(PermissionSpec {
            filesystem,
            command,
            network,
            persistence: PersistenceScope::Once,
            scopes: vec![],
        });
        let invokes = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::empty();
        registry.register(Probe(invokes.clone()));
        let agent = Agent::new(
            Arc::new(Model(AtomicUsize::new(0))),
            registry,
            policy,
            Default::default(),
        );
        let approval = ExactApproval(AtomicUsize::new(0));
        let mut session = Session::new(&workspace, "synthetic", None);
        let mut results = Vec::new();
        agent
            .run_turn(
                &mut session,
                "Run only the synthetic capability counter",
                &approval,
                &AtomicBool::new(false),
                &mut |event: &TurnEvent| {
                    if let TurnEvent::ToolResult { tool, ok, .. } = event {
                        if tool == NAME {
                            results.push(*ok);
                        }
                    }
                },
            )
            .await
            .unwrap();
        assert_eq!(results, vec![allowed]);
        assert_eq!(invokes.load(Ordering::SeqCst), usize::from(allowed));
        assert_eq!(
            approval.0.load(Ordering::SeqCst),
            usize::from(allowed),
            "denials cannot be overridden by an approval callback"
        );
        let resolved = workspace.canonicalize().unwrap();
        assert!(resolved.parent() == Some(parent.as_path()));
        std::fs::remove_dir_all(resolved).unwrap();
    }
}

struct AllowRecommendation;
#[async_trait]
impl owo_agent_core::Reviewer for AllowRecommendation {
    async fn review(
        &self,
        _: &PermissionRequest,
        context: Option<&str>,
    ) -> owo_agent_core::ReviewVerdict {
        assert_eq!(context, Some("synthetic side-effect request"));
        owo_agent_core::ReviewVerdict::Allow
    }
}
struct ApprovalChoice {
    allow: bool,
    calls: AtomicUsize,
}
#[async_trait]
impl Approver for ApprovalChoice {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.allow && request.tool == NAME && request.args == json!({"synthetic":true}) {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}
#[tokio::test]
async fn model_allow_recommendation_cannot_authorize_side_effects() {
    for allow in [false, true] {
        let parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.work-logs/dimension-tool-host");
        std::fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let workspace = parent.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&workspace).unwrap();
        let invokes = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::empty();
        registry.register(Probe(invokes.clone()));
        let mut agent = Agent::new(
            Arc::new(Model(AtomicUsize::new(0))),
            registry,
            Policy::new(&workspace),
            Default::default(),
        );
        agent.set_reviewer(Some(Arc::new(AllowRecommendation)));
        let approval = ApprovalChoice {
            allow,
            calls: AtomicUsize::new(0),
        };
        let mut session = Session::new(&workspace, "synthetic", None);
        let mut results = Vec::new();
        agent
            .run_turn(
                &mut session,
                "synthetic side-effect request",
                &approval,
                &AtomicBool::new(false),
                &mut |event: &TurnEvent| {
                    if let TurnEvent::ToolResult { tool, ok, .. } = event {
                        if tool == NAME {
                            results.push(*ok);
                        }
                    }
                },
            )
            .await
            .unwrap();
        assert_eq!(
            invokes.load(Ordering::SeqCst),
            usize::from(allow),
            "a model recommendation must not substitute for the actual approval"
        );
        assert_eq!(
            approval.calls.load(Ordering::SeqCst),
            1,
            "concrete external approval is required"
        );
        assert_eq!(results, vec![allow]);
        let audit = agent.audit_log();
        let audit = audit.lock().unwrap();
        assert!(
            !audit
                .entries
                .iter()
                .any(|entry| entry.event == "auto_review" && entry.approved == Some(true)),
            "model must not record itself as the side-effect authorization"
        );
        drop(audit);
        let resolved = workspace.canonicalize().unwrap();
        assert_eq!(resolved.parent(), Some(parent.as_path()));
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
