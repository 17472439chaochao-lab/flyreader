//! 本地文件 → 书架书籍 JSON + 章节缓存行（对应 main/localBooks/importWorker.ts 的解析段）。
//!
//! 这里是纯计算：读文件、解码、切章、组装 JSON，不碰数据库，因此可以整体丢进
//! `spawn_blocking`。写库（章节事务 + 书架 upsert）留在命令侧的 Store 锁内完成。

use serde_json::{json, Value};

use super::book_ids::{chapter_url, local_book_id};
use super::epub_parse::parse_epub_buffer;
use super::md_parse::parse_md_buffer;
use super::txt_split::{decode_txt_buffer, split_txt_chapters};
use crate::store::now_ms;
use super::SplitChapter;

/// 写库用的一行章节缓存（与 chapters 表列一一对应）。
#[derive(Debug)]
pub struct ImportRow {
    pub url: String,
    pub index: i64,
    pub title: String,
    pub content: String,
}

/// 解析结果：尚未合并既有阅读进度的书籍 JSON、待写入的章节行、提示文案。
#[derive(Debug)]
pub struct ParsedLocalBook {
    pub book: Value,
    pub rows: Vec<ImportRow>,
    pub message: String,
}

/// 导入文件大小上限：再大说明不是正常电子书，直接拒绝。
pub const MAX_IMPORT_BYTES: u64 = 512 * 1024 * 1024;

/// 解析前的文件预检（对应原版 importLocal.ts 里发起解析之前做的那几步）。
pub fn precheck_file(file_path: &str) -> Result<(), String> {
    let ext = node_extname(file_path).to_lowercase();
    if ext != ".txt" && ext != ".epub" && ext != ".md" && ext != ".markdown" {
        return Err("仅支持 .txt、.epub 与 .md".to_string());
    }
    let meta = std::fs::metadata(file_path).map_err(|e| format!("无法读取文件：{e}"))?;
    if !meta.is_file() {
        return Err("不是有效文件".to_string());
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err("文件超过 512MB，暂不支持导入".to_string());
    }
    Ok(())
}

/// 解析本地 TXT / EPUB / Markdown 文件。错误信息文案与原版 worker 抛出的保持一致。
pub fn parse_local_file(file_path: &str) -> Result<ParsedLocalBook, String> {
    if file_path.is_empty() {
        return Err("缺少导入参数".to_string());
    }
    let raw_ext = node_extname(file_path);
    let ext = raw_ext.to_lowercase();
    let is_md = ext == ".md" || ext == ".markdown";
    if ext != ".txt" && ext != ".epub" && !is_md {
        return Err("仅支持 .txt、.epub 与 .md".to_string());
    }

    let buf = std::fs::read(file_path).map_err(|e| e.to_string())?;
    let file_name = node_basename_without_ext(file_path, &raw_ext);
    let book_id = local_book_id(file_path);
    let now = now_ms();

    let mut name = file_name.clone();
    let mut author = "佚名".to_string();
    let parts: Vec<SplitChapter>;
    let mut cover_url: Option<String> = None;
    let mut kind = "TXT";
    let mut local_format = "txt";
    if ext == ".txt" {
        let text = decode_txt_buffer(&buf);
        parts = split_txt_chapters(&text, &file_name);
    } else if is_md {
        kind = "MD";
        local_format = "md";
        let meta = parse_md_buffer(&buf, &file_name);
        name = pick_title(&meta.title, &file_name);
        parts = meta.chapters;
    } else {
        kind = "EPUB";
        local_format = "epub";
        let meta = parse_epub_buffer(&buf, &file_name)?;
        name = pick_title(&meta.title, &file_name);
        author = pick_title(&meta.author, "佚名");
        cover_url = meta.cover_data_url;
        parts = meta.chapters;
    }

    let chapters: Vec<Value> = parts
        .iter()
        .enumerate()
        .map(|(index, p)| {
            let mut obj = json!({
                "title": p.title,
                "url": chapter_url(&book_id, index),
                "index": index,
            });
            if let Some(anchor) = &p.md_anchor {
                obj["mdAnchor"] = json!(anchor);
            }
            obj
        })
        .collect();

    let rows: Vec<ImportRow> = parts
        .iter()
        .enumerate()
        .map(|(index, p)| ImportRow {
            url: chapter_url(&book_id, index),
            index: index as i64,
            title: p.title.clone(),
            content: if p.content.is_empty() {
                "（空）".to_string()
            } else {
                p.content.clone()
            },
        })
        .collect();

    let mut book = json!({
        "id": book_id,
        "name": name,
        "author": author,
        "bookUrl": file_path,
        "origin": "local",
        "originName": "本地",
        "kind": kind,
        "tocUrl": file_path,
        "chapters": chapters,
        "chapterIndex": 0,
        "scrollTop": 0,
        "addedAt": now,
        "updatedAt": now,
        "lastReadAt": 0,
        "isLocal": true,
        "localPath": file_path,
        "localFormat": local_format,
    });
    // JSON.stringify 会丢掉 undefined 字段，这里同样只在有值时才写入
    if let Some(cover) = cover_url {
        book["coverUrl"] = json!(cover);
    }
    if let Some(last) = chapters.last() {
        book["lastChapter"] = last.get("title").cloned().unwrap_or(Value::Null);
    }

    let message = format!("已导入《{name}》· {} 章（{kind}）", chapters.len());
    Ok(ParsedLocalBook {
        book,
        rows,
        message,
    })
}

