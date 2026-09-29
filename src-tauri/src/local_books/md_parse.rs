//! Markdown 导入：保留原文，按标题生成目录（对应 main/localBooks/mdParse.ts）。
//!
//! 每章 `content` 都是整份全文，阅读器靠 `md_anchor`（`md-h-{序号}`）滚动定位，
//! 与 GitHub / Gitee 的文档页行为一致。

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use regex::Regex;

use super::fallback_or;
use super::txt_split::{decode_txt_buffer, strip_bom};
use super::SplitChapter;
use crate::engine::text::normalize_line_endings;

/// ATX 标题（`# 标题`）。
const ATX_PATTERN: &str = r"^(#{1,6})\s+(.+?)\s*#*\s*$";
/// Setext 标题下划线（`===` / `---`）。
const SETEXT_PATTERN: &str = r"^(=+|-+)\s*$";

/// 标题文本清洗用到的行内标记（顺序与原版一致）。
const MARK_PATTERNS: [&str; 6] = [
    r"!\[([^\]]*)\]\([^)]+\)",
    r"\[([^\]]+)\]\([^)]+\)",
    r"`([^`]+)`",
    r"\*\*(.+?)\*\*|__(.+?)__",
    r"\*(.+?)\*|_(.+?)_",
    "[#*_~`]+",
];

pub struct MdMeta {
    pub title: String,
    pub chapters: Vec<SplitChapter>,
}

#[derive(Clone)]
struct MdHeading {
    line: usize,
    level: usize,
    title: String,
    /// 文档中标题序号（h1–h6 全局递增），用于阅读器锚点滚动
    heading_index: usize,
}

thread_local! {
    static MD_RE_CACHE: RefCell<HashMap<&'static str, Option<Arc<Regex>>>> = RefCell::new(HashMap::new());
}

fn md_re(pattern: &'static str) -> Option<Arc<Regex>> {
    MD_RE_CACHE.with(|cache| {
        if let Some(hit) = cache.borrow().get(pattern) {
            return hit.clone();
        }
        let built = Regex::new(pattern).ok().map(Arc::new);
        cache.borrow_mut().insert(pattern, built.clone());
        built
    })
}

fn is_setext_line(line: Option<&&str>) -> bool {
    let Some(raw) = line else { return false };
    md_re(SETEXT_PATTERN)
        .map(|re| re.is_match(raw.trim()))
        .unwrap_or(false)
}

/// 解析 Markdown 缓冲：整份全文 + 标题锚点目录。
pub fn parse_md_buffer(buf: &[u8], fallback_title: &str) -> MdMeta {
    let raw = decode_txt_buffer(buf);
    let text = strip_bom(&normalize_line_endings(&raw)).trim().to_string();
    if text.is_empty() {
        return MdMeta {
            title: fallback_or(fallback_title, "未命名").to_string(),
            chapters: vec![SplitChapter::new(
                fallback_or(fallback_title, "全文"),
                "（文件为空）",
            )],
        };
    }

    let lines: Vec<&str> = text.split('\n').collect();
    let headings = collect_headings(&lines);
    let book_title = pick_book_title(&headings, fallback_title);
    let chapter_heads = pick_chapter_headings(&headings);

    // 单篇文档：整份 Markdown 作为一章
    if chapter_heads.len() < 2 {
        let anchor = headings
            .first()
            .map(|h| format!("md-h-{}", h.heading_index));
        return MdMeta {
            title: book_title.clone(),
            chapters: vec![SplitChapter::with_anchor(book_title, text, anchor)],
        };
    }

    let mut chapters: Vec<SplitChapter> = Vec::new();
    let first_line = chapter_heads[0].line;

    // 标题前有足够前言（徽章、简介等）时，增加「前言」入口，滚到文首
    if first_line > 0 {
        let title_heading = headings.iter().find(|h| h.level == 1 && h.line < first_line);
        let mut pre_start = 0usize;
        if let Some(th) = title_heading.filter(|h| h.line == 0) {
            pre_start = th.line + 1;
            if pre_start < first_line && is_setext_line(lines.get(pre_start)) {
                pre_start += 1;
            }
        }
        let preamble = lines[pre_start..first_line].join("\n").trim().to_string();
        if preamble.chars().count() > 40 {
            chapters.push(SplitChapter::new("前言", text.clone()));
        }
    }

    for h in &chapter_heads {
        chapters.push(SplitChapter::with_anchor(
            h.title.clone(),
            text.clone(),
            Some(format!("md-h-{}", h.heading_index)),
        ));
    }

    if chapters.is_empty() {
        chapters.push(SplitChapter::new(book_title.clone(), text));
    }
    MdMeta {
        title: book_title,
        chapters,
    }
}

