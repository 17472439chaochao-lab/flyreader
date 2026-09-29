//! 在线书源仓库获取的真实联网验证（对应 sourceRepo.ts 的移植）。
//!
//! 需要联网，默认不参与 `cargo test`，手动执行：
//! `cargo test --test online_repo -- --ignored --nocapture`

use fly_reader_lib::engine::source_repo::{fetch_online_sources, OnlineRepoRequest, BUILTIN_REPOS};

fn request(id: &str) -> OnlineRepoRequest {
    let meta = BUILTIN_REPOS.iter().find(|b| b.id == id).expect("内置仓库 id");
    OnlineRepoRequest {
        id: meta.id.to_string(),
        name: None,
        kind: None,
        url: None,
        alt: None,
    }
}

/// 走备用 CDN（国内可达）拉取 XIU2 书源包，验证解析与去重链路。
#[tokio::test]
#[ignore]
async fn xiu2_repo_yields_normalized_sources() {
    let mut seen: Vec<String> = Vec::new();
    let result = fetch_online_sources(vec![request("xiu2")], &mut |p| {
        seen.push(format!("{} {} {} {}", p.repo_name, p.done, p.total, p.found));
    })
    .await;
    for line in &seen {
        println!("{line}");
    }
    for err in &result.errors {
        println!("错误：{err}");
    }
    assert!(
        !result.sources.is_empty(),
        "XIU2 书源包应能解析出至少一个书源"
    );
    let named = result
        .sources
        .iter()
        .filter(|s| s.get("bookSourceName").and_then(serde_json::Value::as_str).is_some())
        .count();
    assert_eq!(named, result.sources.len(), "normalize_sources 要保证有名字");
    println!(
        "共 {} 个书源，仓库成功 {} / 失败 {}",
        result.sources.len(),
        result.repos_ok,
        result.repos_fail
    );
}

/// 三个内置仓库全拉：验证跨仓库按 bookSourceUrl 去重与串行进度推送。
#[tokio::test]
#[ignore]
async fn all_builtin_repos_dedupe_by_url() {
    let mut phases: Vec<(String, String)> = Vec::new();
    let result = fetch_online_sources(
        BUILTIN_REPOS.iter().map(|b| request(b.id)).collect(),
        &mut |p| phases.push((p.phase.to_string(), p.repo_name)),
    )
    .await;
    assert_eq!(
        phases.len(),
        BUILTIN_REPOS.len() * 2,
        "每个仓库都要推 start + done"
    );
    let mut urls: Vec<&str> = result
        .sources
        .iter()
        .filter_map(|s| s.get("bookSourceUrl").and_then(serde_json::Value::as_str))
        .collect();
    let before = urls.len();
    urls.sort_unstable();
    urls.dedup();
    assert_eq!(before, urls.len(), "跨仓库结果必须已按 URL 去重");
    println!(
        "共 {} 个去重书源，仓库成功 {} / 失败 {}，错误 {} 条",
        before,
        result.repos_ok,
        result.repos_fail,
        result.errors.len()
    );
    for err in result.errors.iter().take(5) {
        println!("错误：{err}");
    }
}
