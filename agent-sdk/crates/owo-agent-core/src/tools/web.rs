//! Browser/search tools and their HTTP/parsing helpers.
//!
//! The registry stays in the parent module; this file owns the network-backed web tool family.

use super::{decode_process_output, Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use futures_util::StreamExt;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};

/// 极简 HTML → 文本：去 script/style 与标签，合并空白（web_fetch 用）。
pub(super) fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let Some(start) = rest.find('<') else {
            text.push_str(rest);
            break;
        };
        text.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = rest[start + 1..start + end].to_ascii_lowercase();
        if tag.starts_with("script") || tag.starts_with("style") {
            let close = if tag.starts_with("script") {
                "</script"
            } else {
                "</style"
            };
            match rest[start + end + 1..].find(close) {
                Some(offset) => {
                    let after = start + end + 1 + offset;
                    match rest[after..].find('>') {
                        Some(gt) => {
                            rest = &rest[after + gt + 1..];
                            continue;
                        }
                        None => break,
                    }
                }
                None => break,
            }
        }
        text.push(' ');
        rest = &rest[start + end + 1..];
    }
    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct CachedWebClient {
    proxy: Option<String>,
    client: reqwest::Client,
}

static WEB_CLIENT_CACHE: OnceLock<Mutex<Option<CachedWebClient>>> = OnceLock::new();

fn http_client() -> Result<reqwest::Client, String> {
    let proxy = std::env::var("OWO_HTTP_PROXY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let cache = WEB_CLIENT_CACHE.get_or_init(|| Mutex::new(None));
    let mut cached = cache
        .lock()
        .map_err(|_| "HTTP 客户端缓存不可用".to_string())?;
    if let Some(client) = cached
        .as_ref()
        .filter(|entry| entry.proxy.as_deref() == proxy.as_deref())
        .map(|entry| entry.client.clone())
    {
        return Ok(client);
    }
    let client = build_http_client(proxy.as_deref())?;
    *cached = Some(CachedWebClient {
        proxy,
        client: client.clone(),
    });
    Ok(client)
}

fn proxy_from_config(value: &str) -> Result<reqwest::Proxy, String> {
    let parsed = reqwest::Url::parse(value).map_err(|_| {
        "代理配置无效（OWO_HTTP_PROXY），请提供带 http:// 或 https:// 的代理 URL".to_string()
    })?;
    if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h")
        || parsed.host_str().is_none()
    {
        return Err("代理配置无效（OWO_HTTP_PROXY），请检查 URL 协议和主机名".to_string());
    }
    reqwest::Proxy::all(value)
        .map_err(|_| "代理配置无效（OWO_HTTP_PROXY），请检查 URL 格式".to_string())
}

/// 把一块响应体追加到缓冲区，超过上限即截断；返回是否已截断。
pub(super) fn append_capped(buffer: &mut Vec<u8>, chunk: &[u8], max_bytes: usize) -> bool {
    if buffer.len() >= max_bytes {
        return true;
    }
    let remaining = max_bytes - buffer.len();
    if chunk.len() > remaining {
        buffer.extend_from_slice(&chunk[..remaining]);
        true
    } else {
        buffer.extend_from_slice(chunk);
        false
    }
}

fn build_http_client(proxy: Option<&str>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("OwO-Agent/1.0 (+web)");
    // reqwest 不会自动识别 OWO_HTTP_PROXY，故显式应用；同一配置复用连接池，
    // 配置变化时重建 Client。代理 URL 可能含凭据，错误只说明字段和修复方向。
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy_from_config(proxy)?);
    }
    builder
        .build()
        .map_err(|error| format!("HTTP 客户端构造失败：{error}"))
}

