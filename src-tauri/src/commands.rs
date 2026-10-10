//! Tauri 命令层。通道与原 preload 对齐，命名改为 snake_case。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use crate::engine::http::{fetch_text, FetchOptions};
use crate::engine::legado::{
    get_book_info, get_chapter_list_with_budget, get_content, is_source_structural_ok, normalize_sources,
    search_books, test_source,
};
use crate::engine::qidian_rank::{
    fetch_rank_list, list_result_to_value, RankBoard, RankChannel, RankGender,
};
use crate::engine::source_repo::{OnlineRepoRequest, SourceRepoMeta, BUILTIN_REPOS};
use crate::engine::text::ensure_novel_paragraphs;
use crate::store::{blend_match_score, is_local_book, now_ms, ChapterRow, Store};

pub struct StoreState(pub Mutex<Store>);

fn store<'a>(state: &'a State<'_, StoreState>) -> std::sync::MutexGuard<'a, Store> {
    state.0.lock().unwrap()
}

#[tauri::command]
fn app_get_state(state: State<StoreState>, jobs: State<JobsState>) -> Value {
    let mut s = store(&state);
    let mut v = s.app_state();
    let shelf = s.get_shelf();
    v["shelf"] = json!(s.enrich_shelf(&shelf, &jobs.running_ids(JobKind::Cache)));
    v
}

#[tauri::command]
fn app_get_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 轻量检查更新：对比 Gitee Releases 最新版，仅返回结果供关于页提示。
#[tauri::command]
async fn app_check_update(app: AppHandle) -> Result<Value, String> {
    Ok(crate::update_check::check_gitee_update(&app_get_version(app)).await)
}

#[tauri::command]
async fn app_open_external(url: String) -> Result<Value, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Ok(json!({ "ok": false, "message": "仅允许打开 http(s) 链接" }));
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).status();
    #[cfg(windows)]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", &url]).status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).status();
    match r {
        Ok(st) if st.success() => Ok(json!({ "ok": true })),
        Ok(_) => Ok(json!({ "ok": false, "message": "系统未能打开链接" })),
        Err(e) => Ok(json!({ "ok": false, "message": e.to_string() })),
    }
}

#[tauri::command]
fn settings_get(state: State<StoreState>) -> Value {
    store(&state).get_settings()
}

#[tauri::command]
fn settings_save(app: AppHandle, state: State<StoreState>, settings: Value) -> Result<Value, String> {
    let saved = store(&state).save_settings(&settings)?;
    crate::theme::apply_app_theme(&app, &saved);
    Ok(saved)
}

#[tauri::command]
fn settings_add_purify(state: State<StoreState>, rule: String) -> Result<Value, String> {
    store(&state).add_purify_rule(&rule)
}

#[tauri::command]
fn settings_remove_purify(state: State<StoreState>, rule: String) -> Result<Value, String> {
    store(&state).remove_purify_rule(&rule)
}

#[tauri::command]
fn prefs_get(state: State<StoreState>) -> Value {
    store(&state).get_prefs()
}

/// 保存偏好并按需重注册老板键。
/// 必须是异步命令：macOS 的 global-shortcut 注册会派发到主线程并阻塞等待结果，
/// 同步命令跑在主线程上会自锁。
#[tauri::command]
async fn prefs_save(
    app: AppHandle,
    state: State<'_, StoreState>,
    patch: Value,
) -> Result<Value, String> {
    let prefs = store(&state).save_prefs(&patch)?;
    let boss = crate::boss_key::register_from_prefs(&app, &prefs);
    Ok(json!({ "prefs": prefs, "bossKey": boss }))
}

#[tauri::command]
fn fonts_list() -> Vec<String> {
    use font_kit::source::SystemSource;
    let source = SystemSource::new();
    let Ok(families) = source.all_families() else { return vec![] };
    let mut out: Vec<String> = families
        .into_iter()
        .filter(|f| !f.starts_with('.'))
        .collect();
    out.sort();
    out
}

#[tauri::command]
fn search_history(state: State<StoreState>) -> Vec<String> {
    store(&state).get_search_history()
}

#[tauri::command]
fn search_clear_history(state: State<StoreState>) -> Vec<String> {
    store(&state).clear_search_history()
}

#[tauri::command]
fn search_remove_history(state: State<StoreState>, keyword: String) -> Vec<String> {
    store(&state).remove_search_history(&keyword)
}

#[tauri::command]
fn sources_list(state: State<StoreState>) -> Vec<Value> {
    store(&state).list_sources()
}

#[tauri::command]
fn sources_toggle(state: State<StoreState>, url: String, enabled: bool) -> Result<Vec<Value>, String> {
    let mut s = store(&state);
    s.patch_source_field(&url, &json!({ "enabled": enabled }))?;
    Ok(s.get_sources())
}

#[tauri::command]
fn sources_remove(state: State<StoreState>, url: String) -> Result<Vec<Value>, String> {
    let mut s = store(&state);
    s.remove_sources(&[url])
}

#[tauri::command]
fn sources_remove_many(
    state: State<StoreState>,
    urls: Vec<String>,
) -> Result<Value, String> {
    let mut s = store(&state);
    let before = s.get_sources().len();
    let sources = s.remove_sources(&urls)?;
    Ok(json!({ "sources": sources, "removed": before - sources.len() }))
}

// ---------- 书源管理（导入 / 导出 / 在线仓库 / 测试 / 清理） ----------

/// 规范化并导入解析后的书源 JSON（对应 importFromParsed + importResultPayload）。
fn import_from_parsed(s: &mut Store, parsed: Value) -> Result<Value, String> {
    let sources = normalize_sources(parsed);
    if sources.is_empty() {
        return Ok(json!({ "ok": false, "message": "未识别到有效书源" }));
    }
    let (added, skipped) = s.import_sources(&sources)?;
    if added == 0 && skipped > 0 {
        return Ok(json!({
            "ok": true,
            "message": format!("全部 {skipped} 个书源已存在，未导入"),
            "sources": s.get_sources()
        }));
    }
    let message = if skipped > 0 {
        format!("导入完成：新增 {added}，已存在跳过 {skipped}")
    } else {
        format!("导入完成：新增 {added}")
    };
    Ok(json!({ "ok": true, "message": message, "sources": s.get_sources() }))
}

/// 是否 http(s) 地址（对应 /^https?:\/\//i）。
fn is_http_url(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

#[tauri::command]
async fn sources_import_file(app: AppHandle, state: State<'_, StoreState>) -> Result<Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = {
        let dlg = app.dialog().file();
        dlg.set_title("导入 Legado 书源")
            .add_filter("JSON", &["json", "txt"])
            .blocking_pick_file()
    };
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(json!({ "ok": false, "message": "已取消" }));
    };
    let imported = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|raw| serde_json::from_str::<Value>(&raw).map_err(|e| e.to_string()))
        .and_then(|parsed| import_from_parsed(&mut store(&state), parsed));
    Ok(match imported {
        Ok(v) => v,
        Err(msg) => json!({ "ok": false, "message": format!("导入失败：{msg}") }),
    })
}

#[tauri::command]
async fn sources_import_url(
    state: State<'_, StoreState>,
    url: String,
) -> Result<Value, String> {
    let target = url.trim().to_string();
    if !is_http_url(&target) {
        return Ok(json!({ "ok": false, "message": "请输入以 http:// 或 https:// 开头的书源地址" }));
    }
    let fetched = fetch_text(&target, &FetchOptions::default())
        .await
        .map_err(|e| e.to_string())
        .and_then(|page| {
            let body = crate::engine::source_repo::strip_code_fence(&page.text);
            serde_json::from_str::<Value>(&body).map_err(|e| e.to_string())
        })
        .and_then(|parsed| import_from_parsed(&mut store(&state), parsed));
    match fetched {
        Ok(v) => Ok(v),
        Err(msg) => Ok(json!({ "ok": false, "message": format!("URL 导入失败：{msg}") })),
    }
}

#[tauri::command]
fn sources_repo_list() -> Vec<SourceRepoMeta> {
    BUILTIN_REPOS.to_vec()
}

/// 导出时剥离应用本地元数据字段，保持 Legado 兼容（对应 toExportableSource）。
const FLY_SOURCE_META_KEYS: [&str; 7] = [
    "flyTestStatus",
    "flyTestMessage",
    "flyTestAt",
    "flyRespondMs",
    "flySpeedTag",
    "flyMatchScore",
    "flyMatchSamples",
];

fn to_exportable_source(source: &Value) -> Value {
    let mut out = source.clone();
    if let Some(map) = out.as_object_mut() {
        for key in FLY_SOURCE_META_KEYS {
            map.remove(key);
        }
    }
    out
}

#[tauri::command]
async fn sources_export_file(
    app: AppHandle,
    state: State<'_, StoreState>,
    urls: Option<Vec<String>>,
) -> Result<Value, String> {
    let filter: Option<HashSet<String>> = urls
        .filter(|v| !v.is_empty())
        .map(|v| v.into_iter().collect());
    let list: Vec<Value> = {
        let all = store(&state).get_sources();
        match &filter {
            Some(set) => all
                .into_iter()
                .filter(|s| {
                    set.contains(s.get("bookSourceUrl").and_then(Value::as_str).unwrap_or(""))
                })
                .collect(),
            None => all,
        }
    };
    if list.is_empty() {
        return Ok(json!({ "ok": false, "message": "没有可导出的书源" }));
    }
    use tauri_plugin_dialog::DialogExt;
    let stamp = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let picked = {
        let dlg = app.dialog().file();
        dlg.set_title("导出书源")
            .set_file_name(format!("fly-reader-sources-{stamp}.json"))
            .add_filter("JSON", &["json"])
            .blocking_save_file()
    };
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(json!({ "ok": false, "message": "已取消" }));
    };
    let written = (|| -> Result<usize, String> {
        let payload: Vec<Value> = list.iter().map(to_exportable_source).collect();
        let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        std::fs::write(&path, format!("{text}\n")).map_err(|e| e.to_string())?;
        Ok(payload.len())
    })();
    Ok(match written {
        Ok(count) => json!({
            "ok": true,
            "message": format!("已导出 {count} 个书源"),
            "count": count,
            "path": path.to_string_lossy()
        }),
        Err(msg) => json!({ "ok": false, "message": format!("导出失败：{msg}") }),
    })
}

