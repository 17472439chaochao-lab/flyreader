//! Legado 书源解析主流程，对应原 Electron 版 src/main/legado/engine.ts。
//!
//! 书源 JSON 直接以 `serde_json::Value` 透传（Legado 字段极多且松散），
//! 取值用可选链式助手函数模拟原版的 `source.ruleSearch?.bookList` 写法。

use std::collections::HashMap;

use regex::Regex;
use serde_json::{json, Map, Value};

use super::http::{fetch_text, parse_request_url, resolve_url, FetchOptions};
use super::rules::{content_rule_as_html, get_elements, get_string};
use super::text::{ensure_novel_paragraphs, normalize_line_endings};

/// 目录分页上限，与原版一致
const MAX_TOC_PAGES: usize = 30;
/// 正文分页上限，与原版一致
const MAX_CONTENT_PAGES: usize = 20;

fn str_field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = value;
    for key in path {
        cur = cur.get(*key)?;
    }
    cur.as_str()
}

/// 解析书源 `header` 字段为请求头键值。
///
/// 原版只做 `JSON.parse`，失败即整份丢弃；而 Legado 书源普遍写成 JS 对象字面量
/// （单引号 / 注释 / 尾逗号，如「猫眼看书」），于是 `client-*` 与 `Authorization`
/// 全都发不出去。**这里是 FlyReader 刻意偏离原版的一处**：JSON 解析失败时
/// 先把 JS 字面量规范化成 JSON 再试一次。
pub fn parse_header(header: Option<&str>) -> Vec<(String, String)> {
    let Some(raw) = header.map(str::trim).filter(|h| !h.is_empty()) else {
        return Vec::new();
    };
    let parsed = serde_json::from_str::<Value>(raw)
        .or_else(|_| serde_json::from_str::<Value>(&js_object_literal_to_json(raw)));
    match parsed.unwrap_or(Value::Null) {
        Value::Object(obj) => obj
            .into_iter()
            .map(|(k, v)| (k, v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
            .collect(),
        _ => Vec::new(),
    }
}

/// JS 对象字面量 → 可被 JSON 解析的文本：单引号串转双引号串、去注释、去尾逗号。
fn js_object_literal_to_json(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\'' | '"' => {
                let quote = chars[i];
                i += 1;
                let mut body = String::new();
                while i < chars.len() {
                    let ch = chars[i];
                    if ch == '\\' && i + 1 < chars.len() {
                        let next = chars[i + 1];
                        // 单引号串里的 \' 在 JSON 中是非法转义，还原成裸 '
                        if !(quote == '\'' && next == '\'') {
                            body.push('\\');
                        }
                        body.push(next);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    if ch == quote {
                        break;
                    }
                    if ch == '"' {
                        body.push('\\');
                        body.push('"');
                    } else {
                        body.push(ch);
                    }
                }
                out.push('"');
                out.push_str(&body);
                out.push('"');
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
            }
            ',' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if !matches!(chars.get(j), Some('}') | Some(']')) {
                    out.push(',');
                }
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = s.replace("&nbsp;", " ");
    out = out.replace("&lt;", "<");
    out = out.replace("&gt;", ">");
    out = out.replace("&amp;", "&");
    out = out.replace("&quot;", "\"");
    out = out.replace("&#39;", "'");
    if let Ok(re) = Regex::new(r"(?i)&#x([0-9a-f]+);") {
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                let code = u32::from_str_radix(&caps[1], 16).unwrap_or(0);
                char::from_u32(code).map(|c| c.to_string()).unwrap_or_default()
            })
            .to_string();
    }
    if let Ok(re) = Regex::new(r"&#(\d+);") {
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                let code = caps[1].parse::<u32>().unwrap_or(0);
                char::from_u32(code).map(|c| c.to_string()).unwrap_or_default()
            })
            .to_string();
    }
    out
}

