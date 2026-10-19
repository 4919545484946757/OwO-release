//! Browser/search tools and their HTTP/parsing helpers.
//!
//! The registry stays in the parent module; this file owns the network-backed web tool family.

use super::{decode_process_output, Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use futures_util::StreamExt;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};

/// 跳过 `</tag>` 到 `>` 之间的元素内容，返回其后的剩余文本；找不到闭合标签返回 `None`。
fn skip_until_close<'a>(rest: &'a str, tag_name: &str) -> Option<&'a str> {
    let lower = rest.to_ascii_lowercase();
    let close = format!("</{tag_name}");
    let offset = lower.find(&close)?;
    let after = offset + close.len();
    let gt = rest[after..].find('>')?;
    Some(&rest[after + gt + 1..])
}

/// 判断开标签是否为"对用户不可见"元素：可见性样式、`hidden`/`aria-hidden` 属性。
///
/// 只做保守匹配：`display:none`、`visibility:hidden`、`hidden` 属性、
/// `aria-hidden="true"`；不匹配 `opacity:0`（`opacity:0.5` 等半透明内容仍应可见）。
fn is_hidden_tag(tag_lower: &str) -> bool {
    let compact: String = tag_lower.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("display:none")
        || compact.contains("visibility:hidden")
        || compact.contains("aria-hidden=\"true\"")
        || compact.contains("aria-hidden='true'")
        || tag_lower
            .split_whitespace()
            .any(|token| token == "hidden" || token.starts_with("hidden="))
}

/// 保留可见元素骨架的 HTML：丢掉注释 / script / style / 隐藏元素**整棵子树**。
///
/// 与 [`html_to_text`] 同口径（同一套 `is_hidden_tag` + 闭合标签跳读规则），区别只是
/// 这里保留标签本身——[`extract_links`] 需要 `<a href>`，而纯文本化会把它抹掉。
fn strip_hidden_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let Some(start) = rest.find('<') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        if rest[start..].starts_with("<!--") {
            match rest[start + 4..].find("-->") {
                Some(offset) => {
                    rest = &rest[start + 4 + offset + 3..];
                    continue;
                }
                None => break,
            }
        }
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let tag_name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("")
            .to_string();
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let drop_subtree = !closing
            && !tag_name.is_empty()
            && (tag_name == "script"
                || tag_name == "style"
                || (!self_closing && is_hidden_tag(&tag)));
        if drop_subtree {
            match skip_until_close(&rest[start + end + 1..], &tag_name) {
                Some(remaining) => {
                    rest = remaining;
                    continue;
                }
                None => break,
            }
        }
        out.push_str(&rest[start..start + end + 1]);
        rest = &rest[start + end + 1..];
    }
    out
}

