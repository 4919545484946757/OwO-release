//! Host injection for media output publication; ToolHost policy still wraps execution.
use super::{Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use owo_agent_protocol::ToolOutputArtifact;
use serde_json::Value;
use std::path::Path;
use std::sync::{atomic::Ordering, Arc};

/// Recognize supported media container headers for execution and publication.
/// This is a bounded format hint, not full codec validation or a decoding guarantee.
pub fn media_output_mime(bytes: &[u8], kind: &str) -> Option<&'static str> {
    match kind {
        "image" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => Some("image/png"),
        "image" if bytes.starts_with(&[0xff, 0xd8, 0xff]) => Some("image/jpeg"),
        "image" if bytes.starts_with(b"GIF8") => Some("image/gif"),
        "image" if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" => {
            Some("image/webp")
        }
        "audio"
            if bytes.starts_with(b"ID3")
                || (bytes.len() > 1 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0) =>
        {
            Some("audio/mpeg")
        }
        "audio" if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" => {
            Some("audio/wav")
        }
        "audio" if bytes.starts_with(b"OggS") => Some("audio/ogg"),
        "audio" if bytes.starts_with(b"fLaC") => Some("audio/flac"),
        _ => None,
    }
}

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
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let mut result = self.inner.run(ctx, args).await?;
        if ctx.abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("media output publication cancelled".into());
        }
        let outputs = self
            .publisher
            .publish(ctx.workspace, &ctx.session.id, &self.spec().name, &result)
            .await?;
        let object = result
            .as_object_mut()
            .ok_or("media tool returned a non-object result")?;
        object.remove("data_url");
        object.insert(
            "outputs".into(),
            serde_json::to_value(outputs).map_err(|error| error.to_string())?,
        );
        Ok(result)
    }
}
