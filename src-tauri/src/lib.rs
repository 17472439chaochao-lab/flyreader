mod app_menu;
mod boss_key;
mod commands;
mod db;
pub mod engine;
mod local_books;
mod relevance;
mod store;
mod theme;
mod update_check;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = commands::build_app()
        .build(tauri::generate_context!())
        .expect("error while building FlyReader");

    app.run(|_app_handle, event| match event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows, ..
        } => {
            crate::boss_key::on_reopen(_app_handle, has_visible_windows);
        }
        _ => {}
    });
}
