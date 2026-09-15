#[cfg(all(feature = "fixtures", not(debug_assertions)))]
compile_error!("fixtures feature is forbidden in release builds");

use std::sync::Arc;
use tauri::{Emitter, Manager};
pub mod dto;
pub mod native_index;
pub mod state;
mod window_controls;

/// The event carrying every native index status change to the frontend.
pub const NATIVE_INDEX_EVENT: &str = "native-index://status";

#[tauri::command]
fn app_info(state: tauri::State<'_, state::AppState>) -> dto::AppInfo {
    state.app_info()
}

#[tauri::command]
fn db_counts(state: tauri::State<'_, state::AppState>) -> Result<dto::DbCounts, String> {
    state.db_counts().map_err(|error| error.to_string())
}

#[tauri::command]
fn native_index_status(
    index: tauri::State<'_, native_index::NativeIndex>,
) -> dto::NativeIndexStatus {
    index.status()
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
            )?
            .with_native_environment(
                std::env::var_os("XTRACE_NATIVE_HOME"),
                std::env::var_os("XTRACE_PYTHON"),
            )?;
            let native_home = options.native_home.clone();
            let python = options.python.clone();
            let state = state::AppState::build(options, || {
                app.path()
                    .app_data_dir()
                    .map_err(|_| state::StateError::InvalidOption)
            })?;
            let publish: native_index::Publish = {
                let handle = app.handle().clone();
                Arc::new(move |status: &dto::NativeIndexStatus| {
                    // A window not open yet, or already gone, drops the event;
                    // the status remains queryable.
                    let _ = handle.emit(NATIVE_INDEX_EVENT, status);
                })
            };
            let index = match state.database_path() {
                // Fixture startup never selects live data: nothing is indexed.
                None => native_index::NativeIndex::disabled(
                    "fixture mode uses a disposable database",
                    publish,
                ),
                Some(db) => native_index::NativeIndex::start(
                    native_index::NativeIndexOptions {
                        home: match native_home {
                            Some(home) => home,
                            None => app
                                .path()
                                .home_dir()
                                .map_err(|_| state::StateError::InvalidOption)?,
                        },
                        db: db.to_path_buf(),
                        bundle: app
                            .path()
                            .resource_dir()
                            .map_err(|_| state::StateError::InvalidOption)?
                            .join(native_index::BUNDLE_RESOURCE),
                        python,
                    },
                    publish,
                ),
            };
            app.manage(state);
            app.manage(index);
            Ok(())
        })
        .on_window_event(window_controls::on_window_event)
        .invoke_handler(tauri::generate_handler![
            app_info,
            db_counts,
            native_index_status
        ])
        .build(tauri::generate_context!())
        .expect("Could not start XTrace Desktop")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                // The index stops before the database closes: its scan is
                // cancelled, a running reader killed and reaped, within a bound.
                if let Some(index) = app.try_state::<native_index::NativeIndex>() {
                    index.shutdown();
                }
                if let Some(state) = app.try_state::<state::AppState>() {
                    state.shutdown();
                }
            }
        });
}
