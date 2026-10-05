//! The rule activity service as startup builds it, over synthetic homes only:
//! reading never changes the source or the app database, and fixture mode
//! has no source at all.
use std::path::Path;
use xtrace_desktop::rule_activity::RuleActivityService;
use xtrace_desktop::state::{AppState, StartupOptions};

fn live(root: &Path) -> AppState {
    std::fs::create_dir_all(root.join("home")).unwrap();
    AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(root.join("home")),
            ..Default::default()
        },
        || panic!("default path must not be resolved"),
        || panic!("default home must not be resolved"),
    )
    .unwrap()
}

/// Every file under `directory` with its bytes and modification time.
fn snapshot(directory: &Path) -> Vec<(std::path::PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            found.extend(snapshot(&path));
        } else {
            found.push((
                path.clone(),
                std::fs::read(&path).unwrap(),
                metadata.modified().unwrap(),
            ));
        }
    }
    found.sort();
    found
}

#[test]
fn reading_changes_neither_the_source_nor_the_app_database() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    let home = state.native_home().unwrap().to_path_buf();
    let ledger = documented_root(&home).join("ledger");
    std::fs::create_dir_all(&ledger).unwrap();
    std::fs::write(ledger.join("schema_version"), b"2\n").unwrap();
    let now = jiff::Timestamp::now();
    let rows: String = (0..3)
        .map(|index| {
            format!(
                "{}\n",
                serde_json::json!({
                    "fire_id": format!("f-{index}"), "rule_id": "rule-a",
                    "rulebook_id": "book-1", "session_id": "native-1", "mode": "advise",
                    "fired_at": now.to_string(), "excerpt": "SECRET-EXCERPT",
                })
            )
        })
        .collect();
    std::fs::write(ledger.join("fires.jsonl"), rows).unwrap();

    let service = RuleActivityService::new(state.native_home());
    let before = snapshot(&home);
    let counts = serde_json::to_value(state.db_counts().unwrap()).unwrap();
    let database = snapshot(&root.path().join("data"));
    for index in 0..3 {
        let wire = serde_json::to_value(service.read(&format!("read-{index}"))).unwrap();
        assert_eq!(wire["state"], "loaded", "{wire}");
        assert_eq!(wire["counts"]["window_modes"]["advise"], 3);
        assert!(!wire.to_string().contains("SECRET-EXCERPT"));
        assert!(!wire.to_string().contains(home.to_str().unwrap()));
    }
    assert_eq!(snapshot(&home), before, "the source is read, never written");
    // The database files first, before anything here opens the database again.
    assert_eq!(snapshot(&root.path().join("data")), database);
    assert_eq!(
        serde_json::to_value(state.db_counts().unwrap()).unwrap(),
        counts
    );
    assert!(service.close());
    state.shutdown();
}

#[test]
fn a_home_without_a_rulebook_stays_without_one() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    let home = state.native_home().unwrap();
    let service = RuleActivityService::new(Some(home));
    assert_eq!(std::fs::read_dir(home).unwrap().count(), 0);
    let wire = serde_json::to_value(service.read("read-1")).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({
            "state": "unavailable", "source": "default_local_rulebook",
            "part": "root", "reason": "missing", "found_version": null
        })
    );
    assert_eq!(
        std::fs::read_dir(home).unwrap().count(),
        0,
        "no directory or marker is created"
    );
    state.shutdown();
}

/// Fixture startup has no native home, so the service it builds has no
/// source and never resolves the user's own.
#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn fixture_startup_has_no_rule_activity_source() {
    let root = tempfile::TempDir::new().unwrap();
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().to_owned()),
            fixture: Some("F1".into()),
            ..Default::default()
        },
        || panic!("fixture must not resolve live directory"),
        || panic!("fixture must not resolve the home directory"),
    )
    .unwrap();
    assert_eq!(state.native_home(), None);
    let service = RuleActivityService::new(state.native_home());
    assert_eq!(
        serde_json::to_value(service.read("read-1")).unwrap(),
        serde_json::json!({
            "state": "unavailable", "source": "default_local_rulebook",
            "part": "root", "reason": "not_configured", "found_version": null
        })
    );
    state.shutdown();
}

/// The documented default location, spelled out rather than borrowed from
/// the reader, so a changed default fails here.
fn documented_root(home: &Path) -> std::path::PathBuf {
    home.join(".config/memhub-plugin/rulebook")
}
