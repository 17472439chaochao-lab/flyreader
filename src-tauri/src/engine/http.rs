//! HTTP 取页与请求 URL 解析，对应原 Electron 版 src/main/legado/http.ts。

use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

use percent_encoding::{percent_encode, NON_ALPHANUMERIC, AsciiSet};
use regex::Regex;
use reqwest::{
    redirect::{Attempt, Policy},
    Client, Method,
};
use serde_json::Value;
use url::Url;

const DEFAULT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const DEFAULT_ACCEPT: &str = "text/html,application/json,application/xhtml+xml,*/*;q=0.8";
const DEFAULT_ACCEPT_LANGUAGE: &str = "zh-CN,zh;q=0.9,en;q=0.8";
const DEFAULT_TIMEOUT_MS: u64 = 15_000;
/// 单次响应体上限 12MB：fetch_text 与 fetch_bytes_limited 共用，边读边计、超限即中断。
pub const MAX_FETCH_BYTES: usize = 12 * 1024 * 1024;
/// 请求体上限 64KB：parse_request_url 与 fetch_text 两处都检查。
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// 重定向最多 5 跳（含显式复检，见 client() 的自定义策略）。
const MAX_REDIRECTS: usize = 5;
/// 连接建立超时 5 秒（总超时仍由每次请求的 timeout 控制，默认 15 秒）。

/// `encodeURIComponent` 的未转义字符集：字母数字与 `-_.!~*'()`
const URI_COMPONENT_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// 请求选项（方法 / 头 / body / 字符集 / 超时）
#[derive(Clone, Default, Debug)]
pub struct FetchOptions {
    pub method: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub charset: Option<String>,
    pub timeout_ms: Option<u64>,
}

/// 一次取页结果
pub struct Page {
    pub text: String,
    pub final_url: String,
}

fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(DEFAULT_UA)
            .gzip(true)
            .brotli(true)
            .deflate(true)
            // 连接超时 5 秒；总超时仍由每次请求单独设（默认 15 秒）。
            .connect_timeout(Duration::from_secs(5))
            // 重定向显式限 5 跳，且每跳目标都复检同一套 SSRF 校验：
            // 校验失败或跳数超限即停止跟随，不会静默放行到内网地址。
            .redirect(Policy::custom(|attempt: Attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    return attempt.stop();
                }
                match validate_fetch_url(attempt.url().as_str()) {
                    Ok(_) => attempt.follow(),
                    Err(_) => attempt.stop(),
                }
            }))
            .build()
            .expect("reqwest client")
    })
}

/// 该 IP 是否内网/不可对外请求：loopback、私有、链路本地、保留、组播、未指定等一律拒绝。
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // 240.0.0.0/4（Class E 保留段，含 is_reserved 覆盖范围）
                || (o[0] & 0xf0) == 0xf0
                // 198.18.0.0/15（RFC 2544 基准测试段，即 is_benchmarking）
                || (o[0] == 198 && (o[1] & 0xfe) == 18)
                // 100.64.0.0/10（RFC 6598 运营商共享地址段，即 is_shared）
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                || o[0] == 0 // 0.0.0.0/8（本网络）
        }
        IpAddr::V6(v6) => {
            // IPv4 映射地址（::ffff:127.0.0.1）按内嵌的 IPv4 地址再判一次。
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_ip(&IpAddr::V4(mapped));
            }
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 链路本地
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 唯一本地（兜底）
        }
    }
}

