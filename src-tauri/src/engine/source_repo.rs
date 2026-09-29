//! 在线书源仓库获取，对应原 Electron 版 src/main/legado/sourceRepo.ts。
//!
//! 支持三类仓库格式：
//! - json：响应直接是书源 JSON（数组或单个对象，兼容 ```json 代码块包裹）；
//! - sub：订阅列表（每行一个书源 JSON 文件的 URL，递归展开拉取）；
//! - auto：先按 JSON 解析，失败再按订阅列表展开（自定义 URL 用）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use super::http::{fetch_text, FetchOptions};
use super::legado::normalize_sources;

/// 内置书源仓库（社区公开维护，按需增删；备用地址优先，备用失败自动回退主地址）
pub const BUILTIN_REPOS: &[SourceRepoMeta] = &[
    SourceRepoMeta {
        id: "tickmao",
        name: "Novel 书源合集",
        desc: "tickmao/Novel 持续维护的全量 Legado 书源，GitHub 托管",
        kind: "json",
        url: "https://raw.githubusercontent.com/tickmao/Novel/master/sources/legado/full.json",
        alt: Some("https://cdn.jsdelivr.net/gh/tickmao/Novel@master/sources/legado/full.json"),
    },
    SourceRepoMeta {
        id: "xiu2",
        name: "XIU2 书源",
        desc: "XIU2/Yuedu 维护的 Legado 书源包（JSON 数组），GitHub 托管",
        kind: "json",
        url: "https://raw.githubusercontent.com/XIU2/Yuedu/master/shuyuan",
        alt: Some("https://yuedu.xiu2.xyz/shuyuan"),
    },
    SourceRepoMeta {
        id: "yckceo",
        name: "书源导航（yckceo）",
        desc: "书源站点 yckceo.com 提供的精选书源包",
        kind: "json",
        url: "https://www.yckceo.com/yuedu/shuyuan/json/id/1234.json",
        alt: None,
    },
];

/// 单个响应体大小上限（12MB，防超大书源包与异常响应）
const MAX_PAYLOAD_BYTES: usize = 12 * 1024 * 1024;
/// 订阅列表最多展开的子 URL 数
const MAX_SUB_URLS: usize = 120;
/// 订阅展开累计字节预算（超限后剩余条目跳过，防订阅列表炸弹）
const MAX_SUB_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// 订阅展开的并发数（按需适度，避免对书源托管站集中请求）
const SUB_CONCURRENCY: usize = 6;
/// 仓库主体请求超时
const REPO_TIMEOUT_MS: u64 = 20_000;
/// 订阅列表内单个书源文件请求超时
const SUB_TIMEOUT_MS: u64 = 12_000;
/// 单次在线获取最多尝试的仓库数
const MAX_REPOS: usize = 20;

/// 仓库元数据（面板展示用，字段与原 TS 定义一致）
#[derive(Clone, Serialize)]
pub struct SourceRepoMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub kind: &'static str,
    pub url: &'static str,
    pub alt: Option<&'static str>,
}

/// 前端下发的获取请求（内置 id 或自定义 URL）
#[derive(Clone, Deserialize)]
pub struct OnlineRepoRequest {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub alt: Option<String>,
}

