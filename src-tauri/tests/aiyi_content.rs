use fly_reader_lib::engine::legado::get_content;
use serde_json::Value;

fn decode_tags(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            let o = c as u32;
            if (0xE0020..=0xE007E).contains(&o) {
                Some(char::from_u32(o - 0xE0000).unwrap())
            } else {
                None
            }
        })
        .collect()
}

#[tokio::test]
async fn aiyi_chapter_pages() {
    let source: Value = serde_json::from_str(include_str!("/tmp/aiyi_source.json")).unwrap();
    let url = "https://www.aiyiweb.net/item/zTxxxT/sG0HsbHX.htm";
    let content = get_content(&source, url).await.expect("get_content");

    let mut urls = Vec::new();
    let mut cur = String::new();
    for ch in content.chars() {
        let o = ch as u32;
        if (0xE0020..=0xE007E).contains(&o) {
            cur.push(char::from_u32(o - 0xE0000).unwrap());
        } else if !cur.is_empty() {
            if cur.contains("http") {
                urls.push(cur.clone());
            }
            cur.clear();
        }
    }
    if cur.contains("http") {
        urls.push(cur);
    }
    urls.sort();
    urls.dedup();
    println!("unique watermark urls ({}):", urls.len());
    for u in &urls {
        println!("  {u}");
    }

    let cleaned: String = content
        .chars()
        .filter(|c| {
            let o = *c as u32;
            !(0xE0000..=0xE007F).contains(&o) && *c != '\u{7f}'
        })
        .collect();
    println!("raw={} cleaned={}", content.len(), cleaned.chars().count());
    println!("END: {:?}", cleaned.chars().rev().take(80).collect::<String>().chars().rev().collect::<String>());
}
