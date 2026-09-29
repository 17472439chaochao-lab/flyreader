//! `@js` 规则沙箱：QuickJS 隔离执行，对应原 Electron 版 src/main/legado/jsRule.ts。
//!
//! 与原实现的差异说明：
//! - 只注入 `result / baseUrl / key / page` 与 `java` 加密桥，不提供宿主能力
//!   （无 fetch / fs / process）。
//! - **`java.*` 对称解密桥是 FlyReader 独有的**：原版和 Electron 版都没有，
//!   依赖它取章节地址的 API 书源（猫眼看书、笔趣阁）在两版里都读不了正文。
//!   这里按 hutool `AES` 的口径补齐（UTF-8 密钥字节、CBC/PKCS7、输入为 Base64）。
//! - 用 QuickJS 中断处理器实现 2 秒超时，替代 node vm 的 `timeout` 选项，防止规则死循环。

use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use rquickjs::{
    context::EvalOptions, prelude::Func, Context, Ctx, Exception, Object, Runtime,
};
use serde::Deserialize;
use serde_json::Value;

/// 单条 @js 规则执行超时（毫秒），与 jsRule.ts 的 JS_RULE_TIMEOUT_MS 一致。
const JS_RULE_TIMEOUT_MS: u64 = 2000;

/// 注入沙箱的绑定变量
#[derive(Clone, Copy)]
pub struct JsBindings<'a> {
    pub result: &'a str,
    pub base_url: &'a str,
    pub key: &'a str,
    pub page: &'a str,
}

impl Default for JsBindings<'_> {
    fn default() -> Self {
        // 与 jsRule.ts 一致：缺省 page 为数值 1
        Self {
            result: "",
            base_url: "",
            key: "",
            page: "1",
        }
    }
}

thread_local! {
    static SANDBOX: RefCell<Option<(Runtime, Context)>> = const { RefCell::new(None) };
}

/// 沙箱返回值：要么是字符串数组，要么是 JS 侧异常文本
#[derive(Deserialize)]
struct EvalOutcome {
    ok: Option<Vec<Value>>,
    err: Option<String>,
}

/// 包装脚本：把规则表达式收成一侧 JSON，避免 Rust 侧处理 QuickJS 异常对象。
fn wrap_script(code: &str) -> String {
    format!(
        r#"(function(){{
  try {{
    var r = ({code});
    var list = (r === null || r === undefined) ? [] : (r instanceof Array ? r : [r]);
    var out = [];
    for (var i = 0; i < list.length; i++) {{
      out.push(list[i] === null || list[i] === undefined ? '' : String(list[i]));
    }}
    return JSON.stringify({{ ok: out }});
  }} catch (e) {{
    return JSON.stringify({{ err: String(e && e.message ? e.message : e) }});
  }}
}})()"#
    )
}

/// 执行 @js 规则并把结果规范成字符串数组，对应 evalJsRuleToStrings：
/// null/undefined → 空；数组 → 逐项 String()；其它 → 单元素。
pub fn eval_js_to_strings(code: &str, bindings: JsBindings<'_>) -> Result<Vec<String>, String> {
    let script = wrap_script(code);
    let limit = Duration::from_millis(JS_RULE_TIMEOUT_MS);

    SANDBOX.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let runtime = Runtime::new().map_err(|e| format!("@js 沙箱初始化失败：{e}"))?;
            let context = Context::full(&runtime).map_err(|e| format!("@js 沙箱初始化失败：{e}"))?;
            *slot = Some((runtime, context));
        }
        let (runtime, context) = slot.as_ref().ok_or("@js 沙箱未初始化")?;

        // 中断器必须在 context.with 之外装卸：with 内部已持有 runtime 锁，重入会死锁。
        let started = Instant::now();
        runtime.set_interrupt_handler(Some(Box::new(move || started.elapsed() > limit)));
        let evaluated = context.with(|ctx| eval_in_ctx(&ctx, &script, bindings));
        runtime.set_interrupt_handler(None);

        if started.elapsed() > limit {
            // 中断可能让 QuickJS 的引用计数停在半路，丢弃沙箱换取干净的下一轮执行。
            *slot = None;
            return Err(format!("@js 规则执行超时（>{JS_RULE_TIMEOUT_MS}ms）"));
        }
        evaluated
    })
}