/// 在线获取进度事件载荷
#[derive(Clone, Serialize)]
pub struct OnlineFetchProgress {
    #[serde(rename = "repoName")]
    pub repo_name: String,
    pub done: usize,
    pub total: usize,
    pub found: usize,
    pub phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

/// 在线获取汇总结果
pub struct OnlineFetchResult {
    pub sources: Vec<Value>,
    pub errors: Vec<String>,
    pub repos_ok: usize,
    pub repos_fail: usize,
}

/// 合并请求与内置元数据后的实际抓取目标
struct Repo {
    name: String,
    kind: String,
    url: String,
    alt: Option<String>,
}

/// 单个订阅子文件的抓取结果（error 为空表示成功）
struct SubRow {
    sources: Vec<Value>,
    error: String,
}

impl Default for SubRow {
    fn default() -> Self {
        SubRow { sources: Vec::new(), error: String::new() }
    }
}

/// 拉取仓库地址文本，带单响应大小上限；超限返回 None。
async fn fetch_limited_text(url: &str, timeout_ms: u64) -> Option<String> {
    let page = fetch_text(url, &FetchOptions { timeout_ms: Some(timeout_ms), ..Default::default() })
        .await
        .ok()?;
    if page.text.len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    Some(page.text)
}

/// 依次尝试备用/主地址，返回首个成功的响应文本。
/// 备用地址（alt）多为国内 CDN（jsdelivr / 作者自托管），国内网络环境下优先尝试，
/// 主地址（GitHub raw）作为海外访问的兜底回退。
async fn fetch_repo_text(repo: &Repo) -> Option<String> {
    let mut urls = vec![repo.url.as_str()];
    if let Some(alt) = &repo.alt {
        urls.insert(0, alt.as_str());
    }
    for u in urls {
        if let Some(text) = fetch_limited_text(u, REPO_TIMEOUT_MS).await {
            return Some(text);
        }
    }
    None
}

/// 去掉 ```json 代码块包裹（与原 TS 的两次 replace 等价）。
pub(crate) fn strip_code_fence(raw: &str) -> String {
    static FENCE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = FENCE.get_or_init(|| {
        regex::Regex::new(r"(?i)^```(?:json)?\s*|\s*```$").expect("fence regex")
    });
    re.replace_all(raw.trim(), "").trim().to_string()
}

/// 从响应文本解析书源列表（兼容 JSON 数组 / 单个对象 / ```json 代码块包裹）。
fn parse_payload(text: &str) -> Option<Vec<Value>> {
    let body = strip_code_fence(text);
    if body.is_empty() {
        return None;
    }
    let parsed: Value = serde_json::from_str(&body).ok()?;
    let sources = normalize_sources(parsed);
    (!sources.is_empty()).then_some(sources)
}

/// 从订阅列表文本提取子书源 URL（跳过空行与 # 注释，去重后最多 MAX_SUB_URLS 个）。
fn extract_sub_urls(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let lower = l.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            continue;
        }
        if seen.insert(l.to_string()) {
            out.push(l.to_string());
            if out.len() >= MAX_SUB_URLS {
                break;
            }
        }
    }
    out
}

/// 获取单个仓库的全部书源（未跨仓库去重）与错误信息。
async fn fetch_repo(repo: &Repo) -> (Vec<Value>, Vec<String>) {
    let Some(text) = fetch_repo_text(repo).await else {
        return (Vec::new(), vec![format!("「{}」所有地址均不可用或超限", repo.name)]);
    };

    // json / auto：先尝试整体 JSON 解析
    if repo.kind == "json" || repo.kind == "auto" {
        if let Some(sources) = parse_payload(&text) {
            return (sources, Vec::new());
        }
        if repo.kind == "json" {
            return (
                Vec::new(),
                vec![format!("「{}」返回内容不是有效书源 JSON", repo.name)],
            );
        }
    }

    // sub / auto（JSON 失败后）：按订阅列表展开
    let sub_urls = extract_sub_urls(&text);
    if sub_urls.is_empty() {
        let hint = if repo.kind == "auto" {
            "既不是 JSON 也未找到订阅 URL"
        } else {
            "订阅列表未找到有效 URL"
        };
        return (Vec::new(), vec![format!("「{}」{}", repo.name, hint)]);
    }

    let budget = Arc::new(AtomicUsize::new(0));
    let permits = Arc::new(Semaphore::new(SUB_CONCURRENCY));
    let mut handles = Vec::with_capacity(sub_urls.len());
    for u in sub_urls.clone() {
        let budget = Arc::clone(&budget);
        let permits = Arc::clone(&permits);
        handles.push(tauri::async_runtime::spawn(async move {
            let Ok(_permit) = permits.acquire().await else {
                return SubRow::default();
            };
            if budget.load(Ordering::SeqCst) > MAX_SUB_TOTAL_BYTES {
                return SubRow::default();
            }
            let page = fetch_text(&u, &FetchOptions { timeout_ms: Some(SUB_TIMEOUT_MS), ..Default::default() })
                .await;
            match page {
                Err(e) => SubRow { sources: Vec::new(), error: e },
                Ok(page) if page.text.len() > MAX_PAYLOAD_BYTES => {
                    SubRow { sources: Vec::new(), error: format!("响应超限：{u}") }
                }
                Ok(page) => {
                    budget.fetch_add(page.text.len(), Ordering::SeqCst);
                    match parse_payload(&page.text) {
                        Some(sources) => SubRow { sources, error: String::new() },
                        None => SubRow {
                            sources: Vec::new(),
                            error: format!("不是有效书源 JSON：{u}"),
                        },
                    }
                }
            }
        }));
    }

    let mut rows = Vec::with_capacity(handles.len());
    for h in handles {
        rows.push(h.await.unwrap_or_default());
    }

    let mut errors: Vec<String> = Vec::new();
    let mut sources: Vec<Value> = Vec::new();
    let mut sub_ok = 0usize;
    for row in rows {
        if !row.error.is_empty() {
            errors.push(row.error);
            continue;
        }
        sub_ok += 1;
        sources.extend(row.sources);
    }
    if sub_ok > 0 {
        errors.push(format!(
            "「{}」展开 {} 个子源，成功 {}，失败 {}",
            repo.name,
            sub_urls.len(),
            sub_ok,
            sub_urls.len() - sub_ok
        ));
    }
    (sources, errors)
}