fn pick_title(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

/// node 的 `path.basename`。
fn basename(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    match normalized.rfind('/') {
        Some(i) => normalized[i + 1..].to_string(),
        None => normalized,
    }
}

/// node 的 `path.extname`：返回含点的扩展名并保留原始大小写；`.` 开头的隐藏文件视为无扩展名。
fn node_extname(path: &str) -> String {
    let name = basename(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[i..].to_string(),
        _ => String::new(),
    }
}

/// node 的 `path.basename(path, ext)`：大小写不一致时不剥离（原版即如此）。
fn node_basename_without_ext(path: &str, ext: &str) -> String {
    let name = basename(path);
    if !ext.is_empty() && ext.len() <= name.len() && name.ends_with(ext) {
        return name[..name.len() - ext.len()].to_string();
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers_match_node() {
        assert_eq!(basename("/a/b/都市.TXT"), "都市.TXT");
        assert_eq!(node_extname("/a/b/都市.TXT"), ".TXT");
        assert_eq!(node_extname("/a/b/.txt"), "");
        assert_eq!(node_extname("/a/b/无扩展"), "");
        // 原版把 extname 结果小写后传给 basename(path, ext)，大小写不符则不剥离
        assert_eq!(node_basename_without_ext("/a/都市.TXT", ".txt"), "都市.TXT");
        assert_eq!(node_basename_without_ext("/a/都市.txt", ".txt"), "都市");
    }

    #[test]
    fn unsupported_extension_is_rejected_before_reading() {
        assert_eq!(
            parse_local_file("/tmp/a.docx").unwrap_err(),
            "仅支持 .txt、.epub 与 .md"
        );
        assert_eq!(parse_local_file("").unwrap_err(), "缺少导入参数");
    }

    #[test]
    fn precheck_rejects_unsupported_and_missing() {
        assert_eq!(
            precheck_file("/tmp/a.docx").unwrap_err(),
            "仅支持 .txt、.epub 与 .md"
        );
        assert!(precheck_file("/tmp/不存在.txt").unwrap_err().starts_with("无法读取文件："));
        let dir = std::env::temp_dir().join(format!("flyreader-precheck-{}", now_ms()));
        let dir_named_txt = dir.join("目录.txt");
        std::fs::create_dir_all(&dir_named_txt).unwrap();
        // 扩展名合法但是目录（原版 stat.isFile() 分支）
        assert_eq!(precheck_file(dir_named_txt.to_str().unwrap()).unwrap_err(), "不是有效文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn txt_file_becomes_local_book_rows() {
        let dir = std::env::temp_dir().join(format!("flyreader-txt-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.txt");
        std::fs::write(
            &path,
            "第一章 起\n甲。\n第二章 承\n乙。\n第三章 转\n丙。\n",
        )
        .unwrap();
        let parsed = parse_local_file(path.to_str().unwrap()).unwrap();
        assert_eq!(parsed.book["name"], json!("sample"));
        assert_eq!(parsed.book["isLocal"], json!(true));
        assert_eq!(parsed.book["localFormat"], json!("txt"));
        assert_eq!(parsed.rows.len(), 3);
        assert_eq!(parsed.rows[0].url, format!("local://{}/0", parsed.book["id"].as_str().unwrap()));
        assert!(parsed.message.starts_with("已导入《sample》· 3 章（TXT）"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
