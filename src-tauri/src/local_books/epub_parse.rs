//! EPUB 导入：container.xml → OPF → spine / 目录 → 章节正文与封面
//! （对应 main/localBooks/epubParse.ts）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;

use base64::Engine as _;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use sxd_document::dom::{ChildOfElement as XmlChild, ChildOfRoot as XmlRootChild, Element};
use sxd_document::Package;
use zip::ZipArchive;

use super::SplitChapter;

/// zip bomb 防护上限（对合法 EPUB 足够宽松，对恶意压缩包直接拒绝）。
/// 条目总数上限：正常 EPUB 通常 < 2000。
const MAX_ZIP_ENTRIES: usize = 50_000;
/// 单条目解压后上限 256MB（EPUB 正文/图片不会到这个量级）。
const MAX_ENTRY_UNCOMPRESSED: u64 = 256 * 1024 * 1024;
/// 总解压后上限 1GB。
const MAX_TOTAL_UNCOMPRESSED: u64 = 1024 * 1024 * 1024;
/// 压缩比上限 500:1，且仅对大于 `MIN_RATIO_ENTRY_SIZE` 的条目生效，
/// 避免误伤重复文本很多的合法大文件。
const MAX_COMPRESSION_RATIO: f64 = 500.0;
const MIN_RATIO_ENTRY_SIZE: u64 = 4 * 1024 * 1024;
/// 单章 / 单个 XML 读取上限 12MB。
const MAX_HTML_READ: usize = 12 * 1024 * 1024;
/// 封面读取上限。
const MAX_COVER_READ: usize = 2_500_000;

/// 需要在其后补换行的块级标签（对应 cheerio 的 `$(el).append('\n')`）。
const BLOCK_TAGS: [&str; 12] = [
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr", "section", "article",
];

type Archive<'a> = ZipArchive<Cursor<&'a [u8]>>;

#[derive(Debug)]
pub struct EpubMeta {
    pub title: String,
    pub author: String,
    pub chapters: Vec<SplitChapter>,
    pub cover_data_url: Option<String>,
}
struct ManifestItem {
    id: String,
    href: String,
    media_type: String,
}