/// 章节 HTML/文本 → 空行分隔的小说正文，对应 htmlToNovelText。
pub fn html_to_novel_text(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let mut s = raw.to_string();

    if Regex::new(r"(?i)<\/?[a-z][\s\S]*>")
        .map(|re| re.is_match(&s))
        .unwrap_or(false)
    {
        for (pattern, replacement) in [
            (r"(?is)<(script|style)[^>]*>.*?</\1>", ""),
            (r"(?is)<!--.*?-->", ""),
            (r"(?i)<br\s*/?>", "\n"),
            (
                r"(?i)</?(p|div|section|article|tr|li|h[1-6]|blockquote)(\s[^>]*)?>",
                "\n",
            ),
            (r"(?s)<[^>]+>", ""),
        ] {
            if let Ok(re) = Regex::new(pattern) {
                s = re.replace_all(&s, replacement).to_string();
            }
        }
    }

    s = decode_entities(&s);
    s = normalize_line_endings(&s);
    if let Ok(re) = Regex::new("\u{3000}{2,}") {
        s = re.replace_all(&s, "\n").to_string();
    }
    if let Ok(re) = Regex::new(r"(^|\n)\u{3000}+") {
        s = re.replace_all(&s, "$1").to_string();
    }

    let lines: Vec<String> = s
        .split('\n')
        .map(|line| {
            let collapsed = match Regex::new(r"[ \t\u{c}\u{b}]+") {
                Ok(re) => re.replace_all(line, " ").to_string(),
                Err(_) => line.to_string(),
            };
            collapsed.trim().to_string()
        })
        .filter(|line| !line.is_empty())
        .collect();

    ensure_novel_paragraphs(&lines.join("\n\n"))
}

/// 按书源搜索规则检索书籍列表，对应 searchBooks。
pub async fn search_books(source: &Value, key: &str, page: u32) -> Result<Vec<Value>, String> {
    let search_url = str_field(source, &["searchUrl"]).unwrap_or("");
    let book_list_rule = str_field(source, &["ruleSearch", "bookList"]).unwrap_or("");
    if search_url.is_empty() || book_list_rule.is_empty() {
        return Ok(Vec::new());
    }

    let base = str_field(source, &["bookSourceUrl"]).unwrap_or("");
    let headers = parse_header(str_field(source, &["header"]));
    let mut vars = HashMap::new();
    vars.insert("key".to_string(), key.to_string());
    vars.insert("page".to_string(), page.to_string());
    let (url, options) = parse_request_url(search_url, &vars);
    let full_url = resolve_url(base, &url);

    let page_result = fetch_text(&full_url, &merge_headers(options, &headers)).await?;
    let text = page_result.text;
    let final_url = page_result.final_url;

    let items = get_elements(&text, Some(book_list_rule));
    let mut books = Vec::new();
    for item in items {
        let content = wrap_item(&item);
        let name = get_string(&content, str_field(source, &["ruleSearch", "name"]), &final_url);
        let book_url_raw =
            get_string(&content, str_field(source, &["ruleSearch", "bookUrl"]), &final_url);
        if name.is_empty() || book_url_raw.is_empty() {
            continue;
        }
        let mut book = Map::new();
        book.insert("name".into(), json!(name));
        book.insert(
            "author".into(),
            json!(get_string(
                &content,
                str_field(source, &["ruleSearch", "author"]),
                &final_url
            )),
        );
        book.insert("bookUrl".into(), json!(resolve_url(&final_url, &book_url_raw)));
        for (field, rule) in [
            ("coverUrl", "coverUrl"),
            ("intro", "intro"),
            ("kind", "kind"),
            ("lastChapter", "lastChapter"),
            ("wordCount", "wordCount"),
        ] {
            let val = get_string(&content, str_field(source, &["ruleSearch", rule]), &final_url);
            if !val.is_empty() {
                book.insert(field.into(), json!(val));
            }
        }
        book.insert("origin".into(), json!(base));
        book.insert(
            "originName".into(),
            json!(str_field(source, &["bookSourceName"]).unwrap_or("")),
        );
        books.push(Value::Object(book));
    }
    Ok(books)
}

/// JSON 条目原样解析，HTML 片段包一层容器以便嵌套取值（与原版一致）。
///
/// 表格行/单元格不能塞进 `<div>`：html5ever 会走 foster parenting，把 `<td>` 挪出
/// 包裹层，导致 `td a@text` 等子选择器取空。改用 `<table>`（单元格再包一层 `<tr>`）。
fn wrap_item(item: &str) -> String {
    let trimmed = item.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return item.to_string();
    }
    let head: String = trimmed.chars().take(8).collect::<String>().to_ascii_lowercase();
    if head.starts_with("<tr")
        || head.starts_with("<thead")
        || head.starts_with("<tbody")
        || head.starts_with("<tfoot")
    {
        format!("<table id=\"__wrap__\">{item}</table>")
    } else if head.starts_with("<td") || head.starts_with("<th") {
        format!("<table id=\"__wrap__\"><tr>{item}</tr></table>")
    } else {
        format!("<div id=\"__wrap__\">{item}</div>")
    }
}

