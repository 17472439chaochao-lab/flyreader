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
/// 注意：Markdown 章节的 `content` 只存该章对应的行切片（不再每章 clone 整份全文），
/// `md_anchor` 仍为全文标题序号；前端渲染时按章节内容内标题顺序重编锚点，
/// id 对不上时会按标题文本回退定位（见 ReaderView.findMarkdownScrollTarget）。
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

/// 单本本地书的章节总数上限：超限说明文件异常或切章失控，直接拒绝导入。
pub const MAX_CHAPTERS: usize = 10_000;
/// 单本本地书的章节正文累计上限（字节）：超限直接拒绝导入。
pub const MAX_TOTAL_CHAPTER_BYTES: usize = 200 * 1024 * 1024;

/// 取不到标题时的回退词（原版 `fallbackTitle || '全文'`）。
pub fn fallback_or<'a>(fallback: &'a str, word: &'a str) -> &'a str {
    if fallback.is_empty() {
        word
    } else {
        fallback
    }
}