/// 从多个仓库在线获取书源，跨仓库按 bookSourceUrl 去重（保留先出现的）。
/// 仓库间串行执行以便推送进度并避免瞬时请求风暴；仓库内订阅展开受并发池约束。
pub async fn fetch_online_sources<F>(
    repos: Vec<OnlineRepoRequest>,
    on_progress: &mut F,
) -> OnlineFetchResult
where
    F: FnMut(OnlineFetchProgress),
{
    let metas: Vec<Repo> = repos
        .iter()
        .map(|r| {
            let builtin = BUILTIN_REPOS.iter().find(|b| b.id == r.id);
            Repo {
                name: r
                    .name
                    .clone()
                    .filter(|n| !n.is_empty())
                    .or_else(|| builtin.map(|b| b.name.to_string()))
                    .unwrap_or_else(|| r.id.clone()),
                kind: r
                    .kind
                    .clone()
                    .or_else(|| builtin.map(|b| b.kind.to_string()))
                    .unwrap_or_else(|| "auto".to_string()),
                url: r
                    .url
                    .clone()
                    .filter(|u| !u.is_empty())
                    .or_else(|| builtin.map(|b| b.url.to_string()))
                    .unwrap_or_default(),
                alt: r
                    .alt
                    .clone()
                    .filter(|u| !u.is_empty())
                    .or_else(|| builtin.and_then(|b| b.alt.map(str::to_string))),
            }
        })
        .filter(|m| !m.url.is_empty())
        .take(MAX_REPOS)
        .collect();

    let mut by_url: Vec<(String, Value)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut errors: Vec<String> = Vec::new();
    let mut repos_ok = 0usize;
    let mut repos_fail = 0usize;
    let total = metas.len();

    for (i, repo) in metas.iter().enumerate() {
        on_progress(OnlineFetchProgress {
            repo_name: repo.name.clone(),
            done: i,
            total,
            found: by_url.len(),
            phase: "start",
            ok: None,
        });
        let (sources, repo_errors) = fetch_repo(repo).await;
        let ok = !sources.is_empty();
        for s in sources {
            let url = s
                .get("bookSourceUrl")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !url.is_empty() && seen.insert(url.clone()) {
                by_url.push((url, s));
            }
        }
        errors.extend(repo_errors);
        if ok {
            repos_ok += 1;
        } else {
            repos_fail += 1;
        }
        on_progress(OnlineFetchProgress {
            repo_name: repo.name.clone(),
            done: i + 1,
            total,
            found: by_url.len(),
            phase: "done",
            ok: Some(ok),
        });
    }

    OnlineFetchResult {
        sources: by_url.into_iter().map(|(_, s)| s).collect(),
        errors,
        repos_ok,
        repos_fail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn payload_accepts_array_object_and_fence() {
        let arr = json!([{ "bookSourceName": "a", "bookSourceUrl": "https://a.com" }]);
        assert_eq!(parse_payload(&arr.to_string()).unwrap().len(), 1);
        let one = json!({ "bookSourceName": "b", "bookSourceUrl": "https://b.com" });
        assert_eq!(parse_payload(&one.to_string()).unwrap().len(), 1);
        let fenced = format!("```json\n{arr}\n```");
        assert_eq!(parse_payload(&fenced).unwrap().len(), 1);
        assert!(parse_payload("not json").is_none());
        assert!(parse_payload("[]").is_none());
    }

    #[test]
    fn sub_urls_skip_comments_and_dedupe() {
        let text = "# list\nhttps://a.com/1.json\n\nhttps://a.com/1.json\nhttp://b.com/2.txt\nftp://c\n";
        assert_eq!(
            extract_sub_urls(text),
            vec!["https://a.com/1.json".to_string(), "http://b.com/2.txt".to_string()]
        );
    }
}
