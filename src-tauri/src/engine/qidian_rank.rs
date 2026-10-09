//! 起点移动站排行 / 完本榜抓取。
//!
//! 策略顺序：移动站 HTML（SSR 嵌入数据）→ majax JSON API。
//! 均使用移动 UA；majax 需先拿 `_csrfToken` cookie。

use std::time::Duration;

use regex::Regex;
use reqwest::{
    header::{HeaderMap, COOKIE, SET_COOKIE, USER_AGENT},
    Client,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::OnceLock;

const MOBILE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1";
const MOBILE_ORIGIN: &str = "https://m.qidian.com";
const PAGE_SIZE: usize = 20;
const TIMEOUT_MS: u64 = 15_000;

/// 频道：排行榜 / 完本
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankChannel {
    Rank,
    Finish,
}

/// 排行榜子榜（仅 `Rank` 频道）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankBoard {
    Yuepiao,
    Hotsales,
    Recom,
    Collect,
}

/// 男频 / 女频
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankGender {
    Male,
    Female,
}

impl RankChannel {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim() {
            "rank" => Ok(Self::Rank),
            "finish" => Ok(Self::Finish),
            other => Err(format!("未知频道：{other}（期望 rank / finish）")),
        }
    }
}

impl RankBoard {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim() {
            "yuepiao" => Ok(Self::Yuepiao),
            "hotsales" => Ok(Self::Hotsales),
            "recom" => Ok(Self::Recom),
            "collect" => Ok(Self::Collect),
            other => Err(format!(
                "未知榜单：{other}（期望 yuepiao / hotsales / recom / collect）"
            )),
        }
    }

    fn path_slug(self) -> &'static str {
        match self {
            Self::Yuepiao => "yuepiao",
            Self::Hotsales => "hotsales",
            Self::Recom => "rec",
            Self::Collect => "collect",
        }
    }

    /// majax 路径末段（与移动站实际接口对齐）
    fn majax_list_name(self) -> &'static str {
        match self {
            Self::Yuepiao => "yuepiaolist",
            Self::Hotsales => "hotsaleslist",
            Self::Recom => "reclist",
            Self::Collect => "collectlist",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Yuepiao => "月票榜",
            Self::Hotsales => "畅销榜",
            Self::Recom => "推荐榜",
            Self::Collect => "收藏榜",
        }
    }
}

impl RankGender {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim() {
            "male" | "男生" | "男频" => Ok(Self::Male),
            "female" | "女生" | "女频" => Ok(Self::Female),
            other => Err(format!("未知频道性别：{other}（期望 male / female）")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RankBook {
    pub name: String,
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankListResult {
    pub books: Vec<RankBook>,
    pub page: u32,
    pub has_more: bool,
    /// `mobile` = HTML SSR；`mobile_api` = majax JSON
    pub source: &'static str,
    pub board_label: String,
}

fn mobile_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(MOBILE_UA)
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .timeout(Duration::from_millis(TIMEOUT_MS))
            .build()
            .expect("qidian mobile client")
    })
}

fn cover_url_for_bid(bid: &str) -> Option<String> {
    let bid = bid.trim();
    if bid.is_empty() {
        return None;
    }
    Some(format!(
        "https://bookcover.yuewen.com/qdbimg/349573/{bid}/150"
    ))
}

fn text_field(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(Value::as_str) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
        if let Some(n) = v.get(*k).and_then(Value::as_i64) {
            return Some(n.to_string());
        }
        if let Some(n) = v.get(*k).and_then(Value::as_u64) {
            return Some(n.to_string());
        }
    }
    None
}

fn rank_num(v: &Value) -> Option<u32> {
    if let Some(n) = v.get("rankNum").and_then(Value::as_u64) {
        return u32::try_from(n).ok();
    }
    if let Some(n) = v.get("rank").and_then(Value::as_u64) {
        return u32::try_from(n).ok();
    }
    if let Some(s) = v.get("rankNum").and_then(Value::as_str) {
        return s.trim().parse().ok();
    }
    None
}