/// 统一的取页 URL 校验（SSRF 防护），供 fetch_text、fetch_bytes_limited、
/// source_repo、resolve_url 后的请求入口共用：
/// 只允许 http/https；host 必须是域名或公网 IP；域名先 DNS 解析，
/// 解析出的任一地址落入内网段即拒绝（解析失败也拒绝）。
/// 注意：这里是请求前的校验，reqwest 发请求时会再次解析 DNS，
/// DNS 重绑定（TOCTOU）无法仅靠请求前校验彻底消除，只能收窄窗口。
pub(crate) fn validate_fetch_url(url: &str) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|e| format!("URL 解析失败：{url}（{e}）"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!("只允许 http/https 请求：{url}"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("URL 缺少 host：{url}"))?;
    if host.is_empty() {
        return Err(format!("URL 缺少 host：{url}"));
    }
    // 字面量 IP：直接判定，不走 DNS。
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip(&ip) {
            return Err(format!("拒绝请求内网/保留地址：{url}"));
        }
        return Ok(parsed);
    }
    // 域名：解析后逐个检查，任一内网地址即拒绝；解析失败即拒绝。
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs: Vec<std::net::SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS 解析失败，拒绝请求：{host}（{e}）"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("DNS 解析无结果，拒绝请求：{host}"));
    }
    for addr in &addrs {
        if is_blocked_ip(&addr.ip()) {
            return Err(format!("域名解析到内网/保留地址，拒绝请求：{host} -> {}", addr.ip()));
        }
    }
    Ok(parsed)
}

/// 方法白名单：只允许 GET/POST/PUT/DELETE/HEAD，其余一律拒绝。
fn validate_method(name: &str) -> Result<Method, String> {
    let upper = name.to_uppercase();
    match upper.as_str() {
        "GET" | "POST" | "PUT" | "DELETE" | "HEAD" => Method::from_bytes(upper.as_bytes())
            .map_err(|_| format!("不支持的请求方法：{name}")),
        _ => Err(format!("不支持的请求方法：{name}")),
    }
}

/// 流式读响应体：边读边计，累计超 max_bytes 即中断并返回错误，
/// 避免 `resp.bytes()` 把超大响应一次性读进内存。
/// 返回（字节，Content-Type 头，响应最终 URL）。
async fn read_body_limited(
    mut resp: reqwest::Response,
    max_bytes: usize,
    what: &str,
) -> Result<(Vec<u8>, Option<String>, String), String> {
    let final_url = resp.url().to_string();
    // Content-Length 已声明超限时直接拒绝，不必等读完再发现。
    if let Some(len) = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
    {
        if len > max_bytes {
            return Err(format!("响应超过上限（声明 {len} 字节>{max_bytes}）：{what}"));
        }
    }
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| format!("读取响应失败：{final_url}（{e}）"))?;
        let Some(bytes) = chunk else { break };
        buf.extend_from_slice(&bytes);
        if buf.len() > max_bytes {
            return Err(format!(
                "响应超过上限（>{max_bytes} 字节），已中断：{what}"
            ));
        }
    }
    Ok((buf, content_type, final_url))
}

/// 给 A 路在线封面本地化用的可复用下载函数（签名冻结）：
/// 先走 validate_fetch_url 校验，流式读取、超 max_bytes 中断，
/// 返回（字节，Content-Type 头）。
pub async fn fetch_bytes_limited(
    url: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, Option<String>), String> {
    let validated = validate_fetch_url(url)?;
    let resp = client()
        .get(validated.clone())
        .timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!("请求超时（{DEFAULT_TIMEOUT_MS}ms）：{url}")
            } else {
                format!("请求失败：{url}（{e}）")
            }
        })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}：{}", status.as_u16(), validated));
    }
    let (bytes, content_type, _) = read_body_limited(resp, max_bytes, url).await?;
    Ok((bytes, content_type))
}

fn encode_uri_component(value: &str) -> String {
    percent_encode(value.as_bytes(), URI_COMPONENT_SET).to_string()
}

