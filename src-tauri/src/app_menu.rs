//! 应用菜单与系统「关于」面板，对应原版 setupAppMenu / setupAboutPanel。
//!
//! 原版用 Electron 的 role 菜单项；这里 Tauri 的 PredefinedMenuItem 一一对应，
//! 只有「重新加载 / 强制重新加载 / 开发者工具 / 缩放」在 Tauri 里没有 role，改成自定义项 +
//! 菜单事件（见 commands.rs 的 on_menu_event）。中文标签与层级顺序保持原版一致。

use tauri::menu::{AboutMetadata, MenuBuilder, MenuItem, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Manager};

/// 与 `frontend/src/shared/about.ts` 保持一致（改动时同步）。
const APP_NAME: &str = "FlyReader";
const ABOUT_AUTHOR: &str = "飞鸟传说";
const ABOUT_QQ: &str = "17472439";
const ABOUT_EMAIL: &str = "17472439chaochao@gmail.com";
const ABOUT_REPO: &str = "https://gitee.com/wucc513721/flyreader";
const ABOUT_LICENSE: &str = "MIT";
const ABOUT_FEATURES: &str = "支持本地 TXT / EPUB / Markdown 文档阅读，在线小说搜索与阅读，以及导出功能。";
const ABOUT_OPEN_SOURCE: &str = "本软件为开源项目，采用 MIT 许可证发布。欢迎学习、使用与二次开发；请在衍生作品中保留原作者信息与开源声明。";
const ABOUT_COMPONENTS: [(&str, &str, &str); 11] = [
    ("Tauri 2", "跨平台桌面应用壳（系统 WebView）", "MIT"),
    ("React", "界面框架", "MIT"),
    ("TypeScript", "类型系统与编译", "Apache-2.0"),
    ("Vite", "开发构建工具链", "MIT"),
    ("Rust + rusqlite", "后端与本地 SQLite 数据存储", "MIT"),
    ("reqwest / scraper", "HTTP 与 HTML 解析（书源规则）", "MIT / Apache-2.0"),
    ("quickjs (rquickjs)", "@js 书源规则沙箱执行", "MIT"),
    ("encoding_rs", "网页编码转换", "MIT / Apache-2.0"),
    ("zip", "EPUB（ZIP）解压", "MIT"),
    ("marked", "Markdown 解析（GFM）", "MIT"),
    ("dompurify", "Markdown HTML 消毒", "MPL-2.0 / Apache-2.0"),
];

/// 自定义菜单项 id，commands.rs 的事件分发按这些常量匹配。
pub const ID_RELOAD: &str = "menu_reload";
pub const ID_FORCE_RELOAD: &str = "menu_force_reload";
/// 开发者工具项只在调试构建里存在，常量同样按调试门控（否则 release 构建报 dead_code）。
#[cfg(debug_assertions)]
pub const ID_DEVTOOLS: &str = "menu_devtools";
pub const ID_ZOOM_RESET: &str = "menu_zoom_reset";
pub const ID_ZOOM_IN: &str = "menu_zoom_in";
pub const ID_ZOOM_OUT: &str = "menu_zoom_out";

/// 网页缩放系数状态（Tauri 没有读取当前缩放的比例 API，只能自己记账）。
#[derive(Default)]
pub struct ZoomState(pub std::sync::Mutex<f64>);

const ZOOM_STEP: f64 = 1.2;
const ZOOM_MIN: f64 = 0.5;
const ZOOM_MAX: f64 = 2.0;

/// 按菜单项 id 调整缩放，返回新的系数。
pub fn apply_zoom(app: &AppHandle, id: &str) -> Option<f64> {
    let Some(win) = app.get_webview_window("main") else {
        return None;
    };
    let state = app.state::<ZoomState>();
    let mut guard = state.0.lock().ok()?;
    let current = if id == ID_ZOOM_RESET {
        1.0
    } else if id == ID_ZOOM_IN {
        (*guard * ZOOM_STEP).min(ZOOM_MAX)
    } else {
        (*guard / ZOOM_STEP).max(ZOOM_MIN)
    };
    *guard = current;
    let _ = win.set_zoom(current);
    Some(current)
}

/// 原版 setupAboutPanel 的 credits 文本（macOS 关于面板只支持纯文本）。
fn about_credits() -> String {
    let mut lines: Vec<String> = vec![
        format!("作者：{ABOUT_AUTHOR}"),
        format!("QQ：{ABOUT_QQ}"),
        format!("邮箱：{ABOUT_EMAIL}"),
        format!("仓库：{ABOUT_REPO}"),
        String::new(),
        ABOUT_FEATURES.to_string(),
        String::new(),
        ABOUT_OPEN_SOURCE.to_string(),
        String::new(),
        "主要开源组件：".to_string(),
    ];
    for (name, desc, license) in ABOUT_COMPONENTS {
        lines.push(format!("· {name}（{license}）— {desc}"));
    }
    lines.join("\n")
}

