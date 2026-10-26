//! Shared executor for task-scoped worker turns across Single, Team, and evaluation.
//!
//! Callers resolve the workspace, policy, tool surface, budget, and reviewer role before
//! construction. This runtime owns the Agent loop, model-bound task session, cancellation,
//! output-contract repair, and request accounting; it never grants capabilities itself.

use crate::agent::{Agent, AgentConfig, TurnEvent};
use crate::contract_worker::enforce_worker_output_contract_controlled;
use crate::gateway::{ModelProvider, TokenUsage};
use crate::permissions::{Approver, Policy};
use crate::session::{Session, SessionStore};
use crate::subagent::{TurnEventSink, MAX_SUBAGENT_DEPTH};
use crate::tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

pub(crate) struct WorkerRuntime<'a> {
    pub provider: Arc<dyn ModelProvider>,
    pub approver: &'a dyn Approver,
    pub abort: &'a AtomicBool,
    pub depth: usize,
    pub model: String,
    pub workspace: PathBuf,
    pub registry: ToolRegistry,
    pub policy: Policy,
    pub config: AgentConfig,
    pub system_prompt: Option<String>,
    pub is_critic: bool,
    pub event_sink: Option<TurnEventSink<'a>>,
    pub session_store: Option<Arc<dyn SessionStore>>,
    pub worker_session_id: Option<String>,
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerRuntimeReport {
    pub output: String,
    pub duration_ms: u64,
    pub steps: usize,
    pub model_calls: u32,
    pub usage: TokenUsage,
    pub usage_known: bool,
    pub output_repairs: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerRuntimeError {
    pub message: String,
    pub duration_ms: u64,
    pub steps: usize,
    pub model_calls: u32,
    pub usage: TokenUsage,
    pub usage_known: bool,
    pub output_repairs: u32,
}

impl From<String> for WorkerRuntimeError {
    fn from(message: String) -> Self {
        Self {
            message,
            duration_ms: 0,
            steps: 0,
            model_calls: 0,
            usage: TokenUsage::default(),
            usage_known: false,
            output_repairs: 0,
        }
    }
}

impl WorkerRuntime<'_> {
    pub async fn run_report(self, prompt: &str) -> Result<WorkerRuntimeReport, WorkerRuntimeError> {
        self.run_report_with_images(prompt, &[]).await
    }

    pub async fn run_report_with_images(
        self,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
    ) -> Result<WorkerRuntimeReport, WorkerRuntimeError> {
        self.run_report_with_input(prompt, images, None).await
    }

    pub async fn run_report_with_input(
        self,
        prompt: &str,
        images: &[crate::gateway::MessageImage],
        input: Option<owo_agent_protocol::StoredUserInput>,
    ) -> Result<WorkerRuntimeReport, WorkerRuntimeError> {
        let input = input.unwrap_or_else(|| owo_agent_protocol::StoredUserInput {
            prompt: prompt.to_string(),
            content_parts: Vec::new(),
            legacy_attachments: if images.is_empty() {
                Vec::new()
            } else {
                vec!["worker-image-without-asset-reference".into()]
            },
        });
        let started = Instant::now();
        if self.depth >= MAX_SUBAGENT_DEPTH {
            return Err(format!("子代理深度超限（最多 {MAX_SUBAGENT_DEPTH} 层）").into());
        }
        let configured_turn_cap = self.config.max_turns;
        let turn_deadline = self.config.turn_deadline;
        let request_timeout = self
            .config
            .request_timeout_secs
            .map(|seconds| std::time::Duration::from_secs(seconds as u64));
        let agent = Agent::new(
            Arc::clone(&self.provider),
            self.registry,
            self.policy,
            self.config,
        );
        let mut session = load_worker_session(
            &self.workspace,
            &self.model,
            self.system_prompt.clone(),
            &self.session_store,
            &self.worker_session_id,
            &self.parent_session_id,
        )?;
        if let Some(session_id) = &self.worker_session_id {
            session.id = session_id.clone();
        }
        session.parent_id = self.parent_session_id.clone();
        let session_checkpoint = match (&self.session_store, &self.worker_session_id) {
            (Some(store), Some(_)) => {
                Some(crate::agent::TurnSessionCheckpoint::new(Arc::clone(store)))
            }
            _ => None,
        };
        let event_sink = self.event_sink.clone();
        let observed_steps = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_steps_for_sink = Arc::clone(&observed_steps);
        let mut on_event = move |event: &TurnEvent| {
            if matches!(event, TurnEvent::ToolResult { .. }) {
                observed_steps_for_sink.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if let Some(sink) = &event_sink {
                sink(event);
            }
        };
        let outcome = agent
            .run_turn_with_images_options(
                &mut session,
                prompt,
                images,
                self.approver,
                None,
                self.abort,
                &mut on_event,
                &crate::agent::TurnExecutionOptions {
                    input: Some(input),
                    session_checkpoint,
                    ..Default::default()
                },
            )
            .await;
        let outcome = outcome.map_err(|error| {
            worker_runtime_error(
                format!("子代理执行失败：{error}"),
                started.elapsed().as_millis() as u64,
                observed_steps.load(std::sync::atomic::Ordering::Relaxed),
                &session.transient_model_calls,
            )
        })?;
        let model_calls = u32::try_from(outcome.model_calls.len()).unwrap_or(u32::MAX);
        if outcome.reached_model_turn_limit {
            return Err(WorkerRuntimeError {
                message: format!(
                    "worker_turn_budget_exhausted:模型在 {} 轮预算内未自行给出最终答复",
                    configured_turn_cap
                ),
                duration_ms: started.elapsed().as_millis() as u64,
                steps: outcome.steps,
                model_calls,
                usage: outcome.usage,
                usage_known: outcome.usage_known,
                output_repairs: 0,
            });
        }
        let text = outcome
            .final_text
            .unwrap_or_else(|| format!("（子代理无最终文本，共 {} 步）", outcome.steps));
        let remaining = turn_deadline.map(|budget| budget.saturating_sub(started.elapsed()));
        let repair_timeout = match (remaining, request_timeout) {
            (Some(left), Some(per_request)) => Some(left.min(per_request)),
            (Some(left), None) => Some(left),
            (None, per_request) => per_request,
        };
        // max_turns limits Agent loop rounds, not the separately reserved format
        // correction. Production MeasuredProvider enforces the TaskGraph attempt ceiling
        // across Agent, turn-limit wrap-up, and this correction request.
        let allow_repair = configured_turn_cap == 0
            || outcome.model_calls.len() < configured_turn_cap.saturating_add(1);
        let enforced = match enforce_worker_output_contract_controlled(
            &self.provider,
            Some(&self.model),
            &text,
            self.is_critic,
            Some(self.abort),
            repair_timeout,
            allow_repair,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                let repair_usage_known = error.repairs == 0 || error.usage.is_some();
                let mut usage = outcome.usage;
                if let Some(repair_usage) = error.usage {
                    usage.add(&repair_usage);
                }
                return Err(WorkerRuntimeError {
                    message: error.message,
                    duration_ms: started.elapsed().as_millis() as u64,
                    steps: outcome.steps,
                    model_calls: model_calls.saturating_add(error.repairs),
                    usage,
                    usage_known: outcome.usage_known && repair_usage_known,
                    output_repairs: error.repairs,
                });
            }
        };
        let mut usage = outcome.usage;
        let mut usage_known = outcome.usage_known;
        if enforced.repairs > 0 {
            if let Some(repair_usage) = enforced.usage {
                usage.add(&repair_usage);
            } else {
                usage_known = false;
            }
        }
        Ok(WorkerRuntimeReport {
            output: enforced.text,
            duration_ms: started.elapsed().as_millis() as u64,
            steps: outcome.steps,
            model_calls: model_calls.saturating_add(enforced.repairs),
            usage,
            usage_known,
            output_repairs: enforced.repairs,
        })
    }
}