/// `web_fetch`：抓取 URL 并返回纯文本（网络出口，需审批）。
pub(super) struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_fetch".into(),
            description: "抓取 URL 并返回纯文本（20s 超时、默认 256KB 上限；网络访问需审批）"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "max_bytes": { "type": "integer", "description": "响应上限（默认 262144）" }
                },
                "required": ["url"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let url = required_string(&args, "url")?;
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("url 必须以 http:// 或 https:// 开头".to_string());
        }
        let max_bytes = args
            .get("max_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(262_144)
            .clamp(1024, 1_048_576) as usize;
        let client = http_client()?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|error| format!("抓取失败：{error}"))?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        // 流式读取并在上限处截断：先 bytes() 再截断会把整个响应体读进内存，
        // 超大页面/慢速灌包可能拖垮进程。
        let mut bytes: Vec<u8> = Vec::with_capacity(max_bytes.min(64 * 1024));
        let mut truncated = false;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| format!("读取响应失败：{error}"))?;
            if append_capped(&mut bytes, &chunk, max_bytes) {
                truncated = true;
                break;
            }
        }
        // 响应体未必是 UTF-8：国内站点常见 GBK 且不带 charset 声明，用 lossy 解会整页乱码。
        let raw = decode_process_output(&bytes);
        let text = if content_type.contains("html") {
            html_to_text(&raw)
        } else {
            raw
        };
        Ok(json!({
            "url": url,
            "status": status,
            "content_type": content_type,
            "truncated": truncated,
            "bytes": bytes.len(),
            "text": text,
        }))
    }
}

/// `web_search`：默认走 DuckDuckGo HTML 端点，可用 `OWO_WEB_SEARCH_URL` 覆盖。
pub(super) struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description: "网页搜索（按 DuckDuckGo → Bing → DDG Lite 顺序自动选择可用端点；\
                          可用 OWO_WEB_SEARCH_URL 指定端点、OWO_HTTP_PROXY 指定出网代理）"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let query = required_string(&args, "query")?;
        // 显式配置优先；未配置时按候选顺序尝试（见 SEARCH_ENDPOINTS 说明）。
        let configured = std::env::var("OWO_WEB_SEARCH_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let endpoints: Vec<String> = match configured {
            Some(endpoint) => vec![endpoint],
            None => SEARCH_ENDPOINTS
                .iter()
                .map(|item| item.to_string())
                .collect(),
        };
        let client = http_client()?;
        let mut attempts: Vec<String> = Vec::new();
        for endpoint in &endpoints {
            match client
                .get(endpoint)
                .query(&[("q", query.as_str())])
                .send()
                .await
            {
                Err(error) => attempts.push(format!("{endpoint} → 请求失败：{error}")),
                Ok(response) => {
                    let status = response.status().as_u16();
                    if !(200..300).contains(&status) {
                        attempts.push(format!("{endpoint} → HTTP {status}"));
                        continue;
                    }
                    let mut bytes: Vec<u8> = Vec::with_capacity(64 * 1024);
                    let mut stream = response.bytes_stream();
                    let mut read_error = None;
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(chunk) => {
                                if append_capped(&mut bytes, &chunk, 1_048_576) {
                                    break;
                                }
                            }
                            Err(error) => {
                                read_error = Some(error);
                                break;
                            }
                        }
                    }
                    if let Some(error) = read_error {
                        attempts.push(format!("{endpoint} → 读取响应失败：{error}"));
                        continue;
                    }
                    let body = decode_process_output(&bytes);
                    let results = parse_search_results(endpoint, &body);
                    if results.is_empty() {
                        attempts.push(format!("{endpoint} → 无结果（页面结构可能已变）"));
                        continue;
                    }
                    return Ok(json!({
                        "query": query,
                        "engine": endpoint,
                        "status": status,
                        "count": results.len(),
                        "results": results,
                    }));
                }
            }
        }
        // 全部候选都失败：把每一次的原因列出来，并给出可操作的下一步，
        // 而不是丢一句笼统的"搜索请求失败"让调用方猜。
        Err(format!(
            "搜索失败：{}。排查建议：① 若在受限网络，设置 OWO_HTTP_PROXY 指向可用代理；\
             ② 或用 OWO_WEB_SEARCH_URL 指定可达的搜索端点（如 https://cn.bing.com/search）。",
            attempts.join("；")
        ))
    }
}

/// 搜索端点候选：按顺序尝试，第一个返回可解析结果的胜出。
///
/// 为什么必须 fallback：默认的 DuckDuckGo HTML 端点在部分网络环境**完全不可达**
/// （实测 3 次尝试全部 `error sending request`，而同期 `web_fetch` 抓 example.com /
/// crates.io 均 200——是站点级不可达，不是网络故障）。搜索是高频工具，不该因为一个
/// 境外站点就整体不可用；国内可达的 Bing 作为第二档，DDG Lite 作为第三档（结构更简）。
const SEARCH_ENDPOINTS: &[&str] = &[
    "https://html.duckduckgo.com/html/",
    "https://cn.bing.com/search",
    "https://lite.duckduckgo.com/lite/",
];

