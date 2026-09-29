//! 搜索相关度与书源排序，对齐 `shared/searchRelevance.ts` 与 `shared/types.ts` 的同名函数。
//! 长度按 char 计数：JS 用 UTF-16 码元，中文书名两者一致，只有 emoji 书名会在「≤4」
//! 这类阈值上差 1～2，属于可接受的偏差。

use std::collections::HashMap;

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// 规范化书名/作者：小写、去空白、去书名号与常见标点、去「精校版/txt」等版本文案。
pub fn normalize_search_text(raw: &str) -> String {
    static VERSIONS: OnceLock<Option<Regex>> = OnceLock::new();
    let dropped = |c: char| {
        c.is_whitespace()
            || matches!(c,
                '《' | '》' | '〈' | '〉' | '「' | '」' | '『' | '』' | '【' | '】' | '〔' | '〕' | '[' | ']'
                | '（' | '）' | '(' | ')' | '·' | '・' | '．' | '.' | '。' | '，' | ',' | '、'
                | '：' | ':' | '；' | ';' | '！' | '!' | '？' | '?'
                | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}' | '"' | '\''
                | '|' | '｜' | '-' | '—' | '_' | '~' | '～' | '…')
    };
    let stripped: String = raw.to_lowercase().chars().filter(|c| !dropped(*c)).collect();
    let re = VERSIONS.get_or_init(|| {
        Regex::new(r"(?i)(精校版|无删减版?|完结版?|完本|全本|最新章节|全文阅读|txt|epub|mobi|azw3|vip)").ok()
    });
    match re {
        Some(re) => re.replace_all(&stripped, "").into_owned(),
        None => stripped,
    }
}

/// 最长公共连续子串长度（DP 滚动数组，与原版一致）。
fn longest_common_substring(a: &[char], b: &[char]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut row = vec![0usize; b.len() + 1];
    let mut best = 0usize;
    for x in a {
        let mut prev = 0usize;
        for (j, y) in b.iter().enumerate() {
            let tmp = row[j + 1];
            row[j + 1] = if x == y { prev + 1 } else { 0 };
            if row[j + 1] > best {
                best = row[j + 1];
            }
            prev = tmp;
        }
    }
    best
}

/// 书名相似度 0～1：包含关系按比例给分，否则用最长公共子串相对较短一侧。
pub fn title_similarity(keyword: &str, title: &str) -> f64 {
    let key = normalize_search_text(keyword);
    let name = normalize_search_text(title);
    if key.is_empty() || name.is_empty() {
        return 0.0;
    }
    if name == key {
        return 1.0;
    }
    let (kl, nl) = (key.chars().count(), name.chars().count());
    if name.contains(&key) || key.contains(&name) {
        return (kl.min(nl) as f64) / (kl.max(nl) as f64);
    }
    let key_chars: Vec<char> = key.chars().collect();
    let name_chars: Vec<char> = name.chars().collect();
    let lcs = longest_common_substring(&key_chars, &name_chars);
    let denom = kl.min(nl);
    if denom == 0 {
        0.0
    } else {
        lcs as f64 / denom as f64
    }
}

fn str_field<'a>(book: &'a Value, key: &str) -> &'a str {
    book.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 命中是否与关键词相关：过滤「搜索失败时返回的首页/热榜垃圾结果」。
pub fn is_hit_relevant(keyword: &str, book: &Value) -> bool {
    let key = normalize_search_text(keyword);
    if key.is_empty() {
        return true;
    }
    let name = normalize_search_text(str_field(book, "name"));
    let author = normalize_search_text(str_field(book, "author"));
    // 短词（「遮天」「剑来」）必须整体出现在书名或作者里，否则热榜会大量混入
    if key.chars().count() <= 4 {
        return name.contains(&key) || author.contains(&key);
    }
    if !name.is_empty() {
        if name.contains(&key) {
            return true;
        }
        if key.contains(&name) && name.chars().count() >= 4 {
            return true;
        }
        let key_chars: Vec<char> = key.chars().collect();
        let name_chars: Vec<char> = name.chars().collect();
        let need = key_chars.len().max(1) as f64 * 0.6;
        if longest_common_substring(&key_chars, &name_chars)
            >= need.ceil().max(3.0) as usize
        {
            return true;
        }
    }
    let author_len = author.chars().count();
    if author_len >= 2 {
        if author.contains(&key) {
            return true;
        }
        if key.contains(&author) && author_len >= 3 {
            return true;
        }
    }
    false
}