/// 极简 HTML → 文本：去 script/style/注释/隐藏元素与标签，合并空白（web_fetch 用）。
///
/// 隐藏节点（`<div style="display:none">…`、`<span hidden>`、`aria-hidden="true"`）
/// 必须丢弃：攻击者常把提示注入藏在用户看不见、抓取方却会读进上下文的节点里。
pub(super) fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let Some(start) = rest.find('<') else {
            text.push_str(rest);
            break;
        };
        text.push_str(&rest[..start]);
        // 注释（含条件注释）整段丢弃：注释正文可能含 `>`，不能按"第一个 `>`"切标签。
        if rest[start..].starts_with("<!--") {
            match rest[start + 4..].find("-->") {
                Some(offset) => {
                    rest = &rest[start + 4 + offset + 3..];
                    continue;
                }
                None => break,
            }
        }
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let tag_name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("")
            .to_string();
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        if !closing && (tag_name == "script" || tag_name == "style") {
            match skip_until_close(&rest[start + end + 1..], &tag_name) {
                Some(remaining) => {
                    rest = remaining;
                    continue;
                }
                None => break,
            }
        }
        if !closing && !self_closing && !tag_name.is_empty() && is_hidden_tag(&tag) {
            match skip_until_close(&rest[start + end + 1..], &tag_name) {
                Some(remaining) => {
                    rest = remaining;
                    continue;
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
    fetch: bool,
    client: reqwest::Client,
}

static WEB_CLIENT_CACHE: OnceLock<Mutex<Option<CachedWebClient>>> = OnceLock::new();

/// 网页抓取用客户端（`fetch=true`）：不自动跟随重定向，由抓取工具逐跳校验目标后手动跟随。
fn http_client(fetch: bool) -> Result<reqwest::Client, String> {
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
        .filter(|entry| entry.proxy.as_deref() == proxy.as_deref() && entry.fetch == fetch)
        .map(|entry| entry.client.clone())
    {
        return Ok(client);
    }
    let client = build_http_client(proxy.as_deref(), fetch)?;
    *cached = Some(CachedWebClient {
        proxy,
        fetch,
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

fn build_http_client(proxy: Option<&str>, fetch: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("OwO-Agent/1.0 (+web)");
    if fetch {
        // 手动逐跳跟随，重定向目标也必须过 SSRF 校验（reqwest 内部跟随无法逐跳拦截）。
        builder = builder.redirect(reqwest::redirect::Policy::none());
        if !allow_private_targets() {
            builder = builder.dns_resolver(std::sync::Arc::new(PublicOnlyResolver));
        }
    } else {
        builder = builder.redirect(reqwest::redirect::Policy::limited(5));
    }
    // reqwest 不会自动识别 OWO_HTTP_PROXY，故显式应用；同一配置复用连接池，
    // 配置变化时重建 Client。代理 URL 可能含凭据，错误只说明字段和修复方向。
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy_from_config(proxy)?);
    }
    builder
        .build()
        .map_err(|error| format!("HTTP 客户端构造失败：{error}"))
}

/// 内网/本机地址放行开关（`OWO_WEB_FETCH_ALLOW_PRIVATE=1`，仅内网自建服务场景）。
fn allow_private_targets() -> bool {
    std::env::var("OWO_WEB_FETCH_ALLOW_PRIVATE")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// IPv4 是否属于内网/回环/保留段（SSRF 拒绝清单，含 CGNAT 100.64/10）。
fn ipv4_is_forbidden(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || octets[0] == 0
        || octets[0] >= 240
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 198 && (18..=19).contains(&octets[1]))
}

/// IP 是否属于内网/回环/链路本地/保留段（含 IPv4-mapped IPv6）。
pub(super) fn ip_is_forbidden(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => ipv4_is_forbidden(v4),
        std::net::IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return ipv4_is_forbidden(mapped);
            }
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (segments[0] & 0xffc0) == 0xfe80 // link local fe80::/10
                || (segments[0] == 0x2001 && segments[1] == 0x0db8) // 文档段
        }
    }
}

/// 抓取目标校验：仅 http/https；IP 字面量直接拒绝私网/保留地址。
///
/// DNS 名称的解析结果由 [`PublicOnlyResolver`] 在连接层过滤（防 DNS rebinding）。
pub(super) fn validate_fetch_target(url: &reqwest::Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("web_fetch 仅允许 http/https：{}", url.scheme()));
    }
    if allow_private_targets() {
        return Ok(());
    }
    let host = url.host_str().ok_or_else(|| "URL 缺少主机名".to_string())?;
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if ip_is_forbidden(ip) {
            return Err(format!("web_fetch 拒绝私网/本机目标：{host}"));
        }
    }
    Ok(())
}

