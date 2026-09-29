//! 阶段 4 spike：用真实书源跑通 搜索 → 详情 → 目录 → 正文。
//!
//! 需要联网，默认不参与 `cargo test`，手动执行：
//! `cargo test --test spike -- --ignored --nocapture`

use fly_reader_lib::engine::legado::{
    get_book_info, get_chapter_list, get_content, search_books,
};
use serde_json::Value;

fn sources() -> Vec<Value> {
    let raw = include_str!("fixtures/sources.json");
    serde_json::from_str::<Vec<Value>>(raw).expect("fixtures/sources.json 应为书源数组")
}

fn find_source(haystack: &str) -> Value {
    sources()
        .into_iter()
        .find(|s| {
            s.get("bookSourceName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .contains(haystack)
                || s.get("bookSourceUrl")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .contains(haystack)
        })
        .unwrap_or_else(|| panic!("夹具里没有书源：{haystack}"))
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// CSS 书源全链路：搜索 → 详情 → 目录 → 正文。
///
/// `min_chapters` 为 0 表示该书源的目录在静态 HTML 里拿不到，只验证到详情页为止：
/// 露露书屋的章节列表由前端 JS/AJAX 渲染，静态页 `#chapterlist` 只有一条 `<li>`
/// （`curl` 可复核），原 Electron 版同样取不到目录，属于书源侧限制而非引擎缺陷。
#[tokio::test]
#[ignore = "需要联网"]
async fn css_source_end_to_end() {
    struct Case {
        name: &'static str,
        keyword: &'static str,
        min_chapters: usize,
    }
    let cases = [
        Case { name: "小说", keyword: "我的", min_chapters: 3 },
        Case { name: "露露书屋", keyword: "剑来", min_chapters: 0 },
    ];

    for case in cases {
        let (name, keyword, min_chapters) = (case.name, case.keyword, case.min_chapters);
        let source = find_source(name);

        let books = search_books(&source, keyword, 1)
            .await
            .unwrap_or_else(|e| panic!("{name} 搜索失败：{e}"));
        assert!(!books.is_empty(), "{name} 搜索「{keyword}」无结果");
        let first = &books[0];
        println!(
            "[{name}] 搜索 {} 本，首本《{}》作者 {} → {}",
            books.len(),
            field(first, "name"),
            field(first, "author"),
            field(first, "bookUrl")
        );
        assert!(!field(first, "bookUrl").starts_with('/'), "{name} bookUrl 未转绝对地址");

        let info = get_book_info(&source, field(first, "bookUrl"))
            .await
            .unwrap_or_else(|e| panic!("{name} 详情失败：{e}"));
        println!(
            "[{name}] 详情《{}》tocUrl={} 简介={}字",
            field(&info, "name"),
            field(&info, "tocUrl"),
            field(&info, "intro").chars().count()
        );
        assert!(!field(&info, "tocUrl").is_empty());

        let chapters = get_chapter_list(&source, field(&info, "tocUrl"))
            .await
            .unwrap_or_else(|e| panic!("{name} 目录失败：{e}"));
        if min_chapters == 0 {
            println!(
                "[{name}] 目录仅 {} 章（书源侧 JS 渲染，静态 HTML 无完整列表，与原版行为一致）",
                chapters.len()
            );
            continue;
        }
        assert!(
            chapters.len() > min_chapters,
            "{name} 目录只有 {} 条",
            chapters.len()
        );
        let usable: Vec<&Value> = chapters
            .iter()
            .filter(|c| !field(c, "url").is_empty())
            .collect();
        let last = usable.last().expect("至少一章有链接");
        println!(
            "[{name}] 目录 {} 章，首章《{}》末章《{}》",
            chapters.len(),
            field(&chapters[0], "title"),
            field(last, "title")
        );

        let target = usable.get(1).unwrap_or(&usable[0]);
        let content = get_content(&source, field(target, "url"))
            .await
            .unwrap_or_else(|e| panic!("{name} 正文失败：{e}"));
        println!(
            "[{name}] 正文《{}》{} 字，开头：{}",
            field(target, "title"),
            content.chars().count(),
            content.chars().take(40).collect::<String>()
        );
        assert!(content.chars().count() > 200, "{name} 正文过短");
        assert!(!content.contains('<'), "{name} 正文残留 HTML 标签");
    }
}

/// JSON 书源搜索链路（API 型书源）。
#[tokio::test]
#[ignore = "需要联网"]
async fn json_source_search() {
    for name in ["猫眼", "笔趣阁"] {
        let source = find_source(name);
        let books = search_books(&source, "剑来", 1)
            .await
            .unwrap_or_else(|e| panic!("{name} 搜索失败：{e}"));
        assert!(!books.is_empty(), "{name} 搜索无结果");
        let first = &books[0];
        println!(
            "[{name}] 搜索 {} 本，首本《{}》kind={} cover={}",
            books.len(),
            field(first, "name"),
            field(first, "kind"),
            &field(first, "coverUrl")[..field(first, "coverUrl").len().min(48)]
        );
        assert!(!field(first, "name").is_empty());
        assert!(field(first, "bookUrl").starts_with("http"));
    }
}

/// JSON API 书源的详情/目录/正文：依赖 header 容错、ruleBookInfo.init 重设根，
/// 以及 `java.*` AES 桥解密章节地址 —— 这三处都是 FlyReader 相对 Electron 原版的增强。
///
/// 猫眼看书只如实打印结果：它内嵌的 Bearer JWT 已在 2026-04-26 过期（`exp` 1777203652），
/// 详情/目录接口一律回 `code 4005 认证失败`，属书源侧失效，引擎无从修复（搜索接口不鉴权，仍可用）。
#[tokio::test]
#[ignore = "需要联网"]
async fn json_source_toc_and_content() {
    for (name, must_pass) in [("猫眼", false), ("笔趣阁", true)] {
        let source = find_source(name);
        let books = search_books(&source, "剑来", 1)
            .await
            .unwrap_or_else(|e| panic!("{name} 搜索失败：{e}"));
        let info = get_book_info(&source, field(&books[0], "bookUrl"))
            .await
            .unwrap_or_else(|e| panic!("{name} 详情失败：{e}"));
        let title = field(&info, "name").to_string();
        if !must_pass {
            println!("[{name}] 详情《{title}》tocUrl={}（书源 token 过期，仅作记录）", field(&info, "tocUrl"));
            continue;
        }
        assert_ne!(title, "未知书名", "{name} 详情仍取不到书名（header / init 未生效）");
        println!("[{name}] 详情《{title}》tocUrl={}", field(&info, "tocUrl"));

        let chapters = get_chapter_list(&source, field(&info, "tocUrl"))
            .await
            .unwrap_or_else(|e| panic!("{name} 目录失败：{e}"));
        assert!(chapters.len() > 3, "{name} 目录只有 {} 条", chapters.len());
        println!(
            "[{name}] 目录 {} 章，首章《{}》url={}",
            chapters.len(),
            field(&chapters[0], "title"),
            field(&chapters[0], "url")
        );
        assert!(
            chapters.iter().take(5).any(|c| field(c, "url").starts_with("http")),
            "{name} 章节地址没解出绝对 URL（AES 桥未生效）"
        );

        let mut text = String::new();
        let mut last_err = String::new();
        for chapter in chapters.iter().take(10) {
            let url = field(chapter, "url");
            if url.is_empty() {
                continue;
            }
            match get_content(&source, url).await {
                Ok(body) if body.chars().count() > 100 => {
                    text = body;
                    break;
                }
                Ok(_) => last_err = "正文过短".to_string(),
                Err(e) => last_err = e,
            }
        }
        assert!(
            !text.is_empty(),
            "{name} 前 10 章正文都取不到（最后一步：{last_err}）"
        );
        println!(
            "[{name}] 正文 {} 字，开头：{}",
            text.chars().count(),
            text.chars().take(40).collect::<String>()
        );
    }
}
