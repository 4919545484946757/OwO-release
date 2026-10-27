//! Provider-boundary receipts in the existing run journal, after budget admission.
//! Digests describe gateway values, not HTTP wire bytes or model comprehension.
use async_trait::async_trait;
use owo_agent_core::gateway::{
    ChatMessage, ModelCallMetadata, ModelOutput, ModelProvider, ObservedModelOutput, StreamChunk,
    TokenUsage,
};
use owo_agent_core::project_space_store::{
    NewRunEvent, ProjectSpaceStoreBackend, WorkerInputTarget,
};
use owo_agent_core::task_context::ResolvedTaskContext;
use owo_agent_core::tools::ToolSpec;
use owo_agent_protocol::{ContentPart, RunEvent};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{future::Future, sync::Arc};

pub(super) struct InputObservedProvider {
    inner: Arc<dyn ModelProvider>,
    store: Arc<dyn ProjectSpaceStoreBackend>,
    prepared: RunEvent,
    worker_session: Option<String>,
}

#[derive(Default)]
struct DigestWriter {
    digest: Sha256,
    bytes: u64,
}
impl std::io::Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.digest.update(bytes);
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn identity(value: &(impl Serialize + ?Sized)) -> Result<Value, String> {
    let mut sink = DigestWriter::default();
    serde_json::to_writer(&mut sink, value)
        .map_err(|_| "input/observation_serialization_failed".to_string())?;
    Ok(json!({"sha256":format!("{:x}",sink.digest.finalize()), "bytes":sink.bytes}))
}

// Borrow every ToolSpec field: no schema clone and no Core serialization contract change.
#[derive(Serialize)]
struct ToolView<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a Value,
    effect: &'a Option<owo_agent_core::tool_effects::ToolEffect>,
}
fn request_identity(
    model: Option<&str>,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> Result<Value, String> {
    let tools: Vec<_> = tools
        .iter()
        .map(|tool| ToolView {
            name: &tool.name,
            description: &tool.description,
            input_schema: &tool.input_schema,
            effect: &tool.effect,
        })
        .collect();
    identity(&(model, messages, tools))
}

impl InputObservedProvider {
    pub(super) fn for_worker(
        inner: Arc<dyn ModelProvider>,
        store: Arc<dyn ProjectSpaceStoreBackend>,
        run: &str,
        task: &ResolvedTaskContext,
        worker_session: Option<String>,
    ) -> Result<Arc<dyn ModelProvider>, String> {
        // Compatibility callers outside RoleWorker have no host receipt. Do not
        // infer one from user JSON or attach them to a different concurrent run.
        let Some(prepared) = owo_agent_core::workswarm::current_worker_input_event() else {
            return Ok(inner);
        };
        let target: WorkerInputTarget = serde_json::from_value(prepared.data["target"].clone())
            .map_err(|_| "input/worker_receipt_target_invalid".to_string())?;
        if prepared.run_id != run
            || prepared.event_type != "worker.input.prepared"
            || Some(target.step_id.as_str()) != task.step_id.as_deref()
            || target.task_id != task.task_id
            || target.attempt_id != task.attempt_id
            || target.phase_epoch != task.phase_epoch
        {
            return Err("input/worker_receipt_scope_mismatch".into());
        }
        Ok(Arc::new(Self {
            inner,
            store,
            prepared,
            worker_session,
        }))
    }

    fn candidate(&self, call: &str, kind: &str, data: Value, cause: &RunEvent) -> NewRunEvent {
        let mut event = NewRunEvent::new(
            &self.prepared.run_id,
            format!("provider-input:{call}:{kind}"),
            kind,
            data,
        );
        event.node_id = self.prepared.node_id.clone();
        event.task_id = self.prepared.task_id.clone();
        event.attempt = self.prepared.attempt.clone();
        event.causation_id = Some(cause.event_id.clone());
        event
    }

    fn begin(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<RequestReceipt<'_>, String> {
        let call = uuid::Uuid::new_v4().to_string();
        let input = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .and_then(|m| m.input.as_ref());
        // At most eight small custody summaries. Full input stays hashed even
        // when the display summary is truncated. Never persist inline images,
        // prompt text, tool parameters, legacy filenames, or raw provider errors.
        let parts = input
            .map(|i| {
                i.content_parts
                    .iter()
                    .take(8)
                    .map(|part| match part {
                        ContentPart::Text { text } => {
                            Ok(json!({"type":"text","identity":identity(text)?}))
                        }
                        ContentPart::Image { asset }
                        | ContentPart::Audio { asset }
                        | ContentPart::Video { asset }
                        | ContentPart::File { asset } => {
                            let kind = match part {
                                ContentPart::Image { .. } => "image",
                                ContentPart::Audio { .. } => "audio",
                                ContentPart::Video { .. } => "video",
                                _ => "file",
                            };
                            let reference =
                                if asset.artifact_id.len() <= 256 && asset.sha256.len() <= 64 {
                                    json!({"artifact_id":asset.artifact_id,"sha256":asset.sha256})
                                } else {
                                    json!({"identity":identity(asset)?})
                                };
                            Ok(json!({"type":kind,"asset":reference}))
                        }
                    })
                    .collect::<Result<Vec<Value>, String>>()
            })
            .transpose()?;
        let data = json!({
            "call_id":call, "worker_session_id":self.worker_session,
            "boundary":"before_provider_delegate", "model_delivery":"not_proven",
            "representation":"gateway_chat_message_v1_including_input_metadata",
            "request":request_identity(model,messages,tools)?,
            "model_override":model.map(identity).transpose()?,
            "message_count":messages.len(), "tool_count":tools.len(),
            "image_count":messages.iter().map(|m|m.images.len()).sum::<usize>(),
            "last_user_input":input.map(|i| -> Result<Value,String> { Ok(json!({
                "identity":identity(i)?, "prompt":identity(&i.prompt)?,
                "part_count":i.content_parts.len(),"parts":parts,
                "parts_truncated":i.content_parts.len()>8,
                "legacy_attachment_count":i.legacy_attachments.len()
            })) }).transpose()?
        });
        let event = self
            .store
            .append_run_event(&self.candidate(&call, "model.input.prepared", data, &self.prepared))
            .map_err(|_| "input/provider_receipt_not_committed".to_string())?;
        Ok(RequestReceipt {
            observer: self,
            call,
            event,
            closed: false,
        })
    }

    async fn observe<T, F>(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        execution: F,
    ) -> Result<T, String>
    where
        T: ObservedResult + Send,
        F: Future<Output = Result<T, String>> + Send,
    {
        let mut receipt = self.begin(model, messages, tools)?;
        let result = execution.await;
        receipt.finish(
            result.is_ok(),
            result.as_ref().ok().and_then(ObservedResult::metadata),
        )?;
        result
    }
}

trait ObservedResult {
    fn metadata(&self) -> Option<&ModelCallMetadata> {
        None
    }
}
impl ObservedResult for ModelOutput {}
impl ObservedResult for String {}
impl ObservedResult for ObservedModelOutput {
    fn metadata(&self) -> Option<&ModelCallMetadata> {
        Some(&self.metadata)
    }
}
struct RequestReceipt<'a> {
    observer: &'a InputObservedProvider,
    call: String,
    event: RunEvent,
    closed: bool,
}
impl RequestReceipt<'_> {
    fn finish(
        &mut self,
        succeeded: bool,
        metadata: Option<&ModelCallMetadata>,
    ) -> Result<(), String> {
        let data = json!({
            "call_id":self.call, "boundary":"provider_returned",
            "provider_succeeded":succeeded, "model_delivery":"not_proven",
            "request_id":metadata.and_then(|m|m.request_id.as_ref()).map(identity).transpose()?,
            "model":metadata.and_then(|m|m.model.as_ref()).map(identity).transpose()?,
            "usage":metadata.and_then(|m|m.usage),
            "latency_ms":metadata.and_then(|m|m.latency_ms)
        });
        self.observer
            .store
            .append_run_event(&self.observer.candidate(
                &self.call,
                "model.request.completed",
                data,
                &self.event,
            ))
            .map_err(|_| "input/provider_result_receipt_not_committed".to_string())?;
        self.closed = true;
        Ok(())
    }
}
impl Drop for RequestReceipt<'_> {
    fn drop(&mut self) {
        if !self.closed {
            let data = json!({"call_id":self.call,"boundary":"observer_dropped_before_result_receipt",
                "model_delivery":"unknown","provider_succeeded":null});
            if self
                .observer
                .store
                .append_run_event(&self.observer.candidate(
                    &self.call,
                    "model.request.interrupted",
                    data,
                    &self.event,
                ))
                .is_err()
            {
                // A process crash or database failure can leave an open prepared
                // event. Never manufacture a completion during recovery.
                tracing::warn!("provider observation interrupted receipt could not be committed");
            }
        }
    }
}