/// 解析 EPUB 缓冲，提取书名、作者、章节正文与可选封面。
pub fn parse_epub_buffer(bytes: &[u8], fallback_title: &str) -> Result<EpubMeta, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("无效的 EPUB：无法解压（{e}）"))?;
    guard_zip_bomb(&mut archive)?;

    let container_xml = read_entry_text(&mut archive, "META-INF/container.xml", MAX_HTML_READ)
        .ok_or_else(|| "无效的 EPUB：缺少 META-INF/container.xml".to_string())?;
    let rootfile = parse_xml(&container_xml)
        .and_then(|pkg| {
            let root = root_element(&pkg)?;
            let rf = find_descendant(root, "rootfile")?;
            attr_nonempty(rf, "full-path").or_else(|| attr_nonempty(rf, "fullPath"))
        })
        .ok_or_else(|| "无效的 EPUB：找不到 content.opf".to_string())?;

    let opf_path = rootfile.replace('\\', "/");
    let opf_dir = dirname(&opf_path);
    let opf_xml = read_entry_text(&mut archive, &opf_path, MAX_HTML_READ)
        .ok_or_else(|| format!("无效的 EPUB：无法读取 {opf_path}"))?;
    let Some(opf_pkg) = parse_package(&opf_xml) else {
        return Err("无效的 EPUB：content.opf 格式有误".to_string());
    };
    let Some(opf) = root_element(&opf_pkg) else {
        return Err("无效的 EPUB：content.opf 格式有误".to_string());
    };

    let title = fallback_or_local(
        &text_of_first(opf, &["dc:title", "title"]),
        fallback_title,
        "未命名 EPUB",
    );
    let author = fallback_or_local(&text_of_first(opf, &["dc:creator", "creator"]), "", "佚名");

    let items: Vec<ManifestItem> = descendants(opf, "item")
        .into_iter()
        .filter(|el| !attr(el, "id").is_empty() && !attr(el, "href").is_empty())
        .map(|el| {
            let declared_type = attr(&el, "media-type");
            ManifestItem {
                id: attr(&el, "id"),
                href: resolve_path(&opf_dir, &attr(&el, "href")),
                media_type: if declared_type.is_empty() {
                    attr(&el, "mediaType")
                } else {
                    declared_type
                },
            }
        })
        .collect();

    let spine_ids: Vec<String> = descendants(opf, "itemref")
        .into_iter()
        .map(|el| attr(&el, "idref"))
        .filter(|id| !id.is_empty())
        .collect();

    let toc_titles = load_toc_titles(&mut archive, &items, &opf_dir, opf);
    let nav_ids: Vec<String> = descendants(opf, "item")
        .into_iter()
        .filter(|el| {
            let id = attr(el, "id");
            !id.is_empty() && split_props(&attr(el, "properties")).iter().any(|p| *p == "nav")
        })
        .map(|el| attr(&el, "id"))
        .collect();

    let mut chapters: Vec<SplitChapter> = Vec::new();
    for id in &spine_ids {
        if nav_ids.iter().any(|n| n == id) {
            continue;
        }
        let Some(item) = items.iter().find(|i| i.id == *id) else {
            continue;
        };
        if !is_html_media(&item.media_type, &item.href) {
            continue;
        }
        let Some(html) = read_entry_text(&mut archive, &item.href, MAX_HTML_READ) else {
            continue;
        };
        let content = html_to_plain_text(&html);
        if content.is_empty() {
            continue;
        }
        let guess = chapter_title_guess(&toc_titles, &html, &item.href, chapters.len());
        chapters.push(SplitChapter::new(truncate(&guess, 80), content));
    }

    if chapters.is_empty() {
        return Err("EPUB 中没有可读章节".to_string());
    }

    let cover_data_url = extract_cover_data_url(&mut archive, opf, &items);
    Ok(EpubMeta {
        title: title.trim().to_string(),
        author: author.trim().to_string(),
        chapters,
        cover_data_url,
    })
}

/// 校验 zip 条目元数据，拒绝 zip bomb：条目数、单条目解压大小、压缩比、总解压大小。
fn guard_zip_bomb(archive: &mut Archive<'_>) -> Result<(), String> {
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(format!("EPUB 条目数异常（{}），已拒绝导入", archive.len()));
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let Ok(file) = archive.by_index_raw(i) else {
            continue;
        };
        if file.is_dir() {
            continue;
        }
        let uncompressed = file.size();
        let compressed = file.compressed_size();
        let name = file.name().to_string();
        if uncompressed > MAX_ENTRY_UNCOMPRESSED {
            return Err(format!("EPUB 条目过大：{name}"));
        }
        if uncompressed > MIN_RATIO_ENTRY_SIZE
            && compressed > 0
            && uncompressed as f64 / compressed as f64 > MAX_COMPRESSION_RATIO
        {
            return Err(format!("EPUB 条目压缩比异常：{name}"));
        }
        total += uncompressed;
        if total > MAX_TOTAL_UNCOMPRESSED {
            return Err("EPUB 解压总量超过限制，已拒绝导入".to_string());
        }
    }
    Ok(())
}