/// 发起请求并按字符集解码。取消由调用方在分页循环之间检查（丢弃 future 即中断在途请求）。
/// 所有外发请求都先过 validate_fetch_url（SSRF 校验），响应流式读取、上限 12MB。
pub async fn fetch_text(url: &str, options: &FetchOptions) -> Result<Page, String> {
    let method_name = options.method.clone().unwrap_or_else(|| "GET".to_string());
    let method = validate_method(&method_name)?;
    // 请求体限长 64KB：超长直接拒绝，不发请求。
    if let Some(body) = &options.body {
        if body.len() > MAX_BODY_BYTES {
            return Err(format!(
                "请求体超过上限（{} 字节>{MAX_BODY_BYTES}）：{url}",
                body.len()
            ));
        }
    }
    let validated = validate_fetch_url(url)?;

    let has_accept = options
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("accept"));
    let has_accept_language = options
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("accept-language"));

    let mut builder = client().request(method.clone(), validated.clone());
    // 书源若自带 Accept，不再追加默认值，避免与 application/json 拼成双 Accept
    // （部分站点会因此返回 JSON 字符串化的 HTML）。
    if !has_accept {
        builder = builder.header("accept", DEFAULT_ACCEPT);
    }
    if !has_accept_language {
        builder = builder.header("accept-language", DEFAULT_ACCEPT_LANGUAGE);
    }
    for (k, v) in &options.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    let body = options.body.clone().unwrap_or_default();
    if !body.is_empty() && method != Method::GET && method != Method::HEAD {
        builder = builder.body(body);
        if !options
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            builder = builder.header("content-type", "application/x-www-form-urlencoded");
        }
    }

    let timeout_ms = options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let sent = builder.timeout(Duration::from_millis(timeout_ms)).send().await;
    let resp = sent.map_err(|e| {
        if e.is_timeout() {
            format!("请求超时（{timeout_ms}ms）：{url}")
        } else {
            format!("请求失败：{url}（{}）", e)
        }
    })?;

    let status = resp.status();
    let final_url = resp.url().to_string();
    if !status.is_success() {
        let reason = status.canonical_reason().unwrap_or("");
        return Err(format!(
            "HTTP {}{}：{}",
            status.as_u16(),
            if reason.is_empty() {
                String::new()
            } else {
                format!(" {reason}")
            },
            final_url
        ));
    }

    // 流式读取：边读边计，累计超 12MB 即中断（旧实现 resp.bytes() 会一次性读满内存）。
    // 重定向后的最终 URL 已在 client() 的自定义策略里逐跳复检过 SSRF 校验。
    let (bytes, content_type, final_url) =
        read_body_limited(resp, MAX_FETCH_BYTES, &final_url).await?;
    let charset = options
        .charset
        .clone()
        .filter(|c| !c.trim().is_empty())
        .or_else(|| detect_charset(content_type.as_deref(), &bytes));
    let text = decode_bytes(&bytes, charset.as_deref().unwrap_or("utf-8"));
    Ok(Page {
        // 部分站点在 Accept 含 application/json 时，把整页 HTML 当成 JSON 字符串返回
        // （形如 "\"<!DOCTYPE html>...\\/table>\""）。解包后才能走 CSS/XPath 规则。
        text: maybe_unwrap_json_string(text),
        final_url,
    })
}

/// 若响应体是 JSON 字符串（非对象/数组），解包为内层文本；否则原样返回。
fn maybe_unwrap_json_string(text: String) -> String {
    let trimmed = text.trim();
    if !trimmed.starts_with('"') {
        return text;
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::String(inner)) if !inner.is_empty() => inner,
        _ => text,
    }
}

