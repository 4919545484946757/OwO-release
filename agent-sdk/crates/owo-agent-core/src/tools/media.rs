//! 媒体生成工具：文生图（`generate_image`）与语音合成（`text_to_speech`）。
//!
//! 两者都走 OpenAI 兼容的多模态端点，产物默认写进工作区 `outputs/`，显式目录受任务写范围约束，
//! 便于审计与后续回合引用。凭据只从环境变量读取，绝不进参数、日志或配置。

use super::{media_target::MediaTarget, web::append_capped, Tool, ToolContext, ToolSpec};
use crate::gateway::media_config::{MediaConnection, MediaService};
use async_trait::async_trait;
use base64::Engine as _;
use futures_util::StreamExt;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

/// 单个媒体产物上限：32 MiB。超过即判定异常，避免磁盘被大文件写满。
const MAX_MEDIA_BYTES: usize = 32 * 1024 * 1024;
const MAX_IMAGE_JSON_BYTES: usize = MAX_MEDIA_BYTES.div_ceil(3) * 4 + 64 * 1024;
const MAX_ERROR_BYTES: usize = 16 * 1024;

// Trusted runtime context, never supplied in model arguments. The wrapper scopes
// every poll of the proxied tool future, including its publisher and live registry.
// Builtin media tools do not spawn detached tasks; concurrent workers stay isolated.
tokio::task_local! { static MEDIA_WRITE_SCOPE: Vec<PathBuf>; }
struct ScopedMediaTool {
    inner: Arc<dyn Tool>,
    allowed: Vec<PathBuf>,
}
#[async_trait]
impl Tool for ScopedMediaTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        MEDIA_WRITE_SCOPE
            .scope(self.allowed.clone(), self.inner.run(ctx, args))
            .await
    }
}
pub(crate) fn scoped_workflow_tool(inner: Arc<dyn Tool>, allowed: Vec<PathBuf>) -> Arc<dyn Tool> {
    if matches!(
        inner.spec().name.as_str(),
        "generate_image" | "text_to_speech"
    ) {
        Arc::new(ScopedMediaTool { inner, allowed })
    } else {
        inner
    }
}
tokio::task_local! { static MEDIA_CONNECTION: MediaConnection; }
struct ProfiledMediaTool {
    inner: Arc<dyn Tool>,
    store: crate::gateway::media_profiles::MediaProfileStore,
    service: MediaService,
}
#[async_trait]
impl Tool for ProfiledMediaTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        cancelled(ctx.abort)?;
        let store = self.store.clone();
        let service = self.service;
        let connection = cancellable(
            ctx.abort,
            tokio::task::spawn_blocking(move || store.resolve(service)),
        )
        .await?
        .map_err(|_| "media_profiles/storage_error: 连接读取任务失败".to_string())??;
        connection.require_execution_allowed()?;
        MEDIA_CONNECTION
            .scope(connection, self.inner.run(ctx, args))
            .await
    }
}
pub(crate) fn profiled_media_tool(inner: Arc<dyn Tool>, data_root: &Path) -> Arc<dyn Tool> {
    match MediaService::for_tool(&inner.spec().name) {
        Some(service) => Arc::new(ProfiledMediaTool {
            inner,
            store: crate::gateway::media_profiles::MediaProfileStore::new(data_root),
            service,
        }),
        None => inner,
    }
}
fn execution_connection(service: MediaService) -> Result<MediaConnection, String> {
    MEDIA_CONNECTION
        .try_with(Clone::clone)
        .or_else(|_| MediaConnection::from_env(service))
}
fn cancelled(abort: Option<&AtomicBool>) -> Result<(), String> {
    if abort.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err("executor/cancelled: media operation".into())
    } else {
        Ok(())
    }
}
async fn cancellable<F: std::future::Future>(
    abort: Option<&AtomicBool>,
    future: F,
) -> Result<F::Output, String> {
    tokio::pin!(future);
    loop {
        cancelled(abort)?;
        tokio::select! {
            result = &mut future => { cancelled(abort)?; return Ok(result); }
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}
async fn read_capped_response(
    response: reqwest::Response,
    limit: usize,
    abort: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    cancelled(abort)?;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("media/response_too_large: limit {limit} bytes"));
    }
    let mut buffer = Vec::with_capacity(limit.min(64 * 1024));
    let mut stream = response.bytes_stream();
    while let Some(chunk) = cancellable(abort, stream.next()).await? {
        let chunk = chunk.map_err(|error| format!("media/response_read: {error}"))?;
        if append_capped(&mut buffer, &chunk, limit) {
            return Err(format!("media/response_too_large: limit {limit} bytes"));
        }
    }
    Ok(buffer)
}
fn prepare_target(ctx: &ToolContext<'_>, args: &Value, ext: &str) -> Result<MediaTarget, String> {
    cancelled(ctx.abort)?;
    let path = output_path(
        ctx.workspace,
        args.get("output").and_then(Value::as_str),
        ext,
    )?;
    let relative = path
        .strip_prefix(ctx.workspace)
        .map_err(|e| e.to_string())?;
    MEDIA_WRITE_SCOPE
        .try_with(|allowed| MediaTarget::prepare(ctx.workspace, relative, Some(allowed)))
        .unwrap_or_else(|_| MediaTarget::prepare(ctx.workspace, relative, None))
}

