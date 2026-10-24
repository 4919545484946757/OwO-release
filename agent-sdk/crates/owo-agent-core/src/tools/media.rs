//! 媒体生成工具：文生图（`generate_image`）与语音合成（`text_to_speech`）。
//!
//! 两者都走 OpenAI 兼容的多模态端点，产物写进工作区 `outputs/`，返回值只给相对路径，
//! 便于审计与后续回合引用。凭据只从环境变量读取，绝不进参数、日志或配置。

use super::{web::append_capped, Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use base64::Engine as _;
use futures_util::StreamExt;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// 单个媒体产物上限：32 MiB。超过即判定异常，避免磁盘被大文件写满。
const MAX_MEDIA_BYTES: usize = 32 * 1024 * 1024;

/// 图片下载的独立超时：正常的图片 CDN 几秒内返回，用 180s 的整体超时等一个
/// 卡住的下载只会白白占住回合。生成请求本身仍用 180s（大图生成确实慢）。
const DOWNLOAD_TIMEOUT_SECS: u64 = 60;

/// 媒体端点基址：优先 `OWO_IMAGE_BASE_URL`（可指向独立的图像服务），
/// 其次 `OPENAI_BASE_URL`，最后退回 OpenAI 官方。
fn media_base_url() -> String {
    for key in ["OWO_IMAGE_BASE_URL", "OPENAI_BASE_URL"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim().trim_end_matches('/').to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    "https://api.openai.com/v1".to_string()
}

/// 凭据只认环境变量：`OWO_IMAGE_API_KEY` 优先，退回 `OPENAI_API_KEY`。
fn media_api_key() -> Option<String> {
    for key in ["OWO_IMAGE_API_KEY", "OPENAI_API_KEY"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

/// 本机端点（Ollama / vLLM / 自建服务）允许空密钥。
fn is_local_endpoint(base: &str) -> bool {
    let lower = base.to_ascii_lowercase();
    lower.contains("127.0.0.1")
        || lower.contains("localhost")
        || lower.contains("[::1]")
        || lower.contains("0.0.0.0")
}

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

/// 产物路径：始终落在 `<workspace>/outputs/` 之下，拒绝绝对路径与 `..`。
fn output_path(workspace: &Path, requested: Option<&str>, ext: &str) -> Result<PathBuf, String> {
    let rel = requested.map(str::trim).filter(|value| !value.is_empty());
    let name = match rel {
        Some(value) => {
            let candidate = Path::new(value);
            if candidate.is_absolute()
                || candidate.components().any(|part| {
                    matches!(
                        part,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                return Err("输出路径必须是工作区内的相对路径".to_string());
            }
            let stem = candidate
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("media");
            format!("{}.{}", sanitize_stem(stem), ext)
        }
        None => format!(
            "media-{}.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_millis())
                .unwrap_or(0),
            ext
        ),
    };
    Ok(workspace.join("outputs").join(name))
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
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|error| format!("图片 base64 解码失败：{error}"))?;
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
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| format!("HTTP 客户端不可用：{error}"))
}

fn require_credentials(base: &str) -> Result<Option<String>, String> {
    match media_api_key() {
        Some(key) => Ok(Some(key)),
        None if is_local_endpoint(base) => Ok(None),
        None => Err(
            "模型未配置：文生图/语音需要 OPENAI_API_KEY（或 OWO_IMAGE_API_KEY）；\
                     本地端点可用 OWO_IMAGE_BASE_URL 指向 127.0.0.1"
                .to_string(),
        ),
    }
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
async fn download_image_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|error| {
            format!(
                "下载图片失败（{}s 超时或网络错误）：{error}",
                DOWNLOAD_TIMEOUT_SECS
            )
        })?;
    if !response.status().is_success() {
        return Err(format!("下载图片失败（HTTP {}）", response.status()));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_MEDIA_BYTES as u64 {
            return Err(format!(
                "产物过大（端点声明 {length} 字节），上限 {MAX_MEDIA_BYTES}"
            ));
        }
    }
    let mut buffer: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("读取图片失败：{error}"))?;
        if append_capped(&mut buffer, &chunk, MAX_MEDIA_BYTES) {
            return Err(format!(
                "产物过大（超过 {MAX_MEDIA_BYTES} 字节），已中止下载"
            ));
        }
    }
    Ok(buffer)
}

fn write_media(ctx: &ToolContext<'_>, path: &Path, bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("端点返回了空内容".to_string());
    }
    if bytes.len() > MAX_MEDIA_BYTES {
        return Err(format!(
            "产物过大（{} 字节），上限 {MAX_MEDIA_BYTES}",
            bytes.len()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "输出路径缺少父目录".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
    let parent_meta =
        std::fs::symlink_metadata(parent).map_err(|error| format!("检查输出目录失败：{error}"))?;
    if parent_meta.file_type().is_symlink() {
        return Err("输出目录不能是符号链接".to_string());
    }
    let workspace = std::fs::canonicalize(ctx.workspace)
        .map_err(|error| format!("解析工作区路径失败：{error}"))?;
    let canonical_parent =
        std::fs::canonicalize(parent).map_err(|error| format!("解析输出目录失败：{error}"))?;
    if canonical_parent.parent() != Some(workspace.as_path())
        || canonical_parent.file_name().and_then(|name| name.to_str()) != Some("outputs")
    {
        return Err("媒体产物只能写入工作区 outputs/ 目录".to_string());
    }
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err("媒体产物不能覆盖符号链接".to_string());
        }
    }
    std::fs::write(path, bytes).map_err(|error| format!("写入产物失败：{error}"))?;
    let relative = path
        .strip_prefix(ctx.workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(relative)
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
            description: "文生图：按提示词生成图片并保存到工作区 outputs/，返回相对路径。\
                          走 OpenAI 兼容 /images/generations；端点用 OWO_IMAGE_BASE_URL 或 \
                          OPENAI_BASE_URL，密钥用 OWO_IMAGE_API_KEY 或 OPENAI_API_KEY"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "图片描述（越具体越好）" },
                    "model": { "type": "string", "description": "图像模型，默认 gpt-image-1" },
                    "size": { "type": "string", "description": "尺寸，如 1024x1024" },
                    "output": { "type": "string", "description": "工作区内的相对文件名（可省）" }
                },
                "required": ["prompt"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let prompt = required_string(&args, "prompt")?;
        let base = media_base_url();
        let key = require_credentials(&base)?;
        let model = args
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("gpt-image-1")
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
        if let Some(key) = key.as_deref() {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("文生图请求失败：{error}"))?;
        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|error| format!("端点返回非 JSON：{error}"))?;
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
                download_image_bytes(&client, &url).await?,
                "url".to_string(),
            ),
        };
        let path = output_path(
            ctx.workspace,
            args.get("output").and_then(Value::as_str),
            "png",
        )?;
        let saved = write_media(ctx, &path, &bytes)?;
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
            description: "语音合成：把文本转成音频并保存到工作区 outputs/，返回相对路径。\
                          走 OpenAI 兼容 /audio/speech；端点与密钥同文生图"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "要朗读的文本" },
                    "model": { "type": "string", "description": "语音模型，默认 tts-1" },
                    "voice": { "type": "string", "description": "音色，默认 alloy" },
                    "format": { "type": "string", "description": "mp3 / wav / opus，默认 mp3" },
                    "output": { "type": "string", "description": "工作区内的相对文件名（可省）" }
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
        let base = media_base_url();
        let key = require_credentials(&base)?;
        let model = args
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("tts-1")
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
        let client = build_client()?;
        let endpoint = format!("{base}/audio/speech");
        let mut request = client
            .post(&endpoint)
            .json(&speech_request_body(&model, &text, &voice, &format));
        if let Some(key) = key.as_deref() {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("语音合成请求失败：{error}"))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "语音合成失败（HTTP {status}）：{}",
                detail.chars().take(200).collect::<String>()
            ));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("读取音频失败：{error}"))?;
        let path = output_path(
            ctx.workspace,
            args.get("output").and_then(Value::as_str),
            &format,
        )?;
        let saved = write_media(ctx, &path, &bytes)?;
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
        let old_base = std::env::var_os("OWO_IMAGE_BASE_URL");
        let old_key = std::env::var_os("OWO_IMAGE_API_KEY");
        let temp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::env::set_var("OWO_IMAGE_BASE_URL", format!("http://{address}/v1"));
        std::env::set_var("OWO_IMAGE_API_KEY", "local-fixture-key");

        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
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
                let (content_type, response) = if request
                    .starts_with("POST /v1/images/generations ")
                {
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

        let image = GenerateImageTool
            .run(
                &mut context,
                json!({"prompt":"fixture scene","output":"scene"}),
            )
            .await
            .unwrap();
        assert_eq!(image["path"], "outputs/scene.png");
        assert!(image["data_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(workspace.join("outputs/scene.png").is_file());

        let audio = TextToSpeechTool
            .run(&mut context, json!({"text":"测试语音","output":"voice"}))
            .await
            .unwrap();
        assert_eq!(audio["path"], "outputs/voice.mp3");
        assert!(audio["data_url"]
            .as_str()
            .unwrap()
            .starts_with("data:audio/mpeg;base64,"));
        assert!(workspace.join("outputs/voice.mp3").is_file());

        let requests = server.join().unwrap();
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
        assert!(requests[1].starts_with("POST /v1/audio/speech "));
        let audio_body = requests[1]
            .split_once("\r\n\r\n")
            .or_else(|| requests[1].split_once("\n\n"))
            .map(|(_, body)| body)
            .expect("audio request headers must end before its body");
        let audio_request: Value = serde_json::from_str(audio_body).unwrap();
        assert_eq!(audio_request["input"], "测试语音");

        match old_base {
            Some(value) => std::env::set_var("OWO_IMAGE_BASE_URL", value),
            None => std::env::remove_var("OWO_IMAGE_BASE_URL"),
        }
        match old_key {
            Some(value) => std::env::set_var("OWO_IMAGE_API_KEY", value),
            None => std::env::remove_var("OWO_IMAGE_API_KEY"),
        }
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
    }

    #[test]
    fn media_base_url_trims_trailing_slash() {
        // 环境变量在测试进程里可能已被设置，这里只校验不做 panic 的契约。
        let base = media_base_url();
        assert!(!base.is_empty());
        assert!(!base.ends_with('/'));
    }
}