/// 从 Content-Type 或 HTML meta 检测字符集，对应 detectCharset。
/// 两个正则用 OnceLock 静态编译，不要每次调用都 Regex::new。
fn detect_charset(content_type: Option<&str>, bytes: &[u8]) -> Option<String> {
    static CT_RE: OnceLock<Regex> = OnceLock::new();
    static META_RE: OnceLock<Regex> = OnceLock::new();
    let ct_re = CT_RE.get_or_init(|| {
        Regex::new(r"(?i)charset=([^\s;]+)").expect("charset regex")
    });
    let meta_re = META_RE.get_or_init(|| {
        Regex::new(r#"(?i)charset=["']?([^\s"'/>]+)"#).expect("charset regex")
    });
    if let Some(ct) = content_type {
        if let Some(caps) = ct_re.captures(ct) {
            if let Some(m) = caps.get(1) {
                return Some(normalize_charset(m.as_str()));
            }
        }
    }
    let head: String = bytes
        .iter()
        .take(2048)
        .map(|b| if *b < 0x80 { *b as char } else { '\u{fffd}' })
        .collect();
    if let Some(caps) = meta_re.captures(&head) {
        if let Some(m) = caps.get(1) {
            return Some(normalize_charset(m.as_str()));
        }
    }
    None
}

/// 规范化常见中文编码别名，对应 normalizeCharset。
/// 注意：gb18030 不要并入 gbk——GB18030 字符集比 GBK 大（含四字节字），
/// 并入 gbk 会让 GB18030 专有字解码失败；两者在 encoding_rs 里是不同编码。
fn normalize_charset(raw: &str) -> String {
    let lower = raw.to_lowercase().replace('"', "").replace('\'', "");
    match lower.as_str() {
        "gb2312" | "gbk" | "gb-2312" => "gbk".to_string(),
        "gb18030" => "gb18030".to_string(),
        other => other.to_string(),
    }
}

fn decode_bytes(bytes: &[u8], charset: &str) -> String {
    let encoding = encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let (cow, _, _) = encoding.decode(bytes);
    cow.into_owned()
}

/// 解析 Legado 的 `searchUrl` / 请求串：支持 `{{key}}`、`{{(page+1)}}` 与 `,{...}` 选项段。
/// 请求体超 64KB 或方法不在白名单时返回 Err（调用方用 `?` 透传，见 legado.rs）。
pub fn parse_request_url(
    raw: &str,
    vars: &HashMap<String, String>,
) -> Result<(String, FetchOptions), String> {
    let mut filled = raw.to_string();
    for (k, v) in vars {
        filled = filled.replace(&format!("{{{{{k}}}}}"), &encode_uri_component(v));
    }

    let page: f64 = vars.get("page").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    if let Ok(re) = Regex::new(r"(?s)\{\{\(([^}]+)\)\}\}") {
        filled = re
            .replace_all(&filled, |caps: &regex::Captures| {
                eval_arithmetic(&caps[1], page).unwrap_or_default()
            })
            .to_string();
    }

    let comma = filled.find(",{");
    let comma = match comma {
        None => return Ok((filled, FetchOptions::default())),
        Some(idx) => idx,
    };
    let url_part = filled[..comma].to_string();
    let opt_part = &filled[comma + 1..];
    let parsed: Value = match serde_json::from_str(opt_part) {
        Ok(v) => v,
        Err(_) => return Ok((filled, FetchOptions::default())),
    };

    let mut body = parsed.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
    for (k, v) in vars {
        body = body.replace(&format!("{{{{{k}}}}}"), v);
        body = body.replace(&encode_uri_component(&format!("{{{{{k}}}}}")), v);
    }
    // 请求体限长 64KB：书源选项段里塞超大 body 应直接报错，而不是发出去。
    if body.len() > MAX_BODY_BYTES {
        return Err(format!(
            "请求体超过上限（{} 字节>{MAX_BODY_BYTES}）",
            body.len()
        ));
    }

    let mut headers = Vec::new();
    if let Some(obj) = parsed.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                headers.push((k.clone(), s.to_string()));
            }
        }
    }

    // 方法白名单只允许 GET/POST/PUT/DELETE/HEAD（fetch_text 里还会再检一次）。
    if let Some(method) = parsed.get("method").and_then(|v| v.as_str()) {
        validate_method(method)?;
    }

    let options = FetchOptions {
        method: parsed
            .get("method")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        headers,
        body: Some(body),
        charset: parsed
            .get("charset")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        timeout_ms: None,
    };
    Ok((url_part, options))
}

/// 相对 URL 基于 base 解析，对应 resolveUrl。
/// 非 http/https 一律拒绝：`file:`/`ftp:`/`data:`/`javascript:` 等带协议头的目标
/// 不解析成该目标，退回 base（base 为空时返回空串），不要静默放行；
/// 解析后结果仍不是 http/https 时同样退回 base。
/// 注意：返回 String 而非 Result 是为保持各调用点签名不变，
/// 真正的请求级校验在 fetch_text 入口由 validate_fetch_url 做。
pub fn resolve_url(base: &str, path: &str) -> String {
    if path.is_empty() {
        return base.to_string();
    }
    let lower = path.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return path.to_string();
    }
    if path.contains("://") {
        return base.to_string();
    }
    match Url::parse(base)
        .and_then(|b| b.join(path))
        .or_else(|_| Url::parse(path))
    {
        Ok(u) if u.scheme() == "http" || u.scheme() == "https" => u.to_string(),
        Ok(_) => base.to_string(),
        Err(_) => path.to_string(),
    }
}