fn about_metadata(app: &AppHandle) -> AboutMetadata<'static> {
    let year = chrono::Utc::now().format("%Y").to_string();
    AboutMetadata {
        name: Some(APP_NAME.to_string()),
        version: Some(app.package_info().version.to_string()),
        short_version: None,
        authors: Some(vec![ABOUT_AUTHOR.to_string()]),
        comments: Some(ABOUT_FEATURES.to_string()),
        copyright: Some(format!("Copyright © {year} {ABOUT_AUTHOR}")),
        license: Some(ABOUT_LICENSE.to_string()),
        website: Some(ABOUT_REPO.to_string()),
        website_label: Some("Gitee 仓库".to_string()),
        credits: Some(about_credits()),
        icon: None,
    }
}

/// 构建并设置应用菜单。菜单事件由 commands.rs 的 `on_menu_event` 处理。
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let is_mac = cfg!(target_os = "macos");

    let mut view = SubmenuBuilder::new(app, "查看")
        .item(&MenuItem::with_id(
            app,
            ID_RELOAD,
            "重新加载",
            true,
            Some("CommandOrControl+R"),
        )?)
        .item(&MenuItem::with_id(
            app,
            ID_FORCE_RELOAD,
            "强制重新加载",
            true,
            Some("Shift+CommandOrControl+R"),
        )?);
    // 开发者工具只在调试构建里存在（原版 role 在生产环境同样拿不到 DevTools）
    #[cfg(debug_assertions)]
    {
        let accel = if is_mac {
            "Alt+CommandOrControl+I"
        } else {
            "F12"
        };
        view = view.item(&MenuItem::with_id(app, ID_DEVTOOLS, "开发者工具", true, Some(accel))?);
    }
    view = view
        .separator()
        .item(&MenuItem::with_id(
            app,
            ID_ZOOM_RESET,
            "实际大小",
            true,
            Some("CommandOrControl+0"),
        )?)
        .item(&MenuItem::with_id(
            app,
            ID_ZOOM_IN,
            "放大",
            true,
            Some("CommandOrControl+Equal"),
        )?)
        .item(&MenuItem::with_id(
            app,
            ID_ZOOM_OUT,
            "缩小",
            true,
            Some("CommandOrControl+Minus"),
        )?)
        .separator()
        .fullscreen_with_text("进入全屏幕");
    let view = view.build()?;

    let edit = SubmenuBuilder::new(app, "编辑")
        .undo_with_text("撤销")
        .redo_with_text("重做")
        .separator()
        .cut_with_text("剪切")
        .copy_with_text("复制")
        .paste_with_text("粘贴")
        .select_all_with_text("全选")
        .build()?;

    let mut window_builder = SubmenuBuilder::new(app, "窗口")
        .minimize_with_text("最小化")
        // Electron 的 zoom role 在 Tauri 里对应 maximize（绿灯缩放）
        .maximize_with_text("缩放");
    if is_mac {
        window_builder = window_builder
            .separator()
            .bring_all_to_front_with_text("全部置于顶层");
    } else {
        window_builder = window_builder.close_window_with_text("关闭");
    }
    let window = window_builder.build()?;

    let mut menu = MenuBuilder::new(app);
    if is_mac {
        // macOS 第一个子菜单即应用菜单，标题固定为应用名
        let about = PredefinedMenuItem::about(
            app,
            Some(&format!("关于 {APP_NAME}")),
            Some(about_metadata(app)),
        )?;
        let app_submenu = SubmenuBuilder::new(app, APP_NAME)
            .item(&about)
            .separator()
            .services_with_text("服务")
            .separator()
            .hide_with_text(&format!("隐藏 {APP_NAME}"))
            .hide_others_with_text("隐藏其他")
            .show_all_with_text("全部显示")
            .separator()
            .quit_with_text(&format!("退出 {APP_NAME}"))
            .build()?;
        menu = menu.item(&app_submenu);
    } else {
        // 原版在非 mac 上 unshift 一个「文件」菜单，只有退出项
        let file = SubmenuBuilder::new(app, "文件")
            .quit_with_text(&format!("退出 {APP_NAME}"))
            .build()?;
        menu = menu.item(&file);
    }
    menu = menu.item(&edit).item(&view).item(&window);
    if !is_mac {
        let help = SubmenuBuilder::new(app, "帮助")
            .about(Some(about_metadata(app)))
            .build()?;
        menu = menu.item(&help);
    }
    menu.build()?.set_as_app_menu()?;
    Ok(())
}