#[tauri::command]
async fn sources_fetch_online(
    app: AppHandle,
    state: State<'_, StoreState>,
    repos: Option<Vec<OnlineRepoRequest>>,
) -> Result<Value, String> {
    let list = repos
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| {
            BUILTIN_REPOS
                .iter()
                .map(|b| OnlineRepoRequest {
                    id: b.id.to_string(),
                    name: None,
                    kind: None,
                    url: None,
                    alt: None,
                })
                .collect()
        });
    let sender = app.clone();
    let result = crate::engine::source_repo::fetch_online_sources(list, &mut |p| {
        let _ = sender.emit("sources_fetch_online_progress", &p);
    })
    .await;

    if result.sources.is_empty() {
        let joined = result.errors.iter().take(3).cloned().collect::<Vec<_>>().join("；");
        let detail: String = if joined.is_empty() {
            "未获取到有效书源".to_string()
        } else {
            joined.chars().take(200).collect()
        };
        return Ok(json!({
            "ok": false,
            "message": format!("在线获取失败：{detail}"),
            "sources": store(&state).get_sources()
        }));
    }
    let imported = {
        let mut s = store(&state);
        s.import_sources(&result.sources)
            .map(|(added, skipped)| (added, skipped, s.get_sources()))
    };
    Ok(match imported {
        Ok((added, skipped, sources)) => {
            let mut parts = vec![format!("新增 {added}")];
            if skipped > 0 {
                parts.push(format!("已存在跳过 {skipped}"));
            }
            if result.repos_fail > 0 {
                parts.push(format!("{} 个仓库获取失败", result.repos_fail));
            }
            json!({
                "ok": true,
                "message": format!("在线获取完成：{}", parts.join("，")),
                "sources": sources,
                "added": added,
                "skipped": skipped
            })
        }
        Err(msg) => json!({
            "ok": false,
            "message": format!("在线获取失败：{msg}"),
            "sources": store(&state).get_sources()
        }),
    })
}

#[tauri::command]
fn sources_update(
    state: State<StoreState>,
    old_url: String,
    source: Value,
) -> Result<Vec<Value>, String> {
    let normalized = normalize_sources(json!([source]));
    if normalized.is_empty() {
        return Err("书源格式无效".to_string());
    }
    store(&state).replace_source(&old_url, &normalized[0])
}

/// 把测试结果写回书源字段（对应原版写回的 flyTest* 六项）。
fn test_patch(result: &Value, at: i64) -> Value {
    let respond_ms = result.get("respondMs").and_then(Value::as_f64);
    json!({
        "flyTestStatus": if result.get("ok").and_then(Value::as_bool).unwrap_or(false) { "ok" } else { "fail" },
        "flyTestMessage": result.get("message").cloned().unwrap_or(Value::Null),
        "flyTestAt": at,
        "flyRespondMs": respond_ms,
        "respondTime": respond_ms,
        "flySpeedTag": crate::relevance::speed_tag_from_ms(respond_ms),
    })
}