/// 自定义 DNS 解析器：过滤内网/回环/保留地址，全部被拒时报错。
///
/// 在连接层过滤而不是"先查再连"，可避免 DNS rebinding（校验与连接使用同一批地址）。
struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
            let allowed: Vec<std::net::SocketAddr> =
                addrs.filter(|addr| !ip_is_forbidden(addr.ip())).collect();
            if allowed.is_empty() {
                return Err("目标解析到私网/保留地址，已按 SSRF 策略拒绝".into());
            }
            Ok(Box::new(allowed.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// `web_fetch`：抓取 URL 并返回纯文本（网络出口，需审批）。
pub(super) struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_fetch".into(),
            description: "抓取 URL 并返回纯文本；HTML 页面额外返回去重后的可见链接清单（links）。\
                          20s 超时、默认 256KB 上限；私网/本机地址默认拒绝；网络访问需审批"
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
        let client = http_client(true)?;
        // 手动逐跳跟随重定向：每一跳都先过 SSRF 校验，且上限 5 次（与原先 limited(5) 等价）。
        let mut current =
            reqwest::Url::parse(&url).map_err(|error| format!("URL 无效：{error}"))?;
        let mut redirects = 0usize;
        let (status, content_type, bytes, truncated, final_url);
        'fetch: loop {
            validate_fetch_target(&current)?;
            let response = client
                .get(current.clone())
                .send()
                .await
                .map_err(|error| format!("抓取失败：{error}"))?;
            if response.status().is_redirection() {
                if redirects >= 5 {
                    return Err("重定向次数超过上限（5）".to_string());
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
                    .ok_or_else(|| "重定向响应缺少 Location 头".to_string())?;
                current = current
                    .join(&location)
                    .map_err(|error| format!("重定向目标无效：{error}"))?;
                redirects += 1;
                continue 'fetch;
            }
            let code = response.status().as_u16();
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            // 流式读取并在上限处截断：先 bytes() 再截断会把整个响应体读进内存，
            // 超大页面/慢速灌包可能拖垮进程。
            let mut buffer: Vec<u8> = Vec::with_capacity(max_bytes.min(64 * 1024));
            let mut was_truncated = false;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| format!("读取响应失败：{error}"))?;
                if append_capped(&mut buffer, &chunk, max_bytes) {
                    was_truncated = true;
                    break;
                }
            }
            status = code;
            content_type = mime;
            bytes = buffer;
            truncated = was_truncated;
            final_url = current.as_str().to_string();
            break 'fetch;
        }
        // 响应体未必是 UTF-8：国内站点常见 GBK 且不带 charset 声明，用 lossy 解会整页乱码。
        let raw = decode_process_output(&bytes);
        let is_html = content_type.contains("html");
        let links = if is_html {
            // 纯文本化会丢掉 `<a href>`，而"顺着页面里的资料继续查"是研究型任务的主路径；
            // 额外给出前若干条去重后的绝对链接，模型不必再抓一次 HTML 或猜测 URL 形态。
            extract_links(&raw, &final_url, 12)
        } else {
            Vec::new()
        };
        let text = if is_html { html_to_text(&raw) } else { raw };
        Ok(json!({
            "url": final_url,
            "status": status,
            "content_type": content_type,
            "truncated": truncated,
            "bytes": bytes.len(),
            "links": links,
            "text": text,
        }))
    }
}

