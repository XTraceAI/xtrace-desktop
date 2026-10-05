use crate::update_config;

fn enabled(debug: bool, fixtures: bool, macos: bool, configured: bool) -> bool {
    !debug && !fixtures && macos && configured
}

fn configured(app: &tauri::AppHandle) -> bool {
    let config = app.config().plugins.0.get("updater");
    update_config::validate(config) == Ok(true)
        && config.is_some_and(|value| {
            serde_json::from_value::<tauri_plugin_updater::Config>(value.clone()).is_ok()
        })
}

#[tauri::command]
pub fn updates_enabled(app: tauri::AppHandle) -> bool {
    enabled(
        cfg!(debug_assertions),
        cfg!(feature = "fixtures"),
        cfg!(target_os = "macos"),
        configured(&app),
    )
}

pub fn setup(app: &tauri::AppHandle) -> tauri::Result<()> {
    if updates_enabled(app.clone()) {
        app.plugin(tauri_plugin_updater::Builder::new().build())?;
    }
    Ok(())
}

#[tauri::command]
pub fn restart_after_update(app: tauri::AppHandle) -> Result<(), &'static str> {
    if !updates_enabled(app.clone()) {
        return Err("updates are disabled in this build");
    }
    // Unlike restart() on the main thread, this requests the normal Exit path.
    // Existing worker and database cleanup stays in the app's RunEvent::Exit.
    app.request_restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_require_configured_supported_release() {
        assert!(enabled(false, false, true, true));
        assert!(!enabled(true, false, true, true));
        assert!(!enabled(false, true, true, true));
        assert!(!enabled(false, false, false, true));
        assert!(!enabled(false, false, true, false));
    }

    #[test]
    fn conflicting_signed_version_alias_cannot_enable_updates() {
        let config = serde_json::json!({"pubkey":"unused", "endpoints":["https://updates.example/latest.json"], "requireSignedVersion":true, "require-signed-version":false});
        assert!(serde_json::from_value::<tauri_plugin_updater::Config>(config).is_err());
    }
}
