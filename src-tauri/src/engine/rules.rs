//! Legado 规则解析：CSS / XPath / JSON / `##正则链` / `{{mustache}}` / `@js`，
//! 对应原 Electron 版 src/main/legado/analyzeRule.ts。

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;

use super::http::resolve_url;
use super::js::{eval_js_to_strings, JsBindings};

/// 规则解析模式，对应 AnalyzeMode
#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Default,
    Json,
    XPath,
    Js,
}

const CACHE_MAX: usize = 512;

thread_local! {
    static REGEX_CACHE: RefCell<HashMap<String, Option<Arc<Regex>>>> = RefCell::new(HashMap::new());
    static RULE_CACHE: RefCell<HashMap<String, Arc<ParsedRule>>> = RefCell::new(HashMap::new());
    /// 选择器编译同样要缓存（含失败结果），否则逐字段重复编译同一串规则。
    static SELECTOR_CACHE: RefCell<HashMap<String, Option<Arc<Selector>>>> = RefCell::new(HashMap::new());
}

/// `selector@getType##pattern##replacement...` 的拆解结果
struct ParsedRule {
    selector: String,
    get_type: String,
    regexes: Vec<String>,
}

/// 编译带 flags 的正则（对应 getRegex）。
///
/// 书源规则里的 flags 是 JS 语义：`g`/`y` 在 rust regex 中无对应（替换端已按全局处理），
/// 必须剔除，否则 `(?g)` 会让整个 pattern 编译失败、`##` 替换链静默失效。
pub fn get_regex(pattern: &str, flags: &str) -> Option<Arc<Regex>> {
    let rust_flags: String = flags
        .chars()
        .filter(|c| matches!(c, 'i' | 'm' | 's' | 'u' | 'x'))
        .collect();
    let key = format!("{flags}:{pattern}");
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(hit) = cache.get(&key) {
            return hit.clone();
        }
        if cache.len() > CACHE_MAX {
            cache.clear();
        }
        let built = if rust_flags.is_empty() {
            Regex::new(pattern).ok()
        } else {
            // rust regex 没有独立 flags 参数，用内联 (?flags)
            Regex::new(&format!("(?{rust_flags}){pattern}")).ok()
        }
        .map(Arc::new);
        cache.insert(key, built.clone());
        built
    })
}

fn parse_rule_parts_cached(rule: &str) -> Arc<ParsedRule> {
    RULE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(hit) = cache.get(rule) {
            return hit.clone();
        }
        if cache.len() > CACHE_MAX {
            cache.clear();
        }
        let parsed = Arc::new(parse_rule_parts(rule));
        cache.insert(rule.to_string(), parsed.clone());
        parsed
    })
}

fn compile_selector(selector: &str) -> Option<Arc<Selector>> {
    SELECTOR_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(hit) = cache.get(selector) {
            return hit.clone();
        }
        if cache.len() > CACHE_MAX {
            cache.clear();
        }
        let built = Selector::parse(selector).ok().map(Arc::new);
        cache.insert(selector.to_string(), built.clone());
        built
    })
}

fn is_json_like(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']'))
}

/// 根据规则前缀与内容形态推断解析模式，对应 detectMode。
fn detect_mode(rule: &str, content: &str) -> Mode {
    let r = rule.trim();
    if r.starts_with("@js:") || r.starts_with("<js>") {
        return Mode::Js;
    }
    if r.starts_with("$.") || r.starts_with("$[") || r.starts_with("$..") {
        return Mode::Json;
    }
    if r.starts_with("//") || r.starts_with("./") || r.starts_with('(') || r.starts_with('/') {
        return Mode::XPath;
    }
    if is_json_like(content) && (r.contains("$.") || r.starts_with('$')) {
        return Mode::Json;
    }
    Mode::Default
}

