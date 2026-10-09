//! 数据访问层：书源 / 书架 / 设置 / 偏好 / 搜索历史 / 封面。
//! 对应原 Electron 版 src/main/store.ts + localBooks/covers.ts。
//! 与原实现的一致性说明：
//! - JS 的对象引用相等优化（prev[i] === s 跳过写库）在 Rust 中无对应物，省略；
//!   UPSERT 的 WHERE 未变跳过条件保留（语义等价且有效）。
//! - 封面 data URL 的降采样重编码（原 nativeImage）暂不做，原样存 BLOB（阶段 6 视需要补 image 重编码）。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use base64::Engine as _;

/// 书架 JSON 中的封面哨兵前缀（与 covers.ts 一致）
const COVER_PREFIX: &str = "flycover://";

/// 读 setting 键的 JSON 值
fn read_setting_json(conn: &Connection, key: &str) -> Option<Value> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .ok();
    raw.and_then(|v| serde_json::from_str(&v).ok())
}

fn write_setting_json(conn: &Connection, key: &str, value: &Value) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, serde_json::to_string(value).map_err(|e| e.to_string())?],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}

/// 用指数移动平均融合书源匹配分（对齐 blendMatchScore）。
pub(crate) fn blend_match_score(prev: Option<f64>, sample: f64) -> f64 {
    let next = sample.clamp(0.0, 100.0).round();
    match prev {
        Some(p) if p.is_finite() => p.clamp(0.0, 100.0).round() * 0.65 + next * 0.35,
        _ => next,
    }
    .round()
}

pub fn platform_default_font_family() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "\"PingFang SC\", \"Hiragino Sans GB\", \"Songti SC\", sans-serif"
    }
    #[cfg(windows)]
    {
        "\"Microsoft YaHei\", \"SimSun\", \"Segoe UI\", sans-serif"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "\"Noto Sans CJK SC\", \"WenQuanYi Micro Hei\", \"Source Han Sans SC\", sans-serif"
    }
}

/// 与前端 isLegacyPresetFont 对齐
fn is_legacy_preset_font(font: Option<&str>) -> bool {
    let Some(f) = font else { return true };
    const LEGACY: &[&str] = &[
        "\"Songti SC\", \"Noto Serif SC\", Georgia, serif",
        "\"Songti SC\", \"Noto Serif SC\", \"SimSun\", Georgia, serif",
        "\"PingFang SC\", \"Heiti SC\", \"Microsoft YaHei\", \"Noto Sans SC\", sans-serif",
        "\"Kaiti SC\", \"STKaiti\", \"KaiTi\", serif",
        "\"STFangsong\", \"FangSong\", serif",
        "\"Yuanti SC\", \"STYuanti\", \"PingFang SC\", sans-serif",
    ];
    LEGACY.contains(&f)
}

fn default_reader_settings() -> Value {
    json!({
        "fontSize": 18,
        "lineHeight": 1.8,
        "pageWidth": 720,
        "theme": "paper",
        "fontFamily": platform_default_font_family(),
        "purifyRules": []
    })
}

fn default_prefs() -> Value {
    json!({
        "sourceTestKeyword": "剑来",
        "lastView": "shelf",
        "windowBounds": { "width": 1100, "height": 740 },
        "bossKeyEnabled": true,
        "bossKey": "CommandOrControl+Shift+H",
        "preloadCount": 3,
        "requestConcurrency": 8,
        "shelfSort": "lastRead",
        "eyeCareEnabled": true,
        "eyeCareIntervalMinutes": 120
    })
}

const EYE_CARE_OPTIONS: [f64; 5] = [30.0, 60.0, 120.0, 300.0, 600.0];

fn clamp_eye_care_minutes(value: Option<f64>) -> f64 {
    let n = value.filter(|v| v.is_finite()).map(|v| v.round()).unwrap_or(120.0);
    if EYE_CARE_OPTIONS.contains(&n) {
        return n;
    }
    EYE_CARE_OPTIONS
        .iter()
        .min_by(|a, b| (n - **a).total_cmp(&(*b - n)))
        .copied()
        .unwrap_or(120.0)
}

fn clamp_request_concurrency(value: Option<f64>) -> f64 {
    let n = value
        .filter(|v| v.is_finite())
        .map(|v| v.round())
        .unwrap_or(8.0);
    n.clamp(1.0, 16.0)
}

