//! Host injection for media output publication; ToolHost policy still wraps execution.
use super::{Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use owo_agent_protocol::ToolOutputArtifact;
use serde_json::Value;
use std::path::Path;
use std::sync::{atomic::Ordering, Arc};

#[async_trait]
pub trait ToolOutputPublisher: Send + Sync {
    async fn publish(
        &self,
        workspace: &Path,
        session_id: &str,
        tool: &str,
        result: &Value,
    ) -> Result<Vec<ToolOutputArtifact>, String>;
}

pub(super) struct PublishedMediaTool {
    pub inner: Arc<dyn Tool>,
    pub publisher: Arc<dyn ToolOutputPublisher>,
}

#[async_trait]
impl Tool for PublishedMediaTool {
    fn spec(&self) -> ToolSpec { self.inner.spec() }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let mut result = self.inner.run(ctx, args).await?;
        if ctx.abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("media output publication cancelled".into());
        }
        let outputs = self.publisher.publish(ctx.workspace, &ctx.session.id, &self.spec().name, &result).await?;
        let object = result.as_object_mut().ok_or("media tool returned a non-object result")?;
        object.remove("data_url");
        object.insert("outputs".into(), serde_json::to_value(outputs).map_err(|error| error.to_string())?);
        Ok(result)
    }
}
