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

/// 按偏好重注册老板键，返回原版的 `{ok,message}` 结构。
/// 文案与 `registerBossKeyFromPrefs` 逐条一致。
pub fn register_from_prefs(app: &AppHandle, prefs: &Value) -> Value {
    let state = app.state::<BossState>();
    let previous = state.take_registered();
    if !previous.is_empty() {
        let _ = app.global_shortcut().unregister(previous.as_str());
        state.set_registered("");
    }
    let enabled = prefs
        .get("bossKeyEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !enabled {
        return json!({ "ok": true, "message": "老板键已关闭" });
    }
    let accel = prefs
        .get("bossKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if accel.is_empty() {
        return json!({ "ok": false, "message": "快捷键不能为空" });
    }
    match app.global_shortcut().register(accel.as_str()) {
        Ok(()) => {
            state.set_registered(&accel);
            json!({ "ok": true, "message": format!("老板键已启用：{accel}") })
        }
        Err(_) => json!({
            "ok": false,
            "message": format!("无法注册快捷键：{accel}（可能被占用）")
        }),
    }
}

/// 原版 `app.on('activate')`：Dock 图标点击时，若窗口正被老板键隐藏则恢复显示。
/// 回调本身跑在主线程，可以安全调用 `toggle`。
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
}