/// 图片下载的独立超时：正常的图片 CDN 几秒内返回，用 180s 的整体超时等一个
/// 卡住的下载只会白白占住回合。生成请求本身仍用 180s（大图生成确实慢）。
const DOWNLOAD_TIMEOUT_SECS: u64 = 60;

/// 把用户给的名字压成安全文件名：只留字母数字、`-`、`_`；其余替换为 `_`。
fn sanitize_stem(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars().take(64) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "media".to_string()
    } else {
        trimmed
    }
}

/// Explicit directories are preserved. Bare names retain the outputs/ default.
fn output_path(workspace: &Path, requested: Option<&str>, ext: &str) -> Result<PathBuf, String> {
    let requested = requested.map(str::trim).filter(|value| !value.is_empty());
    let Some(raw) = requested else {
        return Ok(workspace.join("outputs").join(format!(
            "media-{}.{}",
            uuid::Uuid::new_v4(),
            ext
        )));
    };
    // Reject Windows ADS, device names, alternate separators and traversal on all hosts.
    let raw = raw.replace('\\', "/");
    let candidate = Path::new(&raw);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || raw.split('/').any(|part| {
            let stem = part
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || part
                    .chars()
                    .any(|ch| ch.is_control() || ":<>\"|?*".contains(ch))
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
    {
        return Err("media/output_path: invalid workspace-relative path".into());
    }
    let stem = candidate
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("media");
    let name = format!("{}.{}", sanitize_stem(stem), ext);
    let parent = candidate
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("outputs"));
    Ok(workspace.join(parent).join(name))
}

/// 端点返回的图片载荷：base64 直给，或需要二次下载的 URL。
#[derive(Debug, PartialEq, Eq)]
enum ImagePayload {
    Base64(Vec<u8>),
    Url(String),
}

/// 解析 `data[0]`：优先 `b64_json`，其次 `url`，都没有就是协议错误。
fn parse_image_item(item: &serde_json::Value) -> Result<ImagePayload, String> {
    if let Some(encoded) = item.get("b64_json").and_then(serde_json::Value::as_str) {
        let encoded = encoded.trim();
        if encoded.len() > MAX_MEDIA_BYTES.div_ceil(3) * 4 {
            return Err("media/response_too_large: base64 image".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("图片 base64 解码失败：{error}"))?;
        if bytes.len() > MAX_MEDIA_BYTES {
            return Err("media/response_too_large: decoded image".into());
        }
        return Ok(ImagePayload::Base64(bytes));
    }
    if let Some(url) = item.get("url").and_then(serde_json::Value::as_str) {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err("端点返回了空图片 URL".to_string());
        }
        return Ok(ImagePayload::Url(trimmed.to_string()));
    }
    Err("端点既未返回 b64_json 也未返回 url".to_string())
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| format!("HTTP 客户端不可用：{error}"))
}