/// 按 `init` 规则重设解析根；取不到值就退回原内容。
fn apply_init_rule(content: &str, init_rule: &str, base_url: &str) -> String {
    if init_rule.is_empty() {
        return content.to_string();
    }
    let rooted = get_string(content, Some(init_rule), base_url);
    if rooted.is_empty() {
        return content.to_string();
    }
    rooted
}

fn merge_headers(mut options: FetchOptions, headers: &[(String, String)]) -> FetchOptions {
    let mut merged: Vec<(String, String)> = headers.to_vec();
    merged.append(&mut options.headers);
    options.headers = merged;
    options
}

/// 抓取书籍详情页，对应 getBookInfo。
pub async fn get_book_info(source: &Value, book_url: &str) -> Result<Value, String> {
    let headers = parse_header(str_field(source, &["header"]));
    let options = FetchOptions {
        headers,
        ..Default::default()
    };
    let fetched = fetch_text(book_url, &options).await?;
    let base = fetched.final_url.clone();
    let text = fetched.text;

    // ruleBookInfo.init 把响应重设成真正的对象根（API 书源常写成 "$.data"）。
    // 原版完全忽略它，于是详情一律取空、tocUrl 少一层 ID —— 这里是第二处刻意偏离。
    let init_rule = str_field(source, &["ruleBookInfo", "init"]).unwrap_or("");
    let content = apply_init_rule(&text, init_rule, &base);

    let pick = |field: &str| -> String {
        get_string(&content, str_field(source, &["ruleBookInfo", field]), &base)
    };

    let mut info = Map::new();
    let name = pick("name");
    info.insert("name".into(), json!(if name.is_empty() { "未知书名" } else { &name }));
    let author = pick("author");
    info.insert("author".into(), json!(if author.is_empty() { "佚名" } else { &author }));
    info.insert("bookUrl".into(), json!(&base));
    for field in ["coverUrl", "intro", "kind", "lastChapter", "wordCount"] {
        let val = pick(field);
        if !val.is_empty() {
            info.insert(field.into(), json!(val));
        }
    }
    let toc_url = pick("tocUrl");
    let toc_url = if toc_url.is_empty() {
        base.clone()
    } else {
        resolve_url(&base, &toc_url)
    };
    info.insert("tocUrl".into(), json!(toc_url));
    info.insert(
        "origin".into(),
        json!(str_field(source, &["bookSourceUrl"]).unwrap_or("")),
    );
    info.insert(
        "originName".into(),
        json!(str_field(source, &["bookSourceName"]).unwrap_or("")),
    );
    Ok(Value::Object(info))
}

/// 抓取目录（支持 nextTocUrl 分页，最多 30 页），对应 getChapterList。
pub async fn get_chapter_list(source: &Value, toc_url: &str) -> Result<Vec<Value>, String> {
    get_chapter_list_with_budget(source, toc_url, None).await
}