/// 从 OPF/manifest 提取封面并转为 data URL。
fn extract_cover_data_url(
    archive: &mut Archive<'_>,
    opf: Element<'_>,
    items: &[ManifestItem],
) -> Option<String> {
    let mut cover: Option<&ManifestItem> = None;

    for el in descendants(opf, "item") {
        let id = attr(&el, "id");
        if id.is_empty() {
            continue;
        }
        if split_props(&attr(&el, "properties")).iter().any(|p| *p == "cover-image") {
            cover = items.iter().find(|i| i.id == id);
            break;
        }
    }

    if cover.is_none() {
        for el in descendants(opf, "meta") {
            let name = attr(&el, "name").to_lowercase();
            let content = attr(&el, "content");
            if name == "cover" && !content.is_empty() {
                if let Some(item) = items.iter().find(|i| i.id == content) {
                    cover = Some(item);
                    break;
                }
            }
        }
    }

    if cover.is_none() {
        for item in items {
            let is_image = !item.media_type.to_lowercase().starts_with("image/")
                && !img_ext_re().is_some_and(|re| re.is_match(&item.href));
            if is_image {
                continue;
            }
            if contains_ci(&item.id, "cover") || contains_ci(&item.href, "cover") {
                cover = Some(item);
                break;
            }
        }
    }

    let cover = cover?;
    let buf = read_entry_bytes(archive, &cover.href, MAX_COVER_READ)?;
    if buf.is_empty() {
        return None;
    }
    let mut mime = cover.media_type.clone();
    if !mime.starts_with("image/") {
        let ext = cover.href.rsplit('.').next().unwrap_or("").to_lowercase();
        mime = match ext.as_str() {
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            _ => "image/jpeg",
        }
        .to_string();
    }
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&buf)
    ))
}

/// 从 EPUB3 nav 与 EPUB2 NCX 加载 href→标题 映射。
fn load_toc_titles(
    archive: &mut Archive<'_>,
    items: &[ManifestItem],
    opf_dir: &str,
    opf: Element<'_>,
) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();

    for el in descendants(opf, "item") {
        let href = attr(&el, "href");
        if href.is_empty() {
            continue;
        }
        if !split_props(&attr(&el, "properties")).iter().any(|p| *p == "nav") {
            continue;
        }
        let full = resolve_path(opf_dir, &href);
        if let Some(html) = read_entry_text(archive, &full, MAX_HTML_READ) {
            parse_nav_xhtml(&html, &dirname(&full), &mut map);
        }
    }

    for item in items {
        let is_ncx = item.media_type == "application/x-dtbncx+xml"
            || ncx_ext_re().is_some_and(|re| re.is_match(&item.href));
        if !is_ncx {
            continue;
        }
        if let Some(ncx) = read_entry_text(archive, &item.href, MAX_HTML_READ) {
            parse_ncx(&ncx, &dirname(&item.href), &mut map);
        }
    }

    map
}

/// 解析 EPUB3 导航 XHTML，填充目录标题映射。
/// 原版选择器是 `nav[epub\:type="toc"] a, nav#toc a, nav.toc a, nav a` 的并集，
/// 前三条都是第四条的子集，故等价地用 `nav a`（querySelectorAll 与 scraper 同为文档序）。
fn parse_nav_xhtml(html: &str, nav_dir: &str, map: &mut HashMap<String, String>) {
    let doc = Html::parse_document(html);
    let Some(sel) = Selector::parse("nav a").ok() else {
        return;
    };
    for a in doc.select(&sel) {
        let href = a.attr("href").unwrap_or("").split('#').next().unwrap_or("");
        let label = collapse_spaces(&a.text().collect::<Vec<_>>().join(""));
        if href.is_empty() || label.is_empty() {
            continue;
        }
        let full = normalize_href(&resolve_path(nav_dir, href));
        map.entry(full).or_insert_with(|| truncate(&label, 80));
    }
}

/// 解析 EPUB2 NCX，填充目录标题映射。
fn parse_ncx(ncx: &str, ncx_dir: &str, map: &mut HashMap<String, String>) {
    let Some(pkg) = parse_package(ncx) else {
        return;
    };
    let Some(root) = root_element(&pkg) else {
        return;
    };
    for point in descendants(root, "navPoint") {
        let mut label = first_descendant_text(point, "text");
        if label.is_empty() {
            label = first_descendant_text(point, "navLabel");
        }
        let src = descendants(point, "content")
            .first()
            .map(|el| attr(el, "src"))
            .unwrap_or_default();
        let src = src.split('#').next().unwrap_or("");
        if src.is_empty() || label.trim().is_empty() {
            continue;
        }
        let full = normalize_href(&resolve_path(ncx_dir, src));
        map.entry(full).or_insert_with(|| truncate(label.trim(), 80));
    }
}

