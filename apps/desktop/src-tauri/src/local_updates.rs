//! Release information for a native app with updates disabled. No installation.
use crate::state::StateError;

#[cfg(target_os = "macos")]
const PUBLIC_RELEASES_URL: &str = "https://github.com/XTraceAI/xtrace-desktop/releases";

pub fn require_main_window(label: &str) -> Result<(), String> {
    if label != crate::tray::MAIN_LABEL {
        Err("only the main window may view public releases".into())
    } else {
        Ok(())
    }
}

pub fn require_local_updates(fixtures: bool, updates_enabled: bool) -> Result<(), String> {
    if !fixtures && !updates_enabled {
        Ok(())
    } else {
        Err("release information controls are disabled in this build".into())
    }
}

pub fn open_public_releases() -> Result<(), StateError> {
    let status = public_releases_command()?
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| StateError::PublicReleasesUnavailable)?;
    if status.success() {
        Ok(())
    } else {
        Err(StateError::PublicReleasesUnavailable)
    }
}

#[cfg(target_os = "macos")]
fn public_releases_command() -> Result<std::process::Command, StateError> {
    let mut command = std::process::Command::new("/usr/bin/open");
    command.arg(PUBLIC_RELEASES_URL);
    Ok(command)
}

#[cfg(not(target_os = "macos"))]
fn public_releases_command() -> Result<std::process::Command, StateError> {
    Err(StateError::PublicReleasesUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_main_window_can_view_releases() {
        assert!(require_main_window("main").is_ok());
        for label in ["tray", "", "other"] {
            assert!(require_main_window(label).is_err());
        }
    }

    #[test]
    fn local_information_requires_nonfixture_mode_with_updates_disabled() {
        assert!(require_local_updates(false, false).is_ok());
        assert!(require_local_updates(false, true).is_err());
        assert!(require_local_updates(true, false).is_err());
        assert!(require_local_updates(true, true).is_err());
    }

    #[test]
    fn command_refuses_non_main_windows_before_launching() {
        use tauri::test::{INVOKE_KEY, mock_builder, mock_context, noop_assets};
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![crate::open_public_releases])
            .build(mock_context(noop_assets()))
            .unwrap();
        for label in ["tray", "other"] {
            let window = tauri::WebviewWindowBuilder::new(&app, label, Default::default())
                .build()
                .unwrap();
            let result = tauri::test::get_ipc_response(
                &window,
                tauri::webview::InvokeRequest {
                    cmd: "open_public_releases".into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: "tauri://localhost".parse().unwrap(),
                    body: tauri::ipc::InvokeBody::Json(serde_json::json!({
                        "url": "https://example.invalid/"
                    })),
                    headers: Default::default(),
                    invoke_key: INVOKE_KEY.to_string(),
                },
            );
            assert_eq!(
                result.err(),
                Some(serde_json::json!(
                    "only the main window may view public releases"
                ))
            );
        }
    }

    #[cfg(feature = "fixtures")]
    #[test]
    fn command_refuses_fixture_main_before_launching() {
        use tauri::test::{INVOKE_KEY, mock_builder, mock_context, noop_assets};
        let root = tempfile::TempDir::new().unwrap();
        let state = crate::state::AppState::build(
            crate::state::StartupOptions {
                data_dir: Some(root.path().to_owned()),
                fixture: Some("F1".into()),
                ..Default::default()
            },
            || panic!("fixture must not resolve live data"),
            || panic!("fixture must not resolve the home directory"),
        )
        .unwrap();
        let app = mock_builder()
            .manage(state)
            .invoke_handler(tauri::generate_handler![crate::open_public_releases])
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let result = tauri::test::get_ipc_response(
            &window,
            tauri::webview::InvokeRequest {
                cmd: "open_public_releases".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        assert_eq!(
            result.err(),
            Some(serde_json::json!(
                "release information controls are disabled in this build"
            ))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launcher_has_only_the_fixed_release_url() {
        let command = public_releases_command().unwrap();
        assert_eq!(command.get_program(), "/usr/bin/open");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            ["https://github.com/XTraceAI/xtrace-desktop/releases"]
        );
    }
}