/// 小图内联预览：512 KiB 以内的图片返回 `data:` URL，前端可直接显示；
/// 大图只给路径，避免把会话事件撑爆。
fn inline_data_url(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 512 * 1024 {
        return None;
    }
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return None;
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

/// 小段音频内联：512 KiB 以内返回 `data:audio/...`，前端可直接 `<audio controls>` 试听。
fn inline_audio_data_url(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 512 * 1024 {
        return None;
    }
    let mime = if bytes.starts_with(b"ID3")
        || (bytes.len() > 1 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
    {
        "audio/mpeg"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        "audio/wav"
    } else if bytes.starts_with(b"OggS") {
        "audio/ogg"
    } else if bytes.starts_with(b"fLaC") {
        "audio/flac"
    } else {
        return None;
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

/// 下载远端图片：流式读取并封顶。
///
/// 直接 `response.bytes()` 会先把整个响应体读进内存再检查长度，端点返回超大文件时
/// 会先把进程内存吃满；这里按块读取，超过上限立即失败，并优先用 Content-Length 提前拒绝。
async fn download_image_bytes(
    client: &reqwest::Client,
    url: &str,
    abort: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    use crate::gateway::media_config::{media_cloud_enabled, media_network_url};
    let mut target = media_network_url(url, media_cloud_enabled())?;
    for hop in 0..=3 {
        let response = cancellable(
            abort,
            client
                .get(target.clone())
                .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
                .send(),
        )
        .await?
        .map_err(|error| format!("media/download_failed: {}", error.without_url()))?;
        if response.status().is_redirection() {
            if hop == 3 {
                return Err("media/redirect_limit: 图片下载重定向过多".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("media/invalid_redirect: 缺少合法 Location")?;
            let next = target
                .join(location)
                .map_err(|_| "media/invalid_redirect: Location 无效")?;
            if target.scheme() == "https" && next.scheme() != "https" {
                return Err("media/invalid_redirect: 不允许 HTTPS 降级".into());
            }
            target = media_network_url(next.as_str(), media_cloud_enabled())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("media/download_failed: HTTP {}", response.status()));
        }
        return read_capped_response(response, MAX_MEDIA_BYTES, abort).await;
    }
    Err("media/redirect_limit".into())
}
fn write_media(
    ctx: &ToolContext<'_>,
    target: &MediaTarget,
    bytes: &[u8],
) -> Result<String, String> {
    cancelled(ctx.abort)?;
    if bytes.is_empty() || bytes.len() > MAX_MEDIA_BYTES {
        return Err(format!("media/invalid_size: {} bytes", bytes.len()));
    }
    target.write_new(bytes)
}

/// 文生图请求体：`dall-e-*` 才要求 base64 回传（新版 gpt-image 默认就回 b64）。
fn image_request_body(model: &str, prompt: &str, size: &str) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "n": 1,
        "size": size,
    });
    if model.starts_with("dall-e") {
        body["response_format"] = serde_json::json!("b64_json");
    }
    body
}

/// 语音合成请求体。
fn speech_request_body(model: &str, text: &str, voice: &str, format: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "input": text,
        "voice": voice,
        "response_format": format,
    })
}

