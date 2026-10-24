//! Media tools publish into the existing Artifact database and CAS, not a parallel store.
use super::ArtifactReviewState;
use async_trait::async_trait;
use owo_agent_core::project_space_store::ProjectSpaceStoreBackend;
use owo_agent_core::tools::ToolOutputPublisher;
use owo_agent_core::CasStore;
use owo_agent_protocol::{Artifact, ToolOutputArtifact};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncReadExt;

const MAX_MEDIA_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) struct MediaOutputPublisher {
    state: ArtifactReviewState,
    cas_dir: PathBuf,
}

impl MediaOutputPublisher {
    pub(crate) fn new(state: ArtifactReviewState, cas_dir: PathBuf) -> Self {
        Self { state, cas_dir }
    }
}

fn media_type(bytes: &[u8], kind: &str) -> Option<&'static str> {
    match kind {
        "image" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => Some("image/png"),
        "image" if bytes.starts_with(&[0xff, 0xd8, 0xff]) => Some("image/jpeg"),
        "image" if bytes.starts_with(b"GIF8") => Some("image/gif"),
        "image" if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" => Some("image/webp"),
        "audio" if bytes.starts_with(b"ID3") || (bytes.len() > 1 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0) => Some("audio/mpeg"),
        "audio" if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" => Some("audio/wav"),
        "audio" if bytes.starts_with(b"OggS") => Some("audio/ogg"),
        "audio" if bytes.starts_with(b"fLaC") => Some("audio/flac"),
        _ => None,
    }
}

#[async_trait]
impl ToolOutputPublisher for MediaOutputPublisher {
    async fn publish(
        &self, workspace: &Path, session_id: &str, tool: &str, result: &Value,
    ) -> Result<Vec<ToolOutputArtifact>, String> {
        let kind = match tool {
            "generate_image" => "image",
            "text_to_speech" => "audio",
            _ => return Err("unsupported media output tool".into()),
        };
        let relative = result.get("path").and_then(Value::as_str).ok_or("media output path missing")?;
        let relative_path = Path::new(relative);
        if relative_path.as_os_str().is_empty() || relative_path.components().any(|part| !matches!(part, Component::Normal(_))) {
            return Err("media output path must stay inside the workspace".into());
        }
        let root = std::fs::canonicalize(workspace).map_err(|error| error.to_string())?;
        let file_path = std::fs::canonicalize(root.join(relative_path)).map_err(|error| error.to_string())?;
        if !file_path.starts_with(&root) {
            return Err("media output escaped the workspace".into());
        }
        let file = tokio::fs::File::open(&file_path).await.map_err(|error| error.to_string())?;
        if file.metadata().await.map_err(|error| error.to_string())?.len() > MAX_MEDIA_BYTES {
            return Err("media output exceeds 32 MiB".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_MEDIA_BYTES + 1).read_to_end(&mut bytes).await.map_err(|error| error.to_string())?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_MEDIA_BYTES {
            return Err("media output is empty or exceeds 32 MiB".into());
        }
        if result.get("bytes").and_then(Value::as_u64).is_some_and(|expected| expected != bytes.len() as u64) {
            return Err("media output changed before publication".into());
        }
        let mime = media_type(&bytes, kind).ok_or("media output format is not supported")?;
        let cas = CasStore::new(self.cas_dir.clone())?;
        let hash = cas.put(&bytes)?;
        let id = format!("media-{}", uuid::Uuid::new_v4());
        let file_name = file_path.file_name().and_then(|name| name.to_str()).unwrap_or("media");
        let artifact: Artifact = serde_json::from_value(json!({
            "artifact_id": id, "kind": kind, "version": 1,
            "producer": session_id, "content_ref": format!("cas://sha256:{hash}"),
            "source_refs": [format!("session:{session_id}"), format!("tool:{tool}")],
            "classification": "private", "review_state": "draft",
            "created_at": chrono::Utc::now().to_rfc3339(),
            "format": "binary", "media_type": mime, "file_name": file_name,
            "sha256": hash, "size_bytes": bytes.len(),
        })).map_err(|error| error.to_string())?;
        let store = self.state.store().map_err(|error| format!("media artifact store unavailable: {}", error.1.0))?;
        store.save_artifact(&artifact, &format!("session:{session_id}")).await.map_err(|error| error.to_string())?;
        Ok(vec![ToolOutputArtifact {
            artifact_id: id, kind: kind.into(), mime_type: mime.into(),
            bytes: bytes.len() as u64, sha256: hash, path: relative.replace('\\', "/"),
        }])
    }
}
