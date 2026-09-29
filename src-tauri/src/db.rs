//! SQLite 建库、schema 与历史数据迁移。
//! 对应原 Electron 版 src/main/db.ts（electron-store 迁移已废弃，仅保留 SQLite 继承路径）。

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS meta (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS settings (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS sources (
      book_source_url TEXT PRIMARY KEY,
      data TEXT NOT NULL,
      sort_order INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS shelf_books (
      id TEXT PRIMARY KEY,
      data TEXT NOT NULL,
      sort_order INTEGER NOT NULL DEFAULT 0,
      updated_at INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS search_history (
      keyword TEXT PRIMARY KEY,
      searched_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS book_covers (
      book_id TEXT PRIMARY KEY,
      mime TEXT NOT NULL,
      data BLOB NOT NULL
    );

    CREATE TABLE IF NOT EXISTS chapters (
      book_id TEXT NOT NULL,
      chapter_url TEXT NOT NULL,
      chapter_index INTEGER,
      title TEXT,
      content TEXT NOT NULL,
      cached_at INTEGER NOT NULL,
      PRIMARY KEY (book_id, chapter_url)
    );

    CREATE INDEX IF NOT EXISTS idx_chapters_url ON chapters(chapter_url);
    CREATE INDEX IF NOT EXISTS idx_chapters_book ON chapters(book_id);
    CREATE INDEX IF NOT EXISTS idx_shelf_sort ON shelf_books(sort_order, updated_at DESC);
    CREATE INDEX IF NOT EXISTS idx_sources_sort ON sources(sort_order);
    CREATE INDEX IF NOT EXISTS idx_history_time ON search_history(searched_at DESC);
"#;

/// 历史数据候选目录：Electron 版 FlyNovel 用 `fly-novel`，早期 Tauri 骨架用 `com.flynovel.app`
fn legacy_data_dirs(app_data_dir: &Path) -> Vec<PathBuf> {
    let Some(parent) = app_data_dir.parent() else {
        return vec![];
    };
    ["fly-novel", "com.flynovel.app"]
        .iter()
        .map(|n| parent.join(n))
        .filter(|p| p.is_dir() && *p != app_data_dir)
        .collect()
}

/// 旧版数据库文件路径（用于首屏自动继承与设置页显式导入）。
pub fn legacy_db_path(app_data_dir: &Path) -> Option<PathBuf> {
    legacy_data_dirs(app_data_dir)
        .into_iter()
        .map(|d| d.join("fly-novel.sqlite"))
        .find(|p| p.is_file())
}

/// 用 SQLite online backup API 生成一致性快照：不改动旧库内容（仅在需要时恢复其 WAL），
/// 旧版正在写入而拿不到共享锁时返回 SQLITE_BUSY，由调用方提示用户先退出旧版。
pub fn snapshot_db(src: &Path, dst: &Path) -> Result<(), String> {
    let legacy = Connection::open(src).map_err(|e| format!("无法打开旧版数据库：{e}"))?;
    legacy
        .pragma_update(None, "busy_timeout", 3000)
        .map_err(|e| e.to_string())?;
    legacy
        .backup(rusqlite::DatabaseName::Main, dst, None)
        .map_err(|e| format!("读取旧版数据库失败（可能正被 FlyNovel 占用）：{e}"))
}

/// 首次启动：新库不存在且能找到旧版库时，快照继承（schema 相同）。旧库保持原样。
fn inherit_legacy_db(new_db: &Path) -> Option<PathBuf> {
    if new_db.exists() {
        return None;
    }
    let app_data_dir = new_db.parent()?.to_path_buf();
    let src = legacy_db_path(&app_data_dir)?;
    match snapshot_db(&src, new_db) {
        Ok(()) => Some(src),
        Err(e) => {
            eprintln!("[fly-reader] 自动继承旧版数据失败：{e}");
            // 备份中途失败可能已创建目标文件，删除以保证从空库启动
            if new_db.exists() {
                let _ = fs::remove_file(new_db);
            }
            None
        }
    }
}

/// 打开（并按需创建）统一 SQLite 连接。
pub fn open_db(app_data_dir: &Path) -> Result<Connection, String> {
    fs::create_dir_all(app_data_dir).map_err(|e| e.to_string())?;
    let db_path = app_data_dir.join("fly-reader.sqlite");
    let inherited_from = inherit_legacy_db(&db_path);

    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "busy_timeout", 5000).map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    if let Some(src) = inherited_from {
        set_meta(&conn, "legacy_inherited_from", &src.to_string_lossy());
        set_meta(&conn, "legacy_inherited_at", &now_ms().to_string());
    }
    migrate_legacy_chapters(&conn, app_data_dir);
    Ok(conn)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_millis() as i64
}

fn get_meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?", [key], |r| r.get(0))
        .ok()
}

fn set_meta(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .ok();
}

/// 旧版独立 chapters.sqlite 的行复制进统一库（一次性，幂等）。
fn migrate_legacy_chapters(conn: &Connection, app_data_dir: &Path) {
    if get_meta(conn, "migrated_chapters_sqlite").as_deref() == Some("1") {
        return;
    }
    // 自有目录内的文件可改名归档；旧版目录内的文件保持原样（只标记完成）
    let own_path = app_data_dir.join("chapters.sqlite");
    let (legacy_path, archive_after) = if own_path.is_file() {
        (own_path, true)
    } else {
        match legacy_data_dirs(app_data_dir)
            .into_iter()
            .map(|d| d.join("chapters.sqlite"))
            .find(|p| p.is_file())
        {
            Some(p) => (p, false),
            None => {
                set_meta(conn, "migrated_chapters_sqlite", "1");
                return;
            }
        }
    };
    let result = (|| -> Result<(), String> {
        let legacy = Connection::open_with_flags(
            &legacy_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        let mut stmt = legacy
            .prepare(
                "SELECT book_id, chapter_url, chapter_index, title, content, cached_at FROM chapters",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|x| x.ok())
            .collect::<Vec<_>>();
        drop(stmt);
        drop(legacy);

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (book_id, url, idx, title, content, cached_at) in rows {
            tx.execute(
                "INSERT OR IGNORE INTO chapters (book_id, chapter_url, chapter_index, title, content, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![book_id, url, idx, title, content, cached_at],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('migrated_chapters_sqlite', '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_ok() && archive_after {
        let _ = fs::rename(&legacy_path, app_data_dir.join("chapters.sqlite.migrated.bak"));
    }
    set_meta(conn, "migrated_chapters_sqlite", "1");
}