/// 去掉 `@js:` / `<js>…</js>` 前缀。
fn strip_js_prefix(rule: &str) -> String {
    let r = rule.trim();
    if let Some(rest) = r.strip_prefix("@js:") {
        return rest.trim().to_string();
    }
    if let Some(rest) = r.strip_prefix("<js>") {
        let end = rest.find("</js>").unwrap_or(rest.len());
        return rest[..end].trim().to_string();
    }
    r.to_string()
}

/// 拆「前置规则 + 后置 JS」写法，如 `$.path@js:java.aesBase64DecodeToString(…)`
/// 或 `$.path\n<js>…</js>`：先用左半规则取值，再把结果喂给 JS 加工。
///
/// 书源里这类写法专门用来解密章节地址；`@js:` / `<js>` 出现在串首时属于纯 JS 规则，
/// 由 `detect_mode` 处理，这里只认出现在中段的链式形态。
fn split_js_tail(rule: &str) -> Option<(String, String)> {
    let r = rule.trim();
    if let Some(idx) = r.find("@js:") {
        if idx > 0 {
            return Some((
                r[..idx].trim().to_string(),
                r[idx + "@js:".len()..].trim().to_string(),
            ));
        }
    }
    let start = r.find("<js>")?;
    let end = r[start..].find("</js>")? + start;
    if start > 0 {
        return Some((
            r[..start].trim().to_string(),
            r[start + "<js>".len()..end].trim().to_string(),
        ));
    }
    None
}

/// 拆分规则为选择器、取值类型与 `##` 正则段，对应 parseRuleParts。
fn parse_rule_parts(rule: &str) -> ParsedRule {
    let working = rule.trim();
    let mut hash_parts = working.split("##");
    let head = hash_parts.next().unwrap_or("");
    let regexes: Vec<String> = hash_parts.map(str::to_string).collect();

    let mut get_type = String::from("text");
    let mut selector = head.to_string();

    let prefixed = head.starts_with("text.") || head.starts_with("id.") || head.starts_with("class.");
    if prefixed {
        if let Some(at) = head.rfind('@') {
            if at > 0 {
                selector = head[..at].to_string();
                get_type = rest_or_text(head, at);
            }
        }
        return ParsedRule {
            selector,
            get_type,
            regexes,
        };
    }

    if let Some(at_idx) = head.rfind('@') {
        if at_idx > 0 && !head.starts_with("//") && !head.starts_with("$.") {
            let maybe_type = &head[at_idx + 1..];
            let head_type = maybe_type.split("&&").next().unwrap_or("");
            if get_regex(r"^(text|html|href|src|ownText|all|textNodes|content)$", "")
                .is_some_and(|re| re.is_match(head_type))
            {
                selector = head[..at_idx].to_string();
                get_type = maybe_type.to_string();
            }
        }
    }

    ParsedRule {
        selector,
        get_type,
        regexes,
    }
}

fn rest_or_text(head: &str, at: usize) -> String {
    if at + 1 < head.len() {
        head[at + 1..].to_string()
    } else {
        "text".to_string()
    }
}

/// 链式 `##pattern##replacement` 替换，对应 applyRegexChain。
pub fn apply_regex_chain(value: &str, regexes: &[String]) -> String {
    let mut result = value.to_string();
    let mut i = 0;
    while i < regexes.len() {
        let pattern = &regexes[i];
        let replacement = regexes.get(i + 1).map(String::as_str).unwrap_or("");
        i += 2;
        if pattern.is_empty() {
            continue;
        }
        if let Some(re) = get_regex(pattern, "g") {
            result = re.replace_all(&result, replacement).to_string();
        }
    }
    result.trim().to_string()
}

/// 相对链接转绝对，对应 absoluteUrl。
fn absolute_url(base: &str, href: &str) -> String {
    if href.is_empty() {
        return String::new();
    }
    resolve_url(base, href)
}

/// 折叠空白并 trim，等价于 JS 的 `.replace(/\s+/g,' ').trim()`。
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        out.push(ch);
    }
    out
}

fn element_text(el: &ElementRef) -> String {
    el.text().collect::<Vec<_>>().join("")
}

