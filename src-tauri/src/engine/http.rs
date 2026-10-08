//! HTTP 取页与请求 URL 解析，对应原 Electron 版 src/main/legado/http.ts。

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use percent_encoding::{percent_encode, NON_ALPHANUMERIC, AsciiSet};
use regex::Regex;
use reqwest::{Client, Method};
use serde_json::Value;
use url::Url;

const DEFAULT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const DEFAULT_ACCEPT: &str = "text/html,application/json,application/xhtml+xml,*/*;q=0.8";
const DEFAULT_ACCEPT_LANGUAGE: &str = "zh-CN,zh;q=0.9,en;q=0.8";
const DEFAULT_TIMEOUT_MS: u64 = 15_000;

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
            .build()
            .expect("reqwest client")
    })
}

fn encode_uri_component(value: &str) -> String {
    percent_encode(value.as_bytes(), URI_COMPONENT_SET).to_string()
}

/// 发起请求并按字符集解码。取消由调用方在分页循环之间检查（丢弃 future 即中断在途请求）。
pub async fn fetch_text(url: &str, options: &FetchOptions) -> Result<Page, String> {
    let method_name = options.method.clone().unwrap_or_else(|| "GET".to_string());
    let method = Method::from_bytes(method_name.to_uppercase().as_bytes())
        .map_err(|_| format!("不支持的请求方法：{method_name}"))?;

    let has_accept = options
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("accept"));
    let has_accept_language = options
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("accept-language"));

    let mut builder = client().request(method.clone(), url);
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

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应失败：{final_url}（{e}）"))?;
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
fn detect_charset(content_type: Option<&str>, bytes: &[u8]) -> Option<String> {
    if let Some(ct) = content_type {
        if let Some(caps) = Regex::new(r"(?i)charset=([^\s;]+)")
            .ok()
            .and_then(|re| re.captures(ct))
        {
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
    if let Some(caps) = Regex::new(r#"(?i)charset=["']?([^\s"'/>]+)"#)
        .ok()
        .and_then(|re| re.captures(&head))
    {
        if let Some(m) = caps.get(1) {
            return Some(normalize_charset(m.as_str()));
        }
    }
    None
}

/// 规范化常见中文编码别名，对应 normalizeCharset。
fn normalize_charset(raw: &str) -> String {
    let lower = raw.to_lowercase().replace('"', "").replace('\'', "");
    match lower.as_str() {
        "gb2312" | "gbk" | "gb18030" | "gb-2312" => "gbk".to_string(),
        other => other.to_string(),
    }
}

fn decode_bytes(bytes: &[u8], charset: &str) -> String {
    let encoding = encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let (cow, _, _) = encoding.decode(bytes);
    cow.into_owned()
}

/// 解析 Legado 的 `searchUrl` / 请求串：支持 `{{key}}`、`{{(page+1)}}` 与 `,{...}` 选项段。
pub fn parse_request_url(raw: &str, vars: &HashMap<String, String>) -> (String, FetchOptions) {
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
        None => return (filled, FetchOptions::default()),
        Some(idx) => idx,
    };
    let url_part = filled[..comma].to_string();
    let opt_part = &filled[comma + 1..];
    let parsed: Value = match serde_json::from_str(opt_part) {
        Ok(v) => v,
        Err(_) => return (filled, FetchOptions::default()),
    };

    let mut body = parsed.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
    for (k, v) in vars {
        body = body.replace(&format!("{{{{{k}}}}}"), v);
        body = body.replace(&encode_uri_component(&format!("{{{{{k}}}}}")), v);
    }

    let mut headers = Vec::new();
    if let Some(obj) = parsed.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                headers.push((k.clone(), s.to_string()));
            }
        }
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
    (url_part, options)
}

/// 相对 URL 基于 base 解析，对应 resolveUrl。
pub fn resolve_url(base: &str, path: &str) -> String {
    if path.is_empty() {
        return base.to_string();
    }
    if path.starts_with("http://") || path.starts_with("https://") || path.contains("://") {
        return path.to_string();
    }
    match Url::parse(base)
        .and_then(|b| b.join(path))
        .or_else(|_| Url::parse(path))
    {
        Ok(u) => u.to_string(),
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
}
