//! Immutable uploaded inputs reuse the existing Artifact database and CAS.
//! Original filenames are display metadata; they are never the asset identity.
use super::ArtifactReviewState;
use crate::AppState;
use axum::http::StatusCode;
use base64::Engine as _;
use owo_agent_core::project_space_store::ProjectSpaceStoreBackend;
use owo_agent_core::{CasStore, MessageImage};
use owo_agent_protocol::{Artifact, AssetRef, ContentPart};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[path = "input_assets/custody.rs"]
mod custody;
pub(crate) use custody::{clone_parts_for_session, InputAssetContext, InputAssetHost};

type InputError = (StatusCode, String);
const MAX_FILE: usize = 24 * 1024 * 1024;
const MAX_IMAGE: usize = 5 * 1024 * 1024;
const MAX_TOTAL_IMAGES: usize = 20 * 1024 * 1024;
fn invalid(message: &str) -> InputError {
    (StatusCode::BAD_REQUEST, format!("input/{message}"))
}
fn storage(message: impl ToString) -> InputError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("input/storage: {}", message.to_string()),
    )
}
fn cas_dir(state: &impl InputAssetHost) -> PathBuf {
    state.input_data_root().join("workswarm").join("cas")
}
pub(crate) fn is_asset_id(id: &str) -> bool {
    id.strip_prefix("input-")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}