/// 从 HTML 抽取正文链接（绝对化 + 去重 + 截断），供研究型任务顺链跟进。
///
/// 规则：只收 http/https、跳过 `#fragment` 锚点与脚本类 scheme；标题取锚文本并压平
/// 空白、限长 120 字符；按归一化 URL 去重（复用 [`normalize_search_url`] 的口径），
/// 最多 `limit` 条。跳过隐藏元素里的锚（`display:none` 之类），与 `html_to_text` 同口径。
pub(super) fn extract_links(html: &str, base_url: &str, limit: usize) -> Vec<Value> {
    let base = reqwest::Url::parse(base_url).ok();
    let mut seen = std::collections::HashSet::new();
    let mut links = Vec::new();
    // 先在"可见 HTML"上扫描：隐藏子树里的 `<a>` 不能进结果（与 html_to_text 同口径，
    // 否则 `<span style="display:none"><a href=…>` 会把不可见链接带进模型上下文）。
    let visible = strip_hidden_html(html);
    let mut rest = visible.as_str();
    while let Some(position) = rest.find("<a ") {
        let scoped = &rest[position..];
        let Some(close_anchor) = scoped.find("</a>") else {
            break;
        };
        let tag_end = scoped.find('>').unwrap_or(0);
        if tag_end == 0 || tag_end > close_anchor {
            rest = &scoped[close_anchor + 4..];
            continue;
        }
        let tag = scoped[..tag_end].to_ascii_lowercase();
        let href = scoped[..tag_end]
            .find("href=")
            .map(|offset| {
                let value = scoped[offset + 5..tag_end].trim();
                let value = value.trim_matches(|c| c == '"' || c == '\'' || c == ' ');
                value.to_string()
            })
            .unwrap_or_default();
        let text = html_to_text(&scoped[tag_end + 1..close_anchor]);
        rest = &scoped[close_anchor + 4..];
        if href.is_empty() || href.starts_with('#') || is_hidden_tag(tag.trim_start_matches("a")) {
            continue;
        }
        let lower = href.to_ascii_lowercase();
        if lower.starts_with("javascript:")
            || lower.starts_with("mailto:")
            || lower.starts_with("tel:")
            || lower.starts_with("data:")
        {
            continue;
        }
        let absolute = if let Some(base) = &base {
            match base.join(&href) {
                Ok(url) => url.as_str().to_string(),
                Err(_) => continue,
            }
        } else {
            href.clone()
        };
        if !(absolute.starts_with("http://") || absolute.starts_with("https://")) {
            continue;
        }
        if !seen.insert(normalize_search_url(&absolute)) {
            continue;
        }
        let mut title: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.chars().count() > 120 {
            title = title.chars().take(120).collect::<String>() + "…";
        }
        links.push(json!({ "title": title, "url": absolute }));
        if links.len() >= limit {
            break;
        }
    }
    links
}

