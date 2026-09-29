//! 本地电子书导入：TXT / Markdown / EPUB 的读取、解码、切章与写库数据。
//!
//! 原版把整条流水线放进 `node:worker_threads`（readFileSync、iconv 解码、JSZip 解压、
//! 逐章写库都在 worker 里），以免几十 MB 的文件冻结主进程。这里保持同样的意图但换了分工：
//! 解析（纯计算，不碰 SQLite）跑在 `spawn_blocking`，章节写库回到 Store 的单连接事务里——
//! `Connection` 不是 `Send`，不能带进后台线程。
//!
//! 三处刻意偏离原版（本项目决定「只在新版修好」）：
//! - 编码候选走 `encoding_rs`（WHATWG 标签表），其中 `big5` 标签实际是 windows-1252、
//!   `gb2312` 等同 GBK，而不是 iconv-lite 的真 Big5；真 Big5 文本通常会被 UTF-8/GB18030
//!   候选按打分胜出，且少掉一个带巨型索引表的依赖。
//! - Rust 正则没有后向引用，Markdown 的 `**x**` / `*x*` 强调标记改为按强弱各做一次成对替换。
//! - 原版用 cheerio 取 `body` 文本，但 EPUB 的 XHTML 常把 `<title>` 放在解析结果里一起被读到，
//!   导致章节正文开头混入书名；这里显式跳过 `head/title/meta/link`，正文只保留可见内容。

pub mod book_ids;
pub mod epub_parse;
pub mod importer;
pub mod md_parse;
pub mod txt_split;

/// 切章结果。`md_anchor` 只在 Markdown 全文模式下出现（阅读器据此滚动定位）。
#[derive(Clone, Debug)]
pub struct SplitChapter {
    pub title: String,
    pub content: String,
    pub md_anchor: Option<String>,
}

impl SplitChapter {
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        SplitChapter {
            title: title.into(),
            content: content.into(),
            md_anchor: None,
        }
    }

    pub fn with_anchor(
        title: impl Into<String>,
        content: impl Into<String>,
        md_anchor: Option<String>,
    ) -> Self {
        SplitChapter {
            title: title.into(),
            content: content.into(),
            md_anchor,
        }
    }
}

/// 取不到标题时的回退词（原版 `fallbackTitle || '全文'`）。
pub fn fallback_or<'a>(fallback: &'a str, word: &'a str) -> &'a str {
    if fallback.is_empty() {
        word
    } else {
        fallback
    }
}