/// 元素自身的直接文本子节点（不含后代），对应 cheerio ownText。
fn own_text(el: &ElementRef) -> String {
    el.children()
        .filter_map(|n| n.value().as_text().map(|t| &t.text[..]))
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

/// 按 getType 从元素取值，对应 getFromCheerio。
fn get_from_element(el: &ElementRef, get_type: &str, base_url: &str) -> String {
    let ty = get_type.to_lowercase();
    match ty.as_str() {
        "html" => el.inner_html(),
        "text" | "textnodes" | "content" => collapse_whitespace(&element_text(el)),
        "owntext" => own_text(el),
        "href" => {
            let href = el.attr("href").or_else(|| el.attr("data-href")).unwrap_or("");
            absolute_url(base_url, href)
        }
        "src" => {
            let src = el
                .attr("src")
                .or_else(|| el.attr("data-src"))
                .or_else(|| el.attr("data-original"))
                .unwrap_or("");
            absolute_url(base_url, src)
        }
        "all" => el.html(),
        other => {
            if let Some(attr) = el.attr(other) {
                if other.contains("href") || other.contains("src") || other.contains("url") {
                    return absolute_url(base_url, attr);
                }
                return attr.to_string();
            }
            collapse_whitespace(&element_text(el))
        }
    }
}

/// Legado 的 `id.x` / `class.x` 语法转 CSS 选择器。
fn to_css_selector(selector: &str) -> String {
    if let Some(rest) = selector.strip_prefix("id.") {
        format!("#{rest}")
    } else if let Some(rest) = selector.strip_prefix("class.") {
        format!(".{rest}")
    } else {
        selector.to_string()
    }
}

/// 末尾 `.N` 索引（支持 -1 倒数），对应 selectByCss 的 index 处理。
fn split_trailing_index(selector: &str) -> (String, Option<i64>) {
    match get_regex(r"^(.*)\.(-?\d+)$", "") {
        Some(re) => match re.captures(selector) {
            Some(caps) => match caps[2].parse::<i64>() {
                Ok(i) => (caps[1].to_string(), Some(i)),
                Err(_) => (selector.to_string(), None),
            },
            None => (selector.to_string(), None),
        },
        None => (selector.to_string(), None),
    }
}

/// 在文档中按 Legado CSS / `text.` 选择器选节点，对应 selectByCss。
fn select_by_css<'a>(doc: &'a Html, selector: &str) -> Vec<ElementRef<'a>> {
    if let Some(needle) = selector.strip_prefix("text.") {
        let root = doc.root_element();
        let matched: Vec<ElementRef> = root
            .descendent_elements()
            .filter(|el| element_text(el).contains(needle))
            .collect();
        // cheerio 的 `$(el).find('*')` 只含后代，而 scraper 的 descendent_elements 含自身，
        // 必须按节点 id 排除自身，否则任何命中元素都会因"自己含关键词"被过滤掉。
        let filtered: Vec<ElementRef> = matched
            .iter()
            .filter(|el| {
                !el.descendent_elements()
                    .any(|child| child.id() != el.id() && element_text(&child).contains(needle))
            })
            .cloned()
            .collect();
        return if filtered.is_empty() { matched } else { filtered };
    }

    let (base, index) = split_trailing_index(selector);
    let css = to_css_selector(&base);
    let Some(sel) = compile_selector(&css) else {
        return Vec::new();
    };
    let nodes: Vec<ElementRef> = doc.select(&sel).collect();
    match index {
        Some(i) => {
            let idx = if i < 0 { nodes.len() as i64 + i } else { i };
            if (0..nodes.len() as i64).contains(&idx) {
                vec![nodes[idx as usize]]
            } else {
                Vec::new()
            }
        }
        None => nodes,
    }
}

