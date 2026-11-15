use owo_agent_core::permissions::{ApprovalOutcome, Approver, Decision, Level, PermissionRequest};
use owo_agent_core::tools::{Tool, ToolContext, ToolSpec};
use owo_agent_core::{EffectClass, ToolEffect};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

const TOOL: &str = "owobot_delegate";

pub(crate) struct Delegate {
    client: Client,
    endpoint: String,
    turn: String,
    group: String,
    recipients: [String; 2],
    submitted: Mutex<[bool; 2]>,
}

impl Delegate {
    pub(super) fn new(
        client: Client,
        base_url: String,
        turn: String,
        group: String,
        recipients: [String; 2],
    ) -> Self {
        Self {
            client,
            endpoint: format!("{base_url}/api/internal/owobot-delegate"),
            turn,
            group,
            recipients,
            submitted: Mutex::new([false; 2]),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Recipient {
    Worker,
    Reviewer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assignment {
    recipient: Recipient,
    task: String,
}

#[async_trait::async_trait]
impl Tool for Delegate {
    fn spec(&self) -> ToolSpec {
        ToolSpec::with_effect(
            TOOL,
            "Assign a task to the worker or reviewer in this OwOBot project conversation. A result will resume the coordinator.".into(),
            json!({
                "type":"object",
                "additionalProperties":false,
                "properties":{
                    "recipient":{"type":"string","enum":["worker","reviewer"]},
                    "task":{"type":"string","minLength":1,"maxLength":4000},
                },
                "required":["recipient","task"]
            }),
            Some(ToolEffect {
                tool: TOOL.into(),
                class: EffectClass::Write,
                source: "owobot".into(),
                risk_note: Some("Creates a collaboration assignment in the current project".into()),
                annotations: None,
                host_verified_readonly: false,
            }),
        )
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let assignment: Assignment =
            serde_json::from_value(args).map_err(|_| "owobot/invalid_assignment")?;
        let task = assignment.task.trim();
        if task.is_empty() || task.encode_utf16().count() > 4000 {
            return Err("owobot/invalid_assignment".into());
        }
        if ctx.abort.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err("owobot/assignment_cancelled".into());
        }
        let index = match assignment.recipient {
            Recipient::Worker => 0,
            Recipient::Reviewer => 1,
        };
        {
            let mut submitted = self
                .submitted
                .lock()
                .map_err(|_| "owobot/assignment_unavailable")?;
            if std::mem::replace(&mut submitted[index], true) {
                return Err("owobot/recipient_already_assigned_this_turn".into());
            }
        }
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.turn)
            .timeout(Duration::from_secs(8))
            .json(&json!({"groupId":self.group,"targetBot":self.recipients[index],"message":task}))
            .send()
            .await
            .map_err(|_| "owobot/assignment_receipt_unavailable")?;
        if !response.status().is_success() {
            return Err("owobot/assignment_refused".into());
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| "owobot/assignment_receipt_unavailable")?;
        Ok(body)
    }
}

/// Only the operations assigned to the verified project role are authorized.
/// The inherited policy still evaluates the action before this decision.
pub(crate) struct ProjectApprover<'a> {
    pub(crate) authorized: Vec<String>,
    pub(crate) parent: &'a dyn Approver,
}

#[async_trait::async_trait]
impl Approver for ProjectApprover<'_> {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        self.decide_with_notification(request, &mut || {})
            .await
            .decision
    }

    async fn decide_with_notification(
        &self,
        request: &PermissionRequest,
        notify: &mut (dyn FnMut() + Send),
    ) -> ApprovalOutcome {
        if self.authorized.contains(&request.tool) && request.level == Level::Write {
            return ApprovalOutcome {
                decision: Decision::Allow,
                reason: Some("owobot/project_role_tool".into()),
            };
        }
        self.parent.decide_with_notification(request, notify).await
    }
}