fn classify(
    name: &str,
    supplied: Option<&str>,
    bytes: &[u8],
) -> Result<(String, String), InputError> {
    let ext = Path::new(name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let expected = match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    };
    let supplied = supplied
        .filter(|mime| !mime.is_empty())
        .unwrap_or("application/octet-stream");
    if supplied.len() > 127 || !supplied.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid("invalid_mime"));
    }
    if let Some(expected) = expected {
        if bytes.is_empty() || bytes.len() > MAX_IMAGE {
            return Err(invalid("image_size_limit_5_mib"));
        }
        if image_mime(bytes) != Some(expected)
            || (supplied.starts_with("image/") && supplied != expected)
        {
            return Err(invalid("image_format_mismatch"));
        }
        return Ok(("image".into(), expected.into()));
    }
    if supplied.starts_with("image/") {
        return Err(invalid("unsupported_image_format"));
    }
    let kind = if supplied.starts_with("audio/") {
        "audio"
    } else if supplied.starts_with("video/") {
        "video"
    } else {
        "file"
    };
    Ok((kind.into(), supplied.into()))
}
fn store(
    state: &ArtifactReviewState,
) -> Result<std::sync::Arc<owo_agent_core::project_space_store::SqliteProjectSpaceStore>, InputError>
{
    state.store().map_err(|error| storage(error.1 .0))
}
fn uploaded_json(artifact: &Artifact) -> Value {
    json!({ "id": artifact.artifact_id, "name": artifact.file_name, "mime": artifact.media_type,
        "size": artifact.size_bytes, "kind": artifact.kind,
        "asset": {"artifact_id": artifact.artifact_id, "sha256": artifact.sha256},
        "content_part": {"type": artifact.kind, "asset": {"artifact_id": artifact.artifact_id, "sha256": artifact.sha256}},
        "content_url": format!("/artifacts/{}/content?raw=true", artifact.artifact_id) })
}
pub(crate) async fn publish(
    state: &impl InputAssetHost,
    session_id: &str,
    name: &str,
    mime: Option<&str>,
    bytes: &[u8],
) -> Result<Value, InputError> {
    publish_with_sources(state, session_id, name, mime, bytes, &[]).await
}
async fn publish_with_sources(
    state: &impl InputAssetHost,
    session_id: &str,
    name: &str,
    mime: Option<&str>,
    bytes: &[u8],
    sources: &[String],
) -> Result<Value, InputError> {
    publish_with_identity(state, session_id, name, mime, bytes, sources, None).await
}
async fn publish_with_identity(
    state: &impl InputAssetHost,
    session_id: &str,
    name: &str,
    mime: Option<&str>,
    bytes: &[u8],
    sources: &[String],
    identity: Option<String>,
) -> Result<Value, InputError> {
    if bytes.len() > MAX_FILE {
        return Err(invalid("file_size_limit_24_mib"));
    }
    let (kind, media_type) = classify(name, mime, bytes)?;
    let fixed_identity = identity.is_some();
    let hash = CasStore::new(cas_dir(state))
        .map_err(storage)?
        .put(bytes)
        .map_err(storage)?;
    let mut source_refs = vec![format!("session:{session_id}"), "input:upload".to_string()];
    source_refs.extend_from_slice(sources);
    let artifact: Artifact = serde_json::from_value(json!({
        "artifact_id": identity.unwrap_or_else(||format!("input-{}", uuid::Uuid::new_v4())), "kind": kind, "version": 1,
        "producer": session_id, "content_ref": format!("cas://sha256:{hash}"),
        "source_refs": source_refs,
        "classification": "private", "review_state": "draft", "created_at": chrono::Utc::now().to_rfc3339(),
        "format": "binary", "media_type": media_type, "file_name": name, "sha256": hash, "size_bytes": bytes.len()
    })).map_err(storage)?;
    let artifact_store = store(state.input_artifact_state())?;
    if fixed_identity {
        match artifact_store.get_artifact(&artifact.artifact_id).await {
            Ok(existing) => {
                if existing.producer != artifact.producer
                    || existing.source_refs != artifact.source_refs
                    || existing.sha256 != artifact.sha256
                    || existing.content_ref != artifact.content_ref
                    || existing.kind != artifact.kind
                    || existing.media_type != artifact.media_type
                    || existing.file_name != artifact.file_name
                    || existing.size_bytes != artifact.size_bytes
                    || existing.version != 1
                {
                    return Err(invalid("transfer_identity_conflict"));
                }
                return Ok(uploaded_json(&existing));
            }
            Err(owo_agent_core::project_space_store::ProjectSpaceStoreError::NotFound(_)) => {}
            Err(error) => return Err(self::storage(error)),
        }
    }
    artifact_store
        .save_artifact(&artifact, &format!("session:{session_id}"))
        .await
        .map_err(storage)?;
    Ok(uploaded_json(&artifact))
}
pub(crate) async fn list(
    state: &impl InputAssetHost,
    session_id: &str,
) -> Result<Vec<Value>, InputError> {
    let artifacts = store(state.input_artifact_state())?
        .list_artifacts_by_project(&format!("session:{session_id}"))
        .await
        .map_err(storage)?;
    Ok(artifacts
        .iter()
        .filter(|artifact| {
            artifact.producer == session_id
                && artifact.source_refs.iter().any(|r| r == "input:upload")
        })
        .map(uploaded_json)
        .collect())
}
async fn owned(
    state: &impl InputAssetHost,
    session_id: &str,
    id: &str,
) -> Result<Artifact, InputError> {
    if !is_asset_id(id) {
        return Err(invalid("invalid_asset_id"));
    }
    let artifact = store(state.input_artifact_state())?
        .get_artifact(id)
        .await
        .map_err(|error| match error {
            owo_agent_core::project_space_store::ProjectSpaceStoreError::NotFound(_) => {
                (StatusCode::NOT_FOUND, "input/asset_not_found".into())
            }
            other => storage(other),
        })?;
    if artifact.producer != session_id
        || !artifact.source_refs.iter().any(|r| r == "input:upload")
        || !artifact
            .source_refs
            .iter()
            .any(|r| r == &format!("session:{session_id}"))
    {
        return Err((StatusCode::FORBIDDEN, "input/asset_owner_mismatch".into()));
    }
    if artifact.version != 1
        || !valid_hash(&artifact.sha256)
        || artifact.content_ref != format!("cas://sha256:{}", artifact.sha256)
        || artifact.size_bytes > MAX_FILE as u64
    {
        return Err(invalid("invalid_asset_metadata"));
    }
    Ok(artifact)
}
pub(crate) async fn reference(
    state: &impl InputAssetHost,
    session_id: &str,
    id: &str,
) -> Result<ContentPart, InputError> {
    let artifact = owned(state, session_id, id).await?;
    let asset = AssetRef {
        artifact_id: artifact.artifact_id,
        sha256: artifact.sha256,
    };
    match artifact.kind.as_str() {
        "image" => Ok(ContentPart::Image { asset }),
        "audio" => Ok(ContentPart::Audio { asset }),
        "video" => Ok(ContentPart::Video { asset }),
        "file" => Ok(ContentPart::File { asset }),
        _ => Err(invalid("unsupported_asset_kind")),
    }
}
fn bytes(state: &impl InputAssetHost, artifact: &Artifact) -> Result<Vec<u8>, InputError> {
    let file = std::fs::File::open(cas_dir(state).join(&artifact.sha256))
        .map_err(|_| (StatusCode::NOT_FOUND, "input/asset_bytes_missing".into()))?;
    let mut bytes = Vec::new();
    file.take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(storage)?;
    if bytes.len() as u64 != artifact.size_bytes
        || format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256
    {
        return Err(invalid("asset_integrity_mismatch"));
    }
    Ok(bytes)
}
fn materialize(
    workspace: &Path,
    session_id: &str,
    artifact: &Artifact,
    bytes: &[u8],
) -> Result<PathBuf, InputError> {
    let root = workspace.canonicalize().map_err(storage)?;
    let dir = crate::session_api::attachment_dir(&root, session_id);
    std::fs::create_dir_all(&dir).map_err(storage)?;
    let dir = dir.canonicalize().map_err(storage)?;
    if !dir.starts_with(&root) {
        return Err(invalid("attachment_path_escaped"));
    }
    let name = crate::session_api::sanitize_attachment_name(&artifact.file_name)
        .ok_or_else(|| invalid("invalid_asset_name"))?;
    let path = dir.join(format!("{}-{name}", artifact.artifact_id));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file.write_all(bytes).map_err(storage)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let meta = std::fs::symlink_metadata(&path).map_err(storage)?;
            if meta.file_type().is_symlink() || !meta.is_file() || meta.len() != bytes.len() as u64
            {
                return Err(invalid("materialized_asset_changed"));
            }
            let mut current = Vec::new();
            std::fs::File::open(&path)
                .map_err(storage)?
                .take((MAX_FILE + 1) as u64)
                .read_to_end(&mut current)
                .map_err(storage)?;
            if current != bytes {
                return Err(invalid("materialized_asset_changed"));
            }
        }
        Err(error) => return Err(storage(error)),
    }
    Ok(path)
}
#[derive(Default)]
pub(crate) struct ResolvedInputs {
    pub content_parts: Vec<ContentPart>,
    pub images: Vec<MessageImage>,
    pub prompt_suffix: String,
}
pub(crate) async fn resolve(
    state: &impl InputAssetHost,
    session_id: &str,
    workspace: &Path,
    parts: &[ContentPart],
) -> Result<ResolvedInputs, InputError> {
    if parts.len() > 32 {
        return Err(invalid("too_many_content_parts"));
    }
    let mut result = ResolvedInputs {
        content_parts: parts.to_vec(),
        ..Default::default()
    };
    let mut seen = HashSet::new();
    let mut total_images = 0usize;
    for part in parts {
        let (asset, expected) = match part {
            ContentPart::Text { text } => {
                if text.len() > 32 * 1024 {
                    return Err(invalid("text_part_too_large"));
                }
                result.prompt_suffix.push('\n');
                result.prompt_suffix.push_str(text);
                continue;
            }
            ContentPart::Image { asset } => (asset, "image"),
            ContentPart::File { asset } => (asset, "file"),
            ContentPart::Audio { .. } | ContentPart::Video { .. } => {
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "input/audio_video_adapter_not_configured".into(),
                ))
            }
        };
        let artifact = owned(state, session_id, &asset.artifact_id).await?;
        if artifact.sha256 != asset.sha256 || artifact.kind != expected {
            return Err(invalid("asset_reference_mismatch"));
        }
        if !seen.insert(asset.artifact_id.clone()) {
            continue;
        }
        let bytes = bytes(state, &artifact)?;
        if expected == "image" {
            total_images = total_images.saturating_add(bytes.len());
            if result.images.len() >= 8
                || total_images > MAX_TOTAL_IMAGES
                || bytes.len() > MAX_IMAGE
                || image_mime(&bytes) != Some(artifact.media_type.as_str())
            {
                return Err(invalid("image_input_budget_or_format"));
            }
            result.images.push(MessageImage {
                url: format!(
                    "data:{};base64,{}",
                    artifact.media_type,
                    base64::engine::general_purpose::STANDARD.encode(&bytes)
                ),
            });
            result.prompt_suffix.push_str(&format!(
                "\n[图片附件 {}，{} 字节，资产 {}，已附到消息]",
                artifact.file_name,
                bytes.len(),
                asset.artifact_id
            ));
        } else {
            let path = materialize(workspace, session_id, &artifact, &bytes)?;
            result.prompt_suffix.push_str(&format!(
                "\n[文件附件 {}，{} 字节，资产 {}，工作区只读输入路径 {}]",
                artifact.file_name,
                bytes.len(),
                asset.artifact_id,
                path.display()
            ));
        }
    }
    Ok(result)
}