/// 按相关度过滤搜索结果；关键词为空或列表为空时原样返回。
pub fn filter_relevant(keyword: &str, books: &[Value]) -> Vec<Value> {
    if keyword.trim().is_empty() || books.is_empty() {
        return books.to_vec();
    }
    books.iter().filter(|b| is_hit_relevant(keyword, b)).cloned().collect()
}

/// 一次书源搜索的匹配质量分 0～100：空结果 0，全是垃圾结果 8。
pub fn score_match_sample(keyword: &str, raw: &[Value], filtered: &[Value]) -> f64 {
    if raw.is_empty() {
        return 0.0;
    }
    if filtered.is_empty() {
        return 8.0;
    }
    let sims: Vec<f64> = filtered
        .iter()
        .map(|b| title_similarity(keyword, str_field(b, "name")))
        .collect();
    let best = sims.iter().fold(0.0f64, |acc, v| acc.max(*v));
    let avg = sims.iter().sum::<f64>() / sims.len() as f64;
    let precision = filtered.len() as f64 / raw.len() as f64;
    (100.0 * (0.5 * best + 0.3 * avg + 0.2 * precision)).round()
}

/// 实测响应毫秒 → 速度标签；无效返回 None（对应 speedTagFromRespondMs）。
pub(crate) fn speed_tag_from_ms(ms: Option<f64>) -> Option<&'static str> {
    let ms = ms.filter(|v| v.is_finite() && *v >= 0.0)?;
    Some(if ms < 500.0 {
        "turbo"
    } else if ms < 1500.0 {
        "excellent"
    } else if ms < 2000.0 {
        "good"
    } else {
        "slow"
    })
}

fn source_speed_tag(source: &Value) -> Option<&'static str> {
    let ms = source
        .get("flyRespondMs")
        .and_then(Value::as_f64)
        .or_else(|| source.get("respondTime").and_then(Value::as_f64));
    speed_tag_from_ms(ms).or_else(|| {
        source
            .get("flySpeedTag")
            .and_then(Value::as_str)
            .and_then(|t| match t {
                "turbo" => Some("turbo"),
                "excellent" => Some("excellent"),
                "good" => Some("good"),
                "slow" => Some("slow"),
                _ => None,
            })
    })
}

fn source_search_rank(source: &Value) -> i64 {
    match source_speed_tag(source) {
        Some(tag) => match tag {
            "turbo" => 0,
            "excellent" => 1,
            "good" => 2,
            "slow" => 4,
            _ => 3,
        },
        None => 3,
    }
}

/// 有效匹配强度：学习分 + 书架使用加成（1 本 ≈ 18，上限 75）。
fn source_match_strength(source: &Value, shelf_counts: &HashMap<String, usize>) -> f64 {
    let learned = source
        .get("flyMatchScore")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(0.0, 100.0))
        .unwrap_or(0.0);
    let url = source.get("bookSourceUrl").and_then(Value::as_str).unwrap_or("");
    let shelf_boost = (shelf_counts.get(url).copied().unwrap_or(0) as f64 * 18.0).min(75.0);
    (learned + shelf_boost).min(100.0)
}

