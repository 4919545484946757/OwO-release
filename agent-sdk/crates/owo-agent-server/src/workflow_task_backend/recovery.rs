//! SQLite-backed workflow invocation journal for unified tasks.
use owo_agent_core::task_runtime::{TaskRunContext, TaskRuntime, TaskState};
use owo_agent_core::workflow::{
    recovery_fingerprint, WorkflowRecoveryState, WorkflowRecoveryStore, WORKFLOW_RECOVERY_REQUIRED,
};
use owo_agent_core::workspace_provider::{
    CurrentWorkspaceProvider, PrepareWorkspaceRequest, WorkspaceKind, WorkspaceProvider,
    CODE_WORKSPACE_NOT_GIT,
};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) struct TaskWorkflowJournal {
    runtime: Arc<TaskRuntime>,
    context: TaskRunContext,
    scope: String,
    workspace_identity: String,
    workspace_revision: Option<String>,
}
impl TaskWorkflowJournal {
    pub(super) fn new(
        runtime: Arc<TaskRuntime>,
        context: &TaskRunContext,
        workspace: &std::path::Path,
    ) -> Result<Self, String> {
        let root = workspace
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let lease = match CurrentWorkspaceProvider::new(&root).prepare(&PrepareWorkspaceRequest {
            workspace_id: context.task_id.to_string(),
            task_id: Some(context.task_id.to_string()),
            kind: WorkspaceKind::CurrentWorkspace,
            base_root: root.clone(),
        }) {
            Ok(lease) => Some(lease),
            Err(error)
                if error.code == CODE_WORKSPACE_NOT_GIT
                    && !root.ancestors().any(|path| path.join(".git").exists()) =>
            {
                None
            }
            Err(error) => return Err(error.to_string()),
        };
        let workspace_revision = lease.as_ref().and_then(|lease| lease.base_commit.clone());
        let workspace_identity = recovery_fingerprint(&json!({
            "root": root,
            "git_root": lease.as_ref().map(|lease| &lease.path),
            "git_head": workspace_revision,
        }));
        Ok(Self {
            runtime,
            context: context.clone(),
            scope: recovery_fingerprint(&json!({
                "user_goal":context.user_goal,"payload":context.payload,"budget":context.budget,
                "workspace_ref":context.workspace_ref,"session_refs":context.session_refs,
                "policy_profile":context.policy_profile,"workspace":workspace,
            })),
            workspace_identity,
            workspace_revision,
        })
    }
}
impl WorkflowRecoveryStore for TaskWorkflowJournal {
    fn load(&self) -> Result<Option<WorkflowRecoveryState>, String> {
        let mut after = 0;
        let mut latest = None;
        let mut legacy = false;
        loop {
            let events = self
                .runtime
                .events(self.context.task_id.as_str(), after, 200)
                .map_err(|error| error.to_string())?;
            if events.is_empty() {
                break;
            }
            for event in events {
                after = event.sequence;
                if event.event_type == "task.workflow.created"
                    && event.payload["journal_version"] != 2
                {
                    legacy = true;
                }
                if event.event_type == "task.workflow.journal" {
                    if event.payload["scope"] != self.scope {
                        return Err("task scope or execution configuration changed".to_string());
                    }
                    if event.payload["workspace_identity"] != self.workspace_identity {
                        return Err(format!(
                            "{WORKFLOW_RECOVERY_REQUIRED}: workspace identity or Git revision changed or was not recorded"
                        ));
                    }
                    latest = Some(
                        serde_json::from_value(event.payload["state"].clone())
                            .map_err(|error| error.to_string())?,
                    );
                }
            }
        }
        if legacy {
            return Err(format!(
                "{WORKFLOW_RECOVERY_REQUIRED}: previous workflow has no durable invocation journal"
            ));
        }
        Ok(latest)
    }
    fn recovered(&self, state: &WorkflowRecoveryState) -> Result<(), String> {
        self.runtime
            .store()
            .append_event(
                self.context.task_id.as_str(),
                "task.workflow.recovery.loaded",
                self.context.attempt,
                json!({"completed_invocations":state.completed_invocations(),
                "completed_effects":state.completed_effects(),"reserved_tool_calls":state.reserved_tool_calls()}),
                None,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    fn save(&self, state: &WorkflowRecoveryState) -> Result<(), String> {
        let task_id = self.context.task_id.as_str();
        let event = self
            .runtime
            .store()
            .append_event(
                task_id,
                "task.workflow.journal",
                self.context.attempt,
                json!({"scope":self.scope,"workspace_identity":self.workspace_identity,"state":state}),
                None,
            )
            .map_err(|error| error.to_string())?;
        self.runtime.record_checkpoint(task_id,"workflow.invocation",
            &format!("task-event:{}", event.event_id),self.workspace_revision.clone(),vec![],
            json!({"completed_invocations":state.completed_invocations(),
                "completed_effects":state.completed_effects(),"pending_effect":state.has_pending_effect(),
                "reserved_tool_calls":state.reserved_tool_calls()}))
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    fn check_tool_budget(&self, reserved: usize) -> Result<(), String> {
        let task = self
            .runtime
            .get_task(self.context.task_id.as_str())
            .map_err(|error| error.to_string())?;
        if task.state != TaskState::Running || task.current_attempt != self.context.attempt {
            return Err("task is not executable in this attempt".to_string());
        }
        if reserved >= self.context.budget.max_tool_calls as usize {
            return Err("workflow tool call budget exhausted".to_string());
        }
        Ok(())
    }
    fn claim_effect(&self, sequence: usize, step: &Value) -> Result<(), String> {
        let task_id = self.context.task_id.as_str();
        let task = self
            .runtime
            .get_task(task_id)
            .map_err(|error| error.to_string())?;
        if task.state != TaskState::Running || task.current_attempt != self.context.attempt {
            return Err(
                "task paused/cancelled or attempt changed before workflow effect".to_string(),
            );
        }
        self.runtime
            .store()
            .put_idempotency(
                &format!("workflow-effect:{task_id}:{sequence}"),
                task_id,
                &json!({"sequence":sequence,"step_kind":step["kind"]}),
            )
            .map_err(|error| {
                format!("effect intent already claimed or not durably recorded: {error}")
            })?;
        Ok(())
    }
}