fn eval_in_ctx(ctx: &rquickjs::Ctx<'_>, script: &str, b: JsBindings<'_>) -> Result<Vec<String>, String> {
    let globals = ctx.globals();
    set_bindings(&globals, b)?;

    let mut options = EvalOptions::default();
    // 非严格模式：与原版 node vm 一致，书源里 `a=java.createSymmetricCrypto(...)` 这类
    // 未声明直接赋值是常见写法，严格模式下会抛 "a is not defined"。
    options.strict = false;
    let payload = ctx
        .eval_with_options::<String, _>(script, options)
        .map_err(|e| format!("@js 规则执行失败：{e}"))?;
    let outcome: EvalOutcome =
        serde_json::from_str(&payload).map_err(|e| format!("@js 结果解析失败：{e}"))?;
    if let Some(message) = outcome.err {
        return Err(format!("@js 规则执行失败：{message}"));
    }
    Ok(outcome
        .ok
        .unwrap_or_default()
        .into_iter()
        .map(|v| match v {
            Value::String(s) => s,
            other => other.to_string(),
        })
        .collect())
}

fn set_bindings<'js>(
    globals: &rquickjs::Object<'js>,
    b: JsBindings<'_>,
) -> Result<(), String> {
    globals.set("result", b.result).map_err(|e| e.to_string())?;
    globals.set("baseUrl", b.base_url).map_err(|e| e.to_string())?;
    globals.set("key", b.key).map_err(|e| e.to_string())?;
    // 与原版一致：移除能从字符串构造函数的原语，收窄规则攻击面
    globals
        .set("Function", None::<String>)
        .map_err(|e| e.to_string())?;
    globals
        .set("eval", None::<String>)
        .map_err(|e| e.to_string())?;
    install_java_bridge(globals).map_err(|e| e.to_string())?;
    // page 保持数值语义，否则 JS 里 "1" + 1 === "11"
    match b.page.parse::<f64>() {
        Ok(n) => globals.set("page", n),
        Err(_) => globals.set("page", b.page),
    }
    .map_err(|e| e.to_string())
}

/// AES-CBC/PKCS7 解密，密钥长度决定 128/192/256。
fn aes_cbc_decrypt(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, String> {
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    let bad = |e| format!("AES 密钥/IV 长度不合法：{e}");
    if key.len() == 16 {
        let d = <cbc::Decryptor<aes::Aes128>>::new_from_slices(key, iv).map_err(bad)?;
        d.decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|e| format!("AES 解密失败：{e}"))
    } else if key.len() == 24 {
        let d = <cbc::Decryptor<aes::Aes192>>::new_from_slices(key, iv).map_err(bad)?;
        d.decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|e| format!("AES 解密失败：{e}"))
    } else if key.len() == 32 {
        let d = <cbc::Decryptor<aes::Aes256>>::new_from_slices(key, iv).map_err(bad)?;
        d.decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|e| format!("AES 解密失败：{e}"))
    } else {
        Err(format!("AES 密钥长度不合法：{} 字节", key.len()))
    }
}

/// Base64 密文 → UTF-8 明文；任何一步失败都抛异常，避免把垃圾串当成章节地址去请求。
fn aes_base64_decode_to_string(
    ctx: &Ctx<'_>,
    text: &str,
    key: &str,
    iv: &str,
) -> rquickjs::Result<String> {
    let trimmed = text.trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(trimmed))
        .map_err(|e| js_error(ctx, format!("Base64 解码失败：{e}")))?;
    let plain = aes_cbc_decrypt(&bytes, key.as_bytes(), iv.as_bytes())
        .map_err(|e| js_error(ctx, e))?;
    String::from_utf8(plain).map_err(|_| js_error(ctx, "AES 解密结果不是 UTF-8 文本".to_string()))
}

fn js_error(ctx: &Ctx<'_>, message: String) -> rquickjs::Error {
    Exception::throw_message(ctx, &message)
}

