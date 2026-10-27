//! Exact-connection image admission. Declarations do not claim model verification.
use super::ChatMessage;
use owo_agent_protocol::{CustomModelConnection, ModelInputCapabilityBinding};
use std::collections::HashSet;

pub const INPUT_CAPABILITIES_ENV: &str = "OWO_MODEL_INPUT_CAPABILITIES";
const MAX_BINDINGS: usize = 64;
// Stay below Windows per-variable UTF-16 limits, including JSON escaping.
const MAX_CONFIG_BYTES: usize = 24 * 1024;

fn identity(
    model: &str,
    base_url: &str,
    api_format: &str,
    full_url: bool,
) -> Result<(String, String, String, bool), String> {
    let model = model.trim();
    if model.is_empty()
        || model == "default"
        || model.len() > 200
        || model.chars().any(char::is_control)
    {
        return Err("input/capability_config_invalid: invalid model identity".into());
    }
    if base_url.len() > 2048 {
        return Err("input/capability_config_invalid: invalid endpoint identity".into());
    }
    let url = reqwest::Url::parse(base_url.trim())
        .map_err(|_| "input/capability_config_invalid: invalid endpoint identity".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("input/capability_config_invalid: invalid endpoint identity".into());
    }
    let format = match api_format.trim() {
        "" | "openai" => "openai",
        "anthropic" => "anthropic",
        _ => return Err("input/capability_config_invalid: unsupported protocol".into()),
    };
    // Full URLs preserve their path exactly; base URLs ignore trailing separators.
    let base = if full_url {
        url.to_string()
    } else {
        url.as_str().trim_end_matches('/').to_string()
    };
    Ok((model.to_owned(), base, format.to_owned(), full_url))
}

pub fn validate_input_capability_bindings(
    bindings: &[ModelInputCapabilityBinding],
) -> Result<(), String> {
    if bindings.len() > MAX_BINDINGS
        || serde_json::to_vec(bindings)
            .map_err(|_| "input/capability_config_invalid")?
            .len()
            > MAX_CONFIG_BYTES
    {
        return Err("input/capability_config_invalid: at most 64 bindings and 24 KiB".into());
    }
    let mut seen = HashSet::new();
    for binding in bindings {
        let key = identity(
            &binding.model,
            &binding.base_url,
            &binding.api_format,
            binding.use_full_url,
        )?;
        if !seen.insert(key) {
            return Err(
                "input/capability_config_invalid: duplicate connection/model declaration".into(),
            );
        }
    }
    Ok(())
}

pub fn input_capability_bindings_from_env() -> Result<Vec<ModelInputCapabilityBinding>, String> {
    match std::env::var(INPUT_CAPABILITIES_ENV) {
        Err(std::env::VarError::NotPresent) => Ok(Vec::new()),
        Err(_) => Err("input/capability_config_invalid: invalid environment encoding".into()),
        Ok(raw) => {
            if raw.len() > MAX_CONFIG_BYTES {
                return Err("input/capability_config_invalid: configuration exceeds 24 KiB".into());
            }
            let bindings: Vec<ModelInputCapabilityBinding> =
                serde_json::from_str(&raw).map_err(|_| {
                    "input/capability_config_invalid: expected declarations array".to_string()
                })?;
            validate_input_capability_bindings(&bindings)?;
            Ok(bindings)
        }
    }
}

pub fn declared_image_input(
    model: &str,
    base_url: &str,
    api_format: &str,
    full_url: bool,
    explicit: Option<&ModelInputCapabilityBinding>,
    bindings: &[ModelInputCapabilityBinding],
) -> Result<Option<bool>, String> {
    let actual = identity(model, base_url, api_format, full_url)?;
    if let Some(binding) = explicit {
        if identity(
            &binding.model,
            &binding.base_url,
            &binding.api_format,
            binding.use_full_url,
        )? != actual
        {
            return Err("input/capability_binding_mismatch: 图片声明与实际模型、地址或协议不匹配，请重新声明".into());
        }
        return Ok(Some(binding.image_input));
    }
    validate_input_capability_bindings(bindings)?;
    Ok(bindings
        .iter()
        .find(|b| {
            identity(&b.model, &b.base_url, &b.api_format, b.use_full_url)
                .ok()
                .as_ref()
                == Some(&actual)
        })
        .map(|b| b.image_input))
}

pub(crate) fn ensure_image_input(
    model: &str,
    base_url: &str,
    api_format: &str,
    options: Option<&CustomModelConnection>,
    messages: &[ChatMessage],
) -> Result<(), String> {
    // History also goes over the wire. Text-only calls do not need a declaration.
    if !messages.iter().any(|message| !message.images.is_empty()) {
        return Ok(());
    }
    let explicit = options.and_then(|o| o.input_capabilities.as_ref());
    let bindings = if explicit.is_none() {
        input_capability_bindings_from_env()?
    } else {
        Vec::new()
    };
    match declared_image_input(model, base_url, api_format, options.is_some_and(|o| o.use_full_url), explicit, &bindings)? {
        Some(true) => Ok(()),
        Some(false) => Err("input/image_not_supported: 此连接和模型已声明不支持图片输入，请切换支持图片的模型".into()),
        None => Err("input/image_capability_unknown: 此连接和模型的图片能力未知，请在模型配置中明确声明后再发送（声明不代表实测）".into()),
    }
}
