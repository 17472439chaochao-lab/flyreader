//! 轻量检查更新：查 Gitee Releases 最新版并与当前版本比较，只提示不自动下载安装。
//!
//! 对应原版的 `main/updateCheck.ts` + `shared/updateCheck.ts`。原版用 Electron 的 `net` 模块，
//! 这里走独立的 reqwest 请求：更新检查不是书源取页，不需要共用 legado http 的 UA、
//! 字符集嗅探与「非 2xx 即报错」逻辑——原版正是靠 404 区分「仓库尚未发布发行版」。

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};

/// 与 `shared/about.ts` 保持一致的仓库信息（改动时同步前端）。
const APP_NAME: &str = "FlyReader";
const GITEE_OWNER: &str = "wucc513721";
const GITEE_REPO: &str = "flyreader";
const RELEASES_URL: &str = "https://gitee.com/wucc513721/flyreader/releases";
/// 兜底版本号（`APP_ABOUT.version`）：主进程版本为空时使用。
const FALLBACK_VERSION: &str = "1.1.0";
/// 原版用 Electron net 无超时；这里给一个上限，避免关于页一直转圈。
const REQUEST_TIMEOUT_SECS: u64 = 15;

/// 去掉前缀 v/V 并裁剪空白，对应 normalizeVersion。
pub fn normalize_version(version: &str) -> String {
    let trimmed = version.trim();
    match trimmed.strip_prefix('v').or_else(|| trimmed.strip_prefix('V')) {
        Some(rest) => rest.to_string(),
        None => trimmed.to_string(),
    }
}

/// 模拟 `parseInt(x, 10)`：取前导数字，无法解析时为 0（JS 的 NaN 在上游被换成 0）。
fn parse_int_loose(segment: &str) -> i64 {
    let digits: String = segment.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<i64>().unwrap_or(0)
}

fn version_parts(version: &str) -> Vec<i64> {
    normalize_version(version)
        .split(['.', '+', '-'])
        .map(parse_int_loose)
        .collect()
}

/// 按数字段比较两个版本号，a>b 为正、a<b 为负、相等为 0，对应 compareVersions。
pub fn compare_versions(a: &str, b: &str) -> i64 {
    let pa = version_parts(a);
    let pb = version_parts(b);
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return x - y;
        }
    }
    0
}

/// `encodeURIComponent`：字母数字与 `-_.!~*'()` 不转义。
const URI_COMPONENT_SET: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

fn encode_uri_component(value: &str) -> String {
    utf8_percent_encode(value, URI_COMPONENT_SET).to_string()
}

/// 把一次 HTTP 响应换成检查结果，对应 checkGiteeUpdate 的分支。
/// `body` 为响应正文（原版空正文视作 data=null）。
fn release_to_result(status: u16, body: &str, current: &str) -> Value {
    if status == 404 {
        return json!({
            "status": "noRelease",
            "currentVersion": current,
            "message": "仓库尚未发布发行版，请稍后再试或前往仓库查看"
        });
    }
    let data: Value = if body.trim().is_empty() {
        Value::Null
    } else {
        match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return json!({
                    "status": "error",
                    "currentVersion": current,
                    "message": format!("解析更新信息失败：{e}")
                })
            }
        }
    };
    if !(200..300).contains(&status) || !data.is_object() {
        return json!({
            "status": "error",
            "currentVersion": current,
            "message": format!("检查更新失败（HTTP {status}）")
        });
    }

    let text_of = |key: &str| -> String {
        data.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let tag_name = text_of("tag_name");
    let name = text_of("name");
    // 原版的 `a || b || ''`：空串同样继续兜底
    let latest_raw = if !tag_name.is_empty() {
        tag_name.clone()
    } else {
        name.clone()
    };
    let latest = normalize_version(&latest_raw);
    if latest.is_empty() {
        return json!({
            "status": "noRelease",
            "currentVersion": current,
            "message": "最新发行版缺少有效版本号"
        });
    }

    let html_url = text_of("html_url");
    let release_url = if !html_url.is_empty() {
        html_url
    } else {
        format!("{RELEASES_URL}/tag/{}", encode_uri_component(&latest_raw))
    };

    if compare_versions(&latest, current) > 0 {
        let mut result = json!({
            "status": "available",
            "currentVersion": current,
            "latestVersion": latest,
            "releaseUrl": release_url
        });
        // JS 的 undefined 字段在 JSON 里不存在，这里同样只在有值时写入
        if !name.is_empty() {
            result["releaseName"] = json!(name);
        }
        if let Some(body_text) = data.get("body").and_then(Value::as_str) {
            result["releaseNotes"] = json!(body_text.trim());
        }
        return result;
    }

    json!({
        "status": "upToDate",
        "currentVersion": current,
        "latestVersion": latest
    })
}

