//! Explicit per-turn connections. No process environment mutation or cross-provider fallback.
use super::{ModelProvider, OpenAiCompatibleConfig, OpenAiCompatibleProvider};
use crate::anthropic::{AnthropicConfig, AnthropicProvider};
use owo_agent_protocol::CustomModelConnection;
use std::sync::Arc;

pub fn custom_model_provider(
    connection: &CustomModelConnection,
) -> Result<Arc<dyn ModelProvider>, String> {
    let url = reqwest::Url::parse(connection.base_url.trim())
        .map_err(|_| "model_connection/invalid_url".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("model_connection/invalid_url".to_string());
    }
    let model = connection.model.trim();
    if model.is_empty() || model == "default" {
        return Err("model_connection/invalid_model".to_string());
    }
    if connection
        .temperature
        .is_some_and(|v| !v.is_finite() || !(0.0..=2.0).contains(&v))
        || connection
            .timeout_secs
            .is_some_and(|v| !(1..=3600).contains(&v))
    {
        return Err("model_connection/invalid_parameters".to_string());
    }
    let format = connection.api_format.trim();
    if !matches!(format, "" | "openai" | "anthropic") {
        return Err("model_connection/unsupported_format".to_string());
    }
    let local = super::is_local_endpoint(url.as_str());
    let mut key = connection
        .api_key
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    if let Some(name) = connection.api_key_env.as_deref() {
        if !key.is_empty() {
            return Err("model_connection/ambiguous_credential".to_string());
        }
        let reference = name
            .strip_prefix("OWO_CUSTOM_MODEL_SECRET_")
            .ok_or_else(|| "model_connection/invalid_credential_reference".to_string())?;
        let (expected, metadata) = owo_agent_protocol::custom_credential_env_names(reference)
            .ok_or_else(|| "model_connection/invalid_credential_reference".to_string())?;
        if expected != name {
            return Err("model_connection/invalid_credential_reference".to_string());
        }
        let stored: serde_json::Value = std::env::var(metadata)
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
            .ok_or_else(|| {
                "provider/not_configured: credential reference is unavailable".to_string()
            })?;
        let protocol = if format.is_empty() { "openai" } else { format };
        if stored["model"].as_str() != Some(model)
            || stored["base_url"].as_str() != Some(url.as_str())
            || stored["api_format"].as_str() != Some(protocol)
        {
            return Err("model_connection/credential_binding_mismatch".to_string());
        }
        key = std::env::var(name).unwrap_or_default();
        if key.trim().is_empty() {
            return Err("provider/not_configured: credential reference is unavailable".to_string());
        }
    }
    // An absent custom credential may only inherit one for the same configured origin.
    // Never send a default service's credential to an unrelated custom host.
    if key.is_empty() && !local {
        let (base_env, key_env) = if format == "anthropic" {
            ("ANTHROPIC_BASE_URL", "ANTHROPIC_API_KEY")
        } else {
            ("OPENAI_BASE_URL", "OPENAI_API_KEY")
        };
        let same_origin = std::env::var(base_env)
            .ok()
            .and_then(|base| reqwest::Url::parse(&base).ok())
            .is_some_and(|base| base.origin() == url.origin());
        if same_origin {
            key = std::env::var(key_env).unwrap_or_default();
        }
        if key.trim().is_empty() {
            return Err(
                "provider/not_configured: custom endpoint requires its own API key".to_string(),
            );
        }
    }
    let cloud_enabled = std::env::var("OWO_CLOUD_ENABLED")
        .ok()
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(true);
    if format == "anthropic" {
        Ok(Arc::new(
            AnthropicProvider::new(AnthropicConfig {
                base_url: url.to_string(),
                api_key: key,
                model: model.to_string(),
                cloud_enabled,
            })?
            .with_connection_options(connection),
        ))
    } else {
        Ok(Arc::new(
            OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
                base_url: url.to_string(),
                api_key: key,
                model: model.to_string(),
                cloud_enabled,
            })?
            .with_connection_options(connection),
        ))
    }
}