/// 从错误响应里取一句人能看懂的原因（OpenAI 兼容格式 `{"error":{"message":...}}`）。
fn media_error_message(payload: &serde_json::Value) -> String {
    payload
        .get("error")
        .and_then(|value| value.get("message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("未知错误")
        .chars()
        .take(200)
        .collect()
}

/// `generate_image`：调用 OpenAI 兼容的 `/images/generations`，把图片存进 `outputs/`。
pub(super) struct GenerateImageTool;

#[async_trait]
impl Tool for GenerateImageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "generate_image".into(),
            description: "文生图：按提示词生成图片并保存到工作区（默认 outputs/），显式 output 目录必须在任务写范围内，已有文件拒绝覆盖。\
                          走 OpenAI 兼容 /images/generations；端点用 OWO_IMAGE_BASE_URL 或 \
                          OPENAI_BASE_URL，密钥用 OWO_IMAGE_API_KEY、OWO_IMAGE_API_KEY_ENV 或 OPENAI_API_KEY"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "图片描述（越具体越好）" },
                    "model": { "type": "string", "description": "图像模型，缺省使用 OWO_IMAGE_MODEL 或 gpt-image-1" },
                    "size": { "type": "string", "description": "尺寸，如 1024x1024" },
                    "output": { "type": "string", "description": "工作区相对路径，如 images/hero.png；裸文件名默认 outputs/，已有文件拒绝覆盖（可省）" }
                },
                "required": ["prompt"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let prompt = required_string(&args, "prompt")?;
        let target = prepare_target(ctx, &args, "png")?;
        let connection = execution_connection(MediaService::ImageGeneration)?;
        connection.require_execution_allowed()?;
        let base = connection.base_url();
        let key = connection.api_key();
        let model = args
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(connection.model())
            .to_string();
        let size = args
            .get("size")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("1024x1024")
            .to_string();
        let body = image_request_body(&model, &prompt, &size);
        let client = build_client()?;
        let endpoint = format!("{base}/images/generations");
        let mut request = client.post(&endpoint).json(&body);
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = cancellable(ctx.abort, request.send())
            .await?
            .map_err(|error| format!("文生图请求失败：{}", error.without_url()))?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_IMAGE_JSON_BYTES
        } else {
            MAX_ERROR_BYTES
        };
        let raw = read_capped_response(response, limit, ctx.abort).await?;
        let payload: Value =
            serde_json::from_slice(&raw).map_err(|error| format!("端点返回非 JSON：{error}"))?;
        if !status.is_success() {
            return Err(format!(
                "文生图失败（HTTP {status}）：{}",
                media_error_message(&payload)
            ));
        }
        let item = payload
            .get("data")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| "端点未返回图片数据".to_string())?;
        let (bytes, note) = match parse_image_item(item)? {
            ImagePayload::Base64(bytes) => (bytes, "b64_json".to_string()),
            ImagePayload::Url(url) => (
                download_image_bytes(&client, &url, ctx.abort).await?,
                "url".to_string(),
            ),
        };
        let saved = write_media(ctx, &target, &bytes)?;
        let mut result = json!({
            "path": saved,
            "bytes": bytes.len(),
            "model": model,
            "source": note,
        });
        if let Some(data_url) = inline_data_url(&bytes) {
            result["data_url"] = json!(data_url);
        }
        Ok(result)
    }
}

/// `text_to_speech`：调用 `/audio/speech`，把音频存进 `outputs/`。
pub(super) struct TextToSpeechTool;