/// 向 Gitee Releases 查询最新发行版并与 `current_version` 比较。
/// 任何失败都以 `{status:"error", …}` 返回，不抛错（与原版 IPC 一致）。
pub async fn check_gitee_update(current_version: &str) -> Value {
    let current = {
        let normalized = normalize_version(current_version);
        if normalized.is_empty() {
            FALLBACK_VERSION.to_string()
        } else {
            normalized
        }
    };
    let api = format!(
        "https://gitee.com/api/v5/repos/{GITEE_OWNER}/{GITEE_REPO}/releases/latest"
    );
    let resp = reqwest::Client::new()
        .get(&api)
        .header("accept", "application/json")
        .header("user-agent", format!("{APP_NAME}-updater"))
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await;
    match resp {
        Err(e) => json!({
            "status": "error",
            "currentVersion": current,
            "message": e.to_string()
        }),
        Ok(resp) => {
            let status = resp.status().as_u16();
            match resp.text().await {
                Err(e) => json!({
                    "status": "error",
                    "currentVersion": current,
                    "message": e.to_string()
                }),
                Ok(body) => release_to_result(status, &body, &current),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_normalization_strips_prefix_and_compares_numerically() {
        assert_eq!(normalize_version("  v1.0.6 "), "1.0.6");
        assert_eq!(normalize_version("V2"), "2");
        // 数字段比较：1.9 < 1.10，非数字段按 0 处理（与 JS parseInt 的兜底一致）
        assert!(compare_versions("1.9.0", "1.10.0") < 0);
        assert_eq!(compare_versions("1.0.6", "1.0.6+gitee"), 0);
        assert_eq!(compare_versions("v1.0.6", "1.0.6.0"), 0);
        assert_eq!(compare_versions("beta", "0"), 0);
    }

    #[test]
    fn missing_release_and_bad_payloads_map_to_documented_messages() {
        let r = release_to_result(404, "", "1.1.0");
        assert_eq!(r["status"], json!("noRelease"));
        assert_eq!(
            r["message"],
            json!("仓库尚未发布发行版，请稍后再试或前往仓库查看")
        );

        // 空正文 = 原版 data 为 null，走 HTTP 分支
        assert_eq!(
            release_to_result(200, "", "1.1.0")["message"],
            json!("检查更新失败（HTTP 200）")
        );
        assert_eq!(
            release_to_result(500, "{}", "1.1.0")["message"],
            json!("检查更新失败（HTTP 500）")
        );
        assert_eq!(
            release_to_result(200, "{oops", "1.1.0")["status"],
            json!("error")
        );
        assert!(release_to_result(200, "{oops", "1.1.0")["message"]
            .as_str()
            .unwrap()
            .starts_with("解析更新信息失败："));
        assert_eq!(
            release_to_result(200, r#"{"tag_name":""}"#, "1.1.0")["message"],
            json!("最新发行版缺少有效版本号")
        );
    }

    #[test]
    fn newer_release_carries_url_name_and_notes() {
        let body = json!({
            "tag_name": "v1.2.0",
            "name": "1.2.0",
            "body": "  修复导入  \n",
            "html_url": "https://gitee.com/x/releases/v1.2.0"
        })
        .to_string();
        let r = release_to_result(200, &body, "1.1.0");
        assert_eq!(r["status"], json!("available"));
        assert_eq!(r["latestVersion"], json!("1.2.0"));
        assert_eq!(r["releaseUrl"], json!("https://gitee.com/x/releases/v1.2.0"));
        assert_eq!(r["releaseNotes"], json!("修复导入"));

        // tag 无 html_url 时回退到 releases/tag/<encoded>
        let no_url = json!({ "tag_name": "v1.2.0 测试" }).to_string();
        assert_eq!(
            release_to_result(200, &no_url, "1.1.0")["releaseUrl"],
            json!(format!("{RELEASES_URL}/tag/v1.2.0%20%E6%B5%8B%E8%AF%95"))
        );

        let older = json!({ "tag_name": "v1.0.9" }).to_string();
        assert_eq!(release_to_result(200, &older, "1.1.0")["status"], json!("upToDate"));
    }

    /// 真实 Gitee Releases 接口验证（网络相关，`cargo test` 默认跳过）：
    /// `cargo test --lib update_check -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_gitee_release_check() {
        let r = check_gitee_update("0.0.1").await;
        println!("check result: {r}");
        let status = r["status"].as_str().unwrap_or_default();
        assert!(
            matches!(status, "available" | "upToDate" | "noRelease"),
            "接口应返回版本结论，实际为 {status}：{}",
            r["message"]
        );
        assert_eq!(r["currentVersion"], json!("0.0.1"));
    }
}