/// 抓取目录，`per_page_budget` 给每次分页请求加超时（整体由调用方按总预算控制）。
pub async fn get_chapter_list_with_budget(
    source: &Value,
    toc_url: &str,
    per_page_budget: Option<std::time::Duration>,
) -> Result<Vec<Value>, String> {
    let headers = parse_header(str_field(source, &["header"]));
    let chapter_list_rule = str_field(source, &["ruleToc", "chapterList"]).unwrap_or("");
    if chapter_list_rule.is_empty() {
        return Ok(Vec::new());
    }

    let mut chapters: Vec<Value> = Vec::new();
    let mut next: Option<String> = Some(toc_url.to_string());
    let mut guard = 0usize;

    while let Some(url) = next.take() {
        if guard >= MAX_TOC_PAGES {
            break;
        }
        guard += 1;

        let options = FetchOptions {
            headers: headers.clone(),
            ..Default::default()
        };
        let fut = fetch_text(&url, &options);
        let fetched = match per_page_budget {
            Some(budget) => tokio::time::timeout(budget, fut)
                .await
                .map_err(|_| format!("请求超时：{url}"))?,
            None => fut.await,
        }?;
        let base = fetched.final_url.clone();
        let text = fetched.text;

        for item in get_elements(&text, Some(chapter_list_rule)) {
            let content = wrap_item(&item);
            let title =
                get_string(&content, str_field(source, &["ruleToc", "chapterName"]), &base);
            let url_raw =
                get_string(&content, str_field(source, &["ruleToc", "chapterUrl"]), &base);
            let is_volume = str_field(source, &["ruleToc", "isVolume"])
                .map(|rule| !get_string(&content, Some(rule), &base).is_empty())
                .unwrap_or(false);
            if title.is_empty() {
                continue;
            }
            let mut chapter = Map::new();
            chapter.insert("title".into(), json!(title));
            chapter.insert(
                "url".into(),
                json!(if url_raw.is_empty() {
                    String::new()
                } else {
                    resolve_url(&base, &url_raw)
                }),
            );
            chapter.insert("index".into(), json!(chapters.len()));
            chapter.insert("isVolume".into(), json!(is_volume));
            if let Some(rule) = str_field(source, &["ruleToc", "updateTime"]) {
                let updated = get_string(&content, Some(rule), &base);
                if !updated.is_empty() {
                    chapter.insert("updateTime".into(), json!(updated));
                }
            }
            chapters.push(Value::Object(chapter));
        }

        if let Some(rule) = str_field(source, &["ruleToc", "nextTocUrl"]) {
            let n = get_string(&text, Some(rule), &base);
            let resolved = if n.is_empty() {
                None
            } else {
                Some(resolve_url(&base, &n))
            };
            next = resolved.filter(|u| u != &base);
        }
    }

    // 卷名行若没有链接则丢弃，避免目录出现点不动的空行
    chapters.retain(|c| {
        c.get("isVolume").and_then(Value::as_bool).unwrap_or(false) == false
            || !c.get("url").and_then(Value::as_str).unwrap_or("").is_empty()
    });
    Ok(chapters)
}

/// 抓取正文（支持 nextContentUrl 分页，最多 20 页），对应 doGetContent。
pub async fn get_content(source: &Value, chapter_url: &str) -> Result<String, String> {
    let headers = parse_header(str_field(source, &["header"]));
    let content_rule = str_field(source, &["ruleContent", "content"]).unwrap_or("");
    if content_rule.is_empty() {
        return Ok(String::new());
    }

    let mut parts: Vec<String> = Vec::new();
    let mut next: Option<String> = Some(chapter_url.to_string());
    let mut seen: Vec<String> = Vec::new();
    let mut guard = 0usize;

    while let Some(url) = next.take() {
        if guard >= MAX_CONTENT_PAGES || seen.contains(&url) {
            break;
        }
        guard += 1;
        seen.push(url.clone());

        let options = FetchOptions {
            headers: headers.clone(),
            ..Default::default()
        };
        let fetched = fetch_text(&url, &options).await?;
        let base = fetched.final_url.clone();
        let mut html = fetched.text;

        if let Some(pattern) = str_field(source, &["ruleContent", "sourceRegex"]) {
            if let Some(re) = super::rules::get_regex(pattern, "i") {
                if re.find(&html).is_some() {
                    // 原版取 m[0]（首个整体命中）
                    html = re.find(&html).map(|m| m.as_str().to_string()).unwrap_or(html);
                }
            }
        }

        let mut content = get_string(
            &html,
            Some(&content_rule_as_html(content_rule)),
            &base,
        );

        if let Some(replace_regex) = str_field(source, &["ruleContent", "replaceRegex"]) {
            for seg in replace_regex.split('\n').filter(|s| !s.is_empty()) {
                let stripped = seg.strip_prefix("##").unwrap_or(&seg);
                let prepared = format!("x##{stripped}");
                let hash_parts: Vec<&str> = prepared.split("##").skip(1).collect();
                let mut result = content.clone();
                let mut i = 0;
                while i < hash_parts.len() {
                    let pattern = hash_parts[i];
                    let replacement = hash_parts.get(i + 1).copied().unwrap_or("");
                    i += 2;
                    if let Some(re) = super::rules::get_regex(pattern, "g") {
                        result = re.replace_all(&result, replacement).to_string();
                    }
                }
                content = result;
            }
        }

        let formatted = html_to_novel_text(&content);
        if !formatted.is_empty() {
            parts.push(formatted);
        }

        if let Some(rule) = str_field(source, &["ruleContent", "nextContentUrl"]) {
            let n = get_string(&html, Some(rule), &base);
            let resolved = if n.is_empty() {
                None
            } else {
                Some(resolve_url(&base, &n))
            };
            next = resolved.filter(|u| !seen.contains(u));
        }
    }

    Ok(parts.join("\n\n"))
}