/// 注入 Legado 的 `java` 对称解密桥（只补书源实际用到的两个方法，不铺其它宿主能力）。
fn install_java_bridge<'js>(globals: &Object<'js>) -> rquickjs::Result<()> {
    let ctx = globals.ctx().clone();
    let java = Object::new(ctx.clone())?;
    // 书源写成 (data, key, transformation, iv)；transformation 一律按 AES/CBC/PKCS5Padding 处理
    java.set(
        "aesBase64DecodeToString",
        Func::from(
            |ctx: Ctx<'js>, text: String, key: String, _trans: Option<String>, iv: String| {
                aes_base64_decode_to_string(&ctx, &text, &key, &iv)
            },
        ),
    )?;
    java.set(
        "createSymmetricCrypto",
        Func::from(
            move |ctx: Ctx<'js>, _trans: String, key: String, iv: String| -> rquickjs::Result<Object<'js>> {
            let key = Arc::new(key);
            let iv = Arc::new(iv);
            let crypto = Object::new(ctx.clone())?;
            crypto.set(
                "decryptStr",
                Func::from(move |ctx: Ctx<'js>, text: String| {
                    aes_base64_decode_to_string(&ctx, &text, &key, &iv)
                }),
            )?;
            Ok(crypto)
        }),
    )?;
    globals.set("java", java)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_rule_reads_result_and_stringifies() {
        let got = eval_js_to_strings(
            "result.replace(/\\s+/g,'')",
            JsBindings {
                result: "a b  c",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got, vec!["abc".to_string()]);
    }

    #[test]
    fn js_rule_returns_array_items() {
        let got = eval_js_to_strings("['x', 1, null]", JsBindings::default()).unwrap();
        assert_eq!(got, vec!["x", "1", ""]);
    }

    #[test]
    fn js_rule_page_is_numeric() {
        let got = eval_js_to_strings("String(page + 1)", JsBindings {
            page: "2",
            ..Default::default()
        })
        .unwrap();
        assert_eq!(got, vec!["3"]);
    }

    #[test]
    fn js_rule_timeout_is_reported() {
        let err = eval_js_to_strings("(function(){ while(true){} })()", JsBindings::default())
            .unwrap_err();
        assert!(err.contains("超时"), "{err}");
    }

    /// 测试用：AES-CBC/PKCS7 加密后转 Base64，供桥的往返断言取密文。
    fn encrypt_b64(plain: &str, key: &str, iv: &str) -> String {
        use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        let d = <cbc::Encryptor<aes::Aes128>>::new_from_slices(key.as_bytes(), iv.as_bytes())
            .expect("16 字节密钥与 IV");
        let bytes = d.encrypt_padded_vec_mut::<Pkcs7>(plain.as_bytes());
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn java_aes_bridge_decodes_book_source_rules() {
        // 猫眼看书的章节地址就是这个形态：$.path@js:java.aesBase64DecodeToString(result,key,trans,iv)
        let cipher = encrypt_b64("/novel/386971/1.html?path=abc", "f041c49714d39908", "0123456789abcdef");
        let code = format!(
            "java.aesBase64DecodeToString(result,\"f041c49714d39908\",\"AES/CBC/PKCS5Padding\",\"0123456789abcdef\")"
        );
        let got = eval_js_to_strings(
            &code,
            JsBindings {
                result: &cipher,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got, vec!["/novel/386971/1.html?path=abc".to_string()]);
    }

    #[test]
    fn java_symmetric_crypto_factory_carries_key_and_iv() {
        // 笔趣阁写法：createSymmetricCrypto(trans,key,iv).decryptStr(String(result))
        let cipher = encrypt_b64("第 3 章", "f041c49714d39908", "0123456789abcdef");
        let got = eval_js_to_strings(
            "java.createSymmetricCrypto(\"AES/CBC/PKCS5Padding\",\"f041c49714d39908\",\"0123456789abcdef\").decryptStr(String(result))",
            JsBindings {
                result: &cipher,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got, vec!["第 3 章".to_string()]);
    }

    #[test]
    fn java_bridge_still_has_no_host_capabilities() {
        // 解密桥之外仍然是纯沙箱：没有 fetch / process
        assert!(eval_js_to_strings("fetch(result)", JsBindings::default()).is_err());
        assert!(eval_js_to_strings("process.cwd()", JsBindings::default()).is_err());
    }

    #[test]
    fn java_bridge_rejects_bad_ciphertext() {
        // 密文不合法要报错而不是返回垃圾，否则会拿垃圾地址去请求
        let err = eval_js_to_strings(
            "java.aesBase64DecodeToString(result,\"f041c49714d39908\",\"AES/CBC/PKCS5Padding\",\"0123456789abcdef\")",
            JsBindings {
                result: "not base64 at all!!",
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("Base64") || err.contains("AES"), "{err}");
    }

    #[test]
    fn js_rule_runs_in_sloppy_mode_like_node_vm() {
        // 笔趣阁的 <js> 块就是这种未声明直接赋值
        let got = eval_js_to_strings(
            "a=String(result)+'/read'",
            JsBindings {
                result: "x",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got, vec!["x/read".to_string()]);
    }
}
