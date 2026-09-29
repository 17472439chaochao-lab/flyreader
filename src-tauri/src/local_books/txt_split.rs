//! TXT 编码探测与按标题行启发式切章（对应 main/localBooks/txtSplit.ts）。

use std::sync::OnceLock;

use regex::Regex;

use super::fallback_or;
use super::SplitChapter;
use crate::engine::text::normalize_line_endings;

/// 超过这个长度的行不可能是标题（原版 MAX_HEADING_LEN）。
const MAX_HEADING_LEN: usize = 80;
/// 无标题可切时的单章体积上限（原版 chunk = 12000）。
const SIZE_CHUNK: usize = 12000;

const HEADING_PATTERN: &str = concat!(
    r"^[\s　\[【《（(]*第[零〇一二三四五六七八九十百千万两0-9]+[章节回卷部集篇].{0,60}$",
    "|",
    r"^[\s　\[【《（(]*(?:序章|序言|楔子|引子|前言|绪论|后记|尾声|终章|番外(?:篇)?|感言|完本感言).{0,40}$",
    "|",
    r"^[\s　\[【]*(?:Chapter|CHAPTER|Ch\.?)\s*[0-9IVXLCDM]+\.?\s*.{0,50}$",
    "|",
    r"^[\s　]*[0-9]{1,4}[\.、．]\s*\S.{0,40}$"
);

/// 纯装饰行（省略号、破折号分隔线等）不能当标题。
const FILLER_PATTERN: &str = r"^[…—\-·•\s　]+$";

/// 「强标题」：足够多时才相信宽泛的编号行，避免把正文里的 `1.` 列表当目录。
const STRONG_PATTERN: &str = r"(?i)第.+[章节回卷部集篇]|楔子|序章|Chapter";

fn heading_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(HEADING_PATTERN).ok()).as_ref()
}

fn filler_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(FILLER_PATTERN).ok()).as_ref()
}

fn strong_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(STRONG_PATTERN).ok()).as_ref()
}

fn edge_brackets_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[\s　\[【《（(]+|[\]】》）)\s　]+$").ok()).as_ref()
}

/// 去掉首部 UTF-8 BOM（原版 `replace(/^\uFEFF/, '')`）。
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{FEFF}').unwrap_or(text)
}

/// 清理章节标题首尾括号空白并截断长度（对应 sanitizeTitle）。
fn sanitize_title(t: &str) -> String {
    let cleaned = match edge_brackets_re() {
        Some(re) => re.replace_all(t, "").into_owned(),
        None => t.to_string(),
    };
    let cut: String = cleaned.chars().take(80).collect();
    if cut.is_empty() {
        "未命名章节".to_string()
    } else {
        cut
    }
}

/// 将 TXT 全文按标题行切分为章节；命中过少时按体积切分。
pub fn split_txt_chapters(raw: &str, fallback_title: &str) -> Vec<SplitChapter> {
    let normalized = normalize_line_endings(raw);
    let text = strip_bom(&normalized).trim().to_string();
    if text.is_empty() {
        return vec![SplitChapter::new(
            fallback_or(fallback_title, "全文"),
            "（文件为空）",
        )];
    }

    let lines: Vec<&str> = text.split('\n').collect();
    let Some(re) = heading_re() else {
        return split_by_size(&text, fallback_or(fallback_title, "全文"));
    };
    let mut hits: Vec<(usize, String)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_HEADING_LEN {
            continue;
        }
        if !re.is_match(trimmed) {
            continue;
        }
        if filler_re().map(|re| re.is_match(trimmed)).unwrap_or(false) {
            continue;
        }
        hits.push((i, sanitize_title(trimmed)));
    }

    let strong: Vec<(usize, String)> = hits
        .iter()
        .filter(|(_, title)| strong_re().map(|re| re.is_match(title)).unwrap_or(false))
        .cloned()
        .collect();
    let use_hits = if strong.len() >= 3 { strong } else { hits };

    if use_hits.len() < 2 {
        return split_by_size(&text, fallback_or(fallback_title, "全文"));
    }

    let mut chapters: Vec<SplitChapter> = Vec::new();
    if use_hits[0].0 > 0 {
        let preamble = lines[..use_hits[0].0].join("\n").trim().to_string();
        if preamble.chars().count() > 80 {
            chapters.push(SplitChapter::new("前言", preamble));
        }
    }

    for i in 0..use_hits.len() {
        let start = use_hits[i].0 + 1;
        let end = if i + 1 < use_hits.len() {
            use_hits[i + 1].0
        } else {
            lines.len()
        };
        let body = lines[start..end].join("\n").trim().to_string();
        let content = if body.is_empty() { "（本章无正文）".to_string() } else { body };
        chapters.push(SplitChapter::new(use_hits[i].1.clone(), content));
    }

    if chapters.is_empty() {
        return vec![SplitChapter::new(
            fallback_or(fallback_title, "全文"),
            text,
        )];
    }
    chapters
}

/// 按固定体积切分长文本为多章（对应 splitBySize，切点回退到换行）。
fn split_by_size(text: &str, title: &str) -> Vec<SplitChapter> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= (SIZE_CHUNK as f64 * 1.4) as usize {
        return vec![SplitChapter::new(title, text)];
    }
    let mut parts: Vec<SplitChapter> = Vec::new();
    let mut i = 0usize;
    let mut n = 1usize;
    while i < chars.len() {
        let mut end = (i + SIZE_CHUNK).min(chars.len());
        if end < chars.len() {
            let nl = chars[..=end].iter().rposition(|c| *c == '\n');
            if let Some(pos) = nl {
                if pos > i + SIZE_CHUNK / 2 {
                    end = pos;
                }
            }
        }
        let slice: String = chars[i..end].iter().collect();
        let body = slice.trim();
        parts.push(SplitChapter::new(
            format!("{title} · {n}"),
            if body.is_empty() { "（空）" } else { body },
        ));
        i = end;
        n += 1;
    }
    parts
}