/// 规范化导入的书源 JSON：过滤无效项，仅保留文本书源，对应 normalizeSources。
pub fn normalize_sources(input: Value) -> Vec<Value> {
    let list: Vec<Value> = match input {
        Value::Array(items) => items,
        other @ Value::Object(_) => vec![other],
        _ => Vec::new(),
    };
    list.into_iter()
        .filter_map(|item| {
            let mut source = item.as_object()?.clone();
            let has_url = source
                .get("bookSourceUrl")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
            let has_name = source
                .get("bookSourceName")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
            if !has_url || !has_name {
                return None;
            }
            // 仅接受文本书源（bookSourceType 缺省或 0）
            match source.get("bookSourceType") {
                None | Some(Value::Null) => {}
                Some(Value::Number(n)) if n.as_f64() == Some(0.0) => {}
                _ => return None,
            }
            let enabled = source
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            source.insert("enabled".into(), json!(enabled));
            source.insert("bookSourceType".into(), json!(0));
            Some(Value::Object(source))
        })
        .collect()
}

/// 书源结构自检：搜索流程至少要能跑起来，对应 isSourceStructuralOk。
pub fn is_source_structural_ok(source: &Value) -> bool {
    !str_field(source, &["searchUrl"]).unwrap_or("").is_empty()
        && !str_field(source, &["ruleSearch", "bookList"]).unwrap_or("").is_empty()
        && !str_field(source, &["ruleSearch", "name"]).unwrap_or("").is_empty()
        && !str_field(source, &["ruleSearch", "bookUrl"]).unwrap_or("").is_empty()
}

/// 生成书源测试的关键词候选，对应 buildTestKeywords。
pub fn build_test_keywords(source: &Value, keyword: Option<&str>, history: &[String]) -> Vec<String> {
    const MAX_TEST_KEYWORDS: usize = 3;
    const FALLBACKS: [&str; 6] = ["剑来", "遮天", "斗破苍穹", "完美世界", "庆余年", "雪中悍刀行"];

    let mut list: Vec<String> = Vec::new();
    push_keyword(&mut list, keyword);
    push_keyword(&mut list, str_field(source, &["ruleSearch", "checkKeyWord"]));
    for h in history {
        if list.len() >= MAX_TEST_KEYWORDS {
            break;
        }
        push_keyword(&mut list, Some(h));
    }
    for f in FALLBACKS {
        if list.len() >= MAX_TEST_KEYWORDS {
            break;
        }
        push_keyword(&mut list, Some(f));
    }
    list.truncate(MAX_TEST_KEYWORDS);
    list
}

/// 去重追加候选词（空串与重复都忽略）。
fn push_keyword(list: &mut Vec<String>, value: Option<&str>) {
    let Some(trimmed) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    if !list.iter().any(|k| k == trimmed) {
        list.push(trimmed.to_string());
    }
}

