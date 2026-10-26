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
impl ToolOutputPublisher for MediaOutputPublisher {
    async fn publish(
        &self,
        workspace: &Path,
        session_id: &str,
        tool: &str,
        result: &Value,
    ) -> Result<Vec<ToolOutputArtifact>, String> {
        let kind = match tool {
            "generate_image" => "image",
            "text_to_speech" => "audio",
            _ => return Err("unsupported media output tool".into()),
        };
        let relative = result
            .get("path")
            .and_then(Value::as_str)
            .ok_or("media output path missing")?;
        let relative_path = Path::new(relative);
        if relative_path.as_os_str().is_empty()
            || relative_path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("media output path must stay inside the workspace".into());
        }
        let root = std::fs::canonicalize(workspace).map_err(|error| error.to_string())?;
        let file_path =
            std::fs::canonicalize(root.join(relative_path)).map_err(|error| error.to_string())?;
        if !file_path.starts_with(&root) {
            return Err("media output escaped the workspace".into());
        }
        let file = tokio::fs::File::open(&file_path)
            .await
            .map_err(|error| error.to_string())?;
        if file
            .metadata()
            .await
            .map_err(|error| error.to_string())?
            .len()
            > MAX_MEDIA_BYTES
        {
            return Err("media output exceeds 32 MiB".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_MEDIA_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| error.to_string())?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_MEDIA_BYTES {
            return Err("media output is empty or exceeds 32 MiB".into());
        }
        if result
            .get("bytes")
            .and_then(Value::as_u64)
            .is_some_and(|expected| expected != bytes.len() as u64)
        {
            return Err("media output changed before publication".into());
        }
        let mime = media_type(&bytes, kind).ok_or("media output format is not supported")?;
        let cas = CasStore::new(self.cas_dir.clone())?;
        let hash = cas.put(&bytes)?;
        let id = format!("media-{}", uuid::Uuid::new_v4());
        let file_name = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("media");
        let artifact: Artifact = serde_json::from_value(json!({
            "artifact_id": id, "kind": kind, "version": 1,
            "producer": session_id, "content_ref": format!("cas://sha256:{hash}"),
            "source_refs": [format!("session:{session_id}"), format!("tool:{tool}")],
            "classification": "private", "review_state": "draft",
            "created_at": chrono::Utc::now().to_rfc3339(),
            "format": "binary", "media_type": mime, "file_name": file_name,
            "sha256": hash, "size_bytes": bytes.len(),
        }))
        .map_err(|error| error.to_string())?;
        let store = self
            .state
            .store()
            .map_err(|error| format!("media artifact store unavailable: {}", error.1 .0))?;
        store
            .save_artifact(&artifact, &format!("session:{session_id}"))
            .await
            .map_err(|error| error.to_string())?;
        Ok(vec![ToolOutputArtifact {
            artifact_id: id,
            kind: kind.into(),
            mime_type: mime.into(),
            bytes: bytes.len() as u64,
            sha256: hash,
            path: relative.replace('\\', "/"),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use owo_agent_core::{
        gateway::ModelProvider, sqlite_store::SqliteSessionStore, tools::ToolRegistry, Agent,
        Policy,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    struct Idle;
    #[async_trait]
    impl ModelProvider for Idle {
        async fn complete(
            &self,
            _: &[owo_agent_core::ChatMessage],
            _: &[owo_agent_core::ToolSpec],
        ) -> Result<owo_agent_core::ModelOutput, String> {
            Err("media persistence must not invoke a model".into())
        }
    }

    #[tokio::test]
    async fn media_artifacts_survive_reopen_and_authenticated_binary_delivery_at_all_sizes() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(workspace.join("outputs")).unwrap();
        let data = temp.path().join("data");
        let root = data.join("workswarm");
        let publisher = MediaOutputPublisher::new(
            ArtifactReviewState::new(root.join("space.db")),
            root.join("cas"),
        );
        let agent = Agent::new(
            Arc::new(Idle),
            ToolRegistry::empty(),
            Policy::new(&workspace),
            Default::default(),
        );
        let store = SqliteSessionStore::open(&workspace.join("sessions.db")).unwrap();
        let state = Arc::new(crate::AppState::new(
            agent,
            store,
            workspace.join("traces"),
            data,
            workspace.clone(),
        ));
        let app = crate::build_router(Arc::clone(&state));
        for (tool, prefix, mime) in [
            (
                "generate_image",
                b"\x89PNG\r\n\x1a\n".as_slice(),
                "image/png",
            ),
            ("text_to_speech", b"ID3".as_slice(), "audio/mpeg"),
        ] {
            for size in [2048, 100 * 1024, 1024 * 1024] {
                let mut bytes = vec![42; size];
                bytes[..prefix.len()].copy_from_slice(prefix);
                let path = format!("outputs/{tool}-{size}");
                std::fs::write(workspace.join(&path), &bytes).unwrap();
                let outputs = publisher
                    .publish(
                        &workspace,
                        "test-session",
                        tool,
                        &json!({"path":path,"bytes":size}),
                    )
                    .await
                    .unwrap();
                let output = &outputs[0];
                let reopened = ArtifactReviewState::new(root.join("space.db"));
                let artifact = reopened
                    .store()
                    .unwrap()
                    .get_artifact(&output.artifact_id)
                    .await
                    .unwrap();
                assert_eq!(artifact.sha256, CasStore::hash_of(&bytes));
                assert_eq!(artifact.media_type, mime);
                assert_eq!(artifact.producer, "test-session");
                let uri = format!("/artifacts/{}/content?raw=true", output.artifact_id);
                let response = app
                    .clone()
                    .oneshot(
                        axum::http::Request::builder()
                            .uri(&uri)
                            .header(
                                "Authorization",
                                format!("Bearer {}", state.auth_token.token()),
                            )
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), axum::http::StatusCode::OK);
                assert_eq!(response.headers()["content-type"], mime);
                assert_eq!(response.headers()["accept-ranges"], "bytes");
                let body = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
                    .await
                    .unwrap();
                assert_eq!(body.as_ref(), bytes.as_slice());
                let denied = app
                    .clone()
                    .oneshot(
                        axum::http::Request::builder()
                            .uri(&uri)
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(denied.status(), axum::http::StatusCode::UNAUTHORIZED);
            }
        }
    }

    #[tokio::test]
    async fn media_publication_rejects_paths_changed_sizes_formats_and_missing_files() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("invalid"), b"<html>not media</html>").unwrap();
        std::fs::write(workspace.join("image"), b"\x89PNG\r\n\x1a\n").unwrap();
        let publisher = MediaOutputPublisher::new(
            ArtifactReviewState::new(temp.path().join("space.db")),
            temp.path().join("cas"),
        );
        for args in [
            json!({"path":"../secret"}),
            json!({"path":"missing"}),
            json!({"path":"invalid"}),
            json!({"path":"image","bytes":999}),
        ] {
            assert!(publisher
                .publish(&workspace, "session", "generate_image", &args)
                .await
                .is_err());
        }
        assert!(publisher
            .publish(&workspace, "session", "unknown", &json!({"path":"image"}))
            .await
            .is_err());
        assert!(
            !temp.path().join("space.db").exists(),
            "invalid output must not register an artifact"
        );
    }
}