#[tauri::command]
async fn sources_test(
    state: State<'_, StoreState>,
    url: String,
    keyword: Option<String>,
) -> Result<Value, String> {
    let source = source_of(&state, &url).ok_or_else(|| "书源不存在".to_string())?;
    let history = state
        .0
        .lock()
        .map(|s| s.get_search_history())
        .unwrap_or_default();
    let result = test_source(&source, keyword.as_deref(), &history).await;

    let mut named = match result {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    named.insert("url".into(), json!(url));
    named.insert(
        "name".into(),
        source.get("bookSourceName").cloned().unwrap_or(Value::String(String::new())),
    );
    let result = Value::Object(named);

    let sources = {
        let mut s = store(&state);
        s.patch_source_field(&url, &test_patch(&result, now_ms()))?;
        s.get_sources()
    };
    Ok(json!({ "result": result, "sources": sources }))
}

/// 单个书源的批量测试结局（超时或异常时按原版合成失败项）。
async fn test_one_source(
    source: Value,
    keyword: Option<String>,
    history: Vec<String>,
) -> (String, Value) {
    let name = source
        .get("bookSourceName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let url = source
        .get("bookSourceUrl")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let outcome = tokio::time::timeout(
        Duration::from_millis(SOURCE_TEST_TIMEOUT_MS),
        test_source(&source, keyword.as_deref(), &history),
    )
    .await;
    let result = match outcome {
        Ok(v) => v,
        Err(_) => json!({
            "ok": false,
            "message": format!("书源「{name}」测试超时"),
            "found": 0,
            "respondMs": SOURCE_TEST_TIMEOUT_MS,
            "structuralOk": is_source_structural_ok(&source)
        }),
    };
    (url, result)
}

const SOURCE_TEST_TIMEOUT_MS: u64 = 15_000;

#[tauri::command]
async fn sources_test_all(
    app: AppHandle,
    state: State<'_, StoreState>,
    keyword: Option<String>,
    urls: Option<Vec<String>>,
) -> Result<Value, String> {
    let (list, history) = {
        let mut s = store(&state);
        let all = s.get_sources();
        let filter: Option<HashSet<String>> = urls
            .filter(|v| !v.is_empty())
            .map(|v| v.into_iter().collect());
        let list = match &filter {
            Some(set) => all
                .into_iter()
                .filter(|s| set.contains(s.get("bookSourceUrl").and_then(Value::as_str).unwrap_or("")))
                .collect(),
            None => all,
        };
        (list, s.get_search_history())
    };
    let total = list.len();
    let progress = Arc::new(ProgressEmitter::new(&app, "sources_test_progress", total));
    progress.start(Value::Null);
    if total == 0 {
        progress.finish(Value::Null);
        return Ok(Value::Array(store(&state).get_sources()));
    }

    let concurrency = request_concurrency(&state).clamp(1, total);
    let list = Arc::new(list);
    let cursor = Arc::new(AtomicUsize::new(0));
    let outcomes = Arc::new(Mutex::new(HashMap::<String, Value>::new()));
    let mut handles = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let progress = Arc::clone(&progress);
        let list = Arc::clone(&list);
        let cursor = Arc::clone(&cursor);
        let outcomes = Arc::clone(&outcomes);
        let history = history.clone();
        let keyword = keyword.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            loop {
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                if index >= list.len() {
                    break;
                }
                let source = list[index].clone();
                let name = source
                    .get("bookSourceName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let (url, result) = test_one_source(source, keyword.clone(), history.clone()).await;
                let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
                if let Ok(mut map) = outcomes.lock() {
                    map.insert(url, result);
                }
                progress.step(&name, json!({ "ok": ok }));
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }
    progress.finish(Value::Null);

    let map = outcomes.lock().map(|m| m.clone()).unwrap_or_default();
    let now = now_ms();
    let next: Vec<Value> = store(&state)
        .get_sources()
        .into_iter()
        .map(|s| {
            let url = s.get("bookSourceUrl").and_then(Value::as_str).unwrap_or("");
            match map.get(url) {
                Some(result) => {
                    let mut merged = s;
                    if let (Some(dst), Some(patch)) = (
                        merged.as_object_mut(),
                        test_patch(result, now).as_object()
                    ) {
                        for (k, v) in patch {
                            if v.is_null() {
                                dst.remove(k);
                            } else {
                                dst.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    merged
                }
                None => s
            }
        })
        .collect();
    store(&state).save_sources(&next)?;
    Ok(Value::Array(next))
}

#[tauri::command]
fn sources_remove_invalid(state: State<StoreState>) -> Result<Value, String> {
    let mut s = store(&state);
    let current = s.get_sources();
    let keep: Vec<Value> = current
        .iter()
        .filter(|src| {
            is_source_structural_ok(src)
                && src.get("flyTestStatus").and_then(Value::as_str) != Some("fail")
        })
        .cloned()
        .collect();
    let removed = current.len() - keep.len();
    s.save_sources(&keep)?;
    Ok(json!({ "sources": keep, "removed": removed }))
}

/// 书架返回值统一附加运行时 cache 字段（对应原版 enrichShelf）。
fn enriched_shelf(s: &mut Store, running: &[String]) -> Vec<Value> {
    let list = s.get_shelf();
    s.enrich_shelf(&list, running)
}

#[tauri::command]
fn shelf_list(state: State<StoreState>, jobs: State<JobsState>) -> Vec<Value> {
    let running = jobs.running_ids(JobKind::Cache);
    enriched_shelf(&mut store(&state), &running)
}

#[tauri::command]
fn shelf_upsert(
    state: State<StoreState>,
    jobs: State<JobsState>,
    book: Value,
) -> Result<Vec<Value>, String> {
    let running = jobs.running_ids(JobKind::Cache);
    let mut s = store(&state);
    s.upsert_shelf_book(&book)?;
    let origin = book.get("origin").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if !origin.is_empty() && !is_local_book(&book) {
        s.bump_source_match(&origin, 92.0);
    }
    Ok(enriched_shelf(&mut s, &running))
}

/// 只更新阅读进度：阅读器会高频调用，因此不附加 cache（与原版一致）。
#[tauri::command]
fn shelf_patch_progress(
    state: State<StoreState>,
    id: String,
    patch: Value,
) -> Result<Option<Value>, String> {
    store(&state).patch_shelf_progress(
        &id,
        patch.get("scrollTop").and_then(|v| v.as_f64()),
        patch.get("chapterIndex").and_then(|v| v.as_f64()),
        patch.get("lastReadAt").and_then(|v| v.as_i64()),
    )
}

#[tauri::command]
fn shelf_remove(
    state: State<StoreState>,
    jobs: State<JobsState>,
    id: String,
) -> Result<Vec<Value>, String> {
    jobs.cancel(JobKind::Cache, &id);
    let running = jobs.running_ids(JobKind::Cache);
    let mut s = store(&state);
    s.clear_book_cache(&id);
    s.remove_shelf_book(&id)?;
    Ok(enriched_shelf(&mut s, &running))
}

/// 单书缓存概况（shelf:cacheStatus）；不在书架则返回 null。
#[tauri::command]
fn shelf_cache_status(state: State<StoreState>, jobs: State<JobsState>, id: String) -> Value {
    let running = jobs.running_ids(JobKind::Cache);
    let mut s = store(&state);
    let Some(book) = s
        .get_shelf()
        .into_iter()
        .find(|b| b.get("id").and_then(Value::as_str) == Some(id.as_str()))
    else {
        return Value::Null;
    };
    s.enrich_shelf(&[book], &running)
        .into_iter()
        .next()
        .and_then(|b| b.get("cache").cloned())
        .unwrap_or(Value::Null)
}

// ---------- 书架长任务：整本缓存 / 全架更新 / 导出 TXT ----------

/// 导出 TXT 的开源署名，与 frontend/src/shared/about.ts 的应用信息保持一致。
const APP_EXPORT_NAME: &str = "FlyReader";
const APP_EXPORT_REPO: &str = "https://gitee.com/wucc513721/flyreader";

/// 书籍信息 / 目录接口的时间预算（原版 15000 / 20000 ms）。
const INFO_BUDGET_SECS: u64 = 15;
const TOC_BUDGET_SECS: u64 = 20;
/// 目录分页单页预算：换源时避免单页卡死拖长整体等待
const TOC_PAGE_BUDGET_SECS: u64 = 8;
/// 导出时补抓缺章的预算（原版 15000 ms，比整本缓存的 12s 宽松）。
const EXPORT_CHAPTER_BUDGET_SECS: u64 = 15;

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 阅读位置：书架里的 chapterIndex 由进度回写，可能是浮点形式的整数。
fn chapter_index_of(book: &Value) -> usize {
    book.get("chapterIndex")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0) as usize
}

/// 可阅读章节：跳过卷名行与无 URL 的条目（对应原版 filter(c => c.url && !c.isVolume)）。
fn readable_chapters(chapters: Option<&Value>) -> Vec<Value> {
    chapters
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|c| {
                    !str_of(c, "url").is_empty()
                        && c.get("isVolume").and_then(Value::as_bool) != Some(true)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 目录刷新后把阅读位置重新映射到新目录（对应 matchChapterIndex）。
/// 标题匹配沿用 JS 的双向 includes：空标题会命中第一条有标题的章节。
fn match_chapter_index(prev: &Value, chapters: &[Value]) -> usize {
    if chapters.is_empty() {
        return 0;
    }
    let prev_index = chapter_index_of(prev);
    let current = prev
        .get("chapters")
        .and_then(Value::as_array)
        .and_then(|list| list.get(prev_index));
    let Some(current) = current else {
        return 0;
    };
    let current_url = str_of(current, "url");
    if !current_url.is_empty() {
        if let Some(i) = chapters
            .iter()
            .position(|c| str_of(c, "url") == current_url)
        {
            return i;
        }
    }
    let current_title = str_of(current, "title");
    if let Some(i) = chapters.iter().position(|c| {
        let title = str_of(c, "title");
        !title.is_empty()
            && (title == current_title
                || title.contains(current_title)
                || current_title.contains(title))
    }) {
        return i;
    }
    prev_index.min(chapters.len() - 1)
}

/// 详情规则取空时引擎会填「未知书名 / 佚名」；合并时不要用占位符盖掉书架已有真名。
fn should_apply_info_field(key: &str, info_value: &str, book: &Value) -> bool {
    if info_value.is_empty() {
        return false;
    }
    let existing = str_of(book, key);
    match key {
        "name" if info_value == "未知书名" => existing.is_empty() || existing == "未知书名",
        "author" if info_value == "佚名" => existing.is_empty() || existing == "佚名",
        _ => true,
    }
}

/// 合并书籍信息与目录：只覆盖非空元信息，其余保留书架原值（原版 info.x || book.x）。
fn merge_book_info(
    book: &Value,
    info: &Value,
    toc_url: &str,
    chapters: &[Value],
    chapter_index: Option<usize>,
) -> Value {
    let mut merged = book.clone();
    let Some(obj) = merged.as_object_mut() else {
        return merged;
    };
    for key in ["name", "author", "coverUrl", "intro", "lastChapter"] {
        let value = str_of(info, key);
        if should_apply_info_field(key, value, book) {
            obj.insert(key.to_string(), json!(value));
        }
    }
    if !toc_url.is_empty() {
        obj.insert("tocUrl".into(), json!(toc_url));
    }
    obj.insert("chapters".into(), json!(chapters));
    if let Some(index) = chapter_index {
        obj.insert("chapterIndex".into(), json!(index));
    }
    obj.insert("updatedAt".into(), json!(now_ms()));
    merged
}

/// 取书籍信息与目录；超时文案由调用方给出（原版把 withTimeout 提示写在各调用点）。
async fn fetch_info_and_toc(
    source: &Value,
    book: &Value,
    info_timeout: &str,
    toc_timeout: &str,
) -> Result<(Value, String, Vec<Value>), String> {
    let book_url = str_of(book, "bookUrl").to_string();
    let info = tokio::time::timeout(
        Duration::from_secs(INFO_BUDGET_SECS),
        get_book_info(source, &book_url),
    )
    .await
    .map_err(|_| info_timeout.to_string())??;
    let toc_url = [
        str_of(&info, "tocUrl"),
        str_of(book, "tocUrl"),
        book_url.as_str(),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or("")
    .to_string();
    let chapters = tokio::time::timeout(
        Duration::from_secs(TOC_BUDGET_SECS),
        get_chapter_list_with_budget(source, &toc_url, Some(Duration::from_secs(TOC_PAGE_BUDGET_SECS))),
    )
    .await
    .map_err(|_| toc_timeout.to_string())??;
    Ok((info, toc_url, chapters))
}

/// 单本在线书的目录与元信息刷新（书架「全部更新」的 worker）。
async fn refresh_book(source: &Value, book: &Value, name: &str) -> Result<Value, String> {
    let (info, toc_url, chapters) = fetch_info_and_toc(
        source,
        book,
        &format!("《{name}》更新超时"),
        &format!("《{name}》目录超时"),
    )
    .await?;
    let prev_index = chapter_index_of(book);
    let has_current = book
        .get("chapters")
        .and_then(Value::as_array)
        .and_then(|list| list.get(prev_index))
        .is_some();
    let chapter_index = if has_current && !chapters.is_empty() {
        match_chapter_index(book, &chapters)
    } else {
        0
    };
    Ok(merge_book_info(book, &info, &toc_url, &chapters, Some(chapter_index)))
}

#[tauri::command]
async fn shelf_cache_book(
    app: AppHandle,
    state: State<'_, StoreState>,
    jobs: State<'_, JobsState>,
    id: String,
) -> Result<Vec<Value>, String> {
    if jobs.is_running(JobKind::Cache, &id) {
        return Err("该书正在缓存中".to_string());
    }
    let found = store(&state)
        .get_shelf()
        .into_iter()
        .find(|b| str_of(b, "id") == id)
        .ok_or_else(|| "书架中未找到该书".to_string())?;
    if is_local_book(&found) {
        return Err("本地书籍无需缓存".to_string());
    }
    let origin = str_of(&found, "origin").to_string();
    let source = source_of(&state, &origin).ok_or_else(|| "书源不存在，无法缓存".to_string())?;

    let mut book = found;
    let has_chapters = book
        .get("chapters")
        .and_then(Value::as_array)
        .is_some_and(|list| !list.is_empty());
    if !has_chapters {
        let (info, toc_url, chapters) =
            fetch_info_and_toc(&source, &book, "获取书籍信息超时", "获取目录超时").await?;
        book = merge_book_info(&book, &info, &toc_url, &chapters, None);
        store(&state).upsert_shelf_book(&book)?;
    }

    let chapters = readable_chapters(book.get("chapters"));
    let total = chapters.len();
    let cached_urls: HashSet<String> =
        store(&state).cached_chapter_urls(&id).into_iter().collect();
    let initial_cached = chapters
        .iter()
        .filter(|c| cached_urls.contains(str_of(c, "url")))
        .count();
    let flag = jobs.restart(JobKind::Cache, &id);
    let progress = Arc::new(ProgressEmitter::new(&app, "shelf_cache_progress", total));
    progress.start(json!({ "bookId": id, "cached": initial_cached }));

    let concurrency = request_concurrency(&state).clamp(1, total.max(1));
    let chapters = Arc::new(chapters);
    let cached_urls = Arc::new(cached_urls);
    let cursor = Arc::new(AtomicUsize::new(0));
    let cached_count = Arc::new(AtomicUsize::new(initial_cached));
    let source = Arc::new(source);
    let mut handles = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let app = app.clone();
        let chapters = Arc::clone(&chapters);
        let cached_urls = Arc::clone(&cached_urls);
        let cursor = Arc::clone(&cursor);
        let cached_count = Arc::clone(&cached_count);
        let flag = Arc::clone(&flag);
        let source = Arc::clone(&source);
        let progress = Arc::clone(&progress);
        let id = id.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            loop {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                if index >= total {
                    break;
                }
                let ch = chapters[index].clone();
                let url = str_of(&ch, "url").to_string();
                let title = str_of(&ch, "title").to_string();
                if cached_urls.contains(&url) {
                    progress.step(
                        &title,
                        json!({
                            "bookId": id,
                            "cached": cached_count.load(Ordering::Relaxed),
                            "ok": true
                        }),
                    );
                    continue;
                }
                let fetched = tokio::time::timeout(
                    Duration::from_secs(CHAPTER_BUDGET_SECS),
                    get_content(&source, &url),
                )
                .await;
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(Ok(content)) = fetched {
                    if !content.is_empty() {
                        let state = app.state::<StoreState>();
                        if let Ok(mut store) = state.0.lock() {
                            store.set_chapter_content(
                                &id,
                                &url,
                                &content,
                                ch.get("index").and_then(Value::as_i64),
                                Some(&title),
                            );
                        };
                        cached_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
                progress.step(
                    &title,
                    json!({
                        "bookId": id,
                        "cached": cached_count.load(Ordering::Relaxed),
                        "ok": false
                    }),
                );
            }
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
    let cancelled = flag.load(Ordering::Relaxed);
    jobs.finish(JobKind::Cache, &id);
    let urls: Vec<String> = chapters.iter().map(|c| str_of(c, "url").to_string()).collect();
    let cached = store(&state).count_cached_chapters(&id, &urls);
    progress.finish(json!({
        "bookId": id,
        "phase": if cancelled { "cancelled" } else { "done" },
        "cached": cached
    }));
    let running = jobs.running_ids(JobKind::Cache);
    Ok(enriched_shelf(&mut store(&state), &running))
}

/// 取消整本缓存：与原版 abort + cacheJobs.delete 一致，先把登记摘掉，
/// 书架立刻不再显示「缓存中」；在途 worker 在下一次取章前自行退出。
#[tauri::command]
fn shelf_cancel_cache(state: State<StoreState>, jobs: State<JobsState>, id: String) -> Vec<Value> {
    jobs.cancel(JobKind::Cache, &id);
    jobs.finish(JobKind::Cache, &id);
    let running = jobs.running_ids(JobKind::Cache);
    enriched_shelf(&mut store(&state), &running)
}

#[tauri::command]
async fn shelf_update_all(
    app: AppHandle,
    state: State<'_, StoreState>,
    jobs: State<'_, JobsState>,
) -> Result<Vec<Value>, String> {
    let list = store(&state).get_shelf();
    let online: Vec<Value> = list
        .iter()
        .filter(|b| !is_local_book(b))
        .cloned()
        .collect();
    let total = online.len();
    // 书源表一次性取出，worker 里只做查表，避免每个任务反复克隆全量书源
    let source_map: HashMap<String, Arc<Value>> = store(&state)
        .get_sources()
        .into_iter()
        .map(|s| (str_of(&s, "bookSourceUrl").to_string(), Arc::new(s)))
        .collect();
    let progress = Arc::new(ProgressEmitter::new(&app, "shelf_update_progress", total));
    progress.start(Value::Null);

    let concurrency = request_concurrency(&state).clamp(1, total.max(1));
    let online = Arc::new(online);
    let cursor = Arc::new(AtomicUsize::new(0));
    let slots: Arc<Mutex<Vec<Option<Value>>>> = Arc::new(Mutex::new(vec![None; total]));
    let mut handles = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let online = Arc::clone(&online);
        let cursor = Arc::clone(&cursor);
        let slots = Arc::clone(&slots);
        let progress = Arc::clone(&progress);
        let source_map = source_map.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            loop {
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                if index >= total {
                    break;
                }
                let book = online[index].clone();
                let name = str_of(&book, "name").to_string();
                // 原版 worker 自带兜底：单本失败保留旧条目，不影响整架更新
                let result = match source_map.get(str_of(&book, "origin")) {
                    Some(source) => match refresh_book(source, &book, &name).await {
                        Ok(updated) => updated,
                        Err(e) => {
                            eprintln!("shelf update failed {name}: {e}");
                            book.clone()
                        }
                    },
                    None => {
                        eprintln!("shelf update failed {name}: 书源不存在");
                        book.clone()
                    }
                };
                let ok = result
                    .get("chapters")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty());
                if let Ok(mut list) = slots.lock() {
                    if let Some(slot) = list.get_mut(index) {
                        *slot = Some(result);
                    }
                }
                progress.step(&name, json!({ "ok": ok }));
            }
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
    let updated: Vec<Value> = {
        let taken = slots.lock().map(|v| v.clone()).unwrap_or_default();
        taken
            .into_iter()
            .enumerate()
            .map(|(i, slot)| slot.unwrap_or_else(|| online[i].clone()))
            .collect()
    };
    progress.finish(Value::Null);

    let by_id: HashMap<String, Value> = updated
        .iter()
        .map(|b| (str_of(b, "id").to_string(), b.clone()))
        .collect();
    // 保持书架原有顺序与本地书条目，只替换成功更新过的在线书
    let outcomes: Vec<Value> = list
        .iter()
        .map(|b| {
            by_id
                .get(str_of(b, "id"))
                .cloned()
                .unwrap_or_else(|| b.clone())
        })
        .collect();
    let mut s = store(&state);
    s.save_shelf(&outcomes)?;
    let running = jobs.running_ids(JobKind::Cache);
    Ok(s.enrich_shelf(&outcomes, &running))
}

/// 作者名清洗（对应 shared/author.ts 的 authorName）。
fn author_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "佚名".to_string();
    }
    static PREFIX: OnceLock<Result<Regex, String>> = OnceLock::new();
    let cleaned = match PREFIX.get_or_init(|| Regex::new(r"^(作者|作者名|著者)[:：\s]*").map_err(|e| e.to_string()))
    {
        Ok(re) => re.replace(trimmed, "").trim().to_string(),
        Err(_) => trimmed.to_string(),
    };
    if cleaned.is_empty() {
        "佚名".to_string()
    } else {
        cleaned
    }
}

/// 书名转为可落盘的文件名（对应 sanitizeFileName）。
fn sanitize_file_name(name: &str) -> String {
    static ILLEGAL: OnceLock<Result<Regex, String>> = OnceLock::new();
    static SPACES: OnceLock<Result<Regex, String>> = OnceLock::new();
    let replaced = match ILLEGAL.get_or_init(|| {
        Regex::new(r#"[\\/:*?"<>|]"#).map_err(|e| e.to_string())
    }) {
        Ok(re) => re.replace_all(name, "_").to_string(),
        Err(_) => name.to_string(),
    };
    let collapsed = match SPACES.get_or_init(|| Regex::new(r"\s+").map_err(|e| e.to_string())) {
        Ok(re) => re.replace_all(&replaced, " ").to_string(),
        Err(_) => replaced,
    };
    let trimmed: String = collapsed.trim().chars().take(80).collect();
    if trimmed.is_empty() {
        "未命名".to_string()
    } else {
        trimmed
    }
}

/// 导出正文的排版（书名/作者/署名 + 分章 + 收尾署名），段落与原版逐行一致。
fn build_export_text(book: &Value, chapters: &[Value], contents: &HashMap<String, String>) -> String {
    static MANY_NEWLINES: OnceLock<Result<Regex, String>> = OnceLock::new();
    let name = str_of(book, "name").to_string();
    let author = author_name(str_of(book, "author"));
    let credit =
        format!("开源电子书阅读软件{APP_EXPORT_NAME}，Git地址{APP_EXPORT_REPO}");
    let divider = "—".repeat(24);
    let mut lines: Vec<String> = vec![
        name.clone(),
        format!("作者：{author}"),
        credit.clone(),
        String::new(),
        divider.clone(),
        String::new(),
    ];
    for (i, ch) in chapters.iter().enumerate() {
        let title = {
            let raw = str_of(ch, "title");
            if raw.is_empty() {
                format!("第 {} 章", ch.get("index").and_then(Value::as_i64).unwrap_or(i as i64) + 1)
            } else {
                raw.to_string()
            }
        };
        let body = contents
            .get(str_of(ch, "url"))
            .cloned()
            .unwrap_or_else(|| "（本章未能获取正文）".to_string());
        lines.push(title);
        lines.push(String::new());
        lines.push(body);
        lines.push(String::new());
        lines.push(String::new());
    }
    lines.push(divider);
    lines.push(String::new());
    lines.push(name);
    lines.push(format!("作者：{author}"));
    lines.push(credit);
    let joined = lines.join("\n");
    match MANY_NEWLINES.get_or_init(|| Regex::new(r"\n{4,}").map_err(|e| e.to_string())) {
        Ok(re) => re.replace_all(&joined, "\n\n\n").to_string(),
        Err(_) => joined,
    }
}

#[tauri::command]
async fn shelf_export_txt(
    app: AppHandle,
    state: State<'_, StoreState>,
    id: String,
) -> Result<Value, String> {
    let found = store(&state)
        .get_shelf()
        .into_iter()
        .find(|b| str_of(b, "id") == id);
    let mut working = match found {
        Some(book) => book,
        None => return Ok(json!({ "ok": false, "message": "书架中未找到该书" })),
    };
    if is_local_book(&working) {
        return Ok(json!({ "ok": false, "message": "本地书籍请直接使用原文件" }));
    }
    let origin = str_of(&working, "origin").to_string();
    let source = source_of(&state, &origin);

    let has_chapters = working
        .get("chapters")
        .and_then(Value::as_array)
        .is_some_and(|list| !list.is_empty());
    if !has_chapters {
        let Some(src) = &source else {
            return Ok(json!({
                "ok": false,
                "message": "书源不存在，且没有目录，无法导出"
            }));
        };
        match fetch_info_and_toc(src, &working, "获取书籍信息超时", "获取目录超时").await {
            Ok((info, toc_url, chapters)) => {
                working = merge_book_info(&working, &info, &toc_url, &chapters, None);
                store(&state).upsert_shelf_book(&working)?;
            }
            Err(e) => {
                return Ok(json!({ "ok": false, "message": format!("获取目录失败：{e}") }));
            }
        }
    }

    let chapters = readable_chapters(working.get("chapters"));
    if chapters.is_empty() {
        return Ok(json!({ "ok": false, "message": "没有可导出的章节" }));
    }
    let book_id = str_of(&working, "id").to_string();

    use tauri_plugin_dialog::DialogExt;
    let default_name = format!("{}.txt", sanitize_file_name(str_of(&working, "name")));
    let picked = {
        let dlg = app.dialog().file();
        dlg.set_title("导出为 TXT")
            .set_file_name(default_name)
            .add_filter("文本文件", &["txt"])
            .blocking_save_file()
    };
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(json!({ "ok": false, "message": "已取消" }));
    };

    let mut contents: HashMap<String, String> = HashMap::new();
    let mut missing: Vec<Value> = Vec::new();
    {
        let s = store(&state);
        for ch in &chapters {
            let url = str_of(ch, "url").to_string();
            match s.get_chapter_content(Some(&book_id), &url) {
                Some(text) if !text.trim().is_empty() => {
                    contents.insert(url, ensure_novel_paragraphs(&text));
                }
                _ => missing.push(ch.clone()),
            }
        }
    }

    if !missing.is_empty() {
        if let Some(src) = source {
            // 缺章按书源补抓并顺手写回缓存（原版 mapPool + setChapterContent）
            let total = missing.len();
            let concurrency = request_concurrency(&state).clamp(1, total);
            let missing = Arc::new(missing);
            let cursor = Arc::new(AtomicUsize::new(0));
            let fetched: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
            let src = Arc::new(src);
            let mut handles = Vec::with_capacity(concurrency);
            for _ in 0..concurrency {
                let app = app.clone();
                let missing = Arc::clone(&missing);
                let cursor = Arc::clone(&cursor);
                let fetched = Arc::clone(&fetched);
                let src = Arc::clone(&src);
                let book_id = book_id.clone();
                handles.push(tauri::async_runtime::spawn(async move {
                    loop {
                        let index = cursor.fetch_add(1, Ordering::Relaxed);
                        if index >= total {
                            break;
                        }
                        let ch = missing[index].clone();
                        let url = str_of(&ch, "url").to_string();
                        let title = str_of(&ch, "title").to_string();
                        let outcome = tokio::time::timeout(
                            Duration::from_secs(EXPORT_CHAPTER_BUDGET_SECS),
                            get_content(&src, &url),
                        )
                        .await;
                        if let Ok(Ok(text)) = outcome {
                            if text.trim().is_empty() {
                                continue;
                            }
                            let normalized = ensure_novel_paragraphs(&text);
                            let state = app.state::<StoreState>();
                            if let Ok(mut store) = state.0.lock() {
                                store.set_chapter_content(
                                    &book_id,
                                    &url,
                                    &normalized,
                                    ch.get("index").and_then(Value::as_i64),
                                    Some(&title),
                                );
                            };
                            if let Ok(mut list) = fetched.lock() {
                                list.push((url, normalized));
                            }
                        }
                    }
                }));
            }
            for handle in handles {
                let _ = handle.await;
            }
            let list = fetched.lock().map(|v| v.clone()).unwrap_or_default();
            for (url, body) in list {
                contents.insert(url, body);
            }
        }
    }

    let text = build_export_text(&working, &chapters, &contents);
    let total = chapters.len();
    let exported = chapters
        .iter()
        .filter(|c| contents.contains_key(str_of(c, "url")))
        .count();
    // 与原版一致：UTF-8 BOM 开头，避免 Windows 记事本按本地编码打开后乱码
    let written = std::fs::write(&path, format!("\u{FEFF}{text}")).map_err(|e| e.to_string());
    if let Err(msg) = written {
        return Ok(json!({ "ok": false, "message": format!("写入失败：{msg}") }));
    }
    let file = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let name = str_of(&working, "name").to_string();
    let message = if exported == total {
        format!("已导出《{name}》· {exported} 章 → {file}")
    } else {
        format!("已导出《{name}》· {exported}/{total} 章（部分章节未获取到）→ {file}")
    };
    Ok(json!({
        "ok": true,
        "message": message,
        "path": path.to_string_lossy(),
        "exported": exported,
        "total": total
    }))
}

/// 本地导入解析的超时上限（原版给导入 worker 设的 5 分钟）。
const IMPORT_TIMEOUT_SECS: u64 = 5 * 60;

/// 从指定路径导入本地 TXT/EPUB/Markdown：解析走阻塞线程池，写库回到 Store 锁内。
async fn import_local_book(
    state: &State<'_, StoreState>,
    jobs: &State<'_, JobsState>,
    file_path: String,
) -> Value {
    if let Err(msg) = crate::local_books::importer::precheck_file(&file_path) {
        return json!({ "ok": false, "message": msg });
    }
    let parsing = {
        let path = file_path;
        tauri::async_runtime::spawn_blocking(move || crate::local_books::importer::parse_local_file(&path))
    };
    let parsed = match tokio::time::timeout(Duration::from_secs(IMPORT_TIMEOUT_SECS), parsing).await {
        Err(_elapsed) => {
            return json!({ "ok": false, "message": "导入超时，请检查文件是否损坏" });
        }
        Ok(Err(e)) => {
            return json!({ "ok": false, "message": format!("导入失败：{e}") });
        }
        Ok(Ok(Err(msg))) => return json!({ "ok": false, "message": msg }),
        Ok(Ok(Ok(result))) => result,
    };

    let mut s = store(state);
    let book_id = str_of(&parsed.book, "id").to_string();
    let existing = s
        .get_shelf()
        .into_iter()
        .find(|b| str_of(b, "id") == book_id);
    let chapter_count = parsed
        .book
        .get("chapters")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let last_index = chapter_count.saturating_sub(1) as f64;
    // 重新导入同一个文件时保留既有进度（原版在书架层合并这几个字段）
    let inherited = |key: &str| -> Option<Value> {
        existing
            .as_ref()?
            .get(key)
            .filter(|v| !v.is_null())
            .cloned()
    };
    let fallback_added_at = parsed.book.get("addedAt").cloned().unwrap_or_else(|| json!(now_ms()));
    let chapter_index = inherited("chapterIndex")
        .and_then(|v| v.as_f64())
        .map(|v| v.min(last_index))
        .unwrap_or(0.0);
    let mut book = parsed.book;
    if let Some(obj) = book.as_object_mut() {
        obj.insert(
            "chapterIndex".into(),
            if chapter_index.fract() == 0.0 {
                json!(chapter_index as i64)
            } else {
                json!(chapter_index)
            },
        );
        obj.insert("scrollTop".into(), inherited("scrollTop").unwrap_or(json!(0)));
        obj.insert("addedAt".into(), inherited("addedAt").unwrap_or(fallback_added_at));
        obj.insert("updatedAt".into(), json!(now_ms()));
        obj.insert("lastReadAt".into(), inherited("lastReadAt").unwrap_or(json!(0)));
    }

    let rows: Vec<ChapterRow> = parsed
        .rows
        .iter()
        .map(|r| ChapterRow {
            url: &r.url,
            index: r.index,
            title: &r.title,
            content: &r.content,
        })
        .collect();
    if let Err(msg) = s.replace_chapters(&book_id, &rows) {
        return json!({ "ok": false, "message": msg });
    }
    if let Err(msg) = s.upsert_shelf_book(&book) {
        return json!({ "ok": false, "message": msg });
    }
    let shelf = enriched_shelf(&mut s, &jobs.running_ids(JobKind::Cache));
    let attached = shelf
        .iter()
        .find(|b| str_of(b, "id") == book_id)
        .cloned()
        .unwrap_or_else(|| book.clone());
    json!({
        "ok": true,
        "book": attached,
        "shelf": shelf,
        "message": parsed.message
    })
}

/// 通过系统文件选择器导入本地书籍（TXT / EPUB / Markdown）。
#[tauri::command]
async fn shelf_import_local(
    app: AppHandle,
    state: State<'_, StoreState>,
    jobs: State<'_, JobsState>,
) -> Result<Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = {
        let dlg = app.dialog().file();
        dlg.set_title("打开本地书籍")
            .add_filter("电子书", &["txt", "epub", "md", "markdown"])
            .add_filter("TXT", &["txt"])
            .add_filter("EPUB", &["epub"])
            .add_filter("Markdown", &["md", "markdown"])
            .blocking_pick_file()
    };
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(json!({ "ok": false, "message": "已取消" }));
    };
    Ok(import_local_book(&state, &jobs, path.to_string_lossy().to_string()).await)
}

/// 旧版 FlyNovel 数据是否可导入（供设置页显示与按钮可用性）
#[tauri::command]
fn data_legacy_status(app: AppHandle) -> Value {
    let path = app
        .path()
        .app_data_dir()
        .ok()
        .and_then(|dir| crate::db::legacy_db_path(&dir));
    match path {
        Some(p) => json!({
            "available": true,
            "path": p.to_string_lossy(),
            "bytes": std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
        }),
        None => json!({ "available": false, "path": Value::Null, "bytes": 0 }),
    }
}

/// 显式导入旧版数据（合并语义：同主键覆盖，章节缓存只补缺）
#[tauri::command]
fn data_import_legacy(app: AppHandle, state: State<StoreState>) -> Result<Value, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let src = crate::db::legacy_db_path(&dir).ok_or("未找到旧版 FlyNovel 数据")?;
    let merged = store(&state).import_legacy(&src)?;
    Ok(json!({ "merged": merged, "path": src.to_string_lossy() }))
}

/// 在途任务类型：整本缓存与章节预载可同时对同一本书登记，取消互不影响
///（对应原版各自独立的 cacheJobs / preloadJobs 两张表）。
#[derive(Clone, Copy)]
enum JobKind {
    Cache,
    Preload,
}

impl JobKind {
    fn prefix(self) -> &'static str {
        match self {
            JobKind::Cache => "cache",
            JobKind::Preload => "preload",
        }
    }
}

/// 在途任务登记表。原版用 AbortController 中断在途请求；
/// 这里在页/章之间检查取消标志，语义等价（最多多完成一次请求）。
#[derive(Default)]
pub struct JobsState {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl JobsState {
    /// 登记新一轮任务并返回其取消标志（同键旧任务视为已被替换）
    fn restart(&self, kind: JobKind, id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = self.flags.lock() {
            map.insert(format!("{}:{id}", kind.prefix()), flag.clone());
        }
        flag
    }

    fn finish(&self, kind: JobKind, id: &str) {
        if let Ok(mut map) = self.flags.lock() {
            map.remove(&format!("{}:{id}", kind.prefix()));
        }
    }

    fn cancel(&self, kind: JobKind, id: &str) {
        let key = format!("{}:{id}", kind.prefix());
        if let Ok(map) = self.flags.lock() {
            if let Some(flag) = map.get(&key) {
                flag.store(true, Ordering::Relaxed);
            }
        }
    }

    /// 某书是否已有在途的该类任务（对应原版 cacheJobs.has(id)）
    fn is_running(&self, kind: JobKind, id: &str) -> bool {
        self.flags
            .lock()
            .map(|map| map.contains_key(&format!("{}:{id}", kind.prefix())))
            .unwrap_or(false)
    }

    /// 某类任务当前在途的 bookId 列表
    fn running_ids(&self, kind: JobKind) -> Vec<String> {
        let prefix = format!("{}:", kind.prefix());
        self.flags
            .lock()
            .map(|map| {
                map.keys()
                    .filter(|k| k.starts_with(&prefix))
                    .map(|k| k[prefix.len()..].to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// 按 bookSourceUrl 找书源（对应 findSource）。
fn source_of(state: &State<StoreState>, origin: &str) -> Option<Value> {
    state
        .0
        .lock()
        .ok()?
        .get_sources()
        .into_iter()
        .find(|s| s.get("bookSourceUrl").and_then(Value::as_str) == Some(origin))
}

fn request_concurrency(state: &State<StoreState>) -> usize {
    let n = state
        .0
        .lock()
        .map(|mut s| {
            s.get_prefs()
                .get("requestConcurrency")
                .and_then(Value::as_u64)
                .unwrap_or(4) as usize
        })
        .unwrap_or(4);
    n.clamp(1, 16)
}

/// 正文是否仍像 Markdown（对应 isMarkdownLocalBook 的兜底分支）。
/// Markdown 必须原样返回：段落整理会吞掉空行、把标题粘进正文。
fn looks_like_markdown(content: &str) -> bool {
    static HEADING: OnceLock<Result<Regex, String>> = OnceLock::new();
    static LINK: OnceLock<Result<Regex, String>> = OnceLock::new();
    let heading = HEADING.get_or_init(|| Regex::new(r"(?m)^#{1,6}\s+\S").map_err(|e| e.to_string()));
    let link = LINK.get_or_init(|| Regex::new(r"\[[^\]]+\]\([^)]+\)").map_err(|e| e.to_string()));
    let sample: String = content.chars().take(4000).collect();
    let code_fence = sample.lines().any(|l| l.trim_start().starts_with("```"));
    let table = sample
        .lines()
        .any(|l| l.trim_start().starts_with('|') && l.trim_end().ends_with('|'));
    code_fence
        || table
        || heading.as_ref().map(|re| re.is_match(&sample)).unwrap_or(false)
        || link.as_ref().map(|re| re.is_match(&sample)).unwrap_or(false)
}

/// 单章正文抓取的时间预算，对应原版 withTimeout(..., 12000)。
const CHAPTER_BUDGET_SECS: u64 = 12;

/// 单书源搜索的时间预算（原版同样是 12s），超时就给该书源记 0 分而不是整体失败。
const SEARCH_BUDGET_SECS: u64 = 12;

#[tauri::command]
async fn books_info(
    state: State<'_, StoreState>,
    origin: String,
    book_url: String,
) -> Result<Value, String> {
    let source = source_of(&state, &origin).ok_or("书源不存在")?;
    tokio::time::timeout(
        Duration::from_secs(INFO_BUDGET_SECS),
        get_book_info(&source, &book_url),
    )
    .await
    .map_err(|_| format!("获取详情超时（{INFO_BUDGET_SECS}s）：{book_url}"))?
}

#[tauri::command]
async fn books_toc(
    state: State<'_, StoreState>,
    origin: String,
    toc_url: String,
) -> Result<Vec<Value>, String> {
    let source = source_of(&state, &origin).ok_or("书源不存在")?;
    tokio::time::timeout(
        Duration::from_secs(TOC_BUDGET_SECS),
        get_chapter_list_with_budget(&source, &toc_url, Some(Duration::from_secs(TOC_PAGE_BUDGET_SECS))),
    )
    .await
    .map_err(|_| format!("获取目录超时（{TOC_BUDGET_SECS}s）：{toc_url}"))?
}

#[tauri::command]
async fn books_content(
    state: State<'_, StoreState>,
    origin: String,
    chapter_url: String,
    book_id: Option<String>,
    // refresh=true：跳过本地缓存，强制按书源重新抓取并覆盖写入
    refresh: Option<bool>,
) -> Result<String, String> {
    let refresh = refresh.unwrap_or(false);

    if !refresh {
        let cached = state
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .get_chapter_content(book_id.as_deref(), &chapter_url);
        if let Some(cached) = cached {
            let marked_local = book_id
                .as_deref()
                .map(|id| {
                    state
                        .0
                        .lock()
                        .map(|s| s.is_markdown_local(id))
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if marked_local || looks_like_markdown(&cached) {
                return Ok(cached);
            }
            let normalized = ensure_novel_paragraphs(&cached);
            if !normalized.is_empty() && normalized != cached {
                if let Some(id) = book_id.as_deref() {
                    state
                        .0
                        .lock()
                        .map_err(|e| e.to_string())?
                        .set_chapter_content(id, &chapter_url, &normalized, None, None);
                }
            }
            return Ok(normalized);
        }
    }

    if origin == "local" || chapter_url.starts_with("local://") {
        return Err(if refresh {
            "本地书籍无法重新加载".to_string()
        } else {
            "本地章节内容缺失，请重新打开本地文件".to_string()
        });
    }
    let source = source_of(&state, &origin).ok_or(if refresh {
        "书源不存在，无法重新加载".to_string()
    } else {
        "书源不存在或网络不可用，且本地无缓存".to_string()
    })?;
    match get_content(&source, &chapter_url).await {
        Ok(content) => {
            if !content.is_empty() {
                let id = book_id.clone().unwrap_or_else(|| "__browse__".to_string());
                state
                    .0
                    .lock()
                    .map_err(|e| e.to_string())?
                    .set_chapter_content(&id, &chapter_url, &content, None, None);
            }
            Ok(content)
        }
        Err(e) => {
            if refresh {
                // 强制刷新失败时保留原缓存，不把失败页写进正文
                return Err(format!("重新加载失败：{e}"));
            }
            // 网络失败时退回任何一书按 URL 命中的缓存，并把它归到当前书
            let fallback = {
                let store = state.0.lock().map_err(|e| e.to_string())?;
                store
                    .get_chapter_content(book_id.as_deref(), &chapter_url)
                    .or_else(|| store.get_chapter_content(None, &chapter_url))
            };
            match fallback {
                Some(text) => {
                    if let Some(id) = book_id.as_deref() {
                        state
                            .0
                            .lock()
                            .map_err(|e| e.to_string())?
                            .set_chapter_content(id, &chapter_url, &text, None, None);
                    }
                    Ok(text)
                }
                None => Err(e),
            }
        }
    }
}

#[tauri::command]
fn books_cached_urls(state: State<StoreState>, book_id: String) -> Vec<String> {
    if book_id.is_empty() {
        return Vec::new();
    }
    state
        .0
        .lock()
        .map(|s| s.cached_chapter_urls(&book_id))
        .unwrap_or_default()
}

#[tauri::command]
async fn books_preload(
    app: AppHandle,
    state: State<'_, StoreState>,
    jobs: State<'_, JobsState>,
    origin: String,
    book_id: String,
    chapter_urls: Vec<String>,
) -> Result<Vec<String>, String> {
    if book_id.is_empty() {
        return Ok(Vec::new());
    }
    let Some(source) = source_of(&state, &origin) else {
        return Ok(state
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .cached_chapter_urls(&book_id));
    };
    let concurrency = request_concurrency(&state);
    let cached: Vec<String> = state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .cached_chapter_urls(&book_id);
    let cached_set: HashSet<String> = cached.into_iter().collect();
    let queue = Arc::new(Mutex::new(VecDeque::<String>::from_iter(
        chapter_urls
            .into_iter()
            .filter(|u| !u.is_empty() && !cached_set.contains(u)),
    )));
    if queue.lock().map_err(|e| e.to_string())?.is_empty() {
        return Ok(cached_set.into_iter().collect());
    }

    let flag = jobs.restart(JobKind::Preload, &book_id);
    let source = Arc::new(source);
    let mut handles = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let app = app.clone();
        let queue = Arc::clone(&queue);
        let flag = Arc::clone(&flag);
        let source = Arc::clone(&source);
        let book_id = book_id.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            loop {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                // 取任务与抓取之间必须释放队列锁，否则 MutexGuard 会跨 await
                let url = {
                    let Ok(mut pending) = queue.lock() else { break };
                    match pending.pop_front() {
                        Some(u) => u,
                        None => break,
                    }
                };
                let fetched = tokio::time::timeout(
                    Duration::from_secs(CHAPTER_BUDGET_SECS),
                    get_content(&source, &url),
                )
                .await;
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(Ok(content)) = fetched {
                    if !content.is_empty() {
                        let state = app.state::<StoreState>();
                        if let Ok(mut store) = state.0.lock() {
                            store.set_chapter_content(&book_id, &url, &content, None, None);
                        };
                    }
                }
            }
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
    jobs.finish(JobKind::Preload, &book_id);
    Ok(state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .cached_chapter_urls(&book_id))
}

#[tauri::command]
fn books_cancel_preload(jobs: State<JobsState>, book_id: String) {
    jobs.cancel(JobKind::Preload, &book_id);
}

// ---------- 长任务进度事件 ----------

/// 进度事件推送骨架：对应原版 runPoolWithProgress 的 safeSend + 150ms 节流。
/// start / done 一定推送，中间的 progress 按时间窗合并，避免上千次 IPC。
struct ProgressEmitter {
    app: AppHandle,
    channel: &'static str,
    total: usize,
    done: AtomicUsize,
    last: Mutex<std::time::Instant>,
}

impl ProgressEmitter {
    fn new(app: &AppHandle, channel: &'static str, total: usize) -> Self {
        ProgressEmitter {
            app: app.clone(),
            channel,
            total,
            done: AtomicUsize::new(0),
            last: Mutex::new(std::time::Instant::now()),
        }
    }

    /// extra 里同名的字段优先，与原版 `{ done, total, current, phase, ...extra }`
    /// 的展开顺序一致（取消缓存时 extra 要把 phase 改成 cancelled）。
    fn emit(&self, done: usize, current: &str, phase: &str, extra: Value) {
        let mut payload = match extra {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        payload.entry("done").or_insert_with(|| json!(done));
        payload.entry("total").or_insert_with(|| json!(self.total));
        payload.entry("current").or_insert_with(|| json!(current));
        payload.entry("phase").or_insert_with(|| json!(phase));
        let _ = self.app.emit(self.channel, Value::Object(payload));
    }

    fn start(&self, extra: Value) {
        self.emit(0, "", "start", extra);
    }

    /// 单个条目完成：递增计数并按节流窗口推送；返回累计完成数
    fn step(&self, current: &str, extra: Value) -> usize {
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        let now = std::time::Instant::now();
        let should = match self.last.lock() {
            Ok(mut last) => {
                if now.duration_since(*last).as_millis() as u64 >= PROGRESS_THROTTLE_MS {
                    *last = now;
                    true
                } else {
                    false
                }
            }
            Err(_) => true,
        };
        if should {
            self.emit(done, current, "progress", extra);
        }
        done
    }

    fn finish(&self, extra: Value) {
        self.emit(self.done.load(Ordering::Relaxed), "", "done", extra);
    }
}

const PROGRESS_THROTTLE_MS: u64 = 150;

// ---------- 起点移动站排行 / 完本 ----------

/// 拉取起点排行或完本列表（仅移动站；HTML → majax）。
#[tauri::command]
async fn rank_qidian_list(
    channel: String,
    board: Option<String>,
    gender: Option<String>,
    page: Option<u32>,
    cat_id: Option<i64>,
) -> Result<Value, String> {
    let channel = RankChannel::parse(&channel)?;
    let board = match channel {
        RankChannel::Rank => {
            let raw = board.unwrap_or_else(|| "yuepiao".into());
            Some(RankBoard::parse(&raw)?)
        }
        RankChannel::Finish => None,
    };
    let gender = RankGender::parse(gender.as_deref().unwrap_or("male"))?;
    let page = page.unwrap_or(1).max(1);
    let cat_id = cat_id.unwrap_or(-1);
    let result = fetch_rank_list(channel, board, gender, page, cat_id).await?;
    Ok(list_result_to_value(result))
}

// ---------- 多书源并发搜索 ----------

#[tauri::command]
async fn books_search(
    app: AppHandle,
    state: State<'_, StoreState>,
    keyword: String,
) -> Result<Value, String> {
    let key = keyword.trim().to_string();

    // 书架在线书数量是排序的强信号（对应 shelfCountByOrigin）
    let shelf_counts: HashMap<String, usize> = {
        let mut s = state.0.lock().map_err(|e| e.to_string())?;
        let mut m: HashMap<String, usize> = HashMap::new();
        for b in s.get_shelf() {
            if is_local_book(&b) {
                continue;
            }
            let origin = match b.get("origin").and_then(Value::as_str) {
                Some(o) if !o.is_empty() => o.to_string(),
                _ => continue,
            };
            *m.entry(origin).or_insert(0) += 1;
        }
        m
    };

    let sources: Vec<Value> = {
        let mut s = state.0.lock().map_err(|e| e.to_string())?;
        let mut list: Vec<Value> = s
            .get_sources()
            .into_iter()
            .filter(|s| {
                s.get("enabled").and_then(Value::as_bool) != Some(false)
                    && !s.get("searchUrl").and_then(Value::as_str).unwrap_or("").is_empty()
            })
            .collect();
        list.sort_by(|a, b| crate::relevance::compare_for_search(a, b, &shelf_counts));
        list
    };

    let total = sources.len();
    let progress = Arc::new(ProgressEmitter::new(&app, "books_search_progress", total));
    progress.start(json!({ "found": 0 }));

    let concurrency = request_concurrency(&state).clamp(1, total.max(1));
    let found = Arc::new(AtomicUsize::new(0));
    let ordered = Arc::new(Mutex::new(Vec::<Value>::new()));
    let samples = Arc::new(Mutex::new(HashMap::<String, f64>::new()));
    let recovered = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    let sources = Arc::new(sources);
    let cursor = Arc::new(AtomicUsize::new(0));

    if total > 0 {
        let mut handles = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let app = app.clone();
            let progress = Arc::clone(&progress);
            let sources = Arc::clone(&sources);
            let cursor = Arc::clone(&cursor);
            let key = key.clone();
            let found = Arc::clone(&found);
            let ordered = Arc::clone(&ordered);
            let samples = Arc::clone(&samples);
            let recovered = Arc::clone(&recovered);
            handles.push(tauri::async_runtime::spawn(async move {
                loop {
                    let index = cursor.fetch_add(1, Ordering::Relaxed);
                    if index >= sources.len() {
                        break;
                    }
                    let source = sources[index].clone();
                    let name = source
                        .get("bookSourceName")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let url = source
                        .get("bookSourceUrl")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let outcome = tokio::time::timeout(
                        Duration::from_secs(SEARCH_BUDGET_SECS),
                        search_books(&source, &key, 1),
                    )
                    .await;
                    match outcome {
                        Ok(Ok(raw)) => {
                            let hits = crate::relevance::filter_relevant(&key, &raw);
                            if let Ok(mut map) = samples.lock() {
                                map.insert(
                                    url.clone(),
                                    crate::relevance::score_match_sample(&key, &raw, &hits),
                                );
                            }
                            if !hits.is_empty() {
                                found.fetch_add(hits.len(), Ordering::Relaxed);
                                if let Ok(mut list) = ordered.lock() {
                                    list.extend(hits.iter().cloned());
                                }
                                if let Ok(mut map) = recovered.lock() {
                                    map.insert(
                                        url,
                                        hits[0].get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                                    );
                                }
                                let _ = app.emit("books_search_partial", &hits);
                            }
                        }
                        _ => {
                            if let Ok(mut map) = samples.lock() {
                                map.insert(url, 0.0);
                            }
                        }
                    }
                    progress.step(&name, json!({ "found": found.load(Ordering::Relaxed) }));
                }
            }));
        }
        for handle in handles {
            let _ = handle.await;
        }
    }

    // 匹配分回写：一次搜索即可让嘈杂书源降权、命中书源提权
    let updated_sources = {
        let mut s = state.0.lock().map_err(|e| e.to_string())?;
        let sample_map = samples.lock().map(|m| m.clone()).unwrap_or_default();
        let hit_map = recovered.lock().map(|m| m.clone()).unwrap_or_default();
        let next: Vec<Value> = if sample_map.is_empty() && hit_map.is_empty() {
            s.get_sources()
        } else {
            let now = now_ms();
            let list: Vec<Value> = s
                .get_sources()
                .into_iter()
                .map(|mut src| {
                    let url = src.get("bookSourceUrl").and_then(Value::as_str).unwrap_or("").to_string();
                    let sample = sample_map.get(&url).copied();
                    let hit = hit_map.get(&url).cloned();
                    if sample.is_none() && hit.is_none() {
                        return src;
                    }
                    if let Some(sample) = sample {
                        if let Some(obj) = src.as_object_mut() {
                            let prev = obj
                                .get("flyMatchScore")
                                .and_then(Value::as_f64)
                                .filter(|v| v.is_finite());
                            obj.insert("flyMatchScore".into(), json!(blend_match_score(prev, sample)));
                            let n = obj.get("flyMatchSamples").and_then(Value::as_f64).unwrap_or(0.0) + 1.0;
                            obj.insert("flyMatchSamples".into(), json!(n));
                        }
                    }
                    if let Some(hit_name) = hit {
                        if src.get("flyTestStatus").and_then(Value::as_str) == Some("fail") {
                            if let Some(obj) = src.as_object_mut() {
                                obj.insert("flyTestStatus".into(), json!("ok"));
                                obj.insert(
                                    "flyTestMessage".into(),
                                    json!(format!("搜索「{key}」命中《{hit_name}》，已自动恢复可用")),
                                );
                                obj.insert("flyTestAt".into(), json!(now));
                            }
                        }
                    }
                    src
                })
                .collect();
            // 走到这一分支说明至少有一个书源被采样过，直接整体回写
            s.save_sources(&list).map_err(|e| e.to_string())?;
            list
        };
        next
    };

    let final_books = {
        let list = ordered.lock().map(|v| v.clone()).unwrap_or_default();
        let mut books = crate::relevance::filter_relevant(&key, &list);
        books.sort_by(|a, b| {
            let sa = crate::relevance::title_similarity(
                &key,
                a.get("name").and_then(Value::as_str).unwrap_or(""),
            );
            let sb = crate::relevance::title_similarity(
                &key,
                b.get("name").and_then(Value::as_str).unwrap_or(""),
            );
            if sa != sb {
                return sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal);
            }
            let ka = shelf_counts
                .get(a.get("origin").and_then(Value::as_str).unwrap_or(""))
                .copied()
                .unwrap_or(0);
            let kb = shelf_counts
                .get(b.get("origin").and_then(Value::as_str).unwrap_or(""))
                .copied()
                .unwrap_or(0);
            if ka != kb {
                return kb.cmp(&ka);
            }
            a.get("originName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(b.get("originName").and_then(Value::as_str).unwrap_or(""))
        });
        books
    };

    let history = {
        let s = state.0.lock().map_err(|e| e.to_string())?;
        if key.is_empty() {
            s.get_search_history()
        } else {
            s.add_search_history(&key)
        }
    };

    let found_total = found.load(Ordering::Relaxed);
    progress.finish(json!({ "found": found_total }));
    Ok(json!({
        "books": final_books,
        "history": history,
        "sources": updated_sources,
    }))
}

/// 在 commands 模块内完成 generate_handler 注册
///（#[tauri::command] 生成的内部宏跨模块不可见）。
pub fn build_app() -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    crate::boss_key::on_shortcut_event(app, event.state);
                })
                .build(),
        )
        .setup(|app| {
            let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            let mut store = crate::store::Store::open(&dir).map_err(|e| e.to_string())?;
            let prefs = store.get_prefs();
            let settings = store.get_settings();
            app.manage(StoreState(std::sync::Mutex::new(store)));
            app.manage(JobsState::default());
            app.manage(crate::boss_key::BossState::default());
            app.manage(crate::app_menu::ZoomState::default());
            let handle = app.handle().clone();
            crate::app_menu::build(&handle).map_err(|e| e.to_string())?;
            crate::theme::apply_app_theme(&handle, &settings);
            // 老板键注册要离开主线程，否则 macOS 上会等自己阻塞（见 boss_key 模块注释）
            tauri::async_runtime::spawn(async move {
                crate::boss_key::register_from_prefs(&handle, &prefs);
            });
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                // Wry 没有绕过缓存的重新加载，两项都走同一个 reload（原版行为一致）
                crate::app_menu::ID_RELOAD | crate::app_menu::ID_FORCE_RELOAD => {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.reload();
                    }
                }
                #[cfg(debug_assertions)]
                crate::app_menu::ID_DEVTOOLS => {
                    if let Some(win) = app.get_webview_window("main") {
                        if win.is_devtools_open() {
                            win.close_devtools();
                        } else {
                            win.open_devtools();
                        }
                    }
                }
                crate::app_menu::ID_ZOOM_RESET
                | crate::app_menu::ID_ZOOM_IN
                | crate::app_menu::ID_ZOOM_OUT => {
                    crate::app_menu::apply_zoom(app, id);
                }
                // 预定义项（复制/最小化等）由系统处理，不会走到这里
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_get_state,
            app_get_version,
            app_check_update,
            app_open_external,
            settings_get,
            settings_save,
            settings_add_purify,
            settings_remove_purify,
            prefs_get,
            prefs_save,
            fonts_list,
            search_history,
            search_clear_history,
            search_remove_history,
            sources_list,
            sources_toggle,
            sources_remove,
            sources_remove_many,
            sources_import_file,
            sources_import_url,
            sources_repo_list,
            sources_fetch_online,
            sources_export_file,
            sources_update,
            sources_test,
            sources_test_all,
            sources_remove_invalid,
            shelf_list,
            shelf_upsert,
            shelf_patch_progress,
            shelf_remove,
            shelf_cache_status,
            shelf_cache_book,
            shelf_cancel_cache,
            shelf_update_all,
            shelf_export_txt,
            shelf_import_local,
            data_legacy_status,
            data_import_legacy,
            books_info,
            books_toc,
            books_content,
            books_cached_urls,
            books_preload,
            books_cancel_preload,
            books_search,
            rank_qidian_list
        ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(url: &str, title: &str, index: i64) -> Value {
        json!({ "url": url, "title": title, "index": index })
    }

    #[test]
    fn readable_chapters_skips_volume_rows_and_blank_urls() {
        let chapters = json!([
            ch("a", "第一卷", 0),
            { "url": "b", "title": "第1章", "index": 1, "isVolume": true },
            { "url": "", "title": "第2章", "index": 2 },
            ch("c", "第3章", 3)
        ]);
        let urls: Vec<String> = readable_chapters(Some(&chapters))
            .iter()
            .map(|c| str_of(c, "url").to_string())
            .collect();
        assert_eq!(urls, vec!["a".to_string(), "c".to_string()]);
        assert!(readable_chapters(None).is_empty());
    }

    #[test]
    fn match_chapter_index_prefers_url_then_title_then_clamps() {
        let prev = json!({
            "chapterIndex": 2,
            "chapters": [ch("u1", "第一章", 0), ch("u2", "第二章", 1), ch("u3", "第三章", 2)]
        });
        // URL 命中：即使标题被改写也按 URL 定位
        let by_url = vec![ch("x", "开头", 0), ch("u3", "第三回", 1)];
        assert_eq!(match_chapter_index(&prev, &by_url), 1);
        // URL 变了则退回标题包含
        let by_title = vec![ch("y", "第一章", 0), ch("z", "第三章（修订）", 1)];
        assert_eq!(match_chapter_index(&prev, &by_title), 1);
        // 都对不上时夹到新目录末尾
        let unrelated = vec![ch("p", "序言", 0)];
        assert_eq!(match_chapter_index(&prev, &unrelated), 0);
        assert_eq!(match_chapter_index(&prev, &[]), 0);
        // 进度回写的 chapterIndex 可能是浮点形式
        let floaty = json!({ "chapterIndex": 0.0, "chapters": [ch("u1", "第一章", 0)] });
        assert_eq!(chapter_index_of(&floaty), 0);
    }

    #[test]
    fn merge_book_info_keeps_existing_metadata_and_refreshes_toc() {
        let book = json!({ "id": "b1", "name": "旧名", "author": "甲", "intro": "原简介", "tocUrl": "old" });
        let info = json!({ "name": "新名", "author": "", "coverUrl": "http://c/1.jpg" });
        let chapters = vec![ch("u1", "第一章", 0)];
        let merged = merge_book_info(&book, &info, "new", &chapters, Some(3));
        assert_eq!(merged["name"], json!("新名"));
        // 空字符串不能把已有作者抹掉（原版 info.author || book.author）
        assert_eq!(merged["author"], json!("甲"));
        assert_eq!(merged["intro"], json!("原简介"));
        assert_eq!(merged["tocUrl"], json!("new"));
        assert_eq!(merged["chapterIndex"], json!(3));
        assert_eq!(merged["chapters"].as_array().map(|a| a.len()), Some(1));
        assert!(merged["updatedAt"].is_number());
        // 不传 chapterIndex 时保持原值
        let kept = merge_book_info(&book, &info, "", &chapters, None);
        assert!(kept.get("chapterIndex").is_none());
        assert_eq!(kept["tocUrl"], json!("old"));
    }

    #[test]
    fn merge_book_info_skips_placeholder_name_and_author() {
        let book = json!({ "name": "都市凡人双修传", "author": "天羽凤凰" });
        let info = json!({ "name": "未知书名", "author": "佚名", "lastChapter": "第1章" });
        let chapters = vec![ch("u1", "第一章", 0)];
        let merged = merge_book_info(&book, &info, "", &chapters, None);
        assert_eq!(merged["name"], json!("都市凡人双修传"));
        assert_eq!(merged["author"], json!("天羽凤凰"));
        assert_eq!(merged["lastChapter"], json!("第1章"));
        // 书架本身已是占位时，允许详情占位写回（无更差）
        let blank = json!({ "name": "未知书名", "author": "佚名" });
        let filled = merge_book_info(&blank, &info, "", &chapters, None);
        assert_eq!(filled["name"], json!("未知书名"));
        assert_eq!(filled["author"], json!("佚名"));
    }

    #[test]
    fn author_and_file_names_are_cleaned_like_the_electron_export() {
        assert_eq!(author_name("作者：张三 "), "张三");
        assert_eq!(author_name("   "), "佚名");
        assert_eq!(author_name("著者：李四"), "李四");
        assert_eq!(sanitize_file_name("a/b:c*d?e\"f<g>|h"), "a_b_c_d_e_f_g__h");
        assert_eq!(sanitize_file_name("  粗   糙  "), "粗 糙");
        assert_eq!(sanitize_file_name("///"), "___");
        let long = sanitize_file_name(&"名".repeat(120));
        assert_eq!(long.chars().count(), 80);
    }

    #[test]
    fn build_export_text_wraps_credit_and_marks_gaps() {
        let book = json!({ "name": "测试书", "author": "作者：甲" });
        let chapters = vec![ch("u1", "第一章", 0), ch("u2", "", 1)];
        let mut contents = HashMap::new();
        contents.insert("u1".to_string(), "正文一".to_string());
        let text = build_export_text(&book, &chapters, &contents);
        assert!(text.starts_with("测试书\n作者：甲\n开源电子书阅读软件FlyReader"));
        assert!(text.contains("正文一"));
        assert!(text.contains("第 2 章"));
        assert!(text.contains("（本章未能获取正文）"));
        assert!(text.ends_with(&format!("作者：甲\n开源电子书阅读软件{APP_EXPORT_NAME}，Git地址{APP_EXPORT_REPO}")));
        // 原版会把 4 个以上连续换行压成 3 个
        assert!(!text.contains("\n\n\n\n"));
    }
}