/// 将章节 HTML 转为纯文本（保留段落换行）。
fn html_to_plain_text(html: &str) -> String {
    let doc = Html::parse_document(html);
    let mut raw = String::new();
    walk_text(doc.root_element(), &mut raw);
    let spaced = raw.replace('\u{A0}', " ");
    let mut out = spaced;
    if let Some(re) = regex_of(r"[ \t]+\n") {
        out = re.replace_all(&out, "\n").into_owned();
    }
    if let Some(re) = regex_of(r"\n{3,}") {
        out = re.replace_all(&out, "\n\n").into_owned();
    }
    out.trim().to_string()
}

fn walk_text(el: ElementRef, out: &mut String) {
    let name = el.value().name();
    if matches!(name, "script" | "style" | "noscript") {
        return;
    }
    // 原版只删 script/style/noscript，于是 `<head><title>` 会被拼进正文首行；
    // 这里连 head 子树一起跳过（新版修好），正文只取 body。
    if matches!(name, "head" | "title" | "meta" | "link") {
        return;
    }
    if name == "br" {
        out.push('\n');
        return;
    }
    for child in el.children() {
        if let Some(t) = child.value().as_text() {
            out.push_str(&t.text[..]);
        } else if let Some(sub) = ElementRef::wrap(child) {
            walk_text(sub, out);
        }
    }
    if BLOCK_TAGS.contains(&name) {
        out.push('\n');
    }
}

/// 从 HTML 中猜测章节标题（h1/h2/title）。
fn extract_html_title(html: &str) -> String {
    let doc = Html::parse_document(html);
    for sel_str in ["h1", "h2", "title"] {
        let Ok(sel) = Selector::parse(sel_str) else {
            continue;
        };
        if let Some(el) = doc.select(&sel).next() {
            let t = collapse_spaces(&el.text().collect::<Vec<_>>().join(""));
            if !t.is_empty() {
                return truncate(&t, 80);
            }
        }
    }
    String::new()
}

/// 目录标题 → 正文标题 → 文件名 → 「第 N 章」（对应 titleGuess 的 || 链）。
fn chapter_title_guess(
    toc: &HashMap<String, String>,
    html: &str,
    href: &str,
    existing: usize,
) -> String {
    if let Some(t) = toc.get(&normalize_href(href)) {
        return t.clone();
    }
    let from_html = extract_html_title(html);
    if !from_html.is_empty() {
        return from_html;
    }
    let base = basename(href);
    let stripped = regex_of(r"(?i)\.(xhtml|html|htm)$")
        .map(|re| re.replace(&base, "").into_owned())
        .unwrap_or_else(|| base.clone());
    if !stripped.is_empty() {
        return stripped;
    }
    format!("第 {} 章", existing + 1)
}

/// 取路径的目录部分。
fn dirname(p: &str) -> String {
    let n = p.replace('\\', "/");
    match n.rfind('/') {
        Some(i) => n[..i].to_string(),
        None => String::new(),
    }
}

/// 取路径的文件名部分。
fn basename(p: &str) -> String {
    let n = p.replace('\\', "/");
    match n.rfind('/') {
        Some(i) => n[i + 1..].to_string(),
        None => n,
    }
}

/// 将相对路径基于目录解析为 zip 内规范化路径。
fn resolve_path(base_dir: &str, rel: &str) -> String {
    let cleaned = rel.replace('\\', "/");
    let cleaned = cleaned.trim_start_matches('/');
    if base_dir.is_empty() {
        return cleaned.to_string();
    }
    let parts = base_dir
        .split('/')
        .filter(|p| !p.is_empty())
        .chain(cleaned.split('/'));
    let mut out: Vec<&str> = Vec::new();
    for part in parts {
        if part == "." || part.is_empty() {
            continue;
        }
        if part == ".." {
            out.pop();
        } else {
            out.push(part);
        }
    }
    out.join("/")
}