fn record_to_book(v: &Value, fallback_rank: Option<u32>) -> Option<RankBook> {
    let name = text_field(v, &["bName", "bookName", "name", "title"])?;
    let author = text_field(v, &["bAuth", "authorName", "author", "writer"]).unwrap_or_default();
    let bid = text_field(v, &["bid", "bookId", "id"]);
    let intro = text_field(v, &["desc", "intro", "description"]);
    let category = text_field(v, &["cat", "category", "catName"]);
    let status = text_field(v, &["state", "status", "bookStatus"]);
    let rank = rank_num(v).or(fallback_rank);
    let cover_url = bid.as_deref().and_then(cover_url_for_bid).or_else(|| {
        text_field(v, &["coverUrl", "cover", "picUrl", "img"])
    });
    Some(RankBook {
        name,
        author,
        cover_url,
        intro,
        category,
        status,
        rank,
        bid,
    })
}

fn records_from_data_obj(data: &Value) -> Vec<RankBook> {
    let list = ["records", "list", "bookList", "books", "items"]
        .iter()
        .find_map(|k| data.get(*k).and_then(Value::as_array));
    let Some(arr) = list else {
        return Vec::new();
    };
    arr.iter()
        .enumerate()
        .filter_map(|(i, v)| record_to_book(v, Some((i as u32) + 1)))
        .collect()
}

fn has_more_from_data(data: &Value, books_len: usize, page: u32) -> bool {
    if let Some(b) = data.get("isLast") {
        // 0 / false => 还有下一页；1 / true => 末页
        if let Some(n) = b.as_i64() {
            return n == 0;
        }
        if let Some(flag) = b.as_bool() {
            return !flag;
        }
    }
    if let Some(max) = data.get("pageMax").and_then(Value::as_u64) {
        return u64::from(page) < max;
    }
    if let Some(total) = data.get("total").and_then(Value::as_u64) {
        return u64::from(page) * (PAGE_SIZE as u64) < total;
    }
    books_len >= PAGE_SIZE
}

/// 解析 majax / 同类 JSON 响应体。
pub fn parse_majax_json(text: &str, page: u32) -> Result<(Vec<RankBook>, bool), String> {
    let root: Value =
        serde_json::from_str(text).map_err(|e| format!("起点 JSON 解析失败：{e}"))?;
    let code = root.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        let msg = root
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("未知错误");
        return Err(format!("起点接口失败（code={code}）：{msg}"));
    }
    let data = root.get("data").cloned().unwrap_or(Value::Null);
    if data.is_null() {
        return Err("起点接口无 data 字段".into());
    }
    let books = records_from_data_obj(&data);
    if books.is_empty() {
        return Err("起点接口 data 中无书籍列表".into());
    }
    let has_more = has_more_from_data(&data, books.len(), page);
    Ok((books, has_more))
}

/// 从 HTML 中抠出嵌入的 JSON 数组 / pageData.records。
/// `expect_gender`：若 SSR 带了 gender 且与请求不一致则拒绝（起点常把女频 query 仍渲成男频榜）。
pub fn parse_rank_html(
    html: &str,
    page: u32,
    expect_gender: RankGender,
) -> Result<(Vec<RankBook>, bool), String> {
    // 优先：pageData 对象（含 pageNum / isLast / records）
    if let Some((books, has_more, embedded_page, embedded_gender)) = extract_page_data(html) {
        if let Some(g) = embedded_gender {
            if g != expect_gender {
                return Err(format!(
                    "HTML SSR gender={} 与请求 {} 不一致，跳过",
                    g.as_str(),
                    expect_gender.as_str()
                ));
            }
        }
        if embedded_page != 0 && embedded_page != page {
            return Err(format!(
                "HTML SSR 仅为第 {embedded_page} 页，无法满足第 {page} 页"
            ));
        }
        if page > 1 && embedded_page == 0 {
            // 未能读到 pageNum，且请求非首页——避免把首页 SSR 误当翻页结果
            return Err("HTML SSR 无可靠页码，跳过".into());
        }
        if books.is_empty() {
            return Err("HTML SSR pageData.records 为空".into());
        }
        return Ok((books, has_more));
    }

    // 兼容：任意 "records":[ ... ]（无 gender 校验，仅男频全站首页可用）
    if expect_gender != RankGender::Male {
        return Err("HTML 无 pageData.gender，女频跳过 records 兜底".into());
    }
    if let Some(arr_text) = extract_json_array_after_key(html, "records") {
        let arr: Value = serde_json::from_str(&arr_text)
            .map_err(|e| format!("解析 records 数组失败：{e}"))?;
        let books = arr
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, v)| record_to_book(v, Some((i as u32) + 1)))
            .collect::<Vec<_>>();
        if books.is_empty() {
            return Err("HTML records 无有效书籍".into());
        }
        if page > 1 {
            return Err("HTML records 无页码校验，跳过非首页".into());
        }
        let has_more = books.len() >= PAGE_SIZE;
        return Ok((books, has_more));
    }

    Err("HTML 中未找到排行 records".into())
}

