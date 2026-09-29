//! 本地书籍 ID 与章节伪 URL 的生成器（对应 main/localBooks/bookIds.ts）。
//!
//! ID 必须是 `sha1(文件绝对路径)` 的前 20 位十六进制：旧版 Electron 的书架与章节缓存
//! 用的就是这个口径，改动会让继承来的本地书正文对不上号。

use sha1_smol::Sha1;

/// 由文件路径生成稳定的本地书籍 ID。
pub fn local_book_id(file_path: &str) -> String {
    let hex = Sha1::from(file_path).digest().to_string();
    format!("local:{}", &hex.chars().take(20).collect::<String>())
}

/// 生成本地章节伪 URL（阅读器按它去 chapters 表取正文）。
pub fn chapter_url(book_id: &str, index: usize) -> String {
    format!("local://{book_id}/{index}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_id_is_stable_sha1_prefix() {
        // Node: createHash('sha1').update('/tmp/示例.txt').digest('hex').slice(0,20)
        let id = local_book_id("/tmp/x.txt");
        assert!(id.starts_with("local:"));
        assert_eq!(id.chars().count(), 6 + 20);
        assert_eq!(id, local_book_id("/tmp/x.txt"));
        assert_ne!(id, local_book_id("/tmp/y.txt"));
        assert_eq!(chapter_url(&id, 7), format!("local://{id}/7"));
    }
}