/// data URL 解析：`data:<mime>[;base64],<payload>`
fn parse_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mime = meta.split(';').next().filter(|s| !s.is_empty()).unwrap_or("image/jpeg").to_string();
    if meta.contains(";base64") {
        let bytes = base64::engine::general_purpose::STANDARD.decode(payload).ok()?;
        Some((mime, bytes))
    } else {
        let decoded = percent_decode(payload);
        Some((mime, decoded.into_bytes()))
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 封面抽离进 book_covers 表，返回应写入书架 JSON 的 coverUrl 值。
/// 与 persistCover 对齐：非 data URL 原样返回；data URL 转哨兵；无效返回 None（删键）。
fn persist_cover(conn: &Connection, book_id: &str, cover_url: Option<&str>) -> Option<String> {
    let url = cover_url?;
    if book_id.is_empty() {
        return Some(url.to_string());
    }
    if !url.starts_with("data:") {
        return Some(url.to_string());
    }
    let (mime, bytes) = parse_data_url(url)?;
    if bytes.is_empty() {
        return None;
    }
    conn.execute(
        "INSERT INTO book_covers (book_id, mime, data) VALUES (?1, ?2, ?3)
         ON CONFLICT(book_id) DO UPDATE SET mime = excluded.mime, data = excluded.data",
        params![book_id, mime, bytes],
    )
    .ok();
    Some(format!("{COVER_PREFIX}{book_id}"))
}

fn get_cover_data_url(conn: &Connection, book_id: &str) -> Option<String> {
    let (mime, data): (String, Vec<u8>) = conn
        .query_row(
            "SELECT mime, data FROM book_covers WHERE book_id = ?1",
            [book_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    Some(format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(data)
    ))
}

/// 落库前清理：删运行时 cache 字段、抽离 data URL 封面。对应 stripCacheField。
fn clean_shelf_book(conn: &Connection, book: &Value) -> Value {
    let mut v = book.clone();
    let Some(obj) = v.as_object_mut() else { return v };
    obj.remove("cache");
    let id = obj.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let cover = obj.get("coverUrl").and_then(|x| x.as_str()).map(|s| s.to_string());
    match persist_cover(conn, &id, cover.as_deref()) {
        Some(c) => {
            obj.insert("coverUrl".into(), json!(c));
        }
        None => {
            obj.remove("coverUrl");
        }
    }
    v
}

/// 批量替换某书章节缓存时的一行（本地导入用）。
pub struct ChapterRow<'a> {
    pub url: &'a str,
    pub index: i64,
    pub title: &'a str,
    pub content: &'a str,
}

/// 书架里该书是否被标记为本地书。
pub(crate) fn is_local_book(book: &Value) -> bool {
    book.get("isLocal").and_then(|v| v.as_bool()) == Some(true)
        || book.get("origin").and_then(|v| v.as_str()) == Some("local")
}

/// shelfMatchBaseline 对齐
fn shelf_match_baseline(count: usize) -> f64 {
    if count == 0 {
        return 0.0;
    }
    (40.0 + count as f64 * 12.0).min(88.0)
}

/// 应用数据仓库：SQLite 连接 + 写穿透内存缓存。整体由 store 层的 Mutex 保护。
pub struct Store {
    conn: Connection,
    sources_cache: Option<Vec<Value>>,
    shelf_cache: Option<Vec<Value>>,
    cover_cache: HashMap<String, String>,
    /// 上次 prefs 保存时间，用于降低高频写
    last_prefs_write: Option<Instant>,
}

impl Store {
    pub fn open(app_data_dir: &std::path::Path) -> Result<Self, String> {
        let conn = crate::db::open_db(app_data_dir)?;
        Ok(Self {
            conn,
            sources_cache: None,
            shelf_cache: None,
            cover_cache: HashMap::new(),
            last_prefs_write: None,
        })
    }

    fn ensure_default_config(&mut self) {
        if read_setting_json(&self.conn, "reader").is_none() {
            write_setting_json(&self.conn, "reader", &default_reader_settings()).ok();
        }
        if read_setting_json(&self.conn, "app").is_none() {
            write_setting_json(&self.conn, "app", &default_prefs()).ok();
        }
    }

    pub fn app_state(&mut self) -> Value {
        self.ensure_default_config();
        let sources = self.sync_match_scores_from_shelf();
        let shelf = self.get_shelf();
        json!({
            "sources": sources,
            "shelf": shelf,
            "settings": self.get_settings(),
            "prefs": self.get_prefs()
        })
    }

    // ---------- 书源 ----------

    pub fn get_sources(&mut self) -> Vec<Value> {
        if let Some(c) = &self.sources_cache {
            return c.clone();
        }
        let mut stmt = self
            .conn
            .prepare("SELECT data FROM sources ORDER BY sort_order ASC, book_source_url ASC")
            .unwrap();
        let list: Vec<Value> = stmt
            .query_map([], |r| {
                let s: String = r.get(0)?;
                Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
            })
            .map(|rows| rows.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();
        self.sources_cache = Some(list.clone());
        list
    }

    /// 对应原 main 的 enrich：列表接口直接返回 sync 后的书源
    pub fn list_sources(&mut self) -> Vec<Value> {
        self.sync_match_scores_from_shelf()
    }

    pub fn save_sources(&mut self, sources: &[Value]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT book_source_url FROM sources").unwrap();
            stmt.query_map([], |r| r.get(0))
                .map(|rows| rows.filter_map(|x| x.ok()).collect())
                .unwrap_or_default()
        };
        let mut incoming: Vec<String> = Vec::new();
        for (i, s) in sources.iter().enumerate() {
            let Some(url) = s.get("bookSourceUrl").and_then(|u| u.as_str()) else { continue };
            if url.is_empty() {
                continue;
            }
            incoming.push(url.to_string());
            tx.execute(
                "INSERT INTO sources (book_source_url, data, sort_order) VALUES (?1, ?2, ?3)
                 ON CONFLICT(book_source_url) DO UPDATE SET
                   data = excluded.data,
                   sort_order = excluded.sort_order
                 WHERE sources.data IS NOT excluded.data
                    OR sources.sort_order IS NOT excluded.sort_order",
                params![url, serde_json::to_string(s).map_err(|e| e.to_string())?, i as i64],
            )
            .map_err(|e| e.to_string())?;
        }
        for url in &existing {
            if !incoming.contains(url) {
                tx.execute("DELETE FROM sources WHERE book_source_url = ?1", [url])
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        let cleaned: Vec<Value> = sources
            .iter()
            .filter(|s| s.get("bookSourceUrl").and_then(|u| u.as_str()).is_some_and(|u| !u.is_empty()))
            .cloned()
            .collect();
        self.sources_cache = Some(cleaned);
        Ok(())
    }

    /// 字段级补丁（值为 null 的键删除）。对应 patchSourceField。
    pub fn patch_source_field(&mut self, url: &str, patch: &Value) -> Result<Option<Value>, String> {
        let found = {
            let cache = self.get_sources();
            cache.iter().find(|s| s.get("bookSourceUrl").and_then(|u| u.as_str()) == Some(url)).cloned()
        };
        let Some(src) = found else { return Ok(None) };
        let mut next = src.clone();
        if let (Some(dst), Some(p)) = (next.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                if v.is_null() {
                    dst.remove(k);
                } else {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
        self.conn
            .execute(
                "UPDATE sources SET data = ?1 WHERE book_source_url = ?2",
                params![serde_json::to_string(&next).map_err(|e| e.to_string())?, url],
            )
            .map_err(|e| e.to_string())?;
        if let Some(cache) = &mut self.sources_cache {
            if let Some(item) = cache.iter_mut().find(|s| s.get("bookSourceUrl").and_then(|u| u.as_str()) == Some(url)) {
                *item = next.clone();
            }
        }
        Ok(Some(next))
    }

    pub fn remove_sources(&mut self, urls: &[String]) -> Result<Vec<Value>, String> {
        let targets: Vec<&String> = urls.iter().filter(|u| !u.is_empty()).collect();
        if !targets.is_empty() {
            let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
            for url in &targets {
                tx.execute("DELETE FROM sources WHERE book_source_url = ?1", [*url])
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            let set: std::collections::HashSet<&str> =
                targets.iter().map(|s| s.as_str()).collect();
            if let Some(cache) = &mut self.sources_cache {
                cache.retain(|s| {
                    !set.contains(s.get("bookSourceUrl").and_then(|u| u.as_str()).unwrap_or(""))
                });
            } else {
                self.sources_cache = None;
            }
        }
        Ok(self.get_sources())
    }

    /// 导入书源：已存在 URL 跳过，新 URL 追加（定向 INSERT，不重写既有行）。对应 importSources。
    pub fn import_sources(&mut self, incoming: &[Value]) -> Result<(usize, usize), String> {
        let current = self.get_sources();
        let mut seen: std::collections::HashSet<String> = current
            .iter()
            .filter_map(|s| s.get("bookSourceUrl").and_then(Value::as_str).map(str::to_string))
            .collect();
        let mut fresh: Vec<Value> = Vec::new();
        let mut skipped = 0usize;
        for s in incoming {
            let Some(url) = s.get("bookSourceUrl").and_then(Value::as_str) else { continue };
            if url.is_empty() {
                continue;
            }
            if !seen.insert(url.to_string()) {
                skipped += 1;
                continue;
            }
            fresh.push(s.clone());
        }
        let added = fresh.len();
        if !fresh.is_empty() {
            let base = current.len();
            let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
            for (i, s) in fresh.iter().enumerate() {
                let url = s.get("bookSourceUrl").and_then(Value::as_str).unwrap_or("");
                tx.execute(
                    "INSERT OR IGNORE INTO sources (book_source_url, data, sort_order) VALUES (?1, ?2, ?3)",
                    params![url, serde_json::to_string(s).map_err(|e| e.to_string())?, (base + i) as i64],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            let mut merged = current;
            merged.extend(fresh);
            self.sources_cache = Some(merged);
        }
        Ok((added, skipped))
    }

    /// 用新书源替换旧 URL 对应项（并去掉与新 URL 冲突的旧项）。对应 replaceSource。
    pub fn replace_source(&mut self, old_url: &str, source: &Value) -> Result<Vec<Value>, String> {
        let new_url = source.get("bookSourceUrl").and_then(Value::as_str).unwrap_or("");
        let mut next: Vec<Value> = self
            .get_sources()
            .into_iter()
            .filter(|s| {
                let u = s.get("bookSourceUrl").and_then(Value::as_str).unwrap_or("");
                u != old_url && u != new_url
            })
            .collect();
        next.push(source.clone());
        self.save_sources(&next)?;
        Ok(next)
    }

    fn sync_match_scores_from_shelf(&mut self) -> Vec<Value> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for b in self.get_shelf() {
            if is_local_book(&b) {
                continue;
            }
            if let Some(origin) = b.get("origin").and_then(|o| o.as_str()) {
                if !origin.is_empty() {
                    *counts.entry(origin.to_string()).or_default() += 1;
                }
            }
        }
        let mut changed = false;
        let next: Vec<Value> = self
            .get_sources()
            .into_iter()
            .map(|mut s| {
                let url = s.get("bookSourceUrl").and_then(|u| u.as_str()).unwrap_or("").to_string();
                let baseline = shelf_match_baseline(counts.get(&url).copied().unwrap_or(0));
                if baseline <= 0.0 {
                    return s;
                }
                let cur = s
                    .get("flyMatchScore")
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite())
                    .unwrap_or(0.0);
                if cur >= baseline {
                    return s;
                }
                changed = true;
                if let Some(obj) = s.as_object_mut() {
                    obj.insert("flyMatchScore".into(), json!(baseline));
                    let samples = obj
                        .get("flyMatchSamples")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                        .max(1.0);
                    obj.insert("flyMatchSamples".into(), json!(samples));
                }
                s
            })
            .collect();
        if changed {
            self.save_sources(&next).ok();
            return next;
        }
        self.get_sources()
    }

    /// 记录一次强相关使用，EMA 融合匹配分（blendMatchScore 对齐，样本默认 90）。
    pub fn bump_source_match(&mut self, url: &str, sample: f64) {
        if url.is_empty() {
            return;
        }
        let found = self
            .get_sources()
            .iter()
            .find(|s| s.get("bookSourceUrl").and_then(|u| u.as_str()) == Some(url))
            .cloned();
        let Some(src) = found else { return };
        let prev = src.get("flyMatchScore").and_then(|v| v.as_f64()).filter(|v| v.is_finite());
        let next = blend_match_score(prev, sample);
        let samples = src.get("flyMatchSamples").and_then(|v| v.as_f64()).unwrap_or(0.0) + 1.0;
        let patch = json!({ "flyMatchScore": next, "flyMatchSamples": samples });
        self.patch_source_field(url, &patch).ok();
    }

    // ---------- 书架 ----------

    fn load_shelf_cache(&mut self) {
        let result: Option<Vec<Value>> = (|| {
            let mut stmt = self
                .conn
                .prepare("SELECT data FROM shelf_books ORDER BY sort_order ASC, updated_at DESC")
                .ok()?;
            let rows = stmt
                .query_map([], |r| {
                    let s: String = r.get(0)?;
                    Ok(serde_json::from_str::<Value>(&s).unwrap_or(Value::Null))
                })
                .ok()?
                .filter_map(|x| x.ok())
                .collect();
            Some(rows)
        })();
        match result {
            Some(rows) => {
                let conn = &self.conn;
                let cover_cache = &mut self.cover_cache;
                self.shelf_cache = Some(
                    rows.into_iter()
                        .map(|b| {
                            let cleaned = strip_cache_no_persist(&b);
                            let with_cover = attach_cover_cached(conn, cover_cache, cleaned);
                            with_cover
                        })
                        .collect(),
                );
            }
            None => self.shelf_cache = None,
        }
    }

    pub fn get_shelf(&mut self) -> Vec<Value> {
        if self.shelf_cache.is_none() {
            self.load_shelf_cache();
        }
        self.shelf_cache.clone().unwrap_or_default()
    }

    pub fn upsert_shelf_book(&mut self, book: &Value) -> Result<Vec<Value>, String> {
        let clean = clean_shelf_book(&self.conn, book);
        let id = clean.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let updated_at = clean
            .get("updatedAt")
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0)
            .unwrap_or_else(now_ms);
        let data = serde_json::to_string(&clean).map_err(|e| e.to_string())?;
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM shelf_books WHERE id = ?1",
                [&id],
                |r| r.get::<_, i64>(0).map(|n| n > 0),
            )
            .unwrap_or(false);
        if exists {
            self.conn
                .execute(
                    "UPDATE shelf_books SET data = ?1, updated_at = ?2 WHERE id = ?3",
                    params![data, updated_at, id],
                )
                .map_err(|e| e.to_string())?;
        } else {
            let min: Option<i64> = self
                .conn
                .query_row("SELECT MIN(sort_order) FROM shelf_books", [], |r| r.get(0))
                .unwrap_or(None);
            let sort = min.map(|m| m - 1).unwrap_or(0);
            self.conn
                .execute(
                    "INSERT INTO shelf_books (id, data, sort_order, updated_at) VALUES (?1, ?2, ?3, ?4)",
                    params![id, data, sort, updated_at],
                )
                .map_err(|e| e.to_string())?;
        }
        self.shelf_cache = None;
        Ok(self.get_shelf())
    }

    /// 整表重写书架：按列表顺序写 sort_order，并删除未出现的行。对应 saveShelf。
    pub fn save_shelf(&mut self, shelf: &[Value]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM shelf_books").unwrap();
            stmt.query_map([], |r| r.get(0))
                .map(|rows| rows.filter_map(|x| x.ok()).collect())
                .unwrap_or_default()
        };
        let mut incoming: Vec<String> = Vec::new();
        for (i, book) in shelf.iter().enumerate() {
            let id = book.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            if id.is_empty() {
                continue;
            }
            let clean = clean_shelf_book(&tx, book);
            let updated_at = clean
                .get("updatedAt")
                .and_then(|v| v.as_i64())
                .filter(|v| *v > 0)
                .unwrap_or_else(now_ms);
            tx.execute(
                "INSERT INTO shelf_books (id, data, sort_order, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                   data = excluded.data,
                   sort_order = excluded.sort_order,
                   updated_at = excluded.updated_at
                 WHERE shelf_books.data IS NOT excluded.data
                    OR shelf_books.sort_order IS NOT excluded.sort_order
                    OR shelf_books.updated_at IS NOT excluded.updated_at",
                params![
                    id.clone(),
                    serde_json::to_string(&clean).map_err(|e| e.to_string())?,
                    i as i64,
                    updated_at
                ],
            )
            .map_err(|e| e.to_string())?;
            incoming.push(id);
        }
        for id in &existing {
            if !incoming.contains(id) {
                tx.execute("DELETE FROM shelf_books WHERE id = ?1", [id])
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.shelf_cache = None;
        Ok(())
    }

    pub fn patch_shelf_progress(
        &mut self,
        id: &str,
        scroll_top: Option<f64>,
        chapter_index: Option<f64>,
        last_read_at: Option<i64>,
    ) -> Result<Option<Value>, String> {
        let row: Option<String> = self
            .conn
            .query_row("SELECT data FROM shelf_books WHERE id = ?1", [id], |r| r.get(0))
            .ok();
        let Some(raw) = row else { return Ok(None) };
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let mut book = clean_shelf_book(&self.conn, &parsed);
        if let Some(obj) = book.as_object_mut() {
            if let Some(t) = scroll_top {
                obj.insert("scrollTop".into(), json!(t.max(0.0).round()));
            }
            if let Some(c) = chapter_index {
                obj.insert("chapterIndex".into(), json!(c.max(0.0).floor()));
            }
            if let Some(r) = last_read_at {
                obj.insert("lastReadAt".into(), json!(r));
            }
            obj.insert("updatedAt".into(), json!(now_ms()));
        }
        let updated_at = book.get("updatedAt").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
        self.conn
            .execute(
                "UPDATE shelf_books SET data = ?1, updated_at = ?2 WHERE id = ?3",
                params![serde_json::to_string(&book).map_err(|e| e.to_string())?, updated_at, id],
            )
            .map_err(|e| e.to_string())?;
        let attached = attach_cover_cached(&self.conn, &mut self.cover_cache, book);
        if let Some(cache) = &mut self.shelf_cache {
            if let Some(item) = cache.iter_mut().find(|b| b.get("id").and_then(|v| v.as_str()) == Some(id)) {
                *item = attached.clone();
            } else {
                self.shelf_cache = None;
            }
        }
        Ok(Some(attached))
    }

    pub fn remove_shelf_book(&mut self, id: &str) -> Result<Vec<Value>, String> {
        self.conn
            .execute("DELETE FROM book_covers WHERE book_id = ?1", [id])
            .ok();
        self.conn
            .execute("DELETE FROM shelf_books WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
        self.cover_cache.remove(id);
        if let Some(cache) = &mut self.shelf_cache {
            cache.retain(|b| b.get("id").and_then(|v| v.as_str()) != Some(id));
        }
        Ok(self.get_shelf())
    }

    // ---------- 章节正文缓存 ----------

    /// 读章节正文缓存：先按书匹配，未命中再按 URL 取最新一条（对应 getChapterContent）。
    pub fn get_chapter_content(&self, book_id: Option<&str>, url: &str) -> Option<String> {
        if let Some(id) = book_id.filter(|i| !i.is_empty()) {
            let hit: Option<String> = self
                .conn
                .query_row(
                    "SELECT content FROM chapters WHERE book_id = ?1 AND chapter_url = ?2",
                    params![id, url],
                    |r| r.get(0),
                )
                .ok();
            if hit.is_some() {
                return hit;
            }
        }
        self.conn
            .query_row(
                "SELECT content FROM chapters WHERE chapter_url = ?1 ORDER BY cached_at DESC LIMIT 1",
                [url],
                |r| r.get(0),
            )
            .ok()
    }

    /// 写入或更新章节缓存；任一要素为空则忽略（与 setChapterContent 一致）。
    pub fn set_chapter_content(
        &mut self,
        book_id: &str,
        url: &str,
        content: &str,
        index: Option<i64>,
        title: Option<&str>,
    ) {
        if book_id.is_empty() || url.is_empty() || content.is_empty() {
            return;
        }
        self.conn
            .execute(
                "INSERT INTO chapters (book_id, chapter_url, chapter_index, title, content, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(book_id, chapter_url) DO UPDATE SET
                   content = excluded.content,
                   chapter_index = COALESCE(excluded.chapter_index, chapters.chapter_index),
                   title = COALESCE(excluded.title, chapters.title),
                   cached_at = excluded.cached_at",
                params![book_id, url, index, title, content, now_ms()],
            )
            .ok();
    }

    /// 用一批章节整体替换某书的正文缓存（单事务先删后插），本地导入落库用。
    pub fn replace_chapters(&mut self, book_id: &str, rows: &[ChapterRow]) -> Result<(), String> {
        let cached_at = now_ms();
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM chapters WHERE book_id = ?1", [book_id])
            .map_err(|e| e.to_string())?;
        for row in rows {
            tx.execute(
                "INSERT INTO chapters (book_id, chapter_url, chapter_index, title, content, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(book_id, chapter_url) DO UPDATE SET
                   content = excluded.content,
                   chapter_index = COALESCE(excluded.chapter_index, chapters.chapter_index),
                   title = COALESCE(excluded.title, chapters.title),
                   cached_at = excluded.cached_at",
                params![book_id, row.url, row.index, row.title, row.content, cached_at],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    /// 书架里该书是否被标记为 Markdown 本地书（localFormat / kind），单行读取避免克隆整个书架。
    pub fn is_markdown_local(&self, book_id: &str) -> bool {
        let Ok(data): rusqlite::Result<String> = self.conn.query_row(
            "SELECT data FROM shelf_books WHERE id = ?1",
            [book_id],
            |r| r.get(0),
        ) else {
            return false;
        };
        let Ok(v) = serde_json::from_str::<Value>(&data) else {
            return false;
        };
        let fmt = v
            .get("localFormat")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let kind = v.get("kind").and_then(Value::as_str).unwrap_or("").to_uppercase();
        fmt == "md" || kind == "MD" || kind == "MARKDOWN"
    }

    /// 某书已缓存的章节 URL（供预载/缓存去重）。
    pub fn cached_chapter_urls(&self, book_id: &str) -> Vec<String> {
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT chapter_url FROM chapters WHERE book_id = ?1")
        else {
            return Vec::new();
        };
        let rows = stmt
            .query_map([book_id], |r| r.get::<_, String>(0))
            .map(|it| it.filter_map(Result::ok).collect())
            .unwrap_or_default();
        rows
    }

    /// 统计给定 URL 中已缓存的条数；分片 IN 查询，避开 SQLite 变量个数上限。
    pub fn count_cached_chapters(&self, book_id: &str, urls: &[String]) -> usize {
        if book_id.is_empty() || urls.is_empty() {
            return 0;
        }
        const CHUNK: usize = 900;
        let mut total = 0usize;
        for chunk in urls.chunks(CHUNK) {
            let marks = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!("SELECT COUNT(*) FROM chapters WHERE book_id = ? AND chapter_url IN ({marks})");
            let Ok(mut stmt) = self.conn.prepare(&sql) else {
                continue;
            };
            let mut args: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 1);
            args.push(&book_id);
            for url in chunk {
                args.push(url);
            }
            if let Ok(n) = stmt.query_row(args.as_slice(), |r| r.get::<_, i64>(0)) {
                total += n.max(0) as usize;
            }
        }
        total
    }

    /// 某书的缓存概况；`chapters` 为空数组时按 0 章处理（与 getBookCacheInfo 一致）。
    pub fn book_cache_info(&self, book_id: &str, chapters: Option<&Value>, caching: bool) -> Value {
        let readable: Vec<String> = chapters
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter(|c| {
                        c.get("url").and_then(Value::as_str).is_some_and(|u| !u.is_empty())
                            && c.get("isVolume").and_then(Value::as_bool) != Some(true)
                    })
                    .filter_map(|c| c.get("url").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let total = readable.len();
        let cached = self.count_cached_chapters(book_id, &readable);
        let status = if caching {
            "caching"
        } else if total == 0 || cached == 0 {
            "none"
        } else if cached >= total {
            "full"
        } else {
            "partial"
        };
        json!({ "bookId": book_id, "total": total, "cached": cached, "status": status })
    }

    /// 给书架条目附加运行时 cache 字段（对应 enrichShelf）；`caching_ids` 是在跑的缓存任务。
    pub fn enrich_shelf(&self, list: &[Value], caching_ids: &[String]) -> Vec<Value> {
        list.iter()
            .map(|book| {
                let id = book.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let mut v = book.clone();
                let cache = if is_local_book(book) {
                    let total = self.readable_chapter_count(&v);
                    json!({ "bookId": id, "total": total, "cached": total, "status": "full" })
                } else {
                    let caching = caching_ids.iter().any(|i| *i == id);
                    self.book_cache_info(&id, v.get("chapters"), caching)
                };
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("cache".into(), cache);
                    if is_local_book(book) {
                        obj.insert("isLocal".into(), json!(true));
                        if obj
                            .get("originName")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .is_empty()
                        {
                            obj.insert("originName".into(), json!("本地"));
                        }
                    }
                }
                v
            })
            .collect()
    }

    fn readable_chapter_count(&self, book: &Value) -> usize {
        book.get("chapters")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter(|c| {
                        c.get("url").and_then(Value::as_str).is_some_and(|u| !u.is_empty())
                            && c.get("isVolume").and_then(Value::as_bool) != Some(true)
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    /// 清除某书章节缓存，返回删除行数。
    pub fn clear_book_cache(&mut self, book_id: &str) -> usize {
        self.conn
            .execute("DELETE FROM chapters WHERE book_id = ?1", [book_id])
            .unwrap_or(0) as usize
    }

    // ---------- 阅读器设置 ----------

    pub fn get_settings(&mut self) -> Value {
        let Some(s) = read_setting_json(&self.conn, "reader") else {
            return default_reader_settings();
        };
        let mut merged = default_reader_settings();
        if let (Some(dst), Some(src)) = (merged.as_object_mut(), s.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        let font = merged.get("fontFamily").and_then(|v| v.as_str()).map(|s| s.to_string());
        let family = if is_legacy_preset_font(font.as_deref()) {
            platform_default_font_family().to_string()
        } else {
            font.unwrap_or_default()
        };
        if !merged.get("purifyRules").map(|v| v.is_array()).unwrap_or(false) {
            merged["purifyRules"] = json!([]);
        }
        merged["fontFamily"] = json!(family);
        merged
    }

    pub fn save_settings(&mut self, settings: &Value) -> Result<Value, String> {
        let mut v = settings.clone();
        if !v.get("purifyRules").map(|x| x.is_array()).unwrap_or(false) {
            v["purifyRules"] = json!([]);
        }
        write_setting_json(&self.conn, "reader", &v)?;
        Ok(self.get_settings())
    }

    pub fn add_purify_rule(&mut self, rule: &str) -> Result<Value, String> {
        let text = rule.trim();
        let mut settings = self.get_settings();
        if text.is_empty() {
            return Ok(settings);
        }
        let Some(arr) = settings.get_mut("purifyRules").and_then(|v| v.as_array_mut()) else {
            return Ok(settings);
        };
        if !arr.iter().any(|r| r.as_str() == Some(text)) {
            arr.push(json!(text));
            write_setting_json(&self.conn, "reader", &settings)?;
        }
        Ok(self.get_settings())
    }

    pub fn remove_purify_rule(&mut self, rule: &str) -> Result<Value, String> {
        let mut settings = self.get_settings();
        if let Some(arr) = settings.get_mut("purifyRules").and_then(|v| v.as_array_mut()) {
            arr.retain(|r| r.as_str() != Some(rule));
        }
        write_setting_json(&self.conn, "reader", &settings)?;
        Ok(self.get_settings())
    }

    // ---------- 偏好 ----------

    pub fn get_prefs(&mut self) -> Value {
        let default = default_prefs();
        let Some(raw) = read_setting_json(&self.conn, "app") else {
            return default;
        };
        let g = |key: &str| raw.get(key).cloned();
        let str_or = |key: &str, fallback: &str| -> String {
            g(key)
                .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| fallback.to_string())
        };
        let bool_or = |key: &str, fallback: bool| -> bool {
            g(key).and_then(|v| v.as_bool()).unwrap_or(fallback)
        };
        let last_view = {
            let v = str_or("lastView", "shelf");
            if matches!(
                v.as_str(),
                "search" | "rank" | "sources" | "settings" | "about" | "shelf"
            ) {
                v
            } else {
                "shelf".into()
            }
        };
        let preload = g("preloadCount")
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
            .map(|v| v.round().clamp(0.0, 20.0))
            .unwrap_or(3.0);
        let bounds = g("windowBounds").unwrap_or(json!({}));
        let width = bounds
            .get("width")
            .and_then(|v| v.as_f64())
            .filter(|v| *v >= 860.0)
            .unwrap_or(1100.0);
        let height = bounds
            .get("height")
            .and_then(|v| v.as_f64())
            .filter(|v| *v >= 560.0)
            .unwrap_or(740.0);
        let mut window_bounds = json!({ "width": width, "height": height });
        if let (Some(dst), Some(x), Some(y)) = (
            window_bounds.as_object_mut(),
            bounds.get("x").and_then(|v| v.as_f64()),
            bounds.get("y").and_then(|v| v.as_f64()),
        ) {
            dst.insert("x".into(), json!(x));
            dst.insert("y".into(), json!(y));
        }
        window_bounds["isMaximized"] = json!(bounds.get("isMaximized").and_then(|v| v.as_bool()).unwrap_or(false));
        json!({
            "sourceTestKeyword": str_or("sourceTestKeyword", "剑来"),
            "lastView": last_view,
            "windowBounds": window_bounds,
            "bossKeyEnabled": bool_or("bossKeyEnabled", true),
            "bossKey": str_or("bossKey", "CommandOrControl+Shift+H"),
            "preloadCount": preload,
            "requestConcurrency": clamp_request_concurrency(g("requestConcurrency").and_then(|v| v.as_f64())),
            "shelfSort": if str_or("shelfSort", "lastRead") == "added" { "added" } else { "lastRead" },
            "eyeCareEnabled": bool_or("eyeCareEnabled", true),
            "eyeCareIntervalMinutes": clamp_eye_care_minutes(
                g("eyeCareIntervalMinutes").and_then(|v| v.as_f64())
                    .or(Some(120.0))
            )
        })
    }

    pub fn save_prefs(&mut self, prefs: &Value) -> Result<Value, String> {
        let merged = {
            let current = self.get_prefs();
            let mut m = current.as_object().cloned().unwrap_or_default();
            if let Some(obj) = prefs.as_object() {
                for (k, v) in obj {
                    m.insert(k.clone(), v.clone());
                }
            }
            Value::Object(m)
        };
        let normalized = {
            let g = |key: &str| merged.get(key).cloned();
            let str_or = |key: &str, fallback: &str| -> String {
                g(key)
                    .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| fallback.to_string())
            };
            let default = default_prefs();
            let bounds = g("windowBounds")
                .filter(|v| v.is_object())
                .unwrap_or_else(|| default["windowBounds"].clone());
            json!({
                "sourceTestKeyword": str_or("sourceTestKeyword", "剑来"),
                "lastView": str_or("lastView", "shelf"),
                "windowBounds": bounds,
                "bossKeyEnabled": g("bossKeyEnabled").and_then(|v| v.as_bool()).unwrap_or(true),
                "bossKey": str_or("bossKey", "CommandOrControl+Shift+H"),
                "preloadCount": g("preloadCount")
                    .and_then(|v| v.as_f64())
                    .map(|v| v.round().clamp(0.0, 20.0))
                    .unwrap_or(3.0),
                "requestConcurrency": clamp_request_concurrency(g("requestConcurrency").and_then(|v| v.as_f64())),
                "shelfSort": if str_or("shelfSort", "lastRead") == "added" { "added" } else { "lastRead" },
                "eyeCareEnabled": g("eyeCareEnabled").and_then(|v| v.as_bool()).unwrap_or(true),
                "eyeCareIntervalMinutes": clamp_eye_care_minutes(g("eyeCareIntervalMinutes").and_then(|v| v.as_f64()))
            })
        };
        // 窗口尺寸/位置由 window-state 插件持久化，偏好节流：高频 lastReadAt 等不重复落盘
        write_setting_json(&self.conn, "app", &normalized)?;
        self.last_prefs_write = Some(Instant::now());
        Ok(normalized)
    }

    // ---------- 搜索历史 ----------

    pub fn get_search_history(&self) -> Vec<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT keyword FROM search_history ORDER BY searched_at DESC, ROWID DESC LIMIT 20")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .map(|rows| rows.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn add_search_history(&self, keyword: &str) -> Vec<String> {
        let key = keyword.trim();
        if !key.is_empty() {
            self.conn
                .execute(
                    "INSERT INTO search_history (keyword, searched_at) VALUES (?1, ?2)
                     ON CONFLICT(keyword) DO UPDATE SET searched_at = excluded.searched_at",
                    params![key, now_ms()],
                )
                .ok();
            self.conn
                .execute(
                    "DELETE FROM search_history WHERE keyword NOT IN (
                       SELECT keyword FROM search_history ORDER BY searched_at DESC, ROWID DESC LIMIT 20
                     )",
                    [],
                )
                .ok();
        }
        self.get_search_history()
    }

    pub fn clear_search_history(&self) -> Vec<String> {
        self.conn.execute("DELETE FROM search_history", []).ok();
        vec![]
    }

    pub fn remove_search_history(&self, keyword: &str) -> Vec<String> {
        let key = keyword.trim();
        if !key.is_empty() {
            self.conn
                .execute("DELETE FROM search_history WHERE keyword = ?1", [key])
                .ok();
        }
        self.get_search_history()
    }

    /// 显式导入旧版 FlyNovel 数据（设置页按钮）：ATTACH 旧库后按主键覆盖合并。
    /// 旧库被 FlyNovel 占用导致读取失败时返回可读错误，不改动本库已有数据。
    pub fn import_legacy(&mut self, src: &std::path::Path) -> Result<Value, String> {
        self.conn
            .execute("ATTACH DATABASE ?1 AS legacy", [src.to_string_lossy().as_ref()])
            .map_err(|e| format!("旧版数据库暂时无法读取（请先退出旧版 FlyNovel 再试）：{e}"))?;
        let merged = merge_legacy_tables(&self.conn);
        self.conn.execute("DETACH DATABASE legacy", []).ok();
        let merged = merged?;
        self.sources_cache = None;
        self.shelf_cache = None;
        self.cover_cache.clear();
        Ok(merged)
    }
}

fn copy_legacy_rows(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.execute(sql, []).map(|_| conn.changes() as i64).map_err(|e| format!("{e}：{sql}"))
}

/// 逐表把 legacy 库的行合并进 main（schema 相同；章节缓存只补缺不覆盖）。
fn merge_legacy_tables(conn: &Connection) -> Result<Value, String> {
    let tables: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM legacy.sqlite_master WHERE type = 'table'")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|x| x.ok())
            .collect();
        rows
    };
    let has = |t: &str| tables.iter().any(|n| n == t);
    let mut out = serde_json::Map::new();
    let mut merge = |key: &str, present: bool, sql: &str| -> Result<(), String> {
        let n = if present { copy_legacy_rows(conn, sql)? } else { 0 };
        out.insert(key.to_string(), json!(n));
        Ok(())
    };
    merge("settings", has("settings"), "INSERT OR REPLACE INTO main.settings (key, value) SELECT key, value FROM legacy.settings")?;
    merge("sources", has("sources"), "INSERT OR REPLACE INTO main.sources (book_source_url, data, sort_order) SELECT book_source_url, data, sort_order FROM legacy.sources")?;
    merge("shelf", has("shelf_books"), "INSERT OR REPLACE INTO main.shelf_books (id, data, sort_order, updated_at) SELECT id, data, sort_order, updated_at FROM legacy.shelf_books")?;
    merge("history", has("search_history"), "INSERT OR REPLACE INTO main.search_history (keyword, searched_at) SELECT keyword, searched_at FROM legacy.search_history")?;
    merge("covers", has("book_covers"), "INSERT OR REPLACE INTO main.book_covers (book_id, mime, data) SELECT book_id, mime, data FROM legacy.book_covers")?;
    merge("chapters", has("chapters"), "INSERT OR IGNORE INTO main.chapters (book_id, chapter_url, chapter_index, title, content, cached_at) SELECT book_id, chapter_url, chapter_index, title, content, cached_at FROM legacy.chapters")?;
    Ok(Value::Object(out))
}

/// loadShelfCache 对齐：库里不应有 cache 字段，仅剥离（不做封面重持久化，避免读路径写库）。
fn strip_cache_no_persist(book: &Value) -> Value {
    let mut v = book.clone();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("cache");
    }
    v
}

fn attach_cover_cached(
    conn: &Connection,
    cover_cache: &mut HashMap<String, String>,
    book: Value,
) -> Value {
    let sentinel = book
        .get("coverUrl")
        .and_then(|c| c.as_str())
        .map(|s| s.starts_with(COVER_PREFIX))
        .unwrap_or(false);
    if !sentinel {
        return book;
    }
    let id = book.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let cached = cover_cache.get(&id).cloned().or_else(|| {
        let d = get_cover_data_url(conn, &id)?;
        cover_cache.insert(id.clone(), d.clone());
        Some(d)
    });
    let mut b = book;
    match cached {
        Some(data_url) => {
            if let Some(obj) = b.as_object_mut() {
                obj.insert("coverUrl".into(), json!(data_url));
            }
        }
        None => {
            if let Some(obj) = b.as_object_mut() {
                obj.remove("coverUrl");
            }
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "flynovel-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open_store(tag: &str) -> Store {
        Store::open(&temp_dir(tag)).unwrap()
    }

    #[test]
    fn defaults_created_on_first_open() {
        let mut s = open_store("defaults");
        let prefs = s.get_prefs();
        assert_eq!(prefs["requestConcurrency"], json!(8));
        assert_eq!(prefs["shelfSort"], json!("lastRead"));
        let settings = s.get_settings();
        assert_eq!(settings["theme"], json!("paper"));
        assert!(settings["fontFamily"].as_str().unwrap().contains("PingFang"));
    }

    #[test]
    fn prefs_normalized_on_save() {
        let mut s = open_store("prefs");
        let saved = s
            .save_prefs(&json!({
                "requestConcurrency": 99.0,
                "eyeCareIntervalMinutes": 45.0,
                "preloadCount": -3.0,
                "bossKey": "  ",
                "shelfSort": "bogus"
            }))
            .unwrap();
        assert_eq!(saved["requestConcurrency"].as_f64(), Some(16.0));
        assert_eq!(saved["eyeCareIntervalMinutes"].as_f64(), Some(30.0));
        assert_eq!(saved["preloadCount"].as_f64(), Some(0.0));
        assert_eq!(saved["bossKey"], json!("CommandOrControl+Shift+H"));
        assert_eq!(saved["shelfSort"], json!("lastRead"));
    }

    #[test]
    fn sources_roundtrip_and_patch() {
        let mut s = open_store("sources");
        let src = json!({ "bookSourceUrl": "https://a.com", "bookSourceName": "A", "enabled": true });
        s.save_sources(&[src]).unwrap();
        assert_eq!(s.get_sources().len(), 1);

        s.patch_source_field("https://a.com", &json!({ "enabled": false, "flyTestStatus": Value::Null }))
            .unwrap();
        let list = s.get_sources();
        assert_eq!(list[0]["enabled"], json!(false));
        assert!(list[0].get("flyTestStatus").is_none());

        let removed = s.remove_sources(&["https://a.com".into()]).unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn shelf_upsert_progress_and_cover_sentinel() {
        let mut s = open_store("shelf");
        let book = json!({
            "id": "b1", "name": "测试", "author": "甲", "origin": "https://a.com",
            "originName": "A", "bookUrl": "https://a.com/1", "chapterIndex": 0,
            "scrollTop": 0, "addedAt": 1, "updatedAt": 1,
            "coverUrl": "data:image/png;base64,aGVsbG8="
        });
        let shelf = s.upsert_shelf_book(&book).unwrap();
        assert_eq!(shelf.len(), 1);
        // 封面已抽离为哨兵，读回时还原 data URL
        assert_eq!(shelf[0]["coverUrl"].as_str().unwrap().starts_with("data:image/png"), true);

        let patched = s
            .patch_shelf_progress("b1", Some(123.7), Some(4.0), Some(999))
            .unwrap()
            .unwrap();
        assert_eq!(patched["scrollTop"].as_f64(), Some(124.0));
        assert_eq!(patched["chapterIndex"].as_f64(), Some(4.0));
        assert_eq!(patched["lastReadAt"], json!(999));

        let after_remove = s.remove_shelf_book("b1").unwrap();
        assert!(after_remove.is_empty());
    }

    fn shelf_ids(s: &mut Store) -> Vec<String> {
        s.get_shelf()
            .iter()
            .map(|b| b["id"].as_str().unwrap_or("").to_string())
            .collect()
    }

    #[test]
    fn save_shelf_rewrites_order_and_drops_missing_rows() {
        let dir = temp_dir("save_shelf");
        let mut s = Store::open(&dir).unwrap();
        let mk = |id: &str| {
            json!({
                "id": id, "name": id, "origin": "https://a.com",
                "bookUrl": format!("https://a.com/{id}"), "chapterIndex": 0,
                "addedAt": 1, "updatedAt": 1
            })
        };
        s.upsert_shelf_book(&mk("a")).unwrap();
        s.upsert_shelf_book(&mk("b")).unwrap();
        s.upsert_shelf_book(&mk("c")).unwrap();
        // 后入架的排在前（sort_order = MIN-1）
        assert_eq!(shelf_ids(&mut s), vec!["c", "b", "a"]);
        // 全架更新按给定列表重写顺序，并删除未出现的条目
        s.save_shelf(&[mk("b"), mk("a")]).unwrap();
        assert_eq!(shelf_ids(&mut s), vec!["b", "a"]);
        let mut reopened = Store::open(&dir).unwrap();
        assert_eq!(shelf_ids(&mut reopened), vec!["b", "a"], "排序与删除要真正落库");
    }

    #[test]
    fn search_history_capped_and_removed() {
        let s = open_store("history");
        for i in 0..25 {
            s.add_search_history(&format!("kw{i}"));
        }
        let h = s.get_search_history();
        assert_eq!(h.len(), 20);
        assert_eq!(h[0], "kw24");
        s.remove_search_history("kw24");
        assert_eq!(s.get_search_history().len(), 19);
        s.clear_search_history();
        assert!(s.get_search_history().is_empty());
    }

    #[test]
    fn legacy_db_inherited_without_touching_source() {
        // 模拟：app_data 的父目录存在 fly-novel/fly-novel.sqlite（Electron 版数据）
        let base = temp_dir("legacy-base");
        let legacy_dir = base.join("fly-novel");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let legacy_file = legacy_dir.join("fly-novel.sqlite");
        {
            let conn = Connection::open(&legacy_file).unwrap();
            conn.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE sources (book_source_url TEXT PRIMARY KEY, data TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO settings VALUES ('app', '{\"bossKey\":\"Ctrl+H\"}');",
            )
            .unwrap();
        }
        let app_data = base.join("com.flyreader.desktop");
        let mut s = Store::open(&app_data).unwrap();
        let prefs = s.get_prefs();
        assert_eq!(prefs["bossKey"], json!("Ctrl+H"));
        // 关键：旧库必须原样保留，Electron 版仍能正常启动
        assert!(legacy_file.is_file());
        assert!(!legacy_dir.join("fly-novel.sqlite.legacy-backup").exists());
    }

    #[test]
    fn import_legacy_merges_rows() {
        // 旧库与新建目录分处不同父目录，避免首屏自动继承抢先导入
        let legacy_file = temp_dir("import-legacy")
            .join("fly-novel")
            .join("fly-novel.sqlite");
        std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
        {
            let conn = Connection::open(&legacy_file).unwrap();
            conn.execute_batch(
                "CREATE TABLE sources (book_source_url TEXT PRIMARY KEY, data TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE shelf_books (id TEXT PRIMARY KEY, data TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO sources VALUES ('https://old.com', '{\"bookSourceUrl\":\"https://old.com\",\"bookSourceName\":\"旧源\"}', 0);
                 INSERT INTO shelf_books VALUES ('b9', '{\"id\":\"b9\",\"name\":\"旧书\",\"origin\":\"https://old.com\"}', 0, 123);",
            )
            .unwrap();
        }
        let mut s = open_store("import-app");
        assert!(s.get_shelf().is_empty());

        let merged = s.import_legacy(&legacy_file).unwrap();
        assert_eq!(merged["sources"].as_i64(), Some(1));
        assert_eq!(merged["shelf"].as_i64(), Some(1));
        assert_eq!(merged.get("covers").and_then(|v| v.as_i64()), Some(0));
        let shelf = s.get_shelf();
        assert_eq!(shelf.len(), 1);
        assert_eq!(shelf[0]["name"], json!("旧书"));
        assert_eq!(s.get_sources().len(), 1);
        assert!(legacy_file.is_file());
    }

    #[test]
    fn chapter_cache_roundtrip_and_cross_book_fallback() {
        let mut s = open_store("chapters");
        s.set_chapter_content("b1", "https://x/1", "正文一", Some(0), Some("第一章"));
        // 空正文不入库，否则缓存里会留下「已缓存」的空章节
        s.set_chapter_content("b1", "https://x/2", "", None, None);
        assert_eq!(
            s.get_chapter_content(Some("b1"), "https://x/1").as_deref(),
            Some("正文一")
        );
        assert_eq!(
            s.get_chapter_content(Some("b2"), "https://x/1").as_deref(),
            Some("正文一"),
            "换源后同一章节 URL 应能按 URL 兜底命中"
        );
        assert!(s.get_chapter_content(Some("b1"), "https://x/2").is_none());
        assert_eq!(s.cached_chapter_urls("b1"), vec!["https://x/1".to_string()]);

        s.set_chapter_content("b1", "https://x/1", "正文一改", None, None);
        assert_eq!(
            s.get_chapter_content(Some("b1"), "https://x/1").as_deref(),
            Some("正文一改"),
            "重复写入应覆写而不是追加"
        );
    }

    #[test]
    fn replace_chapters_overwrites_whole_book() {
        let mut s = open_store("replace-chapters");
        s.set_chapter_content("b1", "local://b1/0", "旧内容", Some(0), Some("旧标题"));
        s.replace_chapters(
            "b1",
            &[
                ChapterRow { url: "local://b1/0", index: 0, title: "第一章", content: "新内容" },
                ChapterRow { url: "local://b1/1", index: 1, title: "第二章", content: "（空）" },
            ],
        )
        .unwrap();
        assert_eq!(
            s.get_chapter_content(Some("b1"), "local://b1/0").as_deref(),
            Some("新内容")
        );
        assert_eq!(s.cached_chapter_urls("b1"), vec!["local://b1/0".to_string(), "local://b1/1".to_string()]);

        // 再次导入更少的章节时，旧书残留的正文要一起清掉
        s.replace_chapters("b1", &[ChapterRow { url: "local://b1/0", index: 0, title: "第一章", content: "只有它" }])
            .unwrap();
        assert_eq!(s.cached_chapter_urls("b1"), vec!["local://b1/0".to_string()]);
    }

    #[test]
    fn book_cache_info_tracks_progress() {
        let mut s = open_store("cache-info");
        let chapters = json!([
            { "url": "u1", "title": "一" },
            { "url": "u2", "title": "二" },
            { "url": "", "title": "第一卷" }
        ]);
        assert_eq!(
            s.book_cache_info("b1", Some(&chapters), false),
            json!({ "bookId": "b1", "total": 2, "cached": 0, "status": "none" }),
            "空 URL 的卷标题条目不计入总章数"
        );
        s.set_chapter_content("b1", "u1", "一", None, None);
        assert_eq!(s.book_cache_info("b1", Some(&chapters), false)["status"], json!("partial"));
        s.set_chapter_content("b1", "u2", "二", None, None);
        assert_eq!(s.book_cache_info("b1", Some(&chapters), false)["status"], json!("full"));
        assert_eq!(s.book_cache_info("b1", Some(&chapters), true)["status"], json!("caching"));
        assert_eq!(s.count_cached_chapters("b2", &["u1".to_string(), "u2".to_string()]), 0);
        assert_eq!(s.clear_book_cache("b1"), 2);
        assert_eq!(s.book_cache_info("b1", Some(&chapters), false)["status"], json!("none"));
    }

    #[test]
    fn enrich_shelf_marks_local_books_and_cache_state() {
        let mut s = open_store("enrich");
        s.upsert_shelf_book(&json!({
            "id": "online", "name": "网文", "origin": "https://src",
            "chapters": [{ "url": "c1" }, { "url": "c2" }]
        }))
        .unwrap();
        s.upsert_shelf_book(&json!({
            "id": "mine", "name": "本地书", "origin": "local", "isLocal": true,
            "chapters": [{ "url": "file://a#1" }]
        }))
        .unwrap();
        s.set_chapter_content("online", "c1", "一", None, None);

        let enriched = {
            let list = s.get_shelf();
            s.enrich_shelf(&list, &["online".to_string()])
        };
        let online = enriched
            .iter()
            .find(|b| b["id"] == json!("online"))
            .expect("online book");
        assert_eq!(online["cache"]["total"], json!(2));
        assert_eq!(online["cache"]["status"], json!("caching"));
        let local = enriched
            .iter()
            .find(|b| b["id"] == json!("mine"))
            .expect("local book");
        assert_eq!(local["cache"]["status"], json!("full"), "本地书章节就在库里，视为已全量缓存");
        assert_eq!(local["isLocal"], json!(true));
        assert_eq!(local["originName"], json!("本地"));
    }

    #[test]
    fn markdown_local_books_are_flagged_without_parsing_content() {
        let mut s = open_store("markdown-flag");
        s.upsert_shelf_book(&json!({ "id": "md", "name": "MD 书", "origin": "local", "isLocal": true, "localFormat": "md" }))
            .unwrap();
        s.upsert_shelf_book(&json!({ "id": "txt", "name": "TXT 书", "origin": "local", "isLocal": true, "localFormat": "txt" }))
            .unwrap();
        assert!(s.is_markdown_local("md"));
        assert!(!s.is_markdown_local("txt"));
        assert!(!s.is_markdown_local("missing"));
    }

    #[test]
    fn import_sources_appends_only_new_urls() {
        let dir = temp_dir("import-sources");
        let mut s = Store::open(&dir).unwrap();
        s.save_sources(&[json!({ "bookSourceUrl": "https://a", "bookSourceName": "A" })])
            .unwrap();
        let incoming = vec![
            json!({ "bookSourceUrl": "https://a", "bookSourceName": "A" }),
            json!({ "bookSourceUrl": "https://b", "bookSourceName": "B" }),
            json!({ "bookSourceUrl": "https://b", "bookSourceName": "B 重复" }),
            json!({ "bookSourceName": "缺少 URL" }),
        ];
        assert_eq!(s.import_sources(&incoming).unwrap(), (1, 2));
        let urls: Vec<String> = s
            .get_sources()
            .iter()
            .map(|x| x["bookSourceUrl"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(urls, vec!["https://a".to_string(), "https://b".to_string()]);
        assert_eq!(s.import_sources(&incoming).unwrap(), (0, 3), "重复导入不应写入");
        drop(s);
        let mut reopened = Store::open(&dir).unwrap();
        assert_eq!(reopened.get_sources().len(), 2, "新增行要真正落库");
    }

    #[test]
    fn replace_source_moves_edited_book_and_drops_url_conflicts() {
        let mut s = open_store("replace-source");
        s.save_sources(&[
            json!({ "bookSourceUrl": "https://old", "bookSourceName": "旧名" }),
            json!({ "bookSourceUrl": "https://keep", "bookSourceName": "另一个" }),
        ])
        .unwrap();
        let next = s
            .replace_source(
                "https://old",
                &json!({ "bookSourceUrl": "https://new", "bookSourceName": "新名" }),
            )
            .unwrap();
        assert_eq!(next.len(), 2);
        assert_eq!(next[1]["bookSourceUrl"], json!("https://new"), "替换项追加到列表末尾");
        assert!(next.iter().all(|x| x["bookSourceUrl"] != json!("https://old")));

        let dup = s
            .replace_source(
                "https://new",
                &json!({ "bookSourceUrl": "https://keep", "bookSourceName": "覆盖" }),
            )
            .unwrap();
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0]["bookSourceName"], json!("覆盖"), "URL 撞车时保留编辑后的版本");
    }
}