#[async_trait]
impl ModelProvider for InputObservedProvider {
    async fn complete_expression(&self, messages: &[ChatMessage]) -> Result<String, String> {
        self.observe(
            None,
            messages,
            &[],
            self.inner.complete_expression(messages),
        )
        .await
    }
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.observe(None, messages, tools, self.inner.complete(messages, tools))
            .await
    }
    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        self.observe(
            None,
            messages,
            tools,
            self.inner.complete_stream(messages, tools, on_delta),
        )
        .await
    }
    async fn complete_with_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.observe(
            model,
            messages,
            tools,
            self.inner.complete_with_model(model, messages, tools),
        )
        .await
    }
    async fn complete_with_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> Result<ObservedModelOutput, String> {
        self.observe(
            model,
            messages,
            tools,
            self.inner
                .complete_with_model_observed(model, messages, tools),
        )
        .await
    }
    async fn complete_stream_with_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<ModelOutput, String> {
        self.observe(
            model,
            messages,
            tools,
            self.inner
                .complete_stream_with_model(model, messages, tools, on_delta),
        )
        .await
    }
    async fn complete_stream_with_reasoning(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.observe(
            None,
            messages,
            tools,
            self.inner
                .complete_stream_with_reasoning(messages, tools, on_chunk),
        )
        .await
    }
    async fn complete_stream_with_reasoning_and_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ModelOutput, String> {
        self.observe(
            model,
            messages,
            tools,
            self.inner
                .complete_stream_with_reasoning_and_model(model, messages, tools, on_chunk),
        )
        .await
    }
    async fn complete_stream_with_reasoning_and_model_observed(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_chunk: &mut (dyn FnMut(StreamChunk) + Send),
    ) -> Result<ObservedModelOutput, String> {
        self.observe(
            model,
            messages,
            tools,
            self.inner
                .complete_stream_with_reasoning_and_model_observed(
                    model, messages, tools, on_chunk,
                ),
        )
        .await
    }
    fn usage_snapshot(&self) -> TokenUsage {
        self.inner.usage_snapshot()
    }
}

#[cfg(test)]
#[path = "provider_observation_tests.rs"]
mod tests;