/// 按端点选择解析器：不同引擎的 HTML 结构完全不同，不能共用一套规则。
fn parse_search_results(endpoint: &str, html: &str) -> Vec<Value> {
    if endpoint.contains("bing.com") {
        parse_bing_results(html)
    } else {
        parse_ddg_results(html)
    }
}

/// 解析 Bing 结果（`<li class="b_algo">` 内的 `<h2><a href=…>`）。
///
/// 候选端点里的 cn.bing.com 是为了在 DuckDuckGo 不可达的网络下仍能搜索；
/// 它的结构是「结果块 → h2 → 锚点」，与 DDG 的 `class="result__a"` 无关。
fn parse_bing_results(html: &str) -> Vec<Value> {
    let mut results = Vec::new();
    let mut rest = html;
    while let Some(position) = rest.find("class=\"b_algo\"") {
        let block = &rest[position..];
        let Some(h2) = block.find("<h2") else {
            break;
        };
        let scoped = &block[h2..];
        let Some(anchor) = scoped.find("<a ") else {
            break;
        };
        let Some(href_offset) = scoped[anchor..].find("href=\"") else {
            break;
        };
        let href_start = anchor + href_offset + 6;
        let Some(href_end) = scoped[href_start..].find('"') else {
            break;
        };
        let url = scoped[href_start..href_start + href_end].to_string();
        let Some(gt) = scoped[href_start..].find('>') else {
            break;
        };
        let title_start = href_start + gt + 1;
        let Some(close) = scoped[title_start..].find("</a>") else {
            break;
        };
        let title = html_to_text(&scoped[title_start..title_start + close]);
        let consumed = position + title_start + close;
        // 只收外链结果：Bing 内嵌了不少站内导航锚点，它们也是 <a>。
        if url.starts_with("http") && !title.is_empty() {
            results.push(json!({ "title": title, "url": url }));
            if results.len() >= 10 {
                break;
            }
        }
        // 无论是否采纳都要前进，否则同一块会被反复解析（死循环）。
        rest = &rest[consumed.min(rest.len())..];
    }
    results
}

/// 解析 DuckDuckGo HTML（`class="result__a"`）与 DDG Lite；最多 10 条。
fn parse_ddg_results(html: &str) -> Vec<Value> {
    let mut results = Vec::new();
    let mut rest = html;
    while let Some(position) = rest.find("class=\"result__a\"") {
        let anchor_start = rest[..position].rfind("<a ").unwrap_or(position);
        let href = rest[anchor_start..position]
            .find("href=\"")
            .map(|offset| {
                let start = anchor_start + offset + 6;
                rest[start..]
                    .find('"')
                    .map(|end| decode_search_href(&rest[start..start + end]))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let Some(gt) = rest[position..].find('>') else {
            break;
        };
        let title_start = position + gt + 1;
        let Some(close) = rest[title_start..].find("</a>") else {
            break;
        };
        let title = html_to_text(&rest[title_start..title_start + close]);
        if !title.is_empty() {
            results.push(json!({ "title": title, "url": href }));
        }
        rest = &rest[title_start + close..];
        if results.len() >= 10 {
            break;
        }
    }
    results
}

/// DDG 跳转链接解码（`uddg=` + percent-encoding）。
pub(super) fn decode_search_href(href: &str) -> String {
    if let Some(index) = href.find("uddg=") {
        let encoded = &href[index + 5..];
        let encoded = encoded.split('&').next().unwrap_or(encoded);
        return percent_decode(encoded);
    }
    if let Some(stripped) = href.strip_prefix("//") {
        return format!("https://{stripped}");
    }
    href.to_string()
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::proxy_from_config;

    #[test]
    fn invalid_proxy_error_never_echoes_credentials() {
        let secret_value = "proxy=top-secret-credential";
        let error = match proxy_from_config(secret_value) {
            Ok(_) => panic!("malformed proxy URL must be rejected"),
            Err(error) => error,
        };
        assert!(!error.contains("top-secret-credential"));
        assert!(!error.contains(secret_value));
        assert!(error.contains("OWO_HTTP_PROXY"));
    }
}