/// 完本频道首页 SSR：从 movie / classic / ds 等模块或 bName 对提取。
pub fn parse_finish_html(html: &str, page: u32) -> Result<(Vec<RankBook>, bool), String> {
    if page > 1 {
        return Err("完本 HTML 门户仅适合首页，翻页走 API".into());
    }
    let mut books = Vec::new();
    for key in ["movie", "classic", "ds", "hot", "books", "records", "list"] {
        if let Some(arr_text) = extract_json_array_after_key(html, key) {
            if let Ok(arr) = serde_json::from_str::<Value>(&arr_text) {
                for (i, v) in arr.as_array().into_iter().flatten().enumerate() {
                    if let Some(b) = record_to_book(v, Some((i as u32) + 1 + books.len() as u32)) {
                        books.push(b);
                    }
                }
            }
        }
    }
    // 去重（按书名+作者）
    books = dedupe_books(books);
    if books.is_empty() {
        return Err("完本 HTML 未解析到书籍".into());
    }
    // 门户页不是完整排行，提示还可翻页走 API
    Ok((books, true))
}

fn dedupe_books(books: Vec<RankBook>) -> Vec<RankBook> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for b in books {
        let key = format!("{}|{}", b.name, b.author);
        if seen.insert(key) {
            out.push(b);
        }
    }
    out
}

fn extract_page_data(html: &str) -> Option<(Vec<RankBook>, bool, u32, Option<RankGender>)> {
    // "pageData":{ ... "records":[... ] ... }
    let key = "\"pageData\"";
    let idx = html.find(key)?;
    let brace = html[idx..].find('{')? + idx;
    let obj_text = slice_balanced_json(&html[brace..])?;
    let data: Value = serde_json::from_str(obj_text).ok()?;
    let books = records_from_data_obj(&data);
    if books.is_empty() {
        return None;
    }
    let embedded_page = data
        .get("pageNum")
        .and_then(Value::as_u64)
        .or_else(|| data.get("page").and_then(Value::as_u64))
        .map(|n| n as u32)
        .unwrap_or(0);
    let embedded_gender = data
        .get("gender")
        .and_then(Value::as_str)
        .and_then(|s| RankGender::parse(s).ok());
    let has_more = has_more_from_data(&data, books.len(), embedded_page.max(1));
    Some((books, has_more, embedded_page, embedded_gender))
}

fn extract_json_array_after_key<'a>(html: &'a str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find(&pat) {
        let idx = search_from + rel;
        let after = &html[idx + pat.len()..];
        let colon = after.find(':')?;
        let rest = after[colon + 1..].trim_start();
        if rest.starts_with('[') {
            if let Some(arr) = slice_balanced_json(rest) {
                return Some(arr.to_string());
            }
        }
        search_from = idx + pat.len();
    }
    None
}

