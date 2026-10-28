use super::*;
use owo_agent_core::gateway::MessageImage;
use owo_agent_core::project_space_store::SqliteProjectSpaceStore;
use owo_agent_protocol::{AssetRef, StoredUserInput};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    calls: AtomicU64,
    mode: u8,
}
#[async_trait]
impl ModelProvider for Fixture {
    fn validate_image_input(&self, model: Option<&str>, has_images: bool) -> Result<(), String> {
        if !has_images || model == Some("model-override") {
            Ok(())
        } else {
            Err("input/capability_binding_mismatch".into())
        }
    }

    async fn complete(&self, _: &[ChatMessage], _: &[ToolSpec]) -> Result<ModelOutput, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            1 => Err("PRIVATE_PROVIDER_ERROR sk-do-not-persist".into()),
            2 => std::future::pending().await,
            _ => Ok(ModelOutput::Text("fixture result".into())),
        }
    }
    async fn complete_with_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ObservedModelOutput, String> {
        assert_eq!(model, Some("model-override"));
        Ok(ObservedModelOutput {
            output: self.complete(messages, tools).await?,
            metadata: ModelCallMetadata {
                request_id: Some("PRIVATE_REQUEST_ID".into()),
                model: Some("provider-model".into()),
                usage: Some(TokenUsage {
                    prompt_tokens: 11,
                    completion_tokens: 7,
                    total_tokens: 18,
                }),
                latency_ms: Some(3),
            },
        })
    }
    async fn complete_stream_with_reasoning_and_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ObservedModelOutput, String> {
        on_chunk(StreamChunk::Reasoning("reasoning fixture".into()));
        on_chunk(StreamChunk::Content("content fixture".into()));
        self.complete_with_model_observed(model, messages, tools)
            .await
    }
    async fn complete_expression(&self, messages: &[ChatMessage]) -> Result<String, String> {
        self.complete(messages, &[]).await?;
        Ok("expression".into())
    }
    fn usage_snapshot(&self) -> TokenUsage {
        TokenUsage {
            prompt_tokens: 11,
            completion_tokens: 7,
            total_tokens: 18,
        }
    }
}
async fn observer(
    mode: u8,
) -> (
    tempfile::TempDir,
    Arc<SqliteProjectSpaceStore>,
    Arc<Fixture>,
    InputObservedProvider,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteProjectSpaceStore::open(&dir.path().join("space.db")).unwrap());
    let team = serde_json::from_value(json!({
        "team_id":"team","goal_id":"goal","mode":"team","members":[],
        "project_space_id":"project","task_graph_ref":null,"template_id":null,
        "shared_context_refs":[],"budget":{},"human_policy":null,"strategy_decision":null,
        "status":"running","created_at":"2026-10-26T00:00:00Z","updated_at":"2026-10-26T00:00:00Z"
    }))
    .unwrap();
    store.save_team_run(&team).await.unwrap();
    let mut candidate = NewRunEvent::new(
        "team",
        "input".into(),
        "worker.input.prepared",
        json!({"target":{"worker_id":"member","step_id":"step","task_id":"task","attempt_id":"attempt","phase_epoch":1}}),
    );
    candidate.node_id = Some("node".into());
    candidate.task_id = Some("task".into());
    candidate.attempt = Some("attempt".into());
    let prepared = store.append_run_event(&candidate).unwrap();
    let fixture = Arc::new(Fixture {
        calls: AtomicU64::new(0),
        mode,
    });
    let provider = InputObservedProvider {
        inner: fixture.clone(),
        store: store.clone(),
        prepared,
        worker_session: Some("worker-session".into()),
    };
    (dir, store, fixture, provider)
}
fn input() -> Vec<ChatMessage> {
    let mut user = ChatMessage::user("PRIVATE_COMPILED_PROMPT".into());
    user.images.push(MessageImage::from_url(
        "data:image/png;base64,PRIVATE_IMAGE_BYTES",
    ));
    user.input = Some(StoredUserInput {
        prompt: "PRIVATE_ORIGINAL_PROMPT".into(),
        legacy_attachments: vec!["PRIVATE_FILENAME".into()],
        content_parts: (0..20)
            .map(|i| ContentPart::Image {
                asset: AssetRef {
                    artifact_id: format!("child-asset-{i}"),
                    sha256: "a".repeat(64),
                },
            })
            .collect(),
    });
    vec![ChatMessage::system("PRIVATE_SYSTEM_PROMPT".into()), user]
}
fn events(store: &SqliteProjectSpaceStore) -> Vec<RunEvent> {
    store
        .run_events_after("team", 0, 256)
        .unwrap()
        .events
        .into_iter()
        .filter(|e| e.event_type.starts_with("model."))
        .collect()
}
#[tokio::test]
async fn exact_request_summary_is_durable_bounded_private_and_causally_linked() {
    let (dir, store, fixture, provider) = observer(0).await;
    let messages = input();
    let mut chunks = Vec::new();
    let output = provider
        .complete_stream_with_reasoning_and_model_observed(
            Some("model-override"),
            &messages,
            &[],
            &mut |chunk| chunks.push(chunk),
        )
        .await
        .unwrap();
    assert_eq!(output.metadata.usage.unwrap().total_tokens, 18);
    assert_eq!(chunks.len(), 2);
    assert!(matches!(chunks[0], StreamChunk::Reasoning(_)));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    let observed = events(&store);
    assert_eq!(observed.len(), 2);
    assert_eq!(
        observed[0].causation_id.as_ref(),
        Some(&provider.prepared.event_id)
    );
    assert_eq!(
        observed[1].causation_id.as_ref(),
        Some(&observed[0].event_id)
    );
    assert_eq!(observed[0].node_id.as_deref(), Some("node"));
    assert_eq!(observed[0].attempt.as_deref(), Some("attempt"));
    assert_eq!(
        observed[0].data["request"],
        identity(&(Some("model-override"), &messages, &[] as &[Value])).unwrap()
    );
    assert_eq!(observed[0].data["last_user_input"]["part_count"], 20);
    assert_eq!(
        observed[0].data["last_user_input"]["parts"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(observed[0].data["last_user_input"]["parts_truncated"], true);
    assert_eq!(observed[1].data["usage"]["total_tokens"], 18);
    assert_eq!(observed[1].data["provider_succeeded"], true);
    assert_eq!(observed[1].data["model_delivery"], "not_proven");
    let raw = serde_json::to_string(&observed).unwrap();
    assert!(!raw.contains("PRIVATE_"));
    assert!(serde_json::to_vec(&observed[0]).unwrap().len() < 16 * 1024);
    let reopened = SqliteProjectSpaceStore::open(&dir.path().join("space.db")).unwrap();
    assert_eq!(events(&reopened), observed);
}
#[tokio::test]
async fn provider_failure_remains_failure_without_logging_error_text() {
    let (_dir, store, fixture, provider) = observer(1).await;
    assert!(provider
        .complete(&input(), &[])
        .await
        .unwrap_err()
        .contains("PRIVATE_PROVIDER_ERROR"));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    let observed = events(&store);
    assert_eq!(observed.len(), 2);
    assert_eq!(observed[1].data["provider_succeeded"], false);
    assert!(!serde_json::to_string(&observed)
        .unwrap()
        .contains("PRIVATE_"));
}
#[tokio::test]
async fn cancelled_future_records_interruption_without_claiming_network_cancellation() {
    let (_dir, store, fixture, provider) = observer(2).await;
    let messages = input();
    tokio::select! {
        biased;
        _=provider.complete(&messages,&[])=>panic!("pending provider returned"),
        _=tokio::task::yield_now()=>{}
    }
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    let observed = events(&store);
    assert_eq!(observed.len(), 2);
    assert_eq!(observed[1].event_type, "model.request.interrupted");
    assert_eq!(observed[1].data["model_delivery"], "unknown");
    assert!(observed[1].data["provider_succeeded"].is_null());
}
#[tokio::test]
async fn missing_run_cannot_start_provider_and_unscoped_calls_forge_no_receipt() {
    let (_dir, store, fixture, mut provider) = observer(0).await;
    provider.prepared.run_id = "missing".into();
    assert_eq!(
        provider.complete(&input(), &[]).await.unwrap_err(),
        "input/provider_receipt_not_committed"
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert!(events(&store).is_empty());
    let plain: Arc<dyn ModelProvider> = fixture.clone();
    let result = InputObservedProvider::for_worker(
        plain.clone(),
        store,
        "team",
        &ResolvedTaskContext::default(),
        Some("session".into()),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&plain, &result));
}
#[tokio::test]
async fn every_provider_entry_keeps_one_request_and_preserves_streams_model_and_usage() {
    let (_dir, store, fixture, provider) = observer(0).await;
    let messages = input();
    provider.complete_expression(&messages).await.unwrap();
    provider.complete(&messages, &[]).await.unwrap();
    provider
        .complete_stream(&messages, &[], &mut |_| {})
        .await
        .unwrap();
    provider
        .complete_with_model(Some("model-override"), &messages, &[])
        .await
        .unwrap();
    provider
        .complete_with_model_observed(Some("model-override"), &messages, &[])
        .await
        .unwrap();
    provider
        .complete_stream_with_model(Some("model-override"), &messages, &[], &mut |_| {})
        .await
        .unwrap();
    provider
        .complete_stream_with_reasoning(&messages, &[], &mut |_| {})
        .await
        .unwrap();
    provider
        .complete_stream_with_reasoning_and_model(
            Some("model-override"),
            &messages,
            &[],
            &mut |_| {},
        )
        .await
        .unwrap();
    provider
        .complete_stream_with_reasoning_and_model_observed(
            Some("model-override"),
            &messages,
            &[],
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 9);
    let observed = events(&store);
    assert_eq!(observed.len(), 18);
    let ids = observed
        .iter()
        .step_by(2)
        .map(|e| e.data["call_id"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), 9);
    assert_eq!(provider.usage_snapshot().total_tokens, 18);
}
#[tokio::test]
async fn budget_rejection_creates_no_provider_input_or_fake_request() {
    use super::super::workswarm_metrics::{
        MeasuredProvider, RequestReservationJournal, RequestUsageCollector, TeamModelRequestBudget,
    };
    let (dir, store, fixture, provider) = observer(0).await;
    let budget = Arc::new(
        TeamModelRequestBudget::new(1, RequestReservationJournal::for_team(dir.path(), "team"))
            .unwrap(),
    );
    let measured = MeasuredProvider::new_with_task_attempt_request_budget(
        Arc::new(provider),
        Arc::new(AtomicU64::new(0)),
        Arc::new(RequestUsageCollector::default()),
        "step".into(),
        "attempt".into(),
        Some(budget),
        None,
    );
    measured.complete(&input(), &[]).await.unwrap();
    assert!(measured.complete(&input(), &[]).await.is_err());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    assert_eq!(events(&store).len(), 2);
}

#[test]
fn borrowed_tool_projection_hashes_all_fields_and_changes_when_schema_changes() {
    let messages = input();
    let mut tools = vec![ToolSpec::with_effect(
        "tool",
        "description".into(),
        json!({"type":"object"}),
        None,
    )];
    let raw_expected = format!(
        r#"[null,{},[{{"name":"tool","description":"description","input_schema":{{"type":"object"}},"effect":null}}]]"#,
        serde_json::to_string(&messages).unwrap()
    );
    let expected = json!({
        "sha256":format!("{:x}", Sha256::digest(raw_expected.as_bytes())),
        "bytes":raw_expected.len()
    });
    let first = request_identity(None, &messages, &tools).unwrap();
    assert_eq!(first, expected);
    tools[0].input_schema = json!({"type":"string"});
    assert_ne!(first, request_identity(None, &messages, &tools).unwrap());
}

#[tokio::test]
async fn declaration_preflight_is_forwarded_without_calls_or_observations() {
    let (_dir, store, fixture, provider) = observer(0).await;
    assert!(provider
        .validate_image_input(Some("model-override"), true)
        .is_ok());
    assert!(provider.validate_image_input(Some("other"), false).is_ok());
    assert_eq!(
        provider
            .validate_image_input(Some("other"), true)
            .unwrap_err(),
        "input/capability_binding_mismatch"
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert!(events(&store).is_empty());
}
