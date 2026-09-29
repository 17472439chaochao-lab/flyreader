//! 阅读主题 → 系统深浅色与窗口背景色，对应原版 applyAppTheme / THEME_WINDOW_BG。
//!
//! 原版设置 `nativeTheme.themeSource` 并遍历窗口 `setBackgroundColor`；Tauri 侧分别是
//! `AppHandle::set_theme` 与 `WebviewWindow::set_background_color`。night 之外都按 light 处理，
//! 因为阅读主题只有 paper/green/night 三档，前两者都是浅色底。
//!
//! 两者最终都落到 AppKit（tao 的 `set_ns_theme` 是裸 msg_send），所以统一用
//! `run_on_main_thread` 投递：命令可能跑在 tokio 工作线程上，而 setup 阶段投递的任务会在下一轮
//! 事件循环执行，同样安全。

use serde_json::Value;
use tauri::{AppHandle, Manager};

/// 与渲染层 CSS 变量同名的三档主题。
const THEME_BG: [(&str, [u8; 3]); 3] = [
    ("paper", [0xe6, 0xeb, 0xe4]),
    ("green", [0xdc, 0xe8, 0xd6]),
    ("night", [0x12, 0x15, 0x16]),
];

fn theme_color(theme: &str) -> tauri::window::Color {
    let rgb = THEME_BG
        .iter()
        .find(|(name, _)| *name == theme)
        .map(|(_, rgb)| *rgb)
        .unwrap_or(THEME_BG[0].1);
    tauri::window::Color(rgb[0], rgb[1], rgb[2], 255)
}

/// 把 settings 里的 theme 换成三档之一，未知值按 paper 兜底（原版 `|| THEME_WINDOW_BG.paper`）。
fn theme_of(settings: &Value) -> &'static str {
    settings
        .get("theme")
        .and_then(Value::as_str)
        .and_then(|t| THEME_BG.iter().find(|(name, _)| *name == t).map(|(name, _)| *name))
        .unwrap_or("paper")
}

/// 应用主题：深浅色开关 + 主窗口背景色。
pub fn apply_app_theme(app: &AppHandle, settings: &Value) {
    let theme = theme_of(settings);
    let color = theme_color(theme);
    let handle = app.clone();
    let dark = theme == "night";
    if let Err(e) = app.run_on_main_thread(move || {
        handle.set_theme(Some(if dark {
            tauri::Theme::Dark
        } else {
            tauri::Theme::Light
        }));
        if let Some(win) = handle.get_webview_window("main") {
            let _ = win.set_background_color(Some(color));
        }
    }) {
        eprintln!("应用主题背景色失败：{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn theme_colors_match_the_css_palette() {
        assert_eq!(theme_color("paper"), tauri::window::Color(0xe6, 0xeb, 0xe4, 255));
        assert_eq!(theme_color("green"), tauri::window::Color(0xdc, 0xe8, 0xd6, 255));
        assert_eq!(theme_color("night"), tauri::window::Color(0x12, 0x15, 0x16, 255));
        // 未知主题回退到 paper（原版 `THEME_WINDOW_BG[theme] || THEME_WINDOW_BG.paper`）
        assert_eq!(theme_color(""), tauri::window::Color(0xe6, 0xeb, 0xe4, 255));
    }

    #[test]
    fn settings_theme_lookup_is_case_sensitive_like_css_vars() {
        let settings = json!({ "theme": "NIGHT" });
        let theme = settings
            .get("theme")
            .and_then(Value::as_str)
            .filter(|t| THEME_BG.iter().any(|(name, _)| name == t))
            .unwrap_or("paper");
        assert_eq!(theme, "paper", "大写主题名不在 CSS 变量里，按 paper 兜底");
    }
}