/// 搜索书源排序：匹配强度 → 速度档 → 实测延迟。
pub fn compare_for_search(a: &Value, b: &Value, shelf_counts: &HashMap<String, usize>) -> std::cmp::Ordering {
    let sa = source_match_strength(a, shelf_counts);
    let sb = source_match_strength(b, shelf_counts);
    if sa != sb {
        return sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal);
    }
    let ra = source_search_rank(a);
    let rb = source_search_rank(b);
    if ra != rb {
        return ra.cmp(&rb);
    }
    let inf = f64::INFINITY;
    let ma = a
        .get("flyRespondMs")
        .and_then(Value::as_f64)
        .or_else(|| a.get("respondTime").and_then(Value::as_f64))
        .unwrap_or(inf);
    let mb = b
        .get("flyRespondMs")
        .and_then(Value::as_f64)
        .or_else(|| b.get("respondTime").and_then(Value::as_f64))
        .unwrap_or(inf);
    ma.partial_cmp(&mb).unwrap_or(std::cmp::Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalization_strips_wrappers_and_version_words() {
        assert_eq!(normalize_search_text("《 遮天 》 精校版"), "遮天");
        // '+' 不在标点白名单里，原版同样保留（此处不追求完美归一，只要求与原版一致）
        assert_eq!(normalize_search_text("剑来(txt+epub)"), "剑来+");
        assert_eq!(normalize_search_text("斗破苍穹——完结版 全文"), "斗破苍穹全文");
    }

    #[test]
    fn short_keyword_requires_full_substring() {
        let good = json!({ "name": "遮天", "author": "辰东" });
        let noise = json!({ "name": "狮舞者", "author": "佚名" });
        assert!(is_hit_relevant("遮天", &good));
        assert!(!is_hit_relevant("遮天", &noise));
        // 作者命中同样算相关（按作者名搜到《遮天》）
        assert!(is_hit_relevant("辰东", &json!({ "name": "遮天", "author": "辰东" })));
    }

    #[test]
    fn long_keyword_allows_strong_overlap() {
        assert!(is_hit_relevant(
            "我有一剑",
            &json!({ "name": "我有一剑开天", "author": "" })
        ));
        assert!(!is_hit_relevant(
            "我有一剑",
            &json!({ "name": "山海异闻录", "author": "路人" })
        ));
    }

    #[test]
    fn similarity_and_sample_score_are_bounded() {
        assert_eq!(title_similarity("遮天", "遮天"), 1.0);
        assert!((title_similarity("遮天", "遮天辰东") - 0.5).abs() < 1e-9);
        assert_eq!(score_match_sample("遮天", &[], &[]), 0.0);
        let raw = vec![json!({ "name": "遮天" }), json!({ "name": "热榜第一" })];
        let hit = vec![json!({ "name": "遮天" })];
        let score = score_match_sample("遮天", &raw, &hit);
        assert!(score > 50.0 && score <= 100.0, "{score}");
    }

    #[test]
    fn sources_sort_by_strength_then_speed() {
        let counts: HashMap<String, usize> = HashMap::new();
        let slow_good = json!({ "bookSourceUrl": "a", "flyMatchScore": 90, "flyRespondMs": 3000 });
        let fast_weak = json!({ "bookSourceUrl": "b", "flyMatchScore": 10, "flyRespondMs": 200 });
        assert_eq!(
            compare_for_search(&slow_good, &fast_weak, &counts),
            std::cmp::Ordering::Less,
            "匹配分高的书源优先，即使它更慢"
        );
        let same_strength = json!({ "bookSourceUrl": "c", "flyMatchScore": 90, "flyRespondMs": 100 });
        assert_eq!(
            compare_for_search(&slow_good, &same_strength, &counts),
            std::cmp::Ordering::Greater,
            "同分时响应快的优先"
        );
    }

    #[test]
    fn shelf_usage_boosts_source_ranking() {
        let mut counts = HashMap::new();
        counts.insert("b".to_string(), 4usize);
        let plain = json!({ "bookSourceUrl": "a", "flyRespondMs": 100 });
        let on_shelf = json!({ "bookSourceUrl": "b", "flyRespondMs": 100 });
        assert_eq!(
            compare_for_search(&plain, &on_shelf, &counts),
            std::cmp::Ordering::Greater,
            "书架在用的书源应排到前面"
        );
    }
}
