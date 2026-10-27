use super::*;
use crate::gateway::{ChatMessage, MessageImage, ModelOutput};
use crate::permissions::AutoApprover;
use crate::sqlite_store::SqliteSessionStore;
use crate::tools::ToolSpec;
use owo_agent_protocol::{AssetRef, ContentPart, StoredUserInput};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

fn runtime<'a>(
    workspace: &Path,
    approver: &'a dyn Approver,
    abort: &'a AtomicBool,
    provider: Arc<dyn ModelProvider>,
    store: Arc<dyn SessionStore>,
) -> WorkerRuntime<'a> {
    WorkerRuntime {
        provider,
        approver,
        abort,
        depth: 0,
        model: "fixture".into(),
        workspace: workspace.to_path_buf(),
        registry: ToolRegistry::new(),
        policy: Policy::new(workspace),
        config: AgentConfig {
            max_turns: 3,
            compaction_enabled: false,
            subagent_depth: 1,
            ..Default::default()
        },
        system_prompt: None,
        is_critic: false,
        event_sink: None,
        session_store: Some(store),
        worker_session_id: Some("checkpoint-worker".into()),
        parent_session_id: Some("parent".into()),
    }
}
fn original_input() -> StoredUserInput {
    StoredUserInput {
        prompt: "original user request".into(),
        content_parts: vec![ContentPart::Image {
            asset: AssetRef {
                artifact_id: "input-child-asset".into(),
                sha256: "b".repeat(64),
            },
        }],
        legacy_attachments: Vec::new(),
    }
}
struct PendingProvider {
    calls: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    store: Arc<dyn SessionStore>,
    after_tool: bool,
}
#[async_trait::async_trait]
impl ModelProvider for PendingProvider {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        _: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let saved = self.store.load("checkpoint-worker").unwrap();
        let user = saved.messages.iter().find(|m| m.role == "user").unwrap();
        assert_eq!(user.input.as_ref(), Some(&original_input()));
        assert_eq!(
            user.images,
            messages.iter().find(|m| m.role == "user").unwrap().images
        );
        if self.after_tool && call == 0 {
            return Ok(ModelOutput::ToolCalls(vec![crate::gateway::ToolCall {
                id: "todo-checkpoint".into(),
                name: "todo".into(),
                arguments: serde_json::json!({"todos":[{"content":"durable completed step","status":"completed"}]}),
            }]));
        }
        if self.after_tool {
            assert!(saved
                .messages
                .iter()
                .any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("todo-checkpoint")));
            assert_eq!(saved.todos[0].status, "completed");
        }
        self.entered.notify_one();
        std::future::pending().await
    }
}
#[tokio::test]
async fn checkpoint_input_and_completed_tool_history_survive_dropped_worker_future() {
    for after_tool in [false, true] {
        let workspace = tempfile::tempdir().unwrap();
        let db = workspace.path().join("sessions.db");
        let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::open(&db).unwrap());
        let entered = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(PendingProvider {
            calls: Arc::clone(&calls),
            entered: Arc::clone(&entered),
            store: Arc::clone(&store),
            after_tool,
        });
        let approver = AutoApprover { allow: true };
        let abort = AtomicBool::new(false);
        let images = [MessageImage::from_url(
            "data:image/png;base64,fixture-pixels",
        )];
        let mut running = Box::pin(
            runtime(
                workspace.path(),
                &approver,
                &abort,
                provider,
                Arc::clone(&store),
            )
            .run_report_with_input(
                "compiled worker prompt",
                &images,
                Some(original_input()),
            ),
        );
        tokio::select! {
            biased;
            result=&mut running=>panic!("provider should remain pending: {result:?}"),
            ready=tokio::time::timeout(std::time::Duration::from_secs(5),entered.notified())=>ready.unwrap(),
        }
        // Independent SQLite connection while the Provider is still pending.
        let reopened = SqliteSessionStore::open(&db).unwrap();
        let during = reopened.load("checkpoint-worker").unwrap();
        assert_eq!(
            during.messages.iter().filter(|m| m.role == "user").count(),
            1
        );
        assert_eq!(during.messages[0].input.as_ref(), Some(&original_input()));
        assert_eq!(during.messages[0].images, images);
        assert_eq!(during.parent_id.as_deref(), Some("parent"));
        drop(running); // Does not run WorkerRuntime's normal terminal-save path.
        let after = reopened.load("checkpoint-worker").unwrap();
        assert_eq!(
            serde_json::to_value(&during.messages).unwrap(),
            serde_json::to_value(&after.messages).unwrap()
        );
        let count = calls.load(Ordering::SeqCst);
        let loaded = load_worker_session(
            workspace.path(),
            "fixture",
            None,
            &Some(Arc::clone(&store)),
            &Some("checkpoint-worker".into()),
            &Some("parent".into()),
        )
        .unwrap();
        assert_eq!(loaded.messages.len(), after.messages.len());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            count,
            "loading history must never replay tools"
        );
        if after_tool {
            assert_eq!(after.todos[0].status, "completed");
        }
    }
}
struct CountedProvider(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl ModelProvider for CountedProvider {
    async fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ModelOutput, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ModelOutput::Text(r#"{"status":"done","summary":"done","artifact":{"kind":"note","format":"text","content":"done"},"evidence":[],"open_issues":[]}"#.into()))
    }
}
#[tokio::test]
async fn checkpoint_failure_blocks_provider_before_any_request() {
    let workspace = tempfile::tempdir().unwrap();
    let approver = AutoApprover { allow: false };
    let abort = AtomicBool::new(false);
    let calls = Arc::new(AtomicUsize::new(0));
    let error = runtime(
        workspace.path(),
        &approver,
        &abort,
        Arc::new(CountedProvider(Arc::clone(&calls))),
        super::tests::failing_store(0),
    )
    .run_report("admitted input")
    .await
    .unwrap_err();
    assert!(error.message.contains("turn/checkpoint_failed"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(error.model_calls, 0);
}
#[tokio::test]
async fn checkpoint_terminal_save_failure_never_reports_success() {
    let workspace = tempfile::tempdir().unwrap();
    let approver = AutoApprover { allow: false };
    let abort = AtomicBool::new(false);
    let calls = Arc::new(AtomicUsize::new(0));
    let error = runtime(
        workspace.path(),
        &approver,
        &abort,
        Arc::new(CountedProvider(Arc::clone(&calls))),
        super::tests::failing_store(2),
    )
    .run_report("admitted input")
    .await
    .unwrap_err();
    assert!(error.message.contains("turn/checkpoint_failed"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.model_calls, 1);
}
#[tokio::test]
async fn checkpoint_precancelled_worker_keeps_input_without_model_call() {
    let workspace = tempfile::tempdir().unwrap();
    let db = workspace.path().join("sessions.db");
    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::open(&db).unwrap());
    let approver = AutoApprover { allow: false };
    let abort = AtomicBool::new(true);
    let calls = Arc::new(AtomicUsize::new(0));
    let error = runtime(
        workspace.path(),
        &approver,
        &abort,
        Arc::new(CountedProvider(Arc::clone(&calls))),
        store,
    )
    .run_report_with_input("compiled", &[], Some(original_input()))
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(error.model_calls, 0);
    let restored = SqliteSessionStore::open(&db)
        .unwrap()
        .load("checkpoint-worker")
        .unwrap();
    assert_eq!(restored.messages.len(), 1);
    assert_eq!(restored.messages[0].input.as_ref(), Some(&original_input()));
}