/// 规范化 href 以便与目录映射比对（小写、去前导斜杠、反斜杠转正斜杠）。
fn normalize_href(href: &str) -> String {
    href.replace('\\', "/").trim_start_matches('/').to_lowercase()
}

/// 判断 manifest 项是否为可读 HTML 章节。
fn is_html_media(media_type: &str, href: &str) -> bool {
    let htmlish = regex_of("(?i)html|xml").is_some_and(|re| re.is_match(media_type));
    let excluded = regex_of("(?i)ncx|opf|svg").is_some_and(|re| re.is_match(media_type));
    if htmlish && !excluded {
        return true;
    }
    regex_of(r"(?i)\.(xhtml|html|htm)$")
        .is_some_and(|re| re.is_match(href))
}

fn split_props(props: &str) -> Vec<&str> {
    props.split_whitespace().collect()
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

fn fallback_or_local<'a>(value: &'a str, fallback: &'a str, last: &'a str) -> String {
    if !value.is_empty() {
        return value.to_string();
    }
    if !fallback.is_empty() {
        return fallback.to_string();
    }
    last.to_string()
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn collapse_spaces(s: &str) -> String {
    regex_of(r"\s+")
        .map(|re| re.replace_all(s, " ").into_owned())
        .unwrap_or_else(|| s.to_string())
        .trim()
        .to_string()
}

thread_local! {
    static EPUB_RE_CACHE: RefCell<HashMap<String, Option<Arc<Regex>>>> = RefCell::new(HashMap::new());
}

fn regex_of(pattern: &str) -> Option<Arc<Regex>> {
    EPUB_RE_CACHE.with(|cache| {
        if let Some(hit) = cache.borrow().get(pattern) {
            return hit.clone();
        }
        let built = Regex::new(pattern).ok().map(Arc::new);
        cache.borrow_mut().insert(pattern.to_string(), built.clone());
        built
    })
}

fn parse_xml(xml: &str) -> Option<Package> {
    sxd_document::parser::parse(xml).ok()
}

/// 严格 XML 解析后取文档元素；OPF/NCX 由工具生成，容错优先级低于结果正确性。
fn parse_package(xml: &str) -> Option<Package> {
    parse_xml(xml)
}

fn root_element(pkg: &Package) -> Option<Element<'_>> {
    let doc = pkg.as_document();
    doc.root().children().into_iter().find_map(|child| match child {
        XmlRootChild::Element(el) => Some(el),
        _ => None,
    })
}

fn local_name(el: Element<'_>) -> String {
    el.name().local_part().to_string()
}

/// 文档序收集所有 local name 匹配的祖先自身与后代元素（对应 getElementsByTagName）。
fn descendants<'a>(root: Element<'a>, name: &str) -> Vec<Element<'a>> {
    let mut out = Vec::new();
    collect_descendants(root, name, &mut out);
    out
}

fn collect_descendants<'a>(el: Element<'a>, name: &str, out: &mut Vec<Element<'a>>) {
    if local_name(el) == name {
        out.push(el);
    }
    for child in el.children() {
        if let XmlChild::Element(child_el) = child {
            collect_descendants(child_el, name, out);
        }
    }
}

fn find_descendant<'a>(root: Element<'a>, name: &str) -> Option<Element<'a>> {
    descendants(root, name).into_iter().next()
}

fn first_descendant_text(el: Element<'_>, name: &str) -> String {
    descendants(el, name)
        .first()
        .map(|target| collapse_spaces(&element_text(*target)))
        .unwrap_or_default()
}

/// 递归取文本（等价于 DOM textContent）。
fn element_text(el: Element<'_>) -> String {
    let mut out = String::new();
    for child in el.children() {
        match child {
            XmlChild::Text(t) => out.push_str(t.text()),
            XmlChild::Element(e) => out.push_str(&element_text(e)),
            _ => {}
        }
    }
    out
}

fn attr(el: &Element<'_>, name: &str) -> String {
    el.attribute(name)
        .map(|a| a.value().to_string())
        .unwrap_or_default()
}