/// 检测编码并解码 TXT 缓冲（UTF-8 / UTF-16 BOM / GB18030 / GBK 等）。
pub fn decode_txt_buffer(buf: &[u8]) -> String {
    if buf.len() >= 3 && buf[..3] == [0xef, 0xbb, 0xbf] {
        return String::from_utf8_lossy(&buf[3..]).into_owned();
    }
    if buf.len() >= 2 && buf[..2] == [0xff, 0xfe] {
        return decode_utf16(&buf[2..], true);
    }
    if buf.len() >= 2 && buf[..2] == [0xfe, 0xff] {
        return decode_utf16(&buf[2..], false);
    }

    let utf8 = String::from_utf8_lossy(buf).into_owned();
    let mut cands: Vec<(f64, String)> = vec![(score_decoded_text(&utf8), utf8.clone())];
    for label in ["gb18030", "gbk", "gb2312", "big5"] {
        let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) else {
            continue;
        };
        let (cow, _, _) = enc.decode(buf);
        let text = cow.into_owned();
        cands.push((score_decoded_text(&text), text));
    }
    // 稳定排序 + 分数降序：同分时保留 UTF-8 候选（与原版 cands 顺序一致）
    cands.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    cands
        .into_iter()
        .next()
        .map(|(_, text)| text)
        .unwrap_or(utf8)
}

/// 把字节流按 UTF-16（默认小端）解码，非法序列换成替换符。
fn decode_utf16(bytes: &[u8], little: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if little {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// 为解码候选打分：更倾向 CJK 多、替换符/控制符少的结果。
fn score_decoded_text(s: &str) -> f64 {
    let sample: String = s.chars().take(12000).collect();
    if sample.is_empty() {
        return -1e9;
    }
    let mut cjk = 0f64;
    let mut punct = 0f64;
    let mut bad = 0f64;
    let mut ctrl = 0f64;
    let mut latin = 0f64;
    for ch in sample.chars() {
        let code = ch as u32;
        if code == 0xfffd {
            bad += 1.0;
            continue;
        }
        if code < 9 || (code > 13 && code < 32) {
            ctrl += 1.0;
            continue;
        }
        if (0x4e00..=0x9fff).contains(&code) {
            cjk += 1.0;
        } else if (0x3400..=0x4dbf).contains(&code) {
            cjk += 1.0;
        } else if (0x3000..=0x303f).contains(&code) || (0xff00..=0xffef).contains(&code) {
            punct += 1.0;
        } else if (0x41..=0x7a).contains(&code) || (0xc0..=0x24f).contains(&code) {
            latin += 1.0;
        }
    }
    cjk * 3.0 + punct - bad * 80.0 - ctrl * 40.0 - (latin - cjk).max(0.0) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_headings_split_the_book() {
        let raw = "书名测试\n\n第一章 起点\n正文甲。\n正文甲续。\n第二章 转折\n正文乙。\n";
        let parts = split_txt_chapters(raw, "示例");
        let titles: Vec<&str> = parts.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, vec!["第一章 起点", "第二章 转折"]);
        assert_eq!(parts[0].content, "正文甲。\n正文甲续。");
        // 「书名测试」只有 4 个字，不足前言长度门槛（80），不会额外生成前言章
    }

    #[test]
    fn too_few_headings_fall_back_to_size_split() {
        let raw = "一".repeat(500);
        let parts = split_txt_chapters(&raw, "示例");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].title, "示例");
    }

    #[test]
    fn long_texts_split_by_size_with_numbered_titles() {
        let mut raw = String::new();
        for i in 0..2000 {
            raw.push_str(&format!("这是第 {i} 行正文，用来把体积推过单章上限。\n"));
        }
        let parts = split_txt_chapters(&raw, "示例");
        assert!(parts.len() >= 2, "超过 1.4 倍上限要切分");
        assert!(parts[0].title.starts_with("示例 · "));
        assert_eq!(parts.last().unwrap().title, format!("示例 · {}", parts.len()));
    }

    #[test]
    fn gbk_bytes_decode_to_cjk() {
        // 「第一章 起点」的 GB18030 编码
        let source = "第一章 起点\n正文。\n";
        let (cow, _, _) = encoding_rs::GBK.encode(source);
        // GBK 编码器输出的是 GBK 码；这里再解回来验证候选打分能选中它
        let decoded = decode_txt_buffer(&cow);
        assert!(decoded.contains("第一章 起点"), "解码结果：{decoded}");
    }

    #[test]
    fn utf16_le_bom_is_detected() {
        let mut bytes = vec![0xffu8, 0xfe];
        for unit in "第一章 甲\n正文。\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_txt_buffer(&bytes), "第一章 甲\n正文。\n");
    }

    #[test]
    fn decoration_lines_are_not_titles_and_titles_are_trimmed() {
        let raw = "…………\n【第一章・起点】\n正文甲。\n【第二章・归途】\n正文乙。\n";
        let parts = split_txt_chapters(raw, "示例");
        let titles: Vec<&str> = parts.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, vec!["第一章・起点", "第二章・归途"]);
    }
}