fn worker_runtime_error(
    message: String,
    duration_ms: u64,
    steps: usize,
    calls: &[crate::agent::ModelCallRecord],
) -> WorkerRuntimeError {
    let (model_calls, usage, usage_known) = summarize_model_calls(calls);
    WorkerRuntimeError {
        message,
        duration_ms,
        steps,
        model_calls,
        usage,
        usage_known,
        output_repairs: 0,
    }
}

fn summarize_model_calls(calls: &[crate::agent::ModelCallRecord]) -> (u32, TokenUsage, bool) {
    let mut usage = TokenUsage::default();
    for call in calls {
        if let Some(request_usage) = call.metadata.usage {
            usage.add(&request_usage);
        }
    }
    let usage_known = !calls.is_empty() && calls.iter().all(|call| call.metadata.usage.is_some());
    (
        u32::try_from(calls.len()).unwrap_or(u32::MAX),
        usage,
        usage_known,
    )
}

fn load_worker_session(
    workspace: &Path,
    model: &str,
    system_prompt: Option<String>,
    session_store: &Option<Arc<dyn SessionStore>>,
    worker_session_id: &Option<String>,
    parent_session_id: &Option<String>,
) -> Result<Session, WorkerRuntimeError> {
    let mut session = if let (Some(store), Some(session_id)) = (session_store, worker_session_id) {
        let legacy_id = session_id
            .rsplit_once("-worker-")
            .map(|(_, hash)| hash)
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()))
            .map(|hash| format!("worker-{hash}"));
        let load_id = if store
            .exists(session_id)
            .map_err(|error| format!("Worker 会话索引查询失败：{error}"))?
        {
            Some(session_id.clone())
        } else if let Some(legacy_id) = legacy_id {
            store
                .exists(&legacy_id)
                .map_err(|error| format!("Worker 历史会话索引查询失败：{error}"))?
                .then_some(legacy_id)
        } else {
            None
        };
        if let Some(load_id) = load_id {
            let loaded = store
                .load(&load_id)
                .map_err(|error| format!("Worker 会话 {session_id} 恢复失败：{error}"))?;
            if loaded.workspace != workspace
                || loaded.id != load_id
                || loaded.parent_id != *parent_session_id
            {
                return Err(format!("Worker 会话 {session_id} 的工作区或父会话归属不匹配").into());
            }
            loaded.with_model_override(Some(model.to_string()))
        } else {
            Session::new(workspace, model.to_string(), system_prompt)
                .with_model_override(Some(model.to_string()))
        }
    } else {
        Session::new(workspace, model.to_string(), system_prompt)
            .with_model_override(Some(model.to_string()))
    };
    session.parent_id = parent_session_id.clone();
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AgentError;
    use crate::gateway::{ChatMessage, ModelOutput};
    use crate::permissions::AutoApprover;
    use crate::session::Session;
    use crate::tools::ToolSpec;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FailingAfterSaves {
        count: AtomicUsize,
        successful: usize,
    }
    pub(super) fn failing_store(successful: usize) -> Arc<dyn SessionStore> {
        Arc::new(FailingAfterSaves {
            count: AtomicUsize::new(0),
            successful,
        })
    }

    impl SessionStore for FailingAfterSaves {
        fn create(
            &self,
            workspace: &Path,
            model: &str,
            system_prompt: Option<&str>,
        ) -> Result<Session, AgentError> {
            Ok(Session::new(
                workspace,
                model.to_string(),
                system_prompt.map(str::to_string),
            ))
        }

        fn load(&self, _id: &str) -> Result<Session, AgentError> {
            Err(AgentError::Session("test store does not load".to_string()))
        }

        fn save(&self, _session: &Session) -> Result<(), AgentError> {
            if self.count.fetch_add(1, Ordering::SeqCst) < self.successful {
                Ok(())
            } else {
                Err(AgentError::Session("test save failure".to_string()))
            }
        }
    }

    struct FailedProvider;

    #[async_trait::async_trait]
    impl ModelProvider for FailedProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Err("provider unavailable".to_string())
        }
    }

    struct ToolThenFailedProvider(AtomicUsize);

    #[async_trait::async_trait]
    impl ModelProvider for ToolThenFailedProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
                    id: "todo-call".to_string(),
                    name: "todo".to_string(),
                    arguments: serde_json::json!({
                        "todos": [{"content": "completed tool step", "status": "completed"}]
                    }),
                }]))
            } else {
                Err("provider unavailable after tool".to_string())
            }
        }
    }

    #[tokio::test]
    async fn provider_failure_preserves_completed_tool_steps() {
        let workspace = tempfile::tempdir().unwrap();
        let approver = AutoApprover { allow: true };
        let abort = AtomicBool::new(false);
        let runtime = WorkerRuntime {
            provider: Arc::new(ToolThenFailedProvider(AtomicUsize::new(0))),
            approver: &approver,
            abort: &abort,
            depth: 0,
            model: "test-model".to_string(),
            workspace: workspace.path().to_path_buf(),
            registry: ToolRegistry::new(),
            policy: Policy::new(workspace.path()),
            config: AgentConfig::default(),
            system_prompt: None,
            is_critic: false,
            event_sink: None,
            session_store: None,
            worker_session_id: None,
            parent_session_id: None,
        };

        let error = runtime
            .run_report("execute a tool then continue")
            .await
            .unwrap_err();

        assert!(error.message.contains("provider unavailable after tool"));
        assert_eq!(error.steps, 1);
        assert_eq!(error.model_calls, 2);
    }

    #[tokio::test]
    async fn provider_failure_keeps_worker_request_count_and_elapsed_time() {
        let workspace = tempfile::tempdir().unwrap();
        let approver = AutoApprover { allow: true };
        let abort = AtomicBool::new(false);
        let runtime = WorkerRuntime {
            provider: Arc::new(FailedProvider),
            approver: &approver,
            abort: &abort,
            depth: 0,
            model: "test-model".to_string(),
            workspace: workspace.path().to_path_buf(),
            registry: ToolRegistry::new(),
            policy: Policy::new(workspace.path()),
            config: AgentConfig::default(),
            system_prompt: None,
            is_critic: false,
            event_sink: None,
            session_store: Some(failing_store(2)),
            worker_session_id: Some("worker-test".to_string()),
            parent_session_id: None,
        };

        let error = runtime.run_report("run one request").await.unwrap_err();

        assert!(error.message.contains("provider unavailable"));
        assert!(error.message.contains("test save failure"));
        assert_eq!(error.model_calls, 1);
        assert!(error.duration_ms >= 5);
        assert!(!error.usage_known);
    }
    #[test]
    fn team_worker_identity_preserves_legacy_history_and_rejects_wrong_parent() {
        let workspace = tempfile::tempdir().unwrap();
        let store: Arc<dyn SessionStore> = Arc::new(
            crate::sqlite_store::SqliteSessionStore::open(&workspace.path().join("index.db"))
                .unwrap(),
        );
        let hash = "a".repeat(64);
        let mut legacy = store.create(workspace.path(), "old-model", None).unwrap();
        legacy.id = format!("worker-{hash}");
        legacy.parent_id = Some("source-chat".into());
        legacy.push(ChatMessage::user("previous task".into()));
        store.save(&legacy).unwrap();
        let restored = load_worker_session(
            workspace.path(),
            "new-model",
            None,
            &Some(Arc::clone(&store)),
            &Some(format!("team-a-worker-{hash}")),
            &Some("source-chat".into()),
        )
        .unwrap();
        assert_eq!(restored.messages.len(), legacy.messages.len());
        assert!(
            store.exists(&legacy.id).unwrap(),
            "original history must be retained"
        );
        assert!(load_worker_session(
            workspace.path(),
            "new-model",
            None,
            &Some(store),
            &Some(format!("team-a-worker-{hash}")),
            &Some("wrong-parent".into())
        )
        .is_err());
    }

    struct ObservedInputProvider {
        observed: Arc<std::sync::Mutex<Vec<ChatMessage>>>,
        fail: bool,
    }
    #[async_trait::async_trait]
    impl ModelProvider for ObservedInputProvider {
        async fn complete(
            &self,
            messages: &[ChatMessage],
            _: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            *self.observed.lock().unwrap() = messages.to_vec();
            if self.fail {
                return Err("fixture provider failure".into());
            }
            Ok(ModelOutput::Text(r#"{"status":"done","summary":"seen","artifact":{"kind":"note","format":"text","content":"accepted"},"evidence":[],"open_issues":[]}"#.into()))
        }
    }
    #[tokio::test]
    async fn worker_model_input_and_reopened_history_keep_same_refs_on_success_and_failure() {
        for fail in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let store: Arc<dyn SessionStore> = Arc::new(
                crate::sqlite_store::SqliteSessionStore::open(&workspace.path().join("history.db"))
                    .unwrap(),
            );
            let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
            let approver = AutoApprover { allow: false };
            let abort = AtomicBool::new(false);
            let runtime = WorkerRuntime {
                provider: Arc::new(ObservedInputProvider {
                    observed: Arc::clone(&observed),
                    fail,
                }),
                approver: &approver,
                abort: &abort,
                depth: 0,
                model: "fixture".into(),
                workspace: workspace.path().to_path_buf(),
                registry: ToolRegistry::empty(),
                policy: Policy::new(workspace.path()),
                config: AgentConfig {
                    max_turns: 2,
                    compaction_enabled: false,
                    ..Default::default()
                },
                system_prompt: None,
                is_critic: false,
                event_sink: None,
                session_store: Some(Arc::clone(&store)),
                worker_session_id: Some("team-worker-input".into()),
                parent_session_id: Some("parent".into()),
            };
            let input = owo_agent_protocol::StoredUserInput {
                prompt: "original prompt".into(),
                content_parts: vec![owo_agent_protocol::ContentPart::Image {
                    asset: owo_agent_protocol::AssetRef {
                        artifact_id: "input-12345678-1234-5678-9012-123456789012".into(),
                        sha256: "a".repeat(64),
                    },
                }],
                legacy_attachments: Vec::new(),
            };
            let images = vec![crate::gateway::MessageImage::from_url(
                "data:image/png;base64,cGl4ZWxz",
            )];
            let result = runtime
                .run_report_with_input(
                    "compiled prompt with attachment metadata",
                    &images,
                    Some(input.clone()),
                )
                .await;
            assert_eq!(result.is_err(), fail);
            let request = observed.lock().unwrap();
            let sent = request.iter().find(|m| m.role == "user").unwrap();
            assert_eq!(sent.images, images);
            assert_eq!(sent.input.as_ref(), Some(&input));
            let restored = store.load("team-worker-input").unwrap();
            assert_eq!(restored.parent_id.as_deref(), Some("parent"));
            let saved = restored.messages.iter().find(|m| m.role == "user").unwrap();
            assert_eq!(saved.input.as_ref(), Some(&input));
            assert_eq!(saved.images, images);
        }
    }
}

#[cfg(test)]
#[path = "worker_checkpoint_tests.rs"]
mod checkpoint_tests;
