//! Explicit session-to-session custody transfer using the existing Artifact store and CAS.
use super::*;
/// Minimal host state for workers; does not retain an AppState/worker registry cycle.
#[derive(Clone)]
pub(crate) struct InputAssetContext {
    data_root: PathBuf,
    artifact_review: ArtifactReviewState,
}
pub(crate) trait InputAssetHost: Send + Sync {
    fn input_data_root(&self) -> &Path;
    fn input_artifact_state(&self) -> &ArtifactReviewState;
}
impl InputAssetHost for AppState {
    fn input_data_root(&self) -> &Path {
        &self.data_root
    }
    fn input_artifact_state(&self) -> &ArtifactReviewState {
        &self.artifact_review
    }
}
impl<T: InputAssetHost + ?Sized> InputAssetHost for std::sync::Arc<T> {
    fn input_data_root(&self) -> &Path {
        self.as_ref().input_data_root()
    }
    fn input_artifact_state(&self) -> &ArtifactReviewState {
        self.as_ref().input_artifact_state()
    }
}
impl InputAssetContext {
    pub(crate) fn for_state(state: &AppState) -> Self {
        Self {
            data_root: state.data_root.clone(),
            artifact_review: state.artifact_review.clone(),
        }
    }
}
impl InputAssetHost for InputAssetContext {
    fn input_data_root(&self) -> &Path {
        &self.data_root
    }
    fn input_artifact_state(&self) -> &ArtifactReviewState {
        &self.artifact_review
    }
}
/// Repeated delivery to the same child produces the same child-owned identity.
/// Parent refs remain unauthorized in the child; provenance is metadata only.
pub(crate) async fn clone_parts_for_session(
    state: &impl InputAssetHost,
    source: &str,
    target: &str,
    parts: &[ContentPart],
    reason: &str,
) -> Result<Vec<ContentPart>, InputError> {
    if parts.len() > 32 || !matches!(reason, "input:fork" | "input:worker") {
        return Err(invalid("invalid_custody_transfer"));
    }
    let mut transferred = Vec::with_capacity(parts.len());
    for part in parts {
        let (asset, kind) = match part {
            ContentPart::Text { text } => {
                if text.len() > 32 * 1024 {
                    return Err(invalid("text_part_too_large"));
                }
                transferred.push(part.clone());
                continue;
            }
            ContentPart::Image { asset } => (asset, "image"),
            ContentPart::File { asset } => (asset, "file"),
            _ => return Err(invalid("unsupported_custody_input")),
        };
        let original = owned(state, source, &asset.artifact_id).await?;
        if original.sha256 != asset.sha256 || original.kind != kind {
            return Err(invalid("asset_reference_mismatch"));
        }
        let data = bytes(state, &original)?;
        let key = serde_json::to_vec(&(
            "session-input-custody-v1",
            reason,
            source,
            target,
            &asset.artifact_id,
            &asset.sha256,
        ))
        .map_err(storage)?;
        let hash = Sha256::digest(&key);
        let id = format!(
            "input-{}",
            uuid::Uuid::from_slice(&hash[..16]).map_err(storage)?
        );
        let sources = [
            reason.to_string(),
            format!("session:{source}"),
            format!("artifact:{}", asset.artifact_id),
        ];
        let uploaded = publish_with_identity(
            state,
            target,
            &original.file_name,
            Some(&original.media_type),
            &data,
            &sources,
            Some(id),
        )
        .await?;
        transferred
            .push(serde_json::from_value(uploaded["content_part"].clone()).map_err(storage)?);
    }
    Ok(transferred)
}