/// 实测书源搜索是否可用（依次换词，任一命中即算通过），对应 testSource。
pub async fn test_source(
    source: &Value,
    keyword: Option<&str>,
    history: &[String],
) -> Value {
    if !is_source_structural_ok(source) {
        return json!({
            "ok": false,
            "message": "规则不完整（需 searchUrl / bookList / name / bookUrl）",
            "found": 0,
            "respondMs": 0,
            "structuralOk": false
        });
    }

    let keys = build_test_keywords(source, keyword, history);
    let started = std::time::Instant::now();
    let mut last_message = format!("搜索「{}」无结果", keys.first().map(String::as_str).unwrap_or(""));
    let mut hit: Option<(String, usize)> = None;

    for key in &keys {
        match search_books(source, key, 1).await {
            Ok(books) if !books.is_empty() => {
                hit = Some((key.clone(), books.len()));
                let first = books[0]
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                last_message = format!("搜索「{key}」找到 {} 本，例如《{first}》", books.len());
                break;
            }
            Ok(_) => last_message = format!("搜索「{key}」无结果"),
            Err(e) => last_message = e,
        }
    }

    let tried = keys
        .iter()
        .map(|k| format!("「{k}」"))
        .collect::<Vec<_>>()
        .join("、");
    let respond_ms = started.elapsed().as_millis() as u64;
    match hit {
        Some((hit_keyword, found)) => json!({
            "ok": true,
            "message": last_message,
            "found": found,
            "respondMs": respond_ms,
            "structuralOk": true,
            "hitKeyword": hit_keyword
        }),
        None => json!({
            "ok": false,
            "message": if keys.len() > 1 {
                format!("尝试 {tried} 均失败（{last_message}）")
            } else {
                last_message
            },
            "found": 0,
            "respondMs": respond_ms,
            "structuralOk": true
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_accepts_json_and_js_literal() {
        // 笔趣阁：合法 JSON
        let json_header = r#"{"client-brand":"vivo","User-Agent":"okhttp/4.9.2"}"#;
        let got = parse_header(Some(json_header));
        assert_eq!(got.len(), 2);
        assert!(got.contains(&("User-Agent".to_string(), "okhttp/4.9.2".to_string())));

        // 猫眼看书：单引号 JS 字面量，原版 JSON.parse 直接失败 → 鉴权头全丢
        let js_header = "{\n'User-Agent': 'okhttp/4.9.2','client-device': '2d37f6b5','Authorization': 'bearerjwt.token'\n}";
        let got = parse_header(Some(js_header));
        assert_eq!(got.len(), 3);
        let find = |k: &str| got.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone());
        assert_eq!(find("User-Agent").as_deref(), Some("okhttp/4.9.2"));
        assert_eq!(find("Authorization").as_deref(), Some("bearerjwt.token"));
    }

    #[test]
    fn header_tolerates_comments_and_trailing_commas() {
        let got = parse_header(Some(
            "{ // 设备标识\n'device':'x', /* 备注 */ 'brand':'y', }",
        ));
        assert_eq!(got.len(), 2);
        assert!(got.contains(&("brand".to_string(), "y".to_string())));
    }

    #[test]
    fn header_keeps_quoting_inside_values_and_fails_soft() {
        // 值里的单引号/大括号不该被规范化搞坏
        let got = parse_header(Some("{'UA':'okhttp/4.9.2','x-tmpl':'{a:\\'b\\'}'}"));
        let find = |k: &str| got.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone());
        assert_eq!(find("UA").as_deref(), Some("okhttp/4.9.2"));
        assert_eq!(find("x-tmpl").as_deref(), Some("{a:'b'}"));

        // 彻底不是对象时静默返回空，与原版失败语义一致
        assert!(parse_header(Some("not a header at all")).is_empty());
        assert!(parse_header(None).is_empty());
    }

    #[test]
    fn init_rule_reshapes_the_response_root() {
        let raw = r#"{"code":0,"msg":"ok","data":{"novelId":"386971","novelName":"剑来"}}"#;
        let rooted = apply_init_rule(raw, "$.data", "");
        assert_eq!(get_string(&rooted, Some("$.novelId"), ""), "386971");
        // init 取不到值时退回原内容，规则为空同样原样返回
        assert_eq!(apply_init_rule(raw, "$.nope", ""), raw);
        assert_eq!(apply_init_rule(raw, "", ""), raw);
    }

    /// 爱意文学网搜索列表：`table.table tr` → wrap 后 CSS 子选择器须仍能取到字段。
    #[test]
    fn table_row_wrap_keeps_css_fields() {
        let html = r#"<table class="table"><tr><th>书名</th><th>作者</th></tr><tr><td><a href="https://www.aiyiweb.net/item/xxs0Tx.htm" title="剑来杂谈" target="_blank">剑来杂谈</a></td><td class="text-muted">浮生半日不得闲</td></tr></table>"#;
        let items = get_elements(html, Some("table.table tr"));
        assert_eq!(items.len(), 2);
        let wrapped = wrap_item(&items[1]);
        assert!(
            wrapped.starts_with("<table"),
            "tr 片段应包进 table，避免 foster parenting：{wrapped}"
        );
        assert_eq!(get_string(&wrapped, Some("td a@text"), ""), "剑来杂谈");
        assert_eq!(
            get_string(&wrapped, Some("td.text-muted@text"), ""),
            "浮生半日不得闲"
        );
        assert_eq!(
            get_string(&wrapped, Some("td a@href"), "https://www.aiyiweb.net"),
            "https://www.aiyiweb.net/item/xxs0Tx.htm"
        );
    }
}

