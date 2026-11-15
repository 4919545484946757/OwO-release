use super::{Actor, DeliveryError, DeliveryResult, Store, View, LIMIT};
use crate::owobot::WorkerRole;
use owo_agent_core::tools::{Tool, ToolContext, ToolSpec};
use owo_agent_core::{EffectClass, ToolEffect};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub(crate) const STAGE: &str = "owobot_stage_delivery";
pub(crate) const REVIEW: &str = "owobot_review_delivery";
const INSPECT: &str = "owobot_read_delivery";

#[derive(Clone, Copy)]
enum Operation {
    Inspect,
    Stage,
    Review,
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Inspect => INSPECT,
            Self::Stage => STAGE,
            Self::Review => REVIEW,
        }
    }

    fn contract(self) -> (&'static str, Value) {
        match self {
            Self::Inspect => (
                "Read the current OwOBot delivery revision and its exact proposed text.",
                json!({"type":"object","additionalProperties":false}),
            ),
            Self::Stage => (
                "Stage the complete set of text outputs or replacements for this task (UTF-8). This only prepares the revision; a reviewer and the user decide whether to apply it. Use baseRevision 0 for the first revision and the current revision number for revisions.",
                json!({"type":"object","additionalProperties":false,
                    "properties":{
                        "baseRevision":{"type":"integer","minimum":0},
                        "files":{"type":"array","minItems":1,"maxItems":12,"items":{
                            "type":"object","additionalProperties":false,
                            "properties":{"path":{"type":"string","description":"Project-relative path with / separators"},"text":{"type":"string","maxLength":LIMIT}},
                            "required":["path","text"]
                        }}
                    },
                    "required":["baseRevision","files"]
                }),
            ),
            Self::Review => (
                "Record an independent approval or change request for the exact delivery revision after reading and checking it. Does not apply any file.",
                json!({"type":"object","additionalProperties":false,
                    "properties":{"revision":{"type":"integer","minimum":1},"approved":{"type":"boolean"},"note":{"type":"string","minLength":1,"maxLength":4096}},
                    "required":["revision","approved","note"]
                }),
            ),
        }
    }
}

struct DeliveryTool {
    store: Store,
    thread: String,
    actor: Actor,
    operation: Operation,
}

pub(in crate::owobot) fn for_role(
    store: &Store,
    thread: &str,
    actor: &Actor,
    role: WorkerRole,
    readonly: bool,
) -> Vec<Arc<dyn Tool>> {
    let mut result: Vec<Arc<dyn Tool>> = Vec::new();
    let mut add = |operation| {
        result.push(Arc::new(DeliveryTool {
            store: store.clone(),
            thread: thread.to_owned(),
            actor: actor.clone(),
            operation,
        }))
    };
    add(Operation::Inspect);
    if !readonly {
        match role {
            WorkerRole::Worker => add(Operation::Stage),
            WorkerRole::Reviewer => add(Operation::Review),
            WorkerRole::Lead => {}
        }
    }
    result
}

#[async_trait::async_trait]
impl Tool for DeliveryTool {
    fn spec(&self) -> ToolSpec {
        let readonly = matches!(self.operation, Operation::Inspect);
        let (description, schema) = self.operation.contract();
        ToolSpec::with_effect(
            self.operation.name(),
            description.into(),
            schema,
            Some(ToolEffect {
                tool: self.operation.name().into(),
                source: "owobot".into(),
                class: if readonly {
                    EffectClass::Read
                } else {
                    EffectClass::Write
                },
                host_verified_readonly: readonly,
                risk_note: None,
                annotations: None,
            }),
        )
    }

    async fn run(&self, context: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        if context
            .abort
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err("owobot/turn_cancelled".into());
        }
        let store = self.store.clone();
        let thread = self.thread.clone();
        let actor = self.actor.clone();
        let operation = self.operation;
        let view = tokio::task::spawn_blocking(move || -> DeliveryResult<View> {
            let invalid = |_| DeliveryError::Invalid("invalid_delivery_arguments");
            match operation {
                Operation::Inspect => store.inspect(&thread, None),
                Operation::Stage => store.stage(
                    &thread,
                    actor,
                    serde_json::from_value(args).map_err(invalid)?,
                ),
                Operation::Review => store.review(
                    &thread,
                    actor,
                    serde_json::from_value(args).map_err(invalid)?,
                ),
            }
        })
        .await
        .map_err(|_| "owobot/delivery_unavailable")?
        .map_err(|error| format!("owobot/{}", error.code()))?;
        if matches!(operation, Operation::Inspect) {
            serde_json::to_value(view).map_err(|_| "owobot/delivery_unavailable".into())
        } else {
            Ok(json!({"taskId": view.task_id, "revision": view.revision, "saved": true}))
        }
    }
}