/// `web_search`：默认走 DuckDuckGo HTML 端点，可用 `OWO_WEB_SEARCH_URL` 覆盖。
pub(super) struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description: "网页搜索（按 DuckDuckGo → Bing → DDG Lite 顺序自动选择可用端点；\
                          结果按 URL 去重后返回，默认最多 8 条；\
                          可用 OWO_WEB_SEARCH_URL 指定端点、OWO_HTTP_PROXY 指定出网代理）"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "返回条数上限（默认 8）"
                    }
                },
                "required": ["query"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let query = required_string(&args, "query")?;
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(8)
            .clamp(1, 20) as usize;
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
        let client = http_client(false)?;
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
                    let results = normalize_search_results(
                        parse_search_results(endpoint, &body),
                        max_results,
                    );
                    if results.is_empty() {
                        attempts.push(format!("{endpoint} → 无可解析结果（页面结构可能已变）"));
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

/// 搜索结果归一化：丢空标题/非 http(s) 链接、按 URL 去重（保留首次顺序），再截断。
///
/// 为什么需要：两个引擎的 HTML 里都夹着站内导航锚点与重复条目（同一结果带不同
/// fragment/结尾斜杠/host 大小写）；`href=""` 或重复链接喂给模型只会挤占上下文，
/// 还容易让模型去抓取不可用的地址。上限由调用方给（默认 8，见 `max_results`）。
pub(super) fn normalize_search_results(results: Vec<Value>, max_results: usize) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::new();
    for result in results {
        let url = result
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let title = result
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if title.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        if !seen.insert(normalize_search_url(url)) {
            continue;
        }
        kept.push(result);
        if kept.len() >= max_results {
            break;
        }
    }
    kept
}

/// 去重键：scheme/host 小写、去 fragment、去结尾 `/`、保留 query（query 不同即不同页）。
fn normalize_search_url(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let trimmed = without_fragment.trim_end_matches('/');
    match trimmed.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = match rest.find('/') {
                Some(index) => (&rest[..index], &rest[index..]),
                None => (rest, ""),
            };
            format!(
                "{}://{}{}",
                scheme.to_ascii_lowercase(),
                host.to_ascii_lowercase(),
                path
            )
        }
        None => trimmed.to_ascii_lowercase(),
    }
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
    use super::{
        extract_links, normalize_search_results, normalize_search_url, parse_bing_results,
        proxy_from_config,
    };
    use serde_json::json;

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

    #[test]
    fn search_results_are_deduped_and_capped() {
        let raw = vec![
            json!({ "title": "A", "url": "https://Example.com/a/" }),
            // 同页不同写法（大小写 / 结尾斜杠 / fragment / utm 之外的 query 保留）→ 去重
            json!({ "title": "A dup", "url": "https://example.com/a#section" }),
            json!({ "title": "B", "url": "https://example.org/b?q=1" }),
            json!({ "title": "B 不同 query", "url": "https://example.org/b?q=2" }),
            json!({ "title": "C", "url": "https://example.net/c" }),
            json!({ "title": "   ", "url": "https://example.net/empty-title" }),
            json!({ "title": "站内锚点", "url": "" }),
            json!({ "title": "相对链接", "url": "/search?q=x" }),
        ];
        let kept = normalize_search_results(raw, 8);
        let urls: Vec<&str> = kept
            .iter()
            .map(|item| item["url"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            urls,
            vec![
                "https://Example.com/a/",
                "https://example.org/b?q=1",
                "https://example.org/b?q=2",
                "https://example.net/c",
            ],
            "只保留首个同页结果、丢掉空标题/非 http(s) 链接：{kept:?}"
        );

        let capped = normalize_search_results(
            (0..10)
                .map(|index| json!({ "title": format!("t{index}"), "url": format!("https://e.com/{index}") }))
                .collect(),
            3,
        );
        assert_eq!(capped.len(), 3, "上限必须生效");
    }

    #[test]
    fn search_url_key_ignores_fragment_and_host_case_only() {
        assert_eq!(
            normalize_search_url("HTTPS://Example.COM/Path/#frag"),
            normalize_search_url("https://example.com/Path")
        );
        assert_ne!(
            normalize_search_url("https://example.com/a?x=1"),
            normalize_search_url("https://example.com/a?x=2")
        );
    }

    #[test]
    fn fetch_text_extracts_absolute_deduped_links() {
        let html = r##"
            <a href="/docs/guide">指南</a>
            <a href="https://example.com/docs/guide#section">指南（锚点）</a>
            <a href="javascript:void(0)">脚本锚点</a>
            <a href="mailto:a@example.com">邮件</a>
            <a href="#top">回到顶部</a>
            <a href="HTTPS://Example.COM/api?x=1"><b>API</b> 参考</a>
            <span style="display:none"><a href="https://hidden.example/secret">隐藏</a></span>
        "##;
        let links = extract_links(html, "https://example.com/page", 12);
        let urls: Vec<&str> = links
            .iter()
            .map(|item| item["url"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            urls,
            vec![
                "https://example.com/docs/guide",
                "https://example.com/api?x=1",
            ],
            "相对路径要绝对化、按归一化 URL 去重、丢掉脚本/邮件/fragment 锚点：{links:?}"
        );
        assert_eq!(links[0]["title"], json!("指南"));
        assert_eq!(
            links[1]["title"],
            json!("API 参考"),
            "锚文本要压平标签与空白"
        );
        // 上限生效（超长列表只取前 N 条）。
        let long = (0..20)
            .map(|index| format!("<a href=\"/p/{index}\">p{index}</a>"))
            .collect::<String>();
        assert_eq!(extract_links(&long, "https://example.com/", 5).len(), 5);
    }

    #[test]
    fn bing_parser_keeps_external_links_in_order() {
        let html = r#"
            <li class="b_algo"><h2><a href="https://first.example/1">第一条</a></h2></li>
            <li class="b_algo"><h2><a href="/internal">站内导航</a></h2></li>
            <li class="b_algo"><h2><a href="https://second.example/2">第二条</a></h2></li>
        "#;
        let results = parse_bing_results(html);
        let urls: Vec<&str> = results
            .iter()
            .map(|item| item["url"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            urls,
            vec!["https://first.example/1", "https://second.example/2"]
        );
        // 归一化后再喂给模型：站内锚点已经在解析层被丢掉。
        let normalized = normalize_search_results(results, 8);
        assert_eq!(normalized.len(), 2);
    }
}