#[async_trait]
impl Tool for TextToSpeechTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "text_to_speech".into(),
            description: "语音合成：走 OpenAI 兼容 /audio/speech；独立连接使用 OWO_TTS_BASE_URL、OWO_TTS_API_KEY 或 OWO_TTS_API_KEY_ENV，模型缺省 OWO_TTS_MODEL/tts-1。未配置独立连接时兼容旧媒体配置。保存到工作区 outputs/（显式目录受任务写范围约束），已有文件拒绝覆盖。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "要朗读的文本" },
                    "model": { "type": "string", "description": "语音模型，缺省使用 OWO_TTS_MODEL 或 tts-1" },
                    "voice": { "type": "string", "description": "音色，默认 alloy" },
                    "format": { "type": "string", "description": "mp3 / wav / opus，默认 mp3" },
                    "output": { "type": "string", "description": "工作区相对路径，如 images/hero.png；裸文件名默认 outputs/，已有文件拒绝覆盖（可省）" }
                },
                "required": ["text"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let text = required_string(&args, "text")?;
        if text.len() > 8_000 {
            return Err("文本超过 8000 字符上限，请分段合成".to_string());
        }
        let connection = execution_connection(MediaService::TextToSpeech)?;
        connection.require_execution_allowed()?;
        let base = connection.base_url();
        let key = connection.api_key();
        let model = args
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(connection.model())
            .to_string();
        let voice = args
            .get("voice")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("alloy")
            .to_string();
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| matches!(*value, "mp3" | "wav" | "opus" | "aac" | "flac"))
            .unwrap_or("mp3")
            .to_string();
        let target = prepare_target(ctx, &args, &format)?;
        let client = build_client()?;
        let endpoint = format!("{base}/audio/speech");
        let mut request = client
            .post(&endpoint)
            .json(&speech_request_body(&model, &text, &voice, &format));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = cancellable(ctx.abort, request.send())
            .await?
            .map_err(|error| format!("语音合成请求失败：{}", error.without_url()))?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_MEDIA_BYTES
        } else {
            MAX_ERROR_BYTES
        };
        let bytes = read_capped_response(response, limit, ctx.abort).await?;
        if !status.is_success() {
            let detail = String::from_utf8_lossy(&bytes);
            return Err(format!(
                "语音合成失败（HTTP {status}）：{}",
                detail.chars().take(200).collect::<String>()
            ));
        }
        let saved = write_media(ctx, &target, &bytes)?;
        let mut result = json!({
            "path": saved,
            "bytes": bytes.len(),
            "model": model,
            "voice": voice,
            "format": format,
        });
        if let Some(data_url) = inline_audio_data_url(&bytes) {
            result["data_url"] = json!(data_url);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::is_local_endpoint;

    #[test]
    fn sanitize_stem_keeps_safe_characters_only() {
        assert_eq!(sanitize_stem("cat photo"), "cat_photo");
        assert_eq!(sanitize_stem("../etc/passwd"), "etc_passwd");
        assert_eq!(sanitize_stem(""), "media");
        assert_eq!(sanitize_stem("___"), "media");
    }

    #[test]
    fn output_path_stays_inside_outputs() {
        let workspace = Path::new("C:/work");
        let path = output_path(workspace, Some("hero"), "png").expect("路径可用");
        assert!(path.ends_with("outputs/hero.png") || path.ends_with("outputs\\hero.png"));
        assert!(output_path(workspace, Some("../escape"), "png").is_err());
        assert!(output_path(workspace, Some("C:/abs"), "png").is_err());
    }

    #[test]
    fn explicit_directories_survive_and_unsafe_paths_are_rejected() {
        let workspace = Path::new("workspace");
        assert_eq!(
            output_path(workspace, Some("images/task/hero.png"), "png").unwrap(),
            workspace.join("images/task/hero.png")
        );
        for raw in [
            "../escape",
            "images/../escape",
            "images/file:stream",
            "images/CON.png",
            "images./a",
            "images//a",
            "./a",
        ] {
            assert!(output_path(workspace, Some(raw), "png").is_err(), "{raw}");
        }
        assert_ne!(
            output_path(workspace, None, "png").unwrap(),
            output_path(workspace, None, "png").unwrap()
        );
    }
    #[tokio::test]
    async fn cancellation_interrupts_a_pending_media_operation() {
        let abort = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&abort);
        let setter = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            trigger.store(true, Ordering::Release);
        });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            cancellable(Some(&abort), std::future::pending::<()>()),
        )
        .await
        .unwrap();
        assert!(result.unwrap_err().contains("executor/cancelled"));
        setter.await.unwrap();
    }
    #[tokio::test]
    async fn worker_media_scopes_are_isolated_between_concurrent_futures() {
        async fn observe(scope: &str) {
            MEDIA_WRITE_SCOPE
                .scope(vec![PathBuf::from(scope)], async {
                    tokio::task::yield_now().await;
                    assert_eq!(
                        MEDIA_WRITE_SCOPE.with(Clone::clone),
                        vec![PathBuf::from(scope)]
                    );
                })
                .await;
        }
        tokio::join!(observe("images/one"), observe("audio/two"));
        assert!(MEDIA_WRITE_SCOPE.try_with(Clone::clone).is_err());
    }
    #[tokio::test]
    async fn response_limits_cover_declared_chunked_and_stalled_bodies() {
        use std::io::{Read, Write};
        async fn fixture(
            raw: &'static [u8],
            limit: usize,
            cancel_body: bool,
        ) -> Result<Vec<u8>, String> {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 8192];
                let _ = stream.read(&mut request);
                stream.write_all(raw).unwrap();
                stream.flush().unwrap();
                if cancel_body {
                    std::thread::sleep(Duration::from_millis(150));
                }
            });
            let abort = Arc::new(AtomicBool::new(false));
            let response = build_client().unwrap().get(url).send().await.unwrap();
            let trigger = Arc::clone(&abort);
            let setter = tokio::spawn(async move {
                if cancel_body {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    trigger.store(true, Ordering::Release);
                }
            });
            let result = read_capped_response(response, limit, Some(&abort)).await;
            setter.await.unwrap();
            server.join().unwrap();
            result
        }
        assert_eq!(
            fixture(
                b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
                3,
                false
            )
            .await
            .unwrap(),
            b"abc"
        );
        assert!(fixture(
            b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\nConnection: close\r\n\r\nx",
            3,
            false
        )
        .await
        .unwrap_err()
        .contains("too_large"));
        assert!(fixture(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n0\r\n\r\n", 3, false).await.unwrap_err().contains("too_large"));
        assert!(fixture(
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\n",
            3,
            true
        )
        .await
        .unwrap_err()
        .contains("cancelled"));
    }

    #[test]
    fn small_png_gets_inline_data_url() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let url = inline_data_url(&png).expect("小图应内联");
        assert!(url.starts_with("data:image/png;base64,"));
        let big = vec![0u8; 512 * 1024 + 1];
        assert!(inline_data_url(&big).is_none(), "超过 512 KiB 不内联");
        assert!(inline_data_url(b"not an image").is_none());
    }

    #[test]
    fn small_mp3_gets_inline_audio() {
        let mp3 = [b'I', b'D', b'3', 0x03, 0x00];
        let url = inline_audio_data_url(&mp3).expect("小音频应内联");
        assert!(url.starts_with("data:audio/mpeg;base64,"));
        assert!(inline_audio_data_url(b"not audio").is_none());
        assert!(inline_audio_data_url(&vec![0u8; 512 * 1024 + 1]).is_none());
    }

    // 进程级环境变量必须在整个请求期间保持不变：这里有意持锁跨 await，
    // 避免并发测试改写 OWO_IMAGE_* 后本次请求读到另一组值。
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn builtin_media_tools_call_local_compat_http_and_save_outputs() {
        use crate::permissions::Policy;
        use crate::session::Session;
        use crate::skill::SkillRegistry;
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _env_guard = ENV_LOCK.lock().unwrap();
        struct RestoreEnv(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for RestoreEnv {
            fn drop(&mut self) {
                for (name, value) in &self.0 {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
        let names = [
            "OWO_IMAGE_BASE_URL",
            "OWO_IMAGE_API_KEY",
            "OWO_IMAGE_API_KEY_ENV",
            "OWO_IMAGE_MODEL",
            "OWO_TTS_BASE_URL",
            "OWO_TTS_API_KEY",
            "OWO_TTS_API_KEY_ENV",
            "OWO_TTS_MODEL",
        ];
        let _restore = RestoreEnv(
            names
                .iter()
                .map(|&name| (name, std::env::var_os(name)))
                .collect(),
        );
        for name in names {
            std::env::remove_var(name);
        }
        let temp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::env::set_var("OWO_IMAGE_BASE_URL", format!("http://{address}/v1"));
        std::env::set_var("OWO_IMAGE_API_KEY", "local-fixture-key");
        std::env::set_var("OWO_IMAGE_MODEL", "fixture-image-model");
        std::env::set_var("OWO_TTS_BASE_URL", format!("http://{address}/speech"));
        std::env::set_var("OWO_TTS_API_KEY", "local-speech-key");
        std::env::set_var("OWO_TTS_MODEL", "fixture-speech-model");

        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for index in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        request.push_str(&line);
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse().unwrap();
                        }
                    }
                    request.push_str(&line);
                }
                let mut body = vec![0; content_length];
                reader.read_exact(&mut body).unwrap();
                request.push_str(&String::from_utf8(body).unwrap());
                let (content_type, response) = if index % 2 == 0 {
                    (
                        "application/json",
                        br#"{"created":1,"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="}]}"#.to_vec(),
                    )
                } else {
                    (
                        "audio/mpeg",
                        b"ID3\x04\x00\x00\x00\x00\x00\x00fixture-audio".to_vec(),
                    )
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                )
                .unwrap();
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
                requests.push(request);
            }
            requests
        });

        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let policy = Policy::new(&workspace);
        let audit = Arc::new(Mutex::new(crate::AuditLog::default()));
        let skills = SkillRegistry::default();
        let elements = Arc::new(Mutex::new(crate::ElementRegistry::new()));
        let mut session = Session::new(&workspace, "fixture-model", None);
        let mut context = ToolContext {
            workspace: &workspace,
            policy: &policy,
            session: &mut session,
            audit: &audit,
            subagent: None,
            skills: &skills,
            elements: &elements,
            fanout: None,
            abort: None,
            questioner: None,
        };

        let image_tool =
            scoped_workflow_tool(Arc::new(GenerateImageTool), vec![workspace.join("images")]);
        let denied = image_tool
            .run(
                &mut context,
                json!({"prompt":"must not send", "output":"outputs/denied.png"}),
            )
            .await
            .unwrap_err();
        assert!(denied.contains("media/write_scope"));
        assert!(!workspace.join("outputs").exists());
        let image = image_tool
            .run(
                &mut context,
                json!({"prompt":"fixture scene","output":"images/scene.png"}),
            )
            .await
            .unwrap();
        assert_eq!(image["path"], "images/scene.png");
        assert!(image["data_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(workspace.join("images/scene.png").is_file());

        let audio_tool = scoped_workflow_tool(
            Arc::new(TextToSpeechTool),
            vec![workspace.join("audio/task")],
        );
        let denied = audio_tool
            .run(
                &mut context,
                json!({"text":"must not send", "output":"audio/other/denied.mp3"}),
            )
            .await
            .unwrap_err();
        assert!(denied.contains("media/write_scope"));
        assert!(!workspace.join("audio").exists());
        let audio = audio_tool
            .run(
                &mut context,
                json!({"text":"测试语音","output":"audio/task/voice.mp3"}),
            )
            .await
            .unwrap();
        assert_eq!(audio["path"], "audio/task/voice.mp3");
        assert!(audio["data_url"]
            .as_str()
            .unwrap()
            .starts_with("data:audio/mpeg;base64,"));
        assert!(workspace.join("audio/task/voice.mp3").is_file());

        use crate::gateway::media_profiles::{
            MediaProfile, MediaProfileCatalog, MediaProfileStore,
        };
        let data_root = temp.path().join("host-data");
        let profiles = [
            (
                MediaService::ImageGeneration,
                "image",
                "profile-image",
                "profile-image-model",
            ),
            (
                MediaService::TextToSpeech,
                "speech",
                "profile-speech",
                "profile-speech-model",
            ),
        ];
        MediaProfileStore::new(&data_root)
            .save(MediaProfileCatalog {
                profiles: profiles
                    .iter()
                    .map(|(service, id, path, model)| MediaProfile {
                        id: (*id).into(),
                        name: (*id).into(),
                        base_url: format!("http://{address}/{path}"),
                        model: (*model).into(),
                        credential_env: None,
                        services: vec![*service],
                    })
                    .collect(),
                assignments: profiles
                    .iter()
                    .map(|(service, id, _, _)| (*service, (*id).into()))
                    .collect(),
                ..Default::default()
            })
            .unwrap();
        let profiled_image = scoped_workflow_tool(
            profiled_media_tool(Arc::new(GenerateImageTool), &data_root),
            vec![workspace.join("images")],
        );
        profiled_image
            .run(
                &mut context,
                json!({"prompt":"profile scene", "output":"images/profile.png"}),
            )
            .await
            .unwrap();
        let profiled_speech = scoped_workflow_tool(
            profiled_media_tool(Arc::new(TextToSpeechTool), &data_root),
            vec![workspace.join("audio/task")],
        );
        profiled_speech
            .run(
                &mut context,
                json!({"text":"profile speech", "output":"audio/task/profile.mp3"}),
            )
            .await
            .unwrap();
        assert!(workspace.join("images/profile.png").is_file());
        assert!(workspace.join("audio/task/profile.mp3").is_file());
        let requests = server.join().unwrap();
        for (index, expected_path, model) in [
            (
                2,
                "/profile-image/images/generations",
                "profile-image-model",
            ),
            (3, "/profile-speech/audio/speech", "profile-speech-model"),
        ] {
            assert!(requests[index].starts_with(&format!("POST {expected_path} ")));
            assert!(!requests[index]
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")));
            assert!(requests[index].contains(model));
        }

        assert!(requests[0].starts_with("POST /v1/images/generations "));
        assert!(requests[0].lines().any(|line| {
            line.to_ascii_lowercase().starts_with("authorization:")
                && line.contains("Bearer local-fixture-key")
        }));
        let image_body = requests[0]
            .split_once("\r\n\r\n")
            .or_else(|| requests[0].split_once("\n\n"))
            .map(|(_, body)| body)
            .expect("image request headers must end before its body");
        let image_request: Value = serde_json::from_str(image_body).unwrap();
        assert_eq!(image_request["prompt"], "fixture scene");
        assert_eq!(image_request["model"], "fixture-image-model");
        assert!(requests[1].starts_with("POST /speech/audio/speech "));
        assert!(requests[1].lines().any(|line| line
            .to_ascii_lowercase()
            .starts_with("authorization:")
            && line.contains("Bearer local-speech-key")));
        assert!(!requests[1].contains("local-fixture-key"));
        let audio_body = requests[1]
            .split_once("\r\n\r\n")
            .or_else(|| requests[1].split_once("\n\n"))
            .map(|(_, body)| body)
            .expect("audio request headers must end before its body");
        let audio_request: Value = serde_json::from_str(audio_body).unwrap();
        assert_eq!(audio_request["input"], "测试语音");
        assert_eq!(audio_request["model"], "fixture-speech-model");
    }

    #[test]
    fn request_bodies_match_openai_compat_shape() {
        let image = image_request_body("gpt-image-1", "a cat", "512x512");
        assert_eq!(image["model"], "gpt-image-1");
        assert_eq!(image["prompt"], "a cat");
        assert_eq!(image["size"], "512x512");
        assert_eq!(image["n"], 1);
        assert!(
            image.get("response_format").is_none(),
            "gpt-image 不需要 response_format"
        );

        let dalle = image_request_body("dall-e-3", "a dog", "1024x1024");
        assert_eq!(dalle["response_format"], "b64_json");

        let speech = speech_request_body("tts-1", "你好", "alloy", "mp3");
        assert_eq!(speech["input"], "你好");
        assert_eq!(speech["voice"], "alloy");
        assert_eq!(speech["response_format"], "mp3");
    }

    #[test]
    fn parses_b64_and_url_payloads() {
        // "aGVsbG8=" 是 "hello"
        let b64 = serde_json::json!({ "b64_json": "aGVsbG8=" });
        assert_eq!(
            parse_image_item(&b64).expect("base64 应解析"),
            ImagePayload::Base64(b"hello".to_vec())
        );
        // 前后空白（部分网关会带换行）要能容忍。
        let padded = serde_json::json!({ "b64_json": " aGVsbG8= " });
        assert_eq!(
            parse_image_item(&padded).expect("带空白也要解析"),
            ImagePayload::Base64(b"hello".to_vec())
        );
        let url = serde_json::json!({ "url": " https://cdn.example.com/a.png " });
        assert_eq!(
            parse_image_item(&url).expect("url 应解析"),
            ImagePayload::Url("https://cdn.example.com/a.png".to_string())
        );
        assert!(parse_image_item(&serde_json::json!({ "url": "  " })).is_err());
        assert!(parse_image_item(&serde_json::json!({})).is_err());
        assert!(parse_image_item(&serde_json::json!({ "b64_json": "!!!not base64!!!" })).is_err());
    }

    #[test]
    fn error_message_is_trimmed_and_readable() {
        let payload = serde_json::json!({ "error": { "message": "invalid api key" } });
        assert_eq!(media_error_message(&payload), "invalid api key");
        assert_eq!(media_error_message(&serde_json::json!({})), "未知错误");
        let long = serde_json::json!({ "error": { "message": "x".repeat(500) } });
        assert_eq!(media_error_message(&long).chars().count(), 200);
    }

    #[test]
    fn local_endpoints_allow_empty_key() {
        assert!(is_local_endpoint("http://127.0.0.1:8000/v1"));
        assert!(is_local_endpoint("http://localhost:11434/v1"));
        assert!(!is_local_endpoint("https://api.openai.com/v1"));
        assert!(!is_local_endpoint("https://localhost.example.com/v1"));
        assert!(!is_local_endpoint("https://example.com/127.0.0.1"));
        assert!(!is_local_endpoint("https://localhost@example.com/v1"));
        assert!(is_local_endpoint("http://[::1]:8000/v1"));
    }

    #[test]
    fn media_base_url_trims_trailing_slash() {
        // 环境变量在测试进程里可能已被设置，这里只校验不做 panic 的契约。
        let base = MediaConnection::from_env(MediaService::ImageGeneration)
            .unwrap()
            .base_url()
            .to_string();
        assert!(!base.is_empty());
        assert!(!base.ends_with('/'));
    }
}