/// `{{(...)}}` 内仅允许数字与 + - * / 括号，绑定 page；含字符串绑定或非法字符返回 None。
/// 对应 evalArithmeticExpr（不使用 eval，从根本上消除代码执行）。
fn eval_arithmetic(expr: &str, page: f64) -> Option<String> {
    #[derive(Clone, Copy)]
    enum Tok {
        Num(f64),
        Op(char),
    }

    let mut tokens: Vec<Tok> = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                j += 1;
            }
            let num: f64 = chars[i..j].iter().collect::<String>().parse().ok()?;
            tokens.push(Tok::Num(num));
            i = j;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            i = j;
            if word == "page" {
                tokens.push(Tok::Num(page));
            } else {
                // 与原版一致：只有 page 可做算术，含 key 等字符串绑定直接放弃
                return None;
            }
            continue;
        }
        if matches!(c, '+' | '-' | '*' | '/' | '(' | ')') {
            tokens.push(Tok::Op(c));
            i += 1;
            continue;
        }
        return None;
    }
    if tokens.is_empty() {
        return None;
    }

    let mut pos = 0usize;
    fn peek(tokens: &[Tok], pos: usize) -> Option<Tok> {
        tokens.get(pos).copied()
    }
    fn parse_factor(tokens: &[Tok], pos: &mut usize) -> f64 {
        match peek(tokens, *pos) {
            None => f64::NAN,
            Some(Tok::Op('+')) => {
                *pos += 1;
                parse_factor(tokens, pos)
            }
            Some(Tok::Op('-')) => {
                *pos += 1;
                -parse_factor(tokens, pos)
            }
            Some(Tok::Op('(')) => {
                *pos += 1;
                let v = parse_expr(tokens, pos);
                if matches!(peek(tokens, *pos), Some(Tok::Op(')'))) {
                    *pos += 1;
                    v
                } else {
                    f64::NAN
                }
            }
            Some(Tok::Num(n)) => {
                *pos += 1;
                n
            }
            Some(_) => f64::NAN,
        }
    }
    fn parse_term(tokens: &[Tok], pos: &mut usize) -> f64 {
        let mut v = parse_factor(tokens, pos);
        loop {
            match peek(tokens, *pos) {
                Some(Tok::Op('*')) => {
                    *pos += 1;
                    v *= parse_factor(tokens, pos);
                }
                Some(Tok::Op('/')) => {
                    *pos += 1;
                    let rhs = parse_factor(tokens, pos);
                    v = if rhs == 0.0 { f64::NAN } else { v / rhs };
                }
                _ => return v,
            }
        }
    }
    fn parse_expr(tokens: &[Tok], pos: &mut usize) -> f64 {
        let mut v = parse_term(tokens, pos);
        loop {
            match peek(tokens, *pos) {
                Some(Tok::Op('+')) => {
                    *pos += 1;
                    v += parse_term(tokens, pos);
                }
                Some(Tok::Op('-')) => {
                    *pos += 1;
                    v -= parse_term(tokens, pos);
                }
                _ => return v,
            }
        }
    }

    let result = parse_expr(&tokens, &mut pos);
    if result.is_nan() || !result.is_finite() || pos != tokens.len() {
        return None;
    }
    Some(if result == result.trunc() && result.abs() < 1e15 {
        format!("{}", result as i64)
    } else {
        result.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwrap_json_stringified_html() {
        let raw = serde_json::to_string("<table class=\"table\"><tr><td>ok</td></tr></table>").unwrap();
        let out = maybe_unwrap_json_string(raw);
        assert!(out.starts_with("<table"), "out={out}");
        assert!(out.contains("class=\"table\""));
    }

    #[test]
    fn leave_json_object_alone() {
        let raw = r#"{"data":[{"name":"剑来"}]}"#.to_string();
        let out = maybe_unwrap_json_string(raw.clone());
        assert_eq!(out, raw);
    }

    #[test]
    fn leave_plain_html_alone() {
        let raw = "<html><body>hi</body></html>".to_string();
        assert_eq!(maybe_unwrap_json_string(raw.clone()), raw);
    }

    #[test]
    fn validate_fetch_url_rejects_non_http_and_private_ips() {
        // 非 http/https 一律拒绝（DNS 都不走）
        assert!(validate_fetch_url("file:///etc/passwd").is_err());
        assert!(validate_fetch_url("ftp://example.com/x").is_err());
        assert!(validate_fetch_url("javascript:alert(1)").is_err());
        assert!(validate_fetch_url("").is_err());
        // 字面量内网 IP：loopback / 私有 / 链路本地（用公网 IP 字面量断言放行，避免测试依赖 DNS）
        assert!(validate_fetch_url("http://127.0.0.1/").is_err());
        assert!(validate_fetch_url("http://10.0.0.5/x").is_err());
        assert!(validate_fetch_url("http://192.168.1.1/x").is_err());
        assert!(validate_fetch_url("http://172.16.0.9/x").is_err());
        assert!(validate_fetch_url("http://169.254.1.2/x").is_err());
        assert!(validate_fetch_url("http://[::1]/x").is_err());
        assert!(validate_fetch_url("http://[fe80::1]/x").is_err());
        assert!(validate_fetch_url("http://[::ffff:127.0.0.1]/x").is_err());
        assert!(validate_fetch_url("https://8.8.8.8/x").is_ok());
        assert!(validate_fetch_url("https://1.1.1.1/x").is_ok());
    }

    #[test]
    fn resolve_url_refuses_non_http_schemes() {
        let base = "https://www.example.com/book/1/";
        assert_eq!(resolve_url(base, "file:///etc/passwd"), base);
        assert_eq!(resolve_url(base, "ftp://x/y"), base);
        assert_eq!(resolve_url(base, "javascript:alert(1)"), base);
        assert_eq!(resolve_url(base, "data:text/html,hi"), base);
        // 绝对 http(s) 与相对路径保持原行为
        assert_eq!(
            resolve_url(base, "https://cdn.example.com/c.jpg"),
            "https://cdn.example.com/c.jpg"
        );
        assert_eq!(
            resolve_url(base, "../cover.jpg"),
            "https://www.example.com/book/cover.jpg"
        );
    }

    #[test]
    fn parse_request_url_rejects_oversize_body_and_bad_method() {
        let vars: HashMap<String, String> = HashMap::new();
        let big_body = "x".repeat(MAX_BODY_BYTES + 1);
        let raw = format!("https://a.example.com/search,{{\"method\":\"POST\",\"body\":\"{big_body}\"}}");
        assert!(parse_request_url(&raw, &vars).is_err());
        let raw = "https://a.example.com/search,{\"method\":\"TRACE\"}".to_string();
        assert!(parse_request_url(&raw, &vars).is_err());
        // 正常解析仍返回 Ok
        let (url, opts) =
            parse_request_url("https://a.example.com/s?q={{key}},{\"method\":\"POST\"}", &{
                let mut m = HashMap::new();
                m.insert("key".to_string(), "剑来".to_string());
                m
            })
            .unwrap();
        assert!(url.contains("a.example.com"));
        assert_eq!(opts.method.as_deref(), Some("POST"));
    }

    #[test]
    fn gb18030_is_kept_separate_from_gbk() {
        assert_eq!(normalize_charset("gb18030"), "gb18030");
        assert_eq!(normalize_charset("GB18030"), "gb18030");
        assert_eq!(normalize_charset("gbk"), "gbk");
        assert_eq!(normalize_charset("gb2312"), "gbk");
    }
}
