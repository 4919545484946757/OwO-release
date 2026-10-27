//! Resolve independent media connections once, for execution and capability views.
//! Environment credentials remain private; a configuration is never a service probe.
use serde_json::{json, Value};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MediaService {
    ImageGeneration,
    TextToSpeech,
}
impl MediaService {
    pub fn for_tool(name: &str) -> Option<Self> {
        match name {
            "generate_image" => Some(Self::ImageGeneration),
            "text_to_speech" => Some(Self::TextToSpeech),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::ImageGeneration => "image_generation",
            Self::TextToSpeech => "text_to_speech",
        }
    }
}

// Do not derive Debug or Serialize: this snapshot owns an ephemeral secret.
#[derive(Clone)]
pub struct MediaConnection {
    base_url: String,
    key: Option<String>,
    model: String,
    origin: String,
    local: bool,
    source: &'static str,
    credential_source: Option<String>,
    service: MediaService,
}
impl MediaConnection {
    pub fn from_env(service: MediaService) -> Result<Self, String> {
        Self::from_lookup(service, |name| std::env::var(name).ok())
    }
    pub fn from_lookup(
        service: MediaService,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let read = |name: &str| {
            lookup(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let independent_speech = service == MediaService::TextToSpeech
            && ["OWO_TTS_BASE_URL", "OWO_TTS_API_KEY", "OWO_TTS_API_KEY_ENV"]
                .iter()
                .any(|n| read(n).is_some());
        let base_names: &[&str] = if independent_speech {
            &["OWO_TTS_BASE_URL", "OPENAI_BASE_URL"]
        } else {
            &["OWO_IMAGE_BASE_URL", "OPENAI_BASE_URL"]
        };
        let base_source = base_names
            .iter()
            .find_map(|name| read(name).map(|value| (*name, value)));
        let base = base_source
            .as_ref()
            .map(|(_, value)| value.as_str())
            .unwrap_or("https://api.openai.com/v1");
        let url = reqwest::Url::parse(base)
            .map_err(|_| "media/invalid_endpoint: 必须提供完整 HTTP(S) URL".to_string())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("media/invalid_endpoint: HTTP(S) 端点不能含内嵌凭据、查询或片段".into());
        }
        let local = local_media_url(&url);
        let ref_name = if independent_speech {
            "OWO_TTS_API_KEY_ENV"
        } else {
            "OWO_IMAGE_API_KEY_ENV"
        };
        let key_names: &[&str] = if independent_speech {
            &["OWO_TTS_API_KEY"]
        } else {
            &["OWO_IMAGE_API_KEY", "OPENAI_API_KEY"]
        };
        let (key, credential_source) = if let Some(name) = read(ref_name) {
            if name.len() > 128
                || !name.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || i > 0 && b.is_ascii_digit()
                })
            {
                return Err("media/invalid_credential_reference: 凭据引用须是环境变量名".into());
            }
            // An explicit missing reference is an error, never a fallback to another provider's key.
            let key = read(&name).ok_or_else(|| {
                "media/credential_reference_missing: 指定的凭据环境变量未设置".to_string()
            })?;
            (Some(key), Some(name))
        } else {
            let selected = key_names
                .iter()
                .find_map(|name| read(name).map(|value| (value, (*name).to_string())));
            match selected {
                Some((key, name)) => (Some(key), Some(name)),
                None => (None, None),
            }
        };
        let (model_name, default_model) = match service {
            MediaService::ImageGeneration => ("OWO_IMAGE_MODEL", "gpt-image-1"),
            MediaService::TextToSpeech => ("OWO_TTS_MODEL", "tts-1"),
        };
        let model = read(model_name).unwrap_or_else(|| default_model.into());
        if model.len() > 256 || model.chars().any(char::is_control) {
            return Err("media/invalid_model: 模型名称格式或长度无效".into());
        }
        let source = if independent_speech {
            "tts_environment"
        } else if base_source
            .as_ref()
            .is_some_and(|(name, _)| *name == "OWO_IMAGE_BASE_URL")
        {
            "legacy_image_environment"
        } else {
            "shared_openai_environment"
        };
        Ok(Self {
            base_url: url.as_str().trim_end_matches('/').to_string(),
            key,
            model,
            origin: url.origin().ascii_serialization(),
            local,
            source,
            credential_source,
            service,
        })
    }
    pub(crate) fn with_named_profile(
        mut self,
        credential_source: Option<String>,
        key: Option<String>,
    ) -> Self {
        self.source = "named_profile";
        self.credential_source = credential_source;
        self.key = key;
        self
    }
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    pub fn api_key(&self) -> Option<&str> {
        self.key.as_deref()
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn configured(&self) -> bool {
        self.local || self.key.is_some()
    }
    pub fn require_execution_allowed(&self) -> Result<(), String> {
        self.require_configured()?;
        media_network_url(&self.base_url, media_cloud_enabled()).map(|_| ())
    }
    pub fn require_configured(&self) -> Result<(), String> {
        if self.configured() {
            return Ok(());
        }
        if self.source == "named_profile" {
            return Err("media/not_configured: 此命名连接未绑定可用的凭据环境变量，请更新连接档；不会借用其他连接的密钥".into());
        }
        Err(match self.service {
            MediaService::ImageGeneration => "media/not_configured: 文生图需要 OWO_IMAGE_API_KEY/OWO_IMAGE_API_KEY_ENV，或 OPENAI_API_KEY；本机端点可免密钥",
            MediaService::TextToSpeech => "media/not_configured: 独立 TTS 连接需要 OWO_TTS_API_KEY/OWO_TTS_API_KEY_ENV；未配置独立连接时兼容既有媒体连接，本机端点可免密钥",
        }.into())
    }
    pub fn status(&self) -> Value {
        json!({"service":self.service.name(),"status":if self.configured(){"configured"}else{"not_configured"},
            "configured":self.configured(),"connected":null,"verified":null,"last_verified_at":null,
            "endpoint_origin":self.origin,"model":self.model,"configuration_source":self.source,
            "credential_source":self.credential_source,"reason":self.require_configured().err()})
    }
}
pub fn media_cloud_enabled() -> bool {
    std::env::var("OWO_CLOUD_ENABLED")
        .ok()
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(true)
}
fn local_media_url(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
    host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
}
/// Also used for generated CDN URLs. Signed queries are allowed, but never echoed.
pub fn media_network_url(raw: &str, cloud_enabled: bool) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw)
        .map_err(|_| "media/invalid_endpoint: HTTP(S) URL 无效".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("media/invalid_endpoint: 只允许不含内嵌凭据的 HTTP(S) URL".into());
    }
    if !cloud_enabled && !local_media_url(&url) {
        return Err("network/cloud_disabled: OWO_CLOUD_ENABLED=false 已阻止云端媒体访问".into());
    }
    Ok(url)
}
pub fn media_connection_status(service: MediaService) -> Value {
    match MediaConnection::from_env(service) {
        Ok(connection) => connection.status(),
        Err(reason) => {
            json!({"service":service.name(),"status":"invalid_configuration","configured":false,
            "connected":null,"verified":null,"last_verified_at":null,"endpoint_origin":null,"model":null,
            "configuration_source":null,"credential_source":null,"reason":reason})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolve(service: MediaService, pairs: &[(&str, &str)]) -> Result<MediaConnection, String> {
        MediaConnection::from_lookup(service, |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, v)| (*v).to_string())
        })
    }
    #[test]
    fn media_connections_keep_image_and_speech_routes_keys_and_models_independent() {
        let vars = [
            ("OWO_IMAGE_BASE_URL", "https://image.example/v1"),
            ("OWO_IMAGE_API_KEY", "fixture-image-secret"),
            ("OWO_IMAGE_MODEL", "image-model"),
            ("OWO_TTS_BASE_URL", "https://speech.example/api"),
            ("OWO_TTS_API_KEY", "fixture-speech-secret"),
            ("OWO_TTS_MODEL", "speech-model"),
        ];
        let image = resolve(MediaService::ImageGeneration, &vars).unwrap();
        let speech = resolve(MediaService::TextToSpeech, &vars).unwrap();
        assert_eq!(image.base_url(), "https://image.example/v1");
        assert_eq!(speech.base_url(), "https://speech.example/api");
        assert_eq!(image.api_key(), Some("fixture-image-secret"));
        assert_eq!(speech.api_key(), Some("fixture-speech-secret"));
        assert_eq!(image.model(), "image-model");
        assert_eq!(speech.model(), "speech-model");
        assert!(image.configured() && speech.configured());
    }
    #[test]
    fn legacy_speech_remains_compatible_without_a_new_connection() {
        let speech = resolve(
            MediaService::TextToSpeech,
            &[
                ("OWO_IMAGE_BASE_URL", "http://127.0.0.1:9999/v1"),
                ("OWO_TTS_MODEL", "custom-speech"),
            ],
        )
        .unwrap();
        assert!(speech.configured());
        assert_eq!(speech.base_url(), "http://127.0.0.1:9999/v1");
        assert_eq!(speech.model(), "custom-speech");
        assert_eq!(
            speech.status()["configuration_source"],
            "legacy_image_environment"
        );
    }
    #[test]
    fn independent_speech_never_borrows_image_or_chat_credentials_implicitly() {
        let speech = resolve(
            MediaService::TextToSpeech,
            &[
                ("OWO_TTS_BASE_URL", "https://speech.example/v1"),
                ("OWO_IMAGE_API_KEY", "fixture-image-secret"),
                ("OPENAI_API_KEY", "fixture-chat-secret"),
            ],
        )
        .unwrap();
        assert!(!speech.configured());
        assert!(speech.require_configured().is_err());
        assert!(speech.api_key().is_none());
    }
    #[test]
    fn explicit_environment_reference_can_share_a_key_but_missing_reference_is_not_fallback() {
        let vars = [
            ("OWO_TTS_BASE_URL", "https://speech.example/v1"),
            ("OWO_TTS_API_KEY_ENV", "SHARED_MEDIA_KEY"),
            ("SHARED_MEDIA_KEY", "fixture-shared-secret"),
        ];
        assert_eq!(
            resolve(MediaService::TextToSpeech, &vars)
                .unwrap()
                .api_key(),
            Some("fixture-shared-secret")
        );
        assert!(resolve(
            MediaService::TextToSpeech,
            &[
                ("OWO_TTS_API_KEY_ENV", "MISSING"),
                ("OWO_TTS_API_KEY", "fixture-other-secret")
            ]
        )
        .is_err());
        assert!(resolve(
            MediaService::ImageGeneration,
            &[("OWO_IMAGE_API_KEY_ENV", "bad-key-name")]
        )
        .is_err());
    }
    #[test]
    fn status_contains_only_configuration_evidence_and_never_secrets_or_url_paths() {
        let c = resolve(
            MediaService::ImageGeneration,
            &[
                ("OWO_IMAGE_BASE_URL", "https://image.example/private/path"),
                ("OWO_IMAGE_API_KEY", "fixture-private-secret"),
            ],
        )
        .unwrap();
        let status = c.status();
        assert_eq!(status["status"], "configured");
        assert!(status["connected"].is_null());
        assert!(status["verified"].is_null());
        assert!(status["last_verified_at"].is_null());
        let text = status.to_string();
        assert!(!text.contains("fixture-private-secret"));
        assert!(!text.contains("/private/path"));
        assert_eq!(status["endpoint_origin"], "https://image.example");
    }
    #[test]
    fn only_parsed_loopback_hosts_allow_missing_credentials() {
        for endpoint in [
            "http://localhost:1234/v1",
            "http://[::1]:1234/v1",
            "http://127.0.0.2/v1",
        ] {
            assert!(resolve(
                MediaService::ImageGeneration,
                &[("OWO_IMAGE_BASE_URL", endpoint)]
            )
            .unwrap()
            .configured());
        }
        for endpoint in [
            "https://localhost.example/v1",
            "https://example.invalid/127.0.0.1",
        ] {
            assert!(!resolve(
                MediaService::ImageGeneration,
                &[("OWO_IMAGE_BASE_URL", endpoint)]
            )
            .unwrap()
            .configured());
        }
    }
    #[test]
    fn credential_bearing_or_non_http_urls_are_rejected_without_echoing_input() {
        for endpoint in [
            "https://user:fixture-secret@example.invalid/v1",
            "https://example.invalid/v1?key=fixture-secret",
            "https://example.invalid/v1#fixture-secret",
            "file:///tmp/x",
        ] {
            let error = resolve(
                MediaService::ImageGeneration,
                &[("OWO_IMAGE_BASE_URL", endpoint)],
            )
            .err()
            .unwrap();
            assert!(!error.contains("fixture-secret"));
        }
    }
    #[test]
    fn cloud_switch_guards_parsed_generation_download_and_redirect_endpoints() {
        for url in [
            "http://localhost:9000/v1",
            "http://[::1]:8000/image",
            "http://127.0.0.2/media?signature=fixture",
        ] {
            assert!(media_network_url(url, false).is_ok());
        }
        for url in [
            "https://cdn.example/image?signature=fixture-private",
            "http://localhost.example/v1",
            "https://example.com/127.0.0.1",
            "https://localhost@example.com/image",
        ] {
            let error = media_network_url(url, false).unwrap_err();
            assert!(!error.contains("fixture-private"));
        }
        assert!(media_network_url("https://cdn.example/image?signature=fixture", true).is_ok());
        assert!(media_network_url("file:///tmp/image", true).is_err());
    }
}