/// CSS 取值，对应 analyzeCss。
fn analyze_css(html: &str, rule: &str, base_url: &str, single: bool) -> Vec<String> {
    let parsed = parse_rule_parts_cached(rule);
    if parsed.selector.is_empty() && parsed.get_type == "text" && parsed.regexes.is_empty() {
        return Vec::new();
    }
    let doc = Html::parse_document(html);

    if parsed.selector.is_empty() || parsed.selector == "@" {
        let val = apply_regex_chain(&element_text(&doc.root_element()), &parsed.regexes);
        return if val.is_empty() { Vec::new() } else { vec![val] };
    }

    let mut out = Vec::new();
    for el in select_by_css(&doc, &parsed.selector) {
        let raw = get_from_element(&el, &parsed.get_type, base_url);
        let val = apply_regex_chain(&raw, &parsed.regexes);
        if !val.is_empty() {
            out.push(val);
            if single {
                break;
            }
        }
    }
    out
}

fn json_value_to_string(item: &Value) -> String {
    match item {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn jsonpath_of(selector: &str) -> String {
    if selector.starts_with('$') {
        selector.to_string()
    } else {
        format!("$.{selector}")
    }
}

/// JSONPath 不认 `@getType` 后缀：正文规则会被 `content_rule_as_html` 改成 `$.content@html`，
/// 直接当路径用一定取空，所以 JSON 模式再剥一次尾部。
/// （`parse_rule_parts` 里对 `$.` 开头不切 `@` 是必要的，避免拆坏 `[?(@.x==1)]` 这类过滤式。）
fn json_path_selector(selector: &str) -> String {
    let Some(re) = get_regex(r"^(.*?)@(text|html|href|src|ownText|all|textNodes|content)$", "i")
    else {
        return selector.to_string();
    };
    match re.captures(selector) {
        Some(caps) => caps[1].to_string(),
        None => selector.to_string(),
    }
}

/// JSONPath 取值，对应 analyzeJson。
fn analyze_json(content: &str, rule: &str, single: bool) -> Vec<String> {
    let parsed = parse_rule_parts_cached(rule);
    let Ok(data) = serde_json::from_str::<Value>(content) else {
        return Vec::new();
    };
    let found = match jsonpath_plus::find(&jsonpath_of(&json_path_selector(&parsed.selector)), &data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in found {
        if item.is_null() {
            continue;
        }
        let val = apply_regex_chain(&json_value_to_string(item), &parsed.regexes);
        if !val.is_empty() {
            out.push(val);
            if single {
                break;
            }
        }
    }
    out
}

/// XPath 取值，对应 analyzeXPath。
///
/// 已知差距：底层 sxd-document 是严格 XML 解析器，而原版用 `@xmldom/xmldom` 的 text/html 模式，
/// 对未闭合标签的现实 HTML 容错更强。现有 4 个书源均不使用 XPath，故先按 XML 口径实现；
/// 若之后接入 XPath 书源，需要再加 HTML5→XML 预处理（scraper 树可直接导出 XHTML）。
fn analyze_xpath(html: &str, rule: &str, base_url: &str, single: bool) -> Vec<String> {
    let parsed = parse_rule_parts_cached(rule);
    // 取值后缀（@text / @html 等）不属于 XPath 表达式本体
    let expr = if rule.contains("/text()") || rule.contains("/@") {
        rule.split("##").next().unwrap_or("").to_string()
    } else {
        parsed.selector.clone()
    };

    let Ok(package) = sxd_document::parser::parse(&normalize_for_xml(html)) else {
        return Vec::new();
    };
    let doc = package.as_document();
    let Ok(value) = sxd_xpath::evaluate_xpath(&doc, &expr) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    match value {
        sxd_xpath::Value::Nodeset(set) => {
            for node in set.iter() {
                let raw = xpath_node_string(node, &parsed, base_url, &expr);
                let val = apply_regex_chain(&raw, &parsed.regexes);
                if !val.is_empty() {
                    out.push(val);
                    if single {
                        break;
                    }
                }
            }
        }
        other => {
            let val = apply_regex_chain(&xpath_scalar_to_string(&other), &parsed.regexes);
            if !val.is_empty() {
                out.push(val);
            }
        }
    }
    out
}

fn xpath_scalar_to_string(value: &sxd_xpath::Value<'_>) -> String {
    match value {
        sxd_xpath::Value::Boolean(b) => b.to_string(),
        sxd_xpath::Value::Number(n) => n.to_string(),
        sxd_xpath::Value::String(s) => s.clone(),
        sxd_xpath::Value::Nodeset(set) => set.iter().map(|n| n.string_value()).collect(),
    }
}

fn xpath_node_string(
    node: sxd_xpath::nodeset::Node<'_>,
    parsed: &ParsedRule,
    base_url: &str,
    expr: &str,
) -> String {
    use sxd_xpath::nodeset::Node;
    let get_type = parsed.get_type.to_lowercase();
    match node {
        Node::Text(t) => t.text().to_string(),
        Node::Attribute(a) => {
            let val = a.value().to_string();
            let link_like = matches!(get_type.as_str(), "href" | "src")
                || expr.contains("@href")
                || expr.contains("@src");
            if link_like {
                absolute_url(base_url, &val)
            } else {
                val
            }
        }
        Node::Element(e) => match get_type.as_str() {
            "html" | "all" => sxd_serialize_element(e),
            "href" => absolute_url(base_url, e.attribute("href").map(|a| a.value()).unwrap_or("")),
            "src" => absolute_url(base_url, e.attribute("src").map(|a| a.value()).unwrap_or("")),
            _ => collapse_whitespace(&sxd_element_text(e)),
        },
        other => other.string_value(),
    }
}

/// 递归取文本（等价于 DOM textContent）。
fn sxd_element_text(el: sxd_document::dom::Element<'_>) -> String {
    let mut out = String::new();
    for child in el.children() {
        use sxd_document::dom::ChildOfElement as C;
        match child {
            C::Text(t) => out.push_str(t.text()),
            C::Element(e) => out.push_str(&sxd_element_text(e)),
            _ => {}
        }
    }
    out
}

/// 极简 outerHTML，仅供 `@html` / 列表规则再嵌套取值使用。
fn sxd_serialize_element(el: sxd_document::dom::Element<'_>) -> String {
    let name = el.name().local_part().to_string();
    let mut attrs = String::new();
    for attr in el.attributes() {
        attrs.push_str(&format!(
            " {}=\"{}\"",
            attr.name().local_part(),
            attr.value().replace('"', "&quot;")
        ));
    }
    let mut inner = String::new();
    for child in el.children() {
        use sxd_document::dom::ChildOfElement as C;
        match child {
            C::Text(t) => inner.push_str(&escape_xml_text(t.text())),
            C::Element(e) => inner.push_str(&sxd_serialize_element(e)),
            C::Comment(c) => inner.push_str(&format!("<!--{}-->", c.text())),
            C::ProcessingInstruction(_) => {}
        }
    }
    format!("<{name}{attrs}>{inner}</{name}>")
}

fn escape_xml_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// 把片段放宽成可被严格 XML 解析器接受的文档：补根节点、去注释。
fn normalize_for_xml(html: &str) -> String {
    let mut s = String::with_capacity(html.len() + 16);
    s.push_str("<root>");
    s.push_str(html);
    s.push_str("</root>");
    // 去掉 doctype 与 XML 注释（注释不可嵌套，留着会直接解析失败）
    if let Some(pos) = s.find("<!DOCTYPE") {
        if let Some(end) = s[pos..].find('>') {
            s.replace_range(pos..pos + end + 1, "");
        }
    }
    while let Some(start) = s.find("<!--") {
        match s[start..].find("-->") {
            Some(offset) => s.replace_range(start..start + offset + 3, ""),
            None => {
                s.truncate(start);
                break;
            }
        }
    }
    s
}

/// 列表规则取元素（bookList / chapterList），对应 getElements。
pub fn get_elements(content: &str, rule: Option<&str>) -> Vec<String> {
    let Some(rule) = rule.filter(|r| !r.trim().is_empty()) else {
        return Vec::new();
    };
    if content.is_empty() {
        return Vec::new();
    }
    match detect_mode(rule, content) {
        Mode::Json => {
            let Ok(data) = serde_json::from_str::<Value>(content) else {
                return Vec::new();
            };
            let parsed = parse_rule_parts_cached(rule);
            let path = jsonpath_of(&json_path_selector(&parsed.selector));
            match jsonpath_plus::find(&path, &data) {
                Ok(found) => found.iter().map(|item| json_value_to_string(item)).collect(),
                Err(_) => Vec::new(),
            }
        }
        Mode::XPath => {
            let parsed = parse_rule_parts_cached(rule);
            let Ok(package) = sxd_document::parser::parse(&normalize_for_xml(content)) else {
                return Vec::new();
            };
            let doc = package.as_document();
            match sxd_xpath::evaluate_xpath(&doc, &parsed.selector) {
                Ok(sxd_xpath::Value::Nodeset(set)) => set
                    .iter()
                    .filter_map(|n| match n {
                        sxd_xpath::nodeset::Node::Element(e) => Some(sxd_serialize_element(e)),
                        sxd_xpath::nodeset::Node::Text(t) => Some(t.text().to_string()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        }
        Mode::Js => eval_js_to_strings(
            &strip_js_prefix(rule),
            JsBindings {
                result: content,
                ..Default::default()
            },
        )
        .unwrap_or_default(),
        Mode::Default => {
            let doc = Html::parse_document(content);
            let parsed = parse_rule_parts_cached(rule);
            select_by_css(&doc, &parsed.selector)
                .into_iter()
                .map(|el| el.html())
                .collect()
        }
    }
}

/// 规范化 `{{...}}` 内嵌规则的前缀，对应 normalizeEmbeddedRule。
fn normalize_embedded_rule(inner: &str) -> String {
    let r = inner.trim();
    for prefix in ["@@", "@css:", "@xpath:", "@json:"] {
        if let Some(rest) = r.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    if let Some(rest) = r.strip_prefix("@.") {
        return rest.trim().to_string();
    }
    r.to_string()
}

/// 展开 `{{rule}}` 插值段（简介等字段常用），对应 expandMustacheRule。
fn expand_mustache_rule(content: &str, rule: &str, base_url: &str) -> String {
    let Some(re) = get_regex(r"\{\{([\s\S]*?)\}\}", "g") else {
        return rule.to_string();
    };
    re.replace_all(rule, |caps: &regex::Captures| {
        get_string(content, Some(&normalize_embedded_rule(&caps[1])), base_url)
    })
    .to_string()
}

/// 按规则取字符串列表，支持 `||` 备选与多模式，对应 getStringList。
pub fn get_string_list(content: &str, rule: Option<&str>, base_url: &str, single: bool) -> Vec<String> {
    let Some(trimmed) = rule.map(str::trim).filter(|r| !r.is_empty()) else {
        return Vec::new();
    };
    if content.is_empty() {
        return Vec::new();
    }

    // 「左规则@js:代码」/「左规则<js>代码</js>」要先整体切一次：
    // 放到下面的 || 循环里切，会把 JS 代码自身的 `||` 拦腰截断。
    if let Some((left, code)) = split_js_tail(trimmed) {
        let mid = get_string(content, Some(&left), base_url);
        if mid.is_empty() {
            return Vec::new();
        }
        return eval_js_to_strings(
            &code,
            JsBindings {
                result: &mid,
                base_url,
                ..Default::default()
            },
        )
        .unwrap_or_default();
    }

    if trimmed.contains("{{") {
        let expanded = expand_mustache_rule(content, trimmed, base_url).trim().to_string();
        return if expanded.is_empty() { Vec::new() } else { vec![expanded] };
    }

    for alt in trimmed.split("||") {
        let out = match detect_mode(alt, content) {
            Mode::Json => analyze_json(content, alt, single),
            Mode::XPath => analyze_xpath(content, alt, base_url, single),
            Mode::Js => eval_js_to_strings(
                &strip_js_prefix(alt),
                JsBindings {
                    result: content,
                    base_url,
                    ..Default::default()
                },
            )
            .unwrap_or_default(),
            Mode::Default => {
                if is_json_like(content) && alt.trim().starts_with('$') {
                    analyze_json(content, alt, single)
                } else {
                    analyze_css(content, alt, base_url, single)
                }
            }
        };
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

/// 按规则取单个字符串（列表首项），对应 getString。
pub fn get_string(content: &str, rule: Option<&str>, base_url: &str) -> String {
    get_string_list(content, rule, base_url, true)
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// 正文规则改走 `@html` 以保留 br/p 结构，对应 contentRuleAsHtml。
pub fn content_rule_as_html(rule: &str) -> String {
    let trimmed = rule.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if get_regex(r"@html\b", "i").is_some_and(|re| re.is_match(trimmed)) {
        return trimmed.to_string();
    }
    if let Some(re) = get_regex(r"@(text|ownText|textNodes|all)\b", "i") {
        if re.is_match(trimmed) {
            return re.replace_all(trimmed, "@html").trim().to_string();
        }
    }
    if !trimmed.contains('@') {
        return format!("{trimmed}@html");
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"<html><body>
      <div class="bookbox">
        <div class="bookname">剑来</div>
        <div class="author">作者：烽火戏诸侯</div>
        <a href="/book/1234/" class="read">开始阅读</a>
        <img data-original="https://cdn.example.com/c/1234.jpg" />
        <p class="intro">　　这是一个　少年　的故事。</p>
      </div>
      <div class="bookbox">
        <div class="bookname">遮天</div>
        <div class="author">作者：辰东</div>
        <a href="/book/5678/">开始阅读</a>
      </div>
    </body></html>"#;

    const JSON: &str = r#"{"data":[{"bookName":"诡秘之主","author":"爱潜水的乌贼","bookId":999},{"bookName":"宿命之环"}],"lastChapter":{"chapterName":"第10章","updTime":"2026-09-01"}}"#;

    #[test]
    fn css_list_and_nested_fields() {
        let items = get_elements(HTML, Some(".bookbox"));
        assert_eq!(items.len(), 2);
        assert_eq!(get_string(&items[0], Some(".bookname@text"), ""), "剑来");
        assert_eq!(get_string(&items[1], Some(".author@text"), ""), "作者：辰东");
    }

    #[test]
    fn css_trailing_index_including_negative() {
        assert_eq!(get_string(HTML, Some(".bookname.0@text"), ""), "剑来");
        assert_eq!(get_string(HTML, Some(".bookname.-1@text"), ""), "遮天");
    }

    #[test]
    fn legacy_id_class_and_text_prefixes() {
        assert_eq!(get_string(HTML, Some("class.bookname@text"), ""), "剑来");
        assert_eq!(get_string(HTML, Some("text.作者：辰东@text"), ""), "作者：辰东");
    }

    #[test]
    fn href_and_src_resolve_against_base() {
        let base = "https://www.yetianlian.info/book/1234/";
        assert_eq!(
            get_string(HTML, Some(".read@href"), base),
            "https://www.yetianlian.info/book/1234/"
        );
        assert_eq!(
            get_string(HTML, Some("img@src"), base),
            "https://cdn.example.com/c/1234.jpg"
        );
    }

    #[test]
    fn regex_chain_strips_noise() {
        assert_eq!(get_string(HTML, Some(".author@text##^作者："), ""), "烽火戏诸侯");
        assert_eq!(get_string(JSON, Some("$.data[0].bookName##之主##"), ""), "诡秘");
    }

    #[test]
    fn json_list_and_deep_fields() {
        let items = get_elements(JSON, Some("$.data[*]"));
        assert_eq!(items.len(), 2);
        assert_eq!(get_string(&items[0], Some("$.bookName"), ""), "诡秘之主");
        assert_eq!(get_string(JSON, Some("$.lastChapter.chapterName"), ""), "第10章");
        assert_eq!(get_string(JSON, Some("$..bookId"), ""), "999");
    }

    #[test]
    fn alternatives_mustache_and_embedded_prefix() {
        assert_eq!(get_string(HTML, Some(".nope@text||.bookname@text"), ""), "剑来");
        assert_eq!(
            get_string(HTML, Some("《{{.bookname@text}}》作者{{class.author.0@text}}"), ""),
            "《剑来》作者作者：烽火戏诸侯"
        );
        assert_eq!(get_string(HTML, Some("{{@@.bookname@text}}"), ""), "剑来");
        assert_eq!(get_string(JSON, Some("{{$.lastChapter.chapterName}}"), ""), "第10章");
    }

    #[test]
    fn js_rule_for_list_and_single() {
        let js = "@js:JSON.parse(result).data.map(function(b){return b.bookName})";
        assert_eq!(
            get_string_list(JSON, Some(js), "", false),
            vec!["诡秘之主".to_string(), "宿命之环".to_string()]
        );
        assert_eq!(
            get_string(JSON, Some("@js:JSON.parse(result).lastChapter.updTime"), ""),
            "2026-09-01"
        );
    }

    #[test]
    fn json_rule_ignores_get_type_suffix() {
        // 正文规则会被 content_rule_as_html 改成 $.content@html，JSONPath 不认这个后缀
        let json = r#"{"content":"第一章\n\n正文段落。"}"#;
        assert_eq!(get_string(json, Some("$.content@html"), ""), "第一章\n\n正文段落。");
        assert_eq!(get_string(json, Some("$.content@text"), ""), "第一章\n\n正文段落。");
        // 过滤式里的 @ 不能被误当成后缀
        let list = r#"{"rows":[{"a":1,"b":"x"},{"a":2,"b":"y"}]}"#;
        assert_eq!(get_string(list, Some("$.rows[?(@.a==2)].b"), ""), "y");
    }

    #[test]
    fn js_tail_runs_after_left_rule() {
        // 猫眼/笔趣阁的 chapterUrl 形态：先 $.path 取密文，再交给后置 JS 解密加工
        let json = r#"{"path":"abc123","other":"x"}"#;
        assert_eq!(
            get_string(json, Some("$.path@js:result.toUpperCase()"), ""),
            "ABC123"
        );
        assert_eq!(
            get_string(json, Some("$.path\n<js>\na=String(result)+'/read'\n</js>"), ""),
            "abc123/read"
        );
        // JS 代码里的 || 不能被备选切分拦腰截断
        assert_eq!(
            get_string(json, Some("$.path@js:result.toUpperCase()||'兜底'"), ""),
            "ABC123"
        );
    }

    #[test]
    fn xpath_on_wellformed_fragment() {
        let xml = r#"<div class="box"><span class="t">标题</span><a href="x/1.html">链</a></div>"#;
        assert_eq!(get_string(xml, Some("//span[@class='t']/text()"), ""), "标题");
        assert_eq!(
            get_string(xml, Some("//a/@href"), "https://b.example.com/a/"),
            "https://b.example.com/a/x/1.html"
        );
        assert_eq!(get_elements(xml, Some("//span")).len(), 1);
    }

    #[test]
    fn content_rule_switches_to_html() {
        assert_eq!(content_rule_as_html("div#content"), "div#content@html");
        assert_eq!(content_rule_as_html(".read@text"), ".read@html");
        assert_eq!(content_rule_as_html(".read@html"), ".read@html");
    }

    #[test]
    fn own_text_ignores_descendants() {
        let frag = r#"<p id="p">正文<span>忽略</span></p>"#;
        assert_eq!(get_string(frag, Some("#p@ownText"), ""), "正文");
        assert_eq!(get_string(frag, Some("#p@text"), ""), "正文忽略");
    }
}
