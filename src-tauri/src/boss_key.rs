//! 老板键：全局快捷键一键隐藏/显示主窗口，对应原版 registerBossKeyFromPrefs / toggleBossVisibility。
//!
//! 注意线程约束：tauri-plugin-global-shortcut 在 macOS 上会把注册/注销操作派发到主线程并阻塞等待结果，
//! 因此这些调用只能来自非主线程（异步命令或 `async_runtime::spawn`）。快捷键触发的回调本身跑在主线程，
//! 所以 `toggle` 里只做窗口显隐与事件推送，不能再调用注册接口。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 老板键运行时状态：窗口是否处于隐藏态、当前已注册的组合键。
#[derive(Default)]
pub struct BossState {
    pub hidden: AtomicBool,
    pub registered: Mutex<String>,
    /// 注销+注册整个换键序列的互斥锁：两次 prefs 保存并发时不互相顶掉对方的键
    pub swap_lock: Mutex<()>,
}

impl BossState {
    fn take_registered(&self) -> String {
        self.registered
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    fn set_registered(&self, accel: &str) {
        if let Ok(mut g) = self.registered.lock() {
            *g = accel.to_string();
        }
    }
}

/// 隐藏或恢复主窗口；macOS 同步 Dock 图标，并把新状态推给渲染层。
pub fn toggle(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<BossState>();
    let hidden = state.hidden.load(Ordering::Relaxed);
    let visible = win.is_visible().unwrap_or(true);
    if hidden || !visible {
        #[cfg(target_os = "macos")]
        let _ = app.set_dock_visibility(true);
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        state.hidden.store(false, Ordering::Relaxed);
    } else {
        let _ = win.hide();
        #[cfg(target_os = "macos")]
        let _ = app.set_dock_visibility(false);
        state.hidden.store(true, Ordering::Relaxed);
    }
    let now_hidden = state.hidden.load(Ordering::Relaxed);
    let _ = app.emit("app_boss_visibility", json!({ "hidden": now_hidden }));
}

/// 插件回调：只在按下时切换（原版 Electron 的 register 回调同样忽略释放事件）。
pub fn on_shortcut_event(app: &AppHandle, state: ShortcutState) {
    if state == ShortcutState::Pressed {
        toggle(app);
    }
}

/// 校验老板键组合：空、单键、退出/关闭类组合一律拒绝。
/// 返回 Err 时是给前端看的中文原因。
fn validate_boss_key(accel: &str) -> Result<(), String> {
    let trimmed = accel.trim();
    if trimmed.is_empty() {
        return Err("快捷键不能为空".to_string());
    }
    let parts: Vec<&str> = trimmed
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 2 {
        return Err(
            "快捷键至少需要两个键组合（例如 CommandOrControl+Shift+H），单键容易误触"
                .to_string(),
        );
    }
    let lower: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    let has_cmd_or_ctrl = lower.iter().any(|p| {
        matches!(
            p.as_str(),
            "command" | "cmd" | "control" | "ctrl" | "commandorcontrol"
        )
    });
    let has_alt = lower
        .iter()
        .any(|p| matches!(p.as_str(), "alt" | "option"));
    let has_other_modifier = lower.iter().any(|p| {
        matches!(p.as_str(), "shift" | "alt" | "option" | "super" | "meta")
    });
    let key = lower.last().cloned().unwrap_or_default();
    // 退出/关闭类组合：Cmd+Q（退出应用）、Cmd+W（关窗口）——只有修饰键恰好是
    // Cmd/Ctrl 时才算（Cmd+Shift+Q 不是退出组合）；Alt+F4（关窗口）有 Alt 即算。
    if (has_cmd_or_ctrl && !has_other_modifier && (key == "q" || key == "w"))
        || (has_alt && key == "f4")
    {
        return Err("该组合键用于退出或关闭窗口，不能设为老板键".to_string());
    }
    Ok(())
}

/// 按偏好重注册老板键，返回原版的 `{ok,message}` 结构。
/// 文案与 `registerBossKeyFromPrefs` 逐条一致。
/// 换键顺序：先注册新键、成功后再注销旧键；注册失败则恢复旧键（重新注册 previous），
/// 避免旧键已注销、新键又没注册上导致老板键中断。
pub fn register_from_prefs(app: &AppHandle, prefs: &Value) -> Value {
    let state = app.state::<BossState>();
    // 注销+注册整个序列加锁，避免两次 prefs 保存并发时互相顶掉对方的键
    let _guard = state.swap_lock.lock().unwrap_or_else(|e| e.into_inner());
    let previous = state.take_registered();
    let enabled = prefs
        .get("bossKeyEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !enabled {
        if !previous.is_empty() {
            let _ = app.global_shortcut().unregister(previous.as_str());
            state.set_registered("");
        }
        return json!({ "ok": true, "message": "老板键已关闭" });
    }
    let accel = prefs
        .get("bossKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    // 校验失败：不动旧键，旧键继续生效
    if let Err(msg) = validate_boss_key(&accel) {
        return json!({ "ok": false, "message": msg });
    }
    if accel == previous {
        return json!({ "ok": true, "message": format!("老板键已启用：{accel}") });
    }
    match app.global_shortcut().register(accel.as_str()) {
        Ok(()) => {
            if !previous.is_empty() {
                let _ = app.global_shortcut().unregister(previous.as_str());
            }
            state.set_registered(&accel);
            json!({ "ok": true, "message": format!("老板键已启用：{accel}") })
        }
        Err(_) => {
            if !previous.is_empty() {
                // 旧键从未注销，重新注册一次只是兜底（已注册时会失败，忽略即可）
                let _ = app.global_shortcut().register(previous.as_str());
                state.set_registered(&previous);
            }
            json!({
                "ok": false,
                "message": format!("无法注册快捷键：{accel}（可能被占用），已恢复原老板键")
            })
        }
    }
}

/// 原版 `app.on('activate')`：Dock 图标点击时，若窗口正被老板键隐藏则恢复显示。
/// 回调本身跑在主线程，可以安全调用 `toggle`。
#[cfg(target_os = "macos")]
pub fn on_reopen(app: &AppHandle, has_visible_windows: bool) {
    if has_visible_windows {
        return;
    }
    if app.state::<BossState>().hidden.load(Ordering::Relaxed) {
        toggle(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boss_state_tracks_registered_accelerator() {
        let state = BossState::default();
        assert_eq!(state.take_registered(), "");
        state.set_registered("CommandOrControl+Shift+H");
        assert_eq!(state.take_registered(), "CommandOrControl+Shift+H");
        state.set_registered("");
        assert_eq!(state.take_registered(), "");
        assert!(!state.hidden.load(Ordering::Relaxed));
    }

    #[test]
    fn boss_key_validation_rejects_empty_single_and_dangerous() {
        assert!(validate_boss_key("").is_err());
        assert!(validate_boss_key("   ").is_err());
        // 单键：只有一个按键，没有修饰键组合
        assert!(validate_boss_key("H").is_err());
        assert!(validate_boss_key("F1").is_err());
        // 退出/关闭类组合
        assert!(validate_boss_key("CommandOrControl+Q").is_err());
        assert!(validate_boss_key("Command+Q").is_err());
        assert!(validate_boss_key("Control+Q").is_err());
        assert!(validate_boss_key("CommandOrControl+W").is_err());
        assert!(validate_boss_key("Cmd+W").is_err());
        assert!(validate_boss_key("Alt+F4").is_err());
        assert!(validate_boss_key("Option+F4").is_err());
        // 正常组合放行
        assert!(validate_boss_key("CommandOrControl+Shift+H").is_ok());
        assert!(validate_boss_key("Alt+Shift+X").is_ok());
        assert!(validate_boss_key("Control+Shift+Q").is_ok(), "带 Shift 的 Q 不是退出组合");
    }
}