fn attr_nonempty(el: Element<'_>, name: &str) -> Option<String> {
    let v = attr(&el, name);
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn text_of_first(root: Element<'_>, tags: &[&str]) -> String {
    for tag in tags {
        let local = tag.rsplit(':').next().unwrap_or(tag);
        for el in descendants(root, local) {
            let text = collapse_spaces(&element_text(el));
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}

fn img_ext_re() -> Option<Arc<Regex>> {
    regex_of(r"(?i)\.(jpe?g|png|gif|webp|svg)$")
}

fn ncx_ext_re() -> Option<Arc<Regex>> {
    regex_of(r"(?i)\.ncx$")
}

/// 按路径查找 zip 条目名（精确优先，其次大小写不敏感回退）。
fn resolve_entry_name(archive: &Archive<'_>, path: &str) -> Option<String> {
    let normalized = path.trim_start_matches('/').to_string();
    if archive.file_names().any(|name| name == normalized) {
        return Some(normalized);
    }
    let lowered = normalized.to_lowercase();
    archive
        .file_names()
        .find(|name| name.replace('\\', "/").to_lowercase() == lowered)
        .map(str::to_string)
}

/// 流式读取 zip 条目，超过上限直接放弃（元数据预检 + 读取中双重保护）。
fn read_entry_bytes(archive: &mut Archive<'_>, path: &str, max_bytes: usize) -> Option<Vec<u8>> {
    let name = resolve_entry_name(archive, path)?;
    let mut file = archive.by_name(&name).ok()?;
    if file.is_dir() || (file.size() as usize) > max_bytes {
        return None;
    }
    let mut buf = Vec::new();
    file.by_ref()
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() > max_bytes {
        return None;
    }
    Some(buf)
}

fn read_entry_text(archive: &mut Archive<'_>, path: &str, max_bytes: usize) -> Option<String> {
    let buf = read_entry_bytes(archive, path, max_bytes)?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_paths_resolve_like_the_ts_helpers() {
        assert_eq!(dirname("OEBPS/content.opf"), "OEBPS");
        assert_eq!(basename("OEBPS/Text/chap_01.xhtml"), "chap_01.xhtml");
        assert_eq!(resolve_path("OEBPS", "../meta.xml"), "meta.xml");
        assert_eq!(resolve_path("OEBPS", "./Text/a.html"), "OEBPS/Text/a.html");
        assert_eq!(resolve_path("", "/OEBPS/x.opf"), "OEBPS/x.opf");
        assert_eq!(normalize_href("/OEBPS\\Text\\A.XHTML"), "oebps/text/a.xhtml");
    }

    #[test]
    fn html_becomes_paragraph_text() {
        let html = "<html><body><h1>标题</h1><p>甲。</p><p>乙。</p><script>var x=1</script></body></html>";
        assert_eq!(html_to_plain_text(html), "标题\n甲。\n乙。");
        assert_eq!(extract_html_title(html), "标题");
    }

    #[test]
    fn media_type_filters_chapter_items() {
        assert!(is_html_media("application/xhtml+xml", "a.xhtml"));
        assert!(is_html_media("", "chapter.html"));
        assert!(!is_html_media("application/x-dtbncx+xml", "toc.ncx"));
        assert!(!is_html_media("image/jpeg", "cover.jpg"));
    }

    #[test]
    fn xml_descendants_follow_document_order() {
        let pkg = sxd_document::parser::parse(
            "<package><manifest><item id=\"a\" href=\"1.xhtml\"/><item id=\"b\" href=\"2.xhtml\"/></manifest></package>",
        )
        .unwrap();
        let root = root_element(&pkg).unwrap();
        let ids: Vec<String> = descendants(root, "item").iter().map(|el| attr(el, "id")).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(text_of_first(root, &["dc:title"]), "");
    }

    use std::io::Write;
    use zip::write::{FileOptions, ZipWriter};
    use zip::CompressionMethod;

    /// 往 zip 里写一个条目。
    fn write_entry(zip: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str, data: &[u8], deflate: bool) {
        let options = if deflate {
            FileOptions::default().compression_method(CompressionMethod::Deflated)
        } else {
            FileOptions::default().compression_method(CompressionMethod::Stored)
        };
        zip.start_file(name, options).unwrap();
        zip.write_all(data).unwrap();
    }

    /// 构造一份最小但结构完整的 EPUB（OPF + NCX + 两章 + 封面）。
    fn build_epub() -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        write_entry(&mut zip, "mimetype", b"application/epub+zip", false);
        write_entry(
            &mut zip,
            "META-INF/container.xml",
            r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.as_bytes(),
            true,
        );
        write_entry(
            &mut zip,
            "OEBPS/content.opf",
            r#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>测试书名</dc:title><dc:creator>测试作者</dc:creator><meta name="cover" content="cover-img"/></metadata><manifest><item id="ch1" href="Text/chap1.xhtml" media-type="application/xhtml+xml"/><item id="ch2" href="Text/chap2.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="cover-img" href="images/cover.jpg" media-type="image/jpeg"/></manifest><spine toc="ncx"><itemref idref="ch1"/><itemref idref="ch2"/></spine></package>"#.as_bytes(),
            true,
        );
        write_entry(
            &mut zip,
            "OEBPS/toc.ncx",
            r#"<?xml version="1.0" encoding="UTF-8"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><head><meta name="dtb:uid" content="uid"/></head><docTitle><text>测试书名</text></docTitle><navMap><navPoint id="n1" playOrder="1"><navLabel><text>起</text></navLabel><content src="Text/chap1.xhtml"/></navPoint><navPoint id="n2" playOrder="2"><navLabel><text>承</text></navLabel><content src="Text/chap2.xhtml"/></navPoint></navMap></ncx>"#.as_bytes(),
            true,
        );
        write_entry(
            &mut zip,
            "OEBPS/Text/chap1.xhtml",
            r#"<html><head><title>不该被用</title></head><body><h1>第一章 起</h1><p>甲。</p><p>乙。</p><script>var x=1</script></body></html>"#.as_bytes(),
            true,
        );
        write_entry(
            &mut zip,
            "OEBPS/Text/chap2.xhtml",
            r#"<html><body><p>丙。</p><p>&#160;丁。</p></body></html>"#.as_bytes(),
            true,
        );
        write_entry(&mut zip, "OEBPS/images/cover.jpg", &[0xff, 0xd8, 0xff, 0xd9], false);
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_real_epub_parses_into_chapters_and_cover() {
        let bytes = build_epub();
        let meta = parse_epub_buffer(&bytes, "回退名").unwrap();
        assert_eq!(meta.title, "测试书名");
        assert_eq!(meta.author, "测试作者");
        // 目录标题优先于正文里的 h1
        let titles: Vec<&str> = meta.chapters.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["起", "承"]);
        assert_eq!(meta.chapters[0].content, "第一章 起\n甲。\n乙。");
        assert_eq!(meta.chapters[1].content, "丙。\n 丁。");
        assert!(meta
            .cover_data_url
            .as_deref()
            .is_some_and(|url| url.starts_with("data:image/jpeg;base64,")));
    }

    #[test]
    fn zip_bomb_ratio_is_rejected() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        // 5MB 全零正文：压缩后极小，压缩比远超 500:1
        write_entry(&mut zip, "boom.txt", &vec![0u8; 5 * 1024 * 1024], true);
        let bytes = zip.finish().unwrap().into_inner();
        let err = parse_epub_buffer(&bytes, "回退名").unwrap_err();
        assert!(err.contains("压缩比异常"), "实际错误：{err}");
    }

    #[test]
    fn missing_container_is_reported() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        write_entry(&mut zip, "mimetype", b"application/epub+zip", false);
        let bytes = zip.finish().unwrap().into_inner();
        assert_eq!(
            parse_epub_buffer(&bytes, "回退名").unwrap_err(),
            "无效的 EPUB：缺少 META-INF/container.xml"
        );
    }
}
