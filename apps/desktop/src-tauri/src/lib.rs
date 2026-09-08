#[cfg(all(feature = "fixtures", not(debug_assertions)))]
compile_error!("fixtures feature is forbidden in release builds");

use tauri::Manager;
pub mod dto;
pub mod state;
mod window_controls;

#[tauri::command]
fn app_info(state: tauri::State<'_, state::AppState>) -> dto::AppInfo {
    state.app_info()
}

#[tauri::command]
fn db_counts(state: tauri::State<'_, state::AppState>) -> Result<dto::DbCounts, String> {
    state.db_counts().map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let options = state::StartupOptions::parse(
                std::env::var_os("XTRACE_DATA_DIR").map(Into::into),
                std::env::var_os("XTRACE_FIXTURE"),
                std::env::args().skip(1),
            )?;
            app.manage(state::AppState::build(options, || {
                app.path()
                    .app_data_dir()
                    .map_err(|_| state::StateError::InvalidOption)
            })?);
            Ok(())
        })
        .on_window_event(window_controls::on_window_event)
        .invoke_handler(tauri::generate_handler![app_info, db_counts])
        .run(tauri::generate_context!())
        .expect("Could not start XTrace Desktop");
}