/// Rebuild native inputs from validated records, rather than trusting cached
/// URLs or mutable derived file paths. Legacy records remain explicitly marked.
pub(crate) async fn rehydrate_history(
    state: &impl InputAssetHost,
    session: &mut owo_agent_core::Session,
) -> Result<(), InputError> {
    for message in &mut session.messages {
        if message.role != "user" {
            continue;
        }
        let Some(input) = &message.input else {
            continue;
        };
        let resolved =
            resolve(state, &session.id, &session.workspace, &input.content_parts).await?;
        if input.legacy_attachments.is_empty() {
            message.content = Some(format!("{}{}", input.prompt, resolved.prompt_suffix));
            message.images = resolved.images;
        }
    }
    Ok(())
}

/// A fork is an explicit custody transfer: create child-owned metadata using
/// the same immutable CAS bytes. Never weaken the normal per-session checks.
pub(crate) async fn clone_history_for_fork(
    state: &impl InputAssetHost,
    source_session: &str,
    child: &mut owo_agent_core::Session,
) -> Result<(), InputError> {
    for message in &mut child.messages {
        let Some(input) = &mut message.input else {
            continue;
        };
        input.content_parts = clone_parts_for_session(
            state,
            source_session,
            &child.id,
            &input.content_parts,
            "input:fork",
        )
        .await?;
    }
    rehydrate_history(state, child).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use owo_agent_core::{
        gateway::ModelProvider, sqlite_store::SqliteSessionStore, tools::ToolRegistry, Agent,
        Policy,
    };
    use std::sync::Arc;
    struct Idle;
    #[async_trait]
    impl ModelProvider for Idle {
        async fn complete(
            &self,
            _: &[owo_agent_core::ChatMessage],
            _: &[owo_agent_core::ToolSpec],
        ) -> Result<owo_agent_core::ModelOutput, String> {
            Err("input tests must not invoke a model".into())
        }
    }
    fn state(temp: &Path) -> AppState {
        let workspace = temp.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        AppState::new(
            Agent::new(
                Arc::new(Idle),
                ToolRegistry::empty(),
                Policy::new(&workspace),
                Default::default(),
            ),
            SqliteSessionStore::open(&workspace.join("sessions.db")).unwrap(),
            workspace.join("traces"),
            temp.to_path_buf(),
            workspace,
        )
    }
    #[tokio::test]
    async fn same_name_uploads_are_immutable_and_survive_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let state = state(temp.path());
        let first = publish(&state, "session-a", "notes.txt", None, b"first")
            .await
            .unwrap();
        let second = publish(&state, "session-a", "notes.txt", None, b"second")
            .await
            .unwrap();
        assert_ne!(first["id"], second["id"]);
        assert_ne!(first["asset"]["sha256"], second["asset"]["sha256"]);
        drop(state);
        let reopened = self::state(temp.path());
        let all = list(&reopened, "session-a").await.unwrap();
        assert_eq!(all.len(), 2);
        for (uploaded, expected) in [
            (&first, b"first".as_slice()),
            (&second, b"second".as_slice()),
        ] {
            let artifact = owned(&reopened, "session-a", uploaded["id"].as_str().unwrap())
                .await
                .unwrap();
            assert_eq!(bytes(&reopened, &artifact).unwrap(), expected);
        }
        assert!(list(&reopened, "session-b").await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn input_references_reject_wrong_owner_digest_kind_and_corrupt_cas() {
        let temp = tempfile::tempdir().unwrap();
        let state = state(temp.path());
        let png = b"\x89PNG\r\n\x1a\nfixture-header";
        let uploaded = publish(&state, "a", "image.png", Some("image/png"), png)
            .await
            .unwrap();
        let asset: AssetRef = serde_json::from_value(uploaded["asset"].clone()).unwrap();
        assert_eq!(
            resolve(
                &state,
                "b",
                &state.workspace,
                &[ContentPart::Image {
                    asset: asset.clone()
                }]
            )
            .await
            .err()
            .unwrap()
            .0,
            StatusCode::FORBIDDEN
        );
        let mut changed = asset.clone();
        changed.sha256 = "0".repeat(64);
        assert!(resolve(
            &state,
            "a",
            &state.workspace,
            &[ContentPart::Image { asset: changed }]
        )
        .await
        .is_err());
        assert!(resolve(
            &state,
            "a",
            &state.workspace,
            &[ContentPart::File {
                asset: asset.clone()
            }]
        )
        .await
        .is_err());
        let images = resolve(
            &state,
            "a",
            &state.workspace,
            &[
                ContentPart::Image {
                    asset: asset.clone(),
                },
                ContentPart::Image {
                    asset: asset.clone(),
                },
            ],
        )
        .await
        .unwrap()
        .images;
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].url,
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png)
            )
        );
        std::fs::write(cas_dir(&state).join(&asset.sha256), b"tampered").unwrap();
        assert!(resolve(
            &state,
            "a",
            &state.workspace,
            &[ContentPart::Image { asset }]
        )
        .await
        .err()
        .unwrap()
        .1
        .contains("integrity"));
    }
    #[tokio::test]
    async fn materialized_files_do_not_follow_later_uploads_or_silent_edits() {
        let temp = tempfile::tempdir().unwrap();
        let state = state(temp.path());
        let upload = publish(&state, "a", "notes.txt", None, b"original")
            .await
            .unwrap();
        let part: ContentPart = serde_json::from_value(upload["content_part"].clone()).unwrap();
        resolve(&state, "a", &state.workspace, std::slice::from_ref(&part))
            .await
            .unwrap();
        publish(&state, "a", "notes.txt", None, b"new version")
            .await
            .unwrap();
        let artifact = owned(&state, "a", upload["id"].as_str().unwrap())
            .await
            .unwrap();
        let path = materialize(&state.workspace, "a", &artifact, b"original").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        std::fs::write(&path, b"modified").unwrap();
        assert!(resolve(&state, "a", &state.workspace, &[part])
            .await
            .err()
            .unwrap()
            .1
            .contains("materialized_asset_changed"));
    }
    #[tokio::test]
    async fn image_formats_budgets_and_unsupported_adapters_fail_explicitly() {
        let temp = tempfile::tempdir().unwrap();
        let state = state(temp.path());
        assert!(publish(&state, "a", "x.png", Some("image/png"), b"not png")
            .await
            .is_err());
        assert!(
            publish(&state, "a", "x.heic", Some("image/heic"), b"fixture")
                .await
                .is_err()
        );
        assert!(classify("x.png", None, &vec![0; MAX_IMAGE + 1]).is_err());
        let uploaded = publish(&state, "a", "x.wav", Some("audio/wav"), b"RIFFfixture")
            .await
            .unwrap();
        let part: ContentPart = serde_json::from_value(uploaded["content_part"].clone()).unwrap();
        assert_eq!(
            resolve(&state, "a", &state.workspace, &[part])
                .await
                .err()
                .unwrap()
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let parts = vec![ContentPart::Text { text: "x".into() }; 33];
        assert!(resolve(&state, "a", &state.workspace, &parts)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn worker_custody_is_idempotent_owned_and_rehydrates_native_history() {
        let temp = tempfile::tempdir().unwrap();
        let state = state(temp.path());
        let parent = state
            .store
            .create(&state.workspace, "fixture", None)
            .unwrap();
        let mut child = owo_agent_core::Session::new(&state.workspace, "fixture", None);
        child.id = "team-fixture-worker-child".into();
        child.parent_id = Some(parent.id.clone());
        let image=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==").unwrap();
        let original = publish(
            &state,
            &parent.id,
            "original.png",
            Some("image/png"),
            &image,
        )
        .await
        .unwrap();
        let part: ContentPart = serde_json::from_value(original["content_part"].clone()).unwrap();
        let context = InputAssetContext::for_state(&state);
        let parts = clone_parts_for_session(
            &context,
            &parent.id,
            &child.id,
            std::slice::from_ref(&part),
            "input:worker",
        )
        .await
        .unwrap();
        assert_eq!(
            parts,
            clone_parts_for_session(
                &context,
                &parent.id,
                &child.id,
                std::slice::from_ref(&part),
                "input:worker"
            )
            .await
            .unwrap()
        );
        assert_ne!(parts[0], part);
        let child_rows = list(&state, &child.id).await.unwrap();
        assert_eq!(child_rows.len(), 1);
        assert_eq!(
            child_rows[0]["asset"]["sha256"],
            original["asset"]["sha256"]
        );
        assert_eq!(
            resolve(
                &context,
                &child.id,
                &state.workspace,
                std::slice::from_ref(&part)
            )
            .await
            .err()
            .unwrap()
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            resolve(&context, &parent.id, &state.workspace, &parts)
                .await
                .err()
                .unwrap()
                .0,
            StatusCode::FORBIDDEN
        );
        assert!(clone_parts_for_session(
            &context,
            "wrong-owner",
            &child.id,
            &[part],
            "input:worker"
        )
        .await
        .is_err());
        let mut message = owo_agent_core::ChatMessage::user("cached derived suffix".into());
        message.input = Some(owo_agent_protocol::StoredUserInput {
            prompt: "inspect original pixels".into(),
            content_parts: parts.clone(),
            legacy_attachments: Vec::new(),
        });
        child.messages.push(message);
        state.store.save(&child).unwrap();
        let mut restored = state.store.load(&child.id).unwrap();
        rehydrate_history(&context, &mut restored).await.unwrap();
        assert_eq!(restored.messages[0].images.len(), 1);
        assert_eq!(
            restored.messages[0].images[0].url,
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&image)
            )
        );
        assert_eq!(
            restored.messages[0].input.as_ref().unwrap().content_parts,
            parts
        );
        assert!(restored.messages[0]
            .content
            .as_ref()
            .unwrap()
            .starts_with("inspect original pixels\n[图片附件"));
        let copied = owned(&context, &child.id, child_rows[0]["id"].as_str().unwrap())
            .await
            .unwrap();
        assert!(copied.source_refs.contains(&"input:worker".to_string()));
        assert!(copied
            .source_refs
            .contains(&format!("artifact:{}", original["id"].as_str().unwrap())));
        use tower::ServiceExt;
        let child_id = child.id.clone();
        let token = state.auth_token.token().to_string();
        let app = crate::build_router(Arc::new(state));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/session/{child_id}/attachments"))
                    .header("host", "127.0.0.1:4096")
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let rows: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(
            rows[0]["content_part"],
            serde_json::to_value(&parts[0]).unwrap()
        );
    }
}
