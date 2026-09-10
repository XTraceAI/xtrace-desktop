#[cfg(all(debug_assertions, feature = "fixtures"))]
use xtrace_desktop::dto::FixtureExport;
use xtrace_desktop::state::{AppState, StartupOptions, StateError};

#[test]
fn fixture_mode_data_dir_override_wins_and_live_store_persists() {
    let root = tempfile::TempDir::new().unwrap();
    let path = root.path().join("explicit");
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(path.clone()),
            fixture: None,
        },
        || panic!("default path must not be resolved"),
    )
    .unwrap();
    assert_eq!(state.app_info().data_dir, path.to_str().unwrap());
    assert_eq!(state.app_info().schema_version, 1);
    assert_eq!(state.app_info().fixture, None);
    assert!(!state.app_info().listening);
    assert_eq!(
        serde_json::to_value(state.db_counts().unwrap()).unwrap(),
        serde_json::json!({"sessions":0,"records":0,"usage":0})
    );
    state.shutdown();
    state.shutdown();
    assert!(matches!(state.db_counts(), Err(StateError::Closed)));
    assert!(path.join("xtrace.db").exists());
    AppState::build(StartupOptions::default(), || Ok(path)).unwrap();
}

#[test]
fn fixture_mode_options_have_explicit_precedence_and_validation() {
    let options =
        StartupOptions::parse(None, Some("F2".into()), ["--fixture".into(), "F1".into()]).unwrap();
    assert_eq!(options.fixture.as_deref(), Some("F1"));
    for args in [
        vec!["--fixture"],
        vec!["--fixture="],
        vec!["--fixture", "--fixture"],
        vec!["--fixture=F1", "--fixture=F2"],
    ] {
        assert!(StartupOptions::parse(None, None, args.into_iter().map(String::from)).is_err());
    }
    assert!(StartupOptions::parse(None, Some(String::new().into()), []).is_err());
}

#[cfg(not(all(debug_assertions, feature = "fixtures")))]
#[test]
fn fixture_mode_is_disabled_without_debug_feature() {
    let result = AppState::build(
        StartupOptions {
            data_dir: None,
            fixture: Some("F1".into()),
        },
        || panic!("fixture must not resolve live directory"),
    );
    assert!(matches!(result, Err(StateError::FixtureDisabled)));
}

#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn fixture_mode_builds_isolated_database_and_generated_export_parity() {
    let root = tempfile::TempDir::new().unwrap();
    let build = || {
        AppState::build(
            StartupOptions {
                data_dir: Some(root.path().into()),
                fixture: Some("F1".into()),
            },
            || panic!("fixture must not resolve live directory"),
        )
        .unwrap()
    };
    let first = build();
    let second = build();
    assert_ne!(first.app_info().data_dir, second.app_info().data_dir);
    let path = std::path::PathBuf::from(first.app_info().data_dir.clone());
    let mut app_info = first.app_info();
    assert_eq!(app_info.fixture.as_deref(), Some("F1"));
    app_info.data_dir = "fixture://F1".into();
    let exported = serde_json::to_value(FixtureExport {
        app_info,
        db_counts: first.db_counts().unwrap(),
    })
    .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("../../ui/fixtures/F1.json")).unwrap();
    assert_eq!(exported, expected);
    assert_eq!(
        exported["db_counts"],
        serde_json::json!({"sessions":1,"records":25,"usage":15})
    );
    first.shutdown();
    assert!(!path.exists());
    assert!(matches!(first.db_counts(), Err(StateError::Closed)));
    first.shutdown();
    assert!(std::path::Path::new(&second.app_info().data_dir).exists());
    let second_path = std::path::PathBuf::from(second.app_info().data_dir);
    drop(second);
    assert!(!second_path.exists());
}

#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn fixture_mode_rejects_skeletons_and_invalid_ids_without_live_reads() {
    for fixture in ["F2", "F20", "../F1", "F999"] {
        let result = AppState::build(
            StartupOptions {
                data_dir: None,
                fixture: Some(fixture.into()),
            },
            || panic!("fixture must not resolve live directory"),
        );
        assert!(matches!(result, Err(StateError::FixtureInvalid)));
    }
}

#[cfg(unix)]
#[test]
fn fixture_mode_non_unicode_selection_fails_before_live_startup() {
    use std::os::unix::ffi::OsStringExt;
    let value = std::ffi::OsString::from_vec(vec![0xff]);
    assert!(matches!(
        StartupOptions::parse(None, Some(value), []),
        Err(StateError::InvalidOption)
    ));
}
