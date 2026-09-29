//! 正文段落整理，对应原 Electron 版 src/shared/novelText.ts。
//!
//! 长度判定统一按 UTF-16 码元计数（`js_len`），与 JS 的 `String.length` 一致，
//! 否则 Rust 的字节长度会让中文段落数放大三倍，拆段/打包阈值全部跑偏。

use regex::Regex;

const PACK_TARGET: usize = 120;

fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// 统一换行符为 `\n`，对应 normalizeLineEndings。
pub fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace(['\u{2028}', '\u{2029}'], "\n")
}

/// 是否为小说正文已有的 HTML 结构标签，对应 `/<\/?[a-z][\s\S]*>/i`。
fn looks_like_html(s: &str) -> bool {
    Regex::new(r"(?i)<\/?[a-z][\s\S]*>").is_ok_and(|re| re.is_match(s))
}

/// 中文网文段落整理，对应 ensureNovelParagraphs。
pub fn ensure_novel_paragraphs(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut s = normalize_line_endings(text).trim().to_string();
    if s.is_empty() {
        return String::new();
    }

    if looks_like_html(&s) {
        s = strip_html_shape(&s);
    }

    s = if let Some(re) = Regex::new("(?i)&nbsp;").ok() {
        re.replace_all(&s, " ").to_string()
    } else {
        s
    };
    if let Some(re) = Regex::new("\u{3000}{2,}").ok() {
        s = re.replace_all(&s, "\n").to_string();
    }
    if let Some(re) = Regex::new(r"(?m)^[\u{3000}]+").ok() {
        s = re.replace_all(&s, "").to_string();
    }

    // 先按已有换行切成块，并合并明显「句中被截断」的短行
    let rough: Vec<String> = split_by_regex(&s, r"\n+")
        .into_iter()
        .map(|line| collapse_inline_spaces(&line).trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    let mut merged: Vec<String> = Vec::new();
    for p in rough {
        match merged.last_mut() {
            Some(prev)
                if !ends_with_sentence_boundary(prev)
                    && !starts_with_quote(&p)
                    && js_len(prev) < 80 =>
            {
                prev.push_str(&p);
            }
            _ => merged.push(p),
        }
    }

    let total: usize = merged.iter().map(|p| js_len(p)).sum();
    let avg = total as f64 / merged.len().max(1) as f64;
    // 仅当几乎没有段落结构（整墙字）时才主动拆分
    let needs_dense_split = merged.len() <= 1 || (merged.len() < 4 && avg > 220.0);

    let mut pieces: Vec<String> = Vec::new();
    if needs_dense_split {
        for block in merged {
            if js_len(&block) <= PACK_TARGET {
                pieces.push(block);
                continue;
            }
            let parts = split_dense_block(&block);
            if parts.is_empty() {
                pieces.push(block);
            } else {
                pieces.extend(parts);
            }
        }
    } else {
        pieces = merged;
    }

    // 仅当段落明显过碎（如历史缓存「一句一段」）时才重新打包
    let avg_after =
        pieces.iter().map(|p| js_len(p)).sum::<usize>() as f64 / pieces.len().max(1) as f64;
    if pieces.len() >= 8 && avg_after < 55.0 {
        pieces = pack_short_paragraphs(&pieces, PACK_TARGET);
    }

    let joined = pieces
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let collapsed = match Regex::new(r"\n{3,}") {
        Ok(re) => re.replace_all(&joined, "\n\n").to_string(),
        Err(_) => joined,
    };
    collapsed.trim().to_string()
}

/// 按正则切分（用于 `\n+` 这类多字符分隔），对应 JS 的 `split(RegExp)`。
fn split_by_regex(text: &str, pattern: &str) -> Vec<String> {
    match Regex::new(pattern) {
        Ok(re) => re.split(text).map(str::to_string).collect(),
        Err(_) => vec![text.to_string()],
    }
}

/// 去掉 HTML 结构标签但保留换行语义，对应 ensureNovelParagraphs 里的四次 replace。
fn strip_html_shape(s: &str) -> String {
    let mut out = s.to_string();
    for (pattern, replacement) in [
        // 脚本与样式整块丢弃（含其后一个闭合标签由非贪婪匹配保证）
        (r"(?is)<(script|style)[^>]*>.*?</\1>", ""),
        (r"(?i)<br\s*/?>", "\n"),
        (r"(?i)</?(p|div|section|article|tr|li|h[1-6]|blockquote)(\s[^>]*)?>", "\n"),
        (r"(?s)<[^>]+>", ""),
    ] {
        if let Ok(re) = Regex::new(pattern) {
            out = re.replace_all(&out, replacement).to_string();
        }
    }
    out
}

/// 段末是否为句末/引用收尾标点，对应 `/[。！？…~～”」』'’]$/`。
fn ends_with_sentence_boundary(s: &str) -> bool {
    matches!(
        s.chars().last(),
        Some('。') | Some('！') | Some('？') | Some('…') | Some('~') | Some('～') | Some('”')
            | Some('」') | Some('』') | Some('\'') | Some('’')
    )
}

/// 段首是否为开引号，对应 `/^[“「『"‘]/`。
fn starts_with_quote(s: &str) -> bool {
    matches!(
        s.chars().next(),
        Some('“') | Some('「') | Some('『') | Some('"') | Some('‘')
    )
}

/// 行内连续空白压成一个空格，对应 `/[ \t\f\v]+/g`。
fn collapse_inline_spaces(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut pending = false;
    for ch in line.chars() {
        if matches!(ch, ' ' | '\t' | '\u{c}' | '\u{b}') {
            pending = true;
            continue;
        }
        if pending {
            out.push(' ');
        }
        pending = false;
        out.push(ch);
    }
    out
}

/// 过密文本拆段，对应 splitDenseBlock。
///
/// 原版三处都用了后向预测 `(?=…)`，而 rust regex 不支持预测：改为把后一个字符也纳入
/// 捕获组，替换时原样补回，效果与「在边界处插入标记」一致。
fn split_dense_block(block: &str) -> Vec<String> {
    let mut s = block.to_string();
    for (pattern, replacement) in [
        // 对话起止处硬换行，阅读节奏更清晰
        (r"([。！？~～]|……|…)([“「『])", "$1\n$2"),
        (r"([”」』])([^“「『”」』\n\s])", "$1\n$2"),
        // 句末标点作为软边界，供后续按目标长度打包
        (r"([。！？][”」』'’]?)([^\n])", "$1\u{1}$2"),
    ] {
        if let Ok(re) = Regex::new(pattern) {
            s = re.replace_all(&s, replacement).to_string();
        }
    }

    let sentences: Vec<String> = s
        .split(['\u{1}', '\n'])
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    pack_short_paragraphs(&sentences, PACK_TARGET)
}

/// 短句打包成目标字数段落，对应 packShortParagraphs。
fn pack_short_paragraphs(parts: &[String], target: usize) -> Vec<String> {
    if parts.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();

    for part in parts {
        if buf.is_empty() {
            buf = part.clone();
            continue;
        }
        if js_len(&buf) >= target {
            out.push(std::mem::take(&mut buf));
            buf = part.clone();
            continue;
        }
        if js_len(&buf) + js_len(part) <= (target as f64 * 1.6).floor() as usize {
            buf.push_str(part);
        } else {
            out.push(std::mem::take(&mut buf));
            buf = part.clone();
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_normalized() {
        assert_eq!(normalize_line_endings("a\r\nb\rc\u{2028}d"), "a\nb\nc\nd");
    }

    #[test]
    fn html_br_and_p_become_paragraphs() {
        let got = ensure_novel_paragraphs("<p>第一段。</p><p>第二段。</p>");
        assert_eq!(got, "第一段。\n\n第二段。");
    }

    #[test]
    fn full_width_indent_is_stripped() {
        let got = ensure_novel_paragraphs("　　开头一句。\n　　第二句。");
        assert_eq!(got, "开头一句。\n\n第二句。");
    }

    #[test]
    fn dense_wall_of_text_is_split_by_sentence() {
        // 与原版一致：无换行的整墙字要长过打包上限（120*1.6）才会被拆成多段
        let mut block = String::new();
        for i in 0..12 {
            block.push_str(&format!(
                "第{i}句是他离开的那天说过的话，风从山口灌进来，把炊烟吹散在半空里，她没有回头。\n"
            ));
        }
        let block = block.replace('\n', "");
        assert!(js_len(&block) > 200, "测试样本需足够长");
        let got = ensure_novel_paragraphs(&block);
        assert!(got.contains("\n\n"), "整墙字应被拆段：{got}");
    }

    #[test]
    fn short_single_block_is_not_split() {
        // 与原版一致：不足打包长度的单块保持原样，不在每个句号后强行断段
        let got = ensure_novel_paragraphs("他走了。天亮了。风也停了。");
        assert_eq!(got, "他走了。天亮了。风也停了。");
    }

    #[test]
    fn truncated_short_lines_are_merged() {
        let got = ensure_novel_paragraphs("少年抬起头\n望向远方的山峰。");
        assert_eq!(got, "少年抬起头望向远方的山峰。");
    }

    #[test]
    fn length_uses_utf16_units_like_js() {
        assert_eq!(js_len("中文abc"), 5);
    }
}