/// 从以 `{` 或 `[` 开头的切片截取平衡 JSON。
fn slice_balanced_json(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let open = *bytes.first()?;
    let close = match open {
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            o if o == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn csrf_token_from_set_cookie(headers: &HeaderMap) -> Option<String> {
    for val in headers.get_all(SET_COOKIE) {
        let Ok(s) = val.to_str() else { continue };
        for part in s.split(';') {
            let part = part.trim();
            if let Some(token) = part.strip_prefix("_csrfToken=") {
                let token = token.trim();
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

fn merge_csrf_cookie(existing: Option<&str>, token: &str) -> String {
    let piece = format!("_csrfToken={token}");
    match existing {
        Some(old) if old.contains("_csrfToken=") => {
            static RE: OnceLock<Regex> = OnceLock::new();
            let re = RE.get_or_init(|| Regex::new(r"_csrfToken=[^;]+").expect("csrf re"));
            re.replace(old, piece.as_str()).to_string()
        }
        Some(old) if !old.is_empty() => format!("{old}; {piece}"),
        _ => piece,
    }
}

async fn fetch_mobile_text(
    url: &str,
    cookie: Option<&str>,
    accept: &str,
) -> Result<(String, Option<String>), String> {
    let mut req = mobile_client()
        .get(url)
        .header(USER_AGENT, MOBILE_UA)
        .header("Accept", accept)
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Referer", format!("{MOBILE_ORIGIN}/"));
    if let Some(c) = cookie {
        req = req.header(COOKIE, c);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求失败：{url}（{e}）"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}：{url}", resp.status().as_u16()));
    }
    let csrf = csrf_token_from_set_cookie(resp.headers());
    let cookie_header = csrf
        .as_deref()
        .map(|token| merge_csrf_cookie(cookie, token))
        .or_else(|| cookie.map(|s| s.to_string()));
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败：{url}（{e}）"))?;
    Ok((text, cookie_header))
}

async fn bootstrap_csrf() -> Result<String, String> {
    let (html, cookie) = fetch_mobile_text(
        &format!("{MOBILE_ORIGIN}/rank/"),
        None,
        "text/html,application/xhtml+xml,*/*;q=0.8",
    )
    .await?;
    if let Some(c) = cookie.as_deref() {
        if c.contains("_csrfToken=") {
            return Ok(c.to_string());
        }
    }
    // 偶发：token 写在 HTML / 内联脚本
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"("_csrfToken"|csrfToken)["'\s:=]+([a-zA-Z0-9\-]+)"#)
            .expect("csrf html re")
    });
    if let Some(caps) = re.captures(&html) {
        let token = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        if !token.is_empty() {
            return Ok(format!("_csrfToken={token}"));
        }
    }
    Err("未能获取起点 _csrfToken".into())
}

fn board_label(channel: RankChannel, board: Option<RankBoard>) -> String {
    match channel {
        RankChannel::Finish => "完本".into(),
        RankChannel::Rank => board.map(|b| b.label().to_string()).unwrap_or_else(|| "排行".into()),
    }
}

fn html_url(
    channel: RankChannel,
    board: Option<RankBoard>,
    gender: RankGender,
    cat_id: i64,
) -> String {
    match channel {
        RankChannel::Finish => match gender {
            RankGender::Male => format!("{MOBILE_ORIGIN}/finish/"),
            RankGender::Female => format!("{MOBILE_ORIGIN}/finish/female/"),
        },
        RankChannel::Rank => {
            let slug = board.unwrap_or(RankBoard::Yuepiao).path_slug();
            // 女频走 /female/ 路径；query 里的 gender 常被 SSR 忽略
            match gender {
                RankGender::Male => format!(
                    "{MOBILE_ORIGIN}/rank/{slug}/?gender=male&catId={cat_id}"
                ),
                RankGender::Female => format!(
                    "{MOBILE_ORIGIN}/rank/{slug}/female/?gender=female&catId={cat_id}"
                ),
            }
        }
    }
}

fn majax_url(
    channel: RankChannel,
    board: Option<RankBoard>,
    gender: RankGender,
    page: u32,
    cat_id: i64,
    csrf: &str,
) -> String {
    let token = csrf
        .strip_prefix("_csrfToken=")
        .or_else(|| {
            csrf.split(';')
                .map(str::trim)
                .find_map(|p| p.strip_prefix("_csrfToken="))
        })
        .unwrap_or(csrf);
    match channel {
        RankChannel::Finish => format!(
            "{MOBILE_ORIGIN}/majax/category/list?_csrfToken={token}&gender={g}&pageNum={page}&orderBy=9&catId={cat_id}&subCatId=&isfinish=1",
            g = gender.as_str()
        ),
        RankChannel::Rank => {
            let list = board.unwrap_or(RankBoard::Yuepiao).majax_list_name();
            format!(
                "{MOBILE_ORIGIN}/majax/rank/{list}?_csrfToken={token}&gender={g}&catId={cat_id}&pageNum={page}",
                g = gender.as_str()
            )
        }
    }
}

/// 拉取一页排行 / 完本列表。
///
/// - 排行：男频全站首页可试 HTML（SSR）→ majax；女频 / 指定分类直接 majax（SSR 常忽略 gender、catId）
/// - 完本：majax JSON 优先（门户 HTML 不是分页排行）；失败再回退 HTML 首页模块（仅全站）
pub async fn fetch_rank_list(
    channel: RankChannel,
    board: Option<RankBoard>,
    gender: RankGender,
    page: u32,
    cat_id: i64,
) -> Result<RankListResult, String> {
    if page == 0 {
        return Err("页码从 1 开始".into());
    }
    if channel == RankChannel::Rank && board.is_none() {
        return Err("排行频道需要指定 board".into());
    }
    let label = board_label(channel, board);
    let mut errors: Vec<String> = Vec::new();

    if channel == RankChannel::Finish {
        return fetch_finish_list(gender, page, cat_id, label, errors).await;
    }

    // 女频与分类筛选跳过 HTML：SSR 经常仍返回男频全站榜
    let try_html = cat_id == -1 && gender == RankGender::Male;
    if try_html {
        let html_url = html_url(channel, board, gender, cat_id);
        match fetch_mobile_text(
            &html_url,
            None,
            "text/html,application/xhtml+xml,*/*;q=0.8",
        )
        .await
        {
            Ok((html, cookie_from_html)) => {
                match parse_rank_html(&html, page, gender) {
                    Ok((books, has_more)) if !books.is_empty() => {
                        return Ok(RankListResult {
                            books,
                            page,
                            has_more,
                            source: "mobile",
                            board_label: label,
                        });
                    }
                    Ok(_) => errors.push("HTML 解析结果为空".into()),
                    Err(e) => errors.push(e),
                }

                let cookie = match cookie_from_html {
                    Some(c) if c.contains("_csrfToken=") => c,
                    _ => match bootstrap_csrf().await {
                        Ok(c) => c,
                        Err(e) => {
                            errors.push(e);
                            return Err(format!(
                                "起点暂时无法抓取：{}",
                                errors.join("；")
                            ));
                        }
                    },
                };
                return fetch_via_majax(
                    channel, board, gender, page, cat_id, &cookie, label, errors,
                )
                .await;
            }
            Err(e) => {
                errors.push(e);
            }
        }
    }

    let cookie = bootstrap_csrf().await.map_err(|e| {
        errors.push(e.clone());
        format!("起点暂时无法抓取：{}", errors.join("；"))
    })?;
    fetch_via_majax(channel, board, gender, page, cat_id, &cookie, label, errors).await
}

/// 完本：majax 优先，HTML 门户仅作全站首页兜底。
async fn fetch_finish_list(
    gender: RankGender,
    page: u32,
    cat_id: i64,
    label: String,
    mut errors: Vec<String>,
) -> Result<RankListResult, String> {
    let cookie = match bootstrap_csrf().await {
        Ok(c) => c,
        Err(e) => {
            errors.push(e);
            return fetch_finish_html_fallback(gender, page, cat_id, label, errors).await;
        }
    };

    match fetch_via_majax(
        RankChannel::Finish,
        None,
        gender,
        page,
        cat_id,
        &cookie,
        label.clone(),
        Vec::new(),
    )
    .await
    {
        Ok(r) => Ok(r),
        Err(e) => {
            errors.push(e);
            fetch_finish_html_fallback(gender, page, cat_id, label, errors).await
        }
    }
}

async fn fetch_finish_html_fallback(
    gender: RankGender,
    page: u32,
    cat_id: i64,
    label: String,
    mut errors: Vec<String>,
) -> Result<RankListResult, String> {
    // 门户页无法按分类过滤
    if page > 1 || cat_id != -1 {
        return Err(format!("起点暂时无法抓取：{}", errors.join("；")));
    }
    let url = html_url(RankChannel::Finish, None, gender, cat_id);
    match fetch_mobile_text(&url, None, "text/html,application/xhtml+xml,*/*;q=0.8").await {
        Ok((html, _)) => match parse_finish_html(&html, page) {
            Ok((books, has_more)) if !books.is_empty() => Ok(RankListResult {
                books,
                page,
                has_more,
                source: "mobile",
                board_label: label,
            }),
            Ok(_) => {
                errors.push("完本 HTML 解析结果为空".into());
                Err(format!("起点暂时无法抓取：{}", errors.join("；")))
            }
            Err(e) => {
                errors.push(e);
                Err(format!("起点暂时无法抓取：{}", errors.join("；")))
            }
        },
        Err(e) => {
            errors.push(e);
            Err(format!("起点暂时无法抓取：{}", errors.join("；")))
        }
    }
}

async fn fetch_via_majax(
    channel: RankChannel,
    board: Option<RankBoard>,
    gender: RankGender,
    page: u32,
    cat_id: i64,
    cookie: &str,
    label: String,
    mut errors: Vec<String>,
) -> Result<RankListResult, String> {
    let url = majax_url(channel, board, gender, page, cat_id, cookie);
    match fetch_mobile_text(&url, Some(cookie), "application/json, text/plain, */*").await {
        Ok((text, _)) => match parse_majax_json(&text, page) {
            Ok((books, has_more)) => Ok(RankListResult {
                books,
                page,
                has_more,
                source: "mobile_api",
                board_label: label,
            }),
            Err(e) => {
                errors.push(e);
                Err(format!("起点暂时无法抓取：{}", errors.join("；")))
            }
        },
        Err(e) => {
            errors.push(e);
            Err(format!("起点暂时无法抓取：{}", errors.join("；")))
        }
    }
}

/// 供 Tauri 命令使用的 JSON 封装。
pub fn list_result_to_value(r: RankListResult) -> Value {
    json!({
        "books": r.books.iter().map(|b| {
            json!({
                "name": b.name,
                "author": b.author,
                "coverUrl": b.cover_url,
                "intro": b.intro,
                "category": b.category,
                "status": b.status,
                "rank": b.rank,
                "bid": b.bid,
            })
        }).collect::<Vec<_>>(),
        "page": r.page,
        "hasMore": r.has_more,
        "source": r.source,
        "boardLabel": r.board_label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_majax_yuepiao_fixture() {
        let text = include_str!("../../tests/fixtures/qidian_rank/majax_yuepiao.json");
        let (books, has_more) = parse_majax_json(text, 1).expect("majax");
        assert!(books.len() >= 2);
        assert_eq!(books[0].name, "夜无疆");
        assert_eq!(books[0].author, "辰东");
        assert_eq!(books[0].rank, Some(1));
        assert!(books[0].cover_url.as_ref().unwrap().contains("1040765595"));
        assert!(has_more);
    }

    #[test]
    fn parse_majax_finish_fixture() {
        let text = include_str!("../../tests/fixtures/qidian_rank/majax_finish.json");
        let (books, has_more) = parse_majax_json(text, 1).expect("finish");
        assert_eq!(books[0].name, "诡秘之主");
        assert!(has_more);
    }

    #[test]
    fn parse_rank_html_fixture() {
        let html = include_str!("../../tests/fixtures/qidian_rank/rank_yuepiao.html");
        let (books, _) = parse_rank_html(html, 1, RankGender::Male).expect("html");
        assert_eq!(books[0].name, "夜无疆");
        assert!(parse_rank_html(html, 2, RankGender::Male).is_err());
        // SSR 标了 male，请求女频应拒绝，避免把男榜当女榜
        let err = parse_rank_html(html, 1, RankGender::Female).unwrap_err();
        assert!(err.contains("不一致"), "{err}");
    }

    #[test]
    fn parse_finish_html_fixture() {
        let html = include_str!("../../tests/fixtures/qidian_rank/finish_hub.html");
        let (books, has_more) = parse_finish_html(html, 1).expect("finish html");
        assert!(books.iter().any(|b| b.name == "庆余年"));
        assert!(books.iter().any(|b| b.name == "斗破苍穹"));
        assert!(has_more);
        assert!(parse_finish_html(html, 2).is_err());
    }

    #[test]
    fn majax_rejects_error_code() {
        let err = parse_majax_json(r#"{"code":1,"msg":"失败"}"#, 1).unwrap_err();
        assert!(err.contains("失败"));
    }

    #[test]
    fn majax_numeric_bid_builds_cover() {
        let text = include_str!("../../tests/fixtures/qidian_rank/majax_finish.json");
        let (books, _) = parse_majax_json(text, 1).expect("finish");
        assert_eq!(books[0].bid.as_deref(), Some("1010868264"));
        assert!(books[0]
            .cover_url
            .as_ref()
            .unwrap()
            .contains("1010868264"));
        assert_eq!(books[0].status.as_deref(), Some("完本"));
    }

    #[test]
    fn has_more_respects_is_last_and_page_max() {
        let last = r#"{"code":0,"msg":"ok","data":{"isLast":1,"records":[{"bName":"A","bAuth":"B"}]}}"#;
        let (_, has_more) = parse_majax_json(last, 50).expect("last");
        assert!(!has_more);

        let paged = r#"{"code":0,"msg":"ok","data":{"pageMax":3,"pageNum":2,"records":[{"bName":"A","bAuth":"B"}]}}"#;
        let (_, has_more) = parse_majax_json(paged, 2).expect("mid");
        assert!(has_more);
        let (_, has_more) = parse_majax_json(
            r#"{"code":0,"msg":"ok","data":{"pageMax":3,"pageNum":3,"records":[{"bName":"A","bAuth":"B"}]}}"#,
            3,
        )
        .expect("end");
        assert!(!has_more);
    }

    #[test]
    fn board_and_channel_parse() {
        assert_eq!(RankChannel::parse("rank").unwrap(), RankChannel::Rank);
        assert_eq!(RankChannel::parse("finish").unwrap(), RankChannel::Finish);
        assert!(RankChannel::parse("x").is_err());
        assert_eq!(RankBoard::parse("recom").unwrap().majax_list_name(), "reclist");
        assert_eq!(RankGender::parse("女频").unwrap(), RankGender::Female);
    }

    /// 真机联调：默认忽略，需要时 `cargo test -p fly-reader live_fetch -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_fetch_rank_and_finish() {
        let rank = fetch_rank_list(
            RankChannel::Rank,
            Some(RankBoard::Yuepiao),
            RankGender::Male,
            1,
            -1,
        )
        .await
        .expect("rank p1");
        assert!(!rank.books.is_empty(), "rank empty: {:?}", rank.source);
        assert!(!rank.books[0].name.is_empty());

        let rank_cat = fetch_rank_list(
            RankChannel::Rank,
            Some(RankBoard::Yuepiao),
            RankGender::Male,
            1,
            21,
        )
        .await
        .expect("rank xuanhuan");
        assert!(!rank_cat.books.is_empty());
        assert_eq!(rank_cat.source, "mobile_api");

        let rank_p2 = fetch_rank_list(
            RankChannel::Rank,
            Some(RankBoard::Hotsales),
            RankGender::Male,
            2,
            -1,
        )
        .await
        .expect("hotsales p2");
        assert!(!rank_p2.books.is_empty());
        assert_eq!(rank_p2.source, "mobile_api");

        let finish = fetch_rank_list(RankChannel::Finish, None, RankGender::Male, 1, 4)
            .await
            .expect("finish urban");
        assert!(!finish.books.is_empty());
        assert_eq!(finish.source, "mobile_api");

        let finish_p2 = fetch_rank_list(RankChannel::Finish, None, RankGender::Female, 2, -1)
            .await
            .expect("finish female p2");
        assert!(!finish_p2.books.is_empty());

        let female = fetch_rank_list(
            RankChannel::Rank,
            Some(RankBoard::Yuepiao),
            RankGender::Female,
            1,
            -1,
        )
        .await
        .expect("female yuepiao");
        assert!(!female.books.is_empty());
        assert_eq!(female.source, "mobile_api");
        assert_ne!(
            female.books[0].name, rank.books[0].name,
            "女频月票不应与男频同榜首"
        );
    }
}
