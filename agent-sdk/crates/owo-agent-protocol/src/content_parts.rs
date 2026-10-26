//! Shared input references. Bytes and ownership are resolved by the host, never by client paths.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRef {
    pub artifact_id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentPart {
    Text { text: String },
    Image { asset: AssetRef },
    Audio { asset: AssetRef },
    Video { asset: AssetRef },
    File { asset: AssetRef },
}

/// Host-validated input snapshot. Provider adapters keep using content/images;
/// clients use this record to display and replay the original user input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredUserInput {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_parts: Vec<ContentPart>,
    /// Legacy filename inputs have no immutable custody proof. Clients must not
    /// silently replay them as text; re-upload is required until migrated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_attachments: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stored_user_input_roundtrips_exact_refs_and_defaults_older_records() {
        let raw = serde_json::json!({"prompt":"original", "content_parts":[{"type":"image","asset":{"artifact_id":"input-original","sha256":"a".repeat(64)}}]});
        let input: StoredUserInput = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(&input).unwrap(), raw);
        assert!(input.legacy_attachments.is_empty());
        let old: StoredUserInput =
            serde_json::from_value(serde_json::json!({"prompt":"text"})).unwrap();
        assert!(old.content_parts.is_empty());
        assert!(old.legacy_attachments.is_empty());
    }
}