/// 收集 ATX（#）与 Setext（=== / ---）标题，跳过代码围栏内的行。
fn collect_headings(lines: &[&str]) -> Vec<MdHeading> {
    let mut hits: Vec<MdHeading> = Vec::new();
    let mut in_fence = false;
    let mut heading_index = 0usize;
    let mut i = 0usize;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            i += 1;
            continue;
        }
        if in_fence {
            i += 1;
            continue;
        }

        let mut matched_atx = false;
        if let Some(re) = md_re(ATX_PATTERN) {
            if let Some(caps) = re.captures(trimmed) {
                let hashes = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let raw_title = caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
                hits.push(MdHeading {
                    line: i,
                    level: hashes.chars().count(),
                    title: sanitize_md_title(&raw_title),
                    heading_index,
                });
                heading_index += 1;
                matched_atx = true;
            }
        }
        if matched_atx {
            i += 1;
            continue;
        }

        if i + 1 < lines.len() && !trimmed.is_empty() && !trimmed.starts_with('#') {
            let under = lines[i + 1].trim();
            let matched = md_re(SETEXT_PATTERN)
                .map(|re| re.is_match(under))
                .unwrap_or(false);
            if matched && under.chars().count() >= 2 {
                let level = if under.starts_with('=') { 1 } else { 2 };
                hits.push(MdHeading {
                    line: i,
                    level,
                    title: sanitize_md_title(trimmed),
                    heading_index,
                });
                heading_index += 1;
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    hits
}

fn pick_book_title(headings: &[MdHeading], fallback: &str) -> String {
    headings
        .iter()
        .find(|h| h.level == 1)
        .map(|h| h.title.clone())
        .unwrap_or_else(|| fallback_or(fallback, "未命名").to_string())
}

/// 选择目录标题：优先同级 H1（≥2）；否则同级 H2（≥2）；再退到最浅且出现 ≥2 次的级别。
/// 若仅有一个 H1 且其后有多个 H2，则用 H2 作目录（H1 作书名）。
fn pick_chapter_headings(headings: &[MdHeading]) -> Vec<MdHeading> {
    if headings.len() < 2 {
        return Vec::new();
    }
    let group = |level: usize| -> Vec<MdHeading> {
        headings.iter().filter(|h| h.level == level).cloned().collect()
    };
    let h1 = group(1);
    let h2 = group(2);
    if h1.len() >= 2 {
        return h1;
    }
    if h1.len() == 1 && h2.len() >= 2 {
        return h2;
    }
    if h2.len() >= 2 {
        return h2;
    }
    for level in 1..=6usize {
        let hits = group(level);
        if hits.len() >= 2 {
            return hits;
        }
    }
    Vec::new()
}

/// 去掉行内标记，只留可读标题文本。
/// 原版用后向引用配对强调标记（`(\*\*|__)(.+?)\1`）；Rust 正则不支持后向引用，
/// 这里按「先强后弱」各展开成两条分支，结果一致。
fn sanitize_md_title(t: &str) -> String {
    let mut out = t.to_string();
    for pattern in MARK_PATTERNS {
        let Some(re) = md_re(pattern) else { continue };
        // 无捕获组的模式（末尾那条）自然取到 None，等价于替换为空串
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                caps.get(1)
                    .or_else(|| caps.get(2))
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default()
            })
            .into_owned();
    }
    let cut: String = out.trim().chars().take(80).collect();
    if cut.is_empty() {
        "未命名章节".to_string()
    } else {
        cut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    fn titles(meta: &MdMeta) -> Vec<&str> {
        meta.chapters.iter().map(|c| c.title.as_str()).collect()
    }

    #[test]
    fn repeated_h1_headings_become_the_toc() {
        let md = "# 第一章 起\n\n正文。\n\n# 第二章 承\n\n正文。\n";
        let meta = parse_md_buffer(&bytes(md), "回退名");
        assert_eq!(meta.title, "第一章 起");
        assert_eq!(titles(&meta), vec!["第一章 起", "第二章 承"]);
        assert_eq!(meta.chapters[0].md_anchor.as_deref(), Some("md-h-0"));
        assert_eq!(meta.chapters[1].md_anchor.as_deref(), Some("md-h-1"));
        // 全文模式：每章内容都是整份 Markdown
        assert_eq!(meta.chapters[1].content, md.trim());
    }

    #[test]
    fn single_h1_falls_back_to_h2_toc() {
        let md = "# 书名\n\n## 一\n\n甲。\n\n## 二\n\n乙。\n";
        let meta = parse_md_buffer(&bytes(md), "回退名");
        assert_eq!(meta.title, "书名");
        assert_eq!(titles(&meta), vec!["一", "二"]);
        assert_eq!(meta.chapters[0].md_anchor.as_deref(), Some("md-h-1"));
    }

    #[test]
    fn single_document_is_one_chapter_with_first_heading_anchor() {
        let md = "# 只有一篇\n\n内容若干。\n";
        let meta = parse_md_buffer(&bytes(md), "回退名");
        assert_eq!(meta.chapters.len(), 1);
        assert_eq!(meta.chapters[0].title, "只有一篇");
        assert_eq!(meta.chapters[0].md_anchor.as_deref(), Some("md-h-0"));
    }

    #[test]
    fn long_preamble_gets_an_entrance() {
        let intro = "这是一段足够长的简介文字，用来触发前言入口的四十个字符门槛，超出这个长度就会额外生成一个前言章。";
        let md = format!("# 书名\n\n{intro}\n\n## 第一章\n\n正文。\n\n## 第二章\n\n正文。\n");
        let meta = parse_md_buffer(&bytes(&md), "回退名");
        assert_eq!(titles(&meta)[0], "前言");
        assert!(meta.chapters[0].md_anchor.is_none());
    }

    #[test]
    fn short_preamble_is_skipped() {
        let md = "# 书名\n\n短简介\n\n## 第一章\n\n正文。\n\n## 第二章\n\n正文。\n";
        let meta = parse_md_buffer(&bytes(md), "回退名");
        assert_eq!(titles(&meta), vec!["第一章", "第二章"]);
    }

    #[test]
    fn inline_marks_are_stripped_from_titles() {
        let md = "# **重点**小节\n\n正文。\n\n# `代码`标题\n\n正文。\n";
        let meta = parse_md_buffer(&bytes(md), "回退名");
        assert_eq!(titles(&meta), vec!["重点小节", "代码标题"]);
    }

    #[test]
    fn setext_headings_are_detected() {
        let md = "标题一\n===\n\n甲。\n\n标题二\n===\n\n乙。\n";
        let meta = parse_md_buffer(&bytes(md), "回退名");
        assert_eq!(titles(&meta), vec!["标题一", "标题二"]);
    }

    #[test]
    fn code_fence_contents_are_not_headings() {
        let md = "# 甲\n\n```\n# 假标题\n```\n\n# 乙\n";
        let meta = parse_md_buffer(&bytes(md), "回退名");
        assert_eq!(titles(&meta), vec!["甲", "乙"]);
    }
}
