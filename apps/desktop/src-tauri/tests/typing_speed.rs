//! The saved typing speed through the application state: default, bounds,
//! persistence across a restart, and the one rate the Dashboard reads for both
//! its current and its previous window.
use jiff::Timestamp;
use serde_json::json;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};
use xtrace_desktop::{
    dto::DashboardMetrics,
    state::{AppState, StartupOptions},
};

const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

fn live(root: &std::path::Path) -> AppState {
    std::fs::create_dir_all(root.join("home")).unwrap();
    AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(root.join("home")),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap()
}

fn at(ms: i64) -> String {
    Timestamp::from_millisecond(ms).unwrap().to_string()
}

fn event(id: &str, ms: i64, human: bool, chars: usize) -> CanonicalRecord {
    let (kind, role) = if human {
        ("user", "user")
    } else {
        ("assistant", "assistant")
    };
    serde_json::from_value(json!({"uuid":id,"type":kind,"timestamp":at(ms),
        "message":{"role":role,"model":"test-claude",
            "content":[{"type":"text","text":"x".repeat(chars)}]}}))
    .unwrap()
}

/// Synthetic sessions relative to the live clock:
/// - `solo`: one human message with no earlier agent event in either window,
///   400 characters now and 800 characters in the previous 7-day window, so
///   M-07 uses the counted lengths in both windows;
/// - `reply`: an agent event and, ten minutes later, a long human message,
///   whose estimate also follows its counted length at every speed.
fn seed(root: &std::path::Path) {
    let now = Timestamp::now().as_millisecond();
    let mut store = Store::open(root.join("data/xtrace.db")).unwrap();
    let sessions: [(&str, Vec<CanonicalRecord>); 2] = [
        (
            "solo",
            vec![
                event("solo-now", now - DAY_MS, true, 400),
                event("solo-before", now - 8 * DAY_MS, true, 800),
            ],
        ),
        (
            "reply",
            vec![
                event("reply-agent", now - 2 * DAY_MS, false, 1),
                event("reply-human", now - 2 * DAY_MS + 10 * MINUTE_MS, true, 4000),
            ],
        ),
    ];
    for (id, records) in sessions {
        let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
        session.surface = Some("cli".into());
        store.upsert_session(&session, false).unwrap();
        store.upsert_records(id, &records, false).unwrap();
    }
}

fn human_minutes(report: &DashboardMetrics) -> (f64, f64) {
    let tile = &report.tiles.human_hours_est;
    (
        tile.value.unwrap() * 60.0,
        tile.delta.previous.unwrap() * 60.0,
    )
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "{actual} is not {expected}"
    );
}

#[test]
fn a_fresh_database_reads_forty_and_refuses_out_of_bounds_speeds() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    assert_eq!(state.typing_speed().unwrap(), 40);
    for wpm in [0, 301, 1_000, u32::MAX] {
        let refused = state.set_typing_speed(wpm).unwrap_err().to_string();
        assert_eq!(
            refused, "typing speed must be a whole number from 1 to 300 words per minute",
            "{wpm}"
        );
        assert_eq!(state.typing_speed().unwrap(), 40, "{wpm} left no trace");
    }
    for wpm in [1, 300, 40] {
        assert_eq!(state.set_typing_speed(wpm).unwrap(), wpm);
        assert_eq!(state.typing_speed().unwrap(), wpm);
    }
    state.shutdown();
}

#[test]
fn a_saved_speed_survives_restart_and_both_windows_read_it() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    seed(root.path());
    let before = state.metrics_dashboard(7).unwrap();
    // 40 WPM = 200 cpm: 400 chars is 2 min, 800 chars 4 min; the 4,000-character reply is 20.
    let (current, previous) = human_minutes(&before);
    close(current, 22.0);
    close(previous, 4.0);

    assert_eq!(state.set_typing_speed(80).unwrap(), 80);
    let after = state.metrics_dashboard(7).unwrap();
    // Every counted length halves its estimate in both windows.
    let (current, previous) = human_minutes(&after);
    close(current, 11.0);
    close(previous, 2.0);
    let tile = &after.tiles.human_hours_est;
    assert_eq!(
        (tile.current_n, tile.previous_n),
        (
            before.tiles.human_hours_est.current_n,
            before.tiles.human_hours_est.previous_n
        )
    );
    // Agent time, messages and usage do not read the speed.
    assert_eq!(
        serde_json::to_value(&after.tiles.agent_hours).unwrap(),
        serde_json::to_value(&before.tiles.agent_hours).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&after.tiles.human_messages).unwrap(),
        serde_json::to_value(&before.tiles.human_messages).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&after.tiles.tokens).unwrap(),
        serde_json::to_value(&before.tiles.tokens).unwrap()
    );
    // Leverage divides agent hours by your hours, which come from message
    // times, not typing: the speed does not move it.
    close(before.tiles.agent_hours.value.unwrap() * 60.0, 10.0);
    assert_eq!(
        serde_json::to_value(&after.tiles.leverage).unwrap(),
        serde_json::to_value(&before.tiles.leverage).unwrap()
    );
    // The daily human bars sum to the same character estimate the tile reports.
    let daily: f64 = after
        .days
        .iter()
        .filter_map(|day| day.human_hours_est)
        .sum::<f64>()
        * 60.0;
    close(daily, 11.0);
    // Every range reads the same speed; the longer ranges also hold the
    // 800-character message, now 2 minutes.
    for days in [14, 30] {
        close(
            human_minutes(&state.metrics_dashboard(days).unwrap()).0,
            13.0,
        );
    }

    state.shutdown();
    let restarted = live(root.path());
    assert_eq!(restarted.typing_speed().unwrap(), 80);
    let (current, previous) = human_minutes(&restarted.metrics_dashboard(7).unwrap());
    close(current, 11.0);
    close(previous, 2.0);
    // Resetting to the default restores the existing numbers exactly.
    assert_eq!(restarted.set_typing_speed(40).unwrap(), 40);
    let reset = restarted.metrics_dashboard(7).unwrap();
    assert_eq!(
        serde_json::to_value(&reset.tiles.human_hours_est).unwrap(),
        serde_json::to_value(&before.tiles.human_hours_est).unwrap()
    );
    restarted.shutdown();
}

#[test]
fn a_refused_save_leaves_the_dashboard_unchanged() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    seed(root.path());
    state.set_typing_speed(60).unwrap();
    let saved = serde_json::to_value(state.metrics_dashboard(7).unwrap().tiles).unwrap();
    assert!(state.set_typing_speed(0).is_err());
    assert!(state.set_typing_speed(301).is_err());
    assert_eq!(state.typing_speed().unwrap(), 60);
    assert_eq!(
        serde_json::to_value(state.metrics_dashboard(7).unwrap().tiles).unwrap(),
        saved
    );
    state.shutdown();
}

#[test]
fn stored_invalid_data_fails_the_dashboard_visibly_until_repaired() {
    let root = tempfile::TempDir::new().unwrap();
    let state = live(root.path());
    seed(root.path());
    let expected = serde_json::to_value(state.metrics_dashboard(7).unwrap().tiles).unwrap();
    let raw = rusqlite::Connection::open(root.path().join("data/xtrace.db")).unwrap();
    for value in ["0", "301", "40.5", "\"40\"", "null"] {
        raw.execute(
            "INSERT INTO settings(key,value_json) VALUES('metrics.typing_speed_wpm',?1)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
            [value],
        )
        .unwrap();
        assert!(state.typing_speed().is_err(), "{value}");
        assert!(state.metrics_dashboard(7).is_err(), "{value}");
        // Reads that do not use the speed are unaffected.
        assert!(state.today().is_ok(), "{value}");
        assert!(state.metrics_environment(7).is_ok(), "{value}");
    }
    assert_eq!(state.set_typing_speed(40).unwrap(), 40);
    assert_eq!(
        serde_json::to_value(state.metrics_dashboard(7).unwrap().tiles).unwrap(),
        expected
    );
    state.shutdown();
}

#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn fixture_mode_starts_at_the_default_and_matches_the_export() {
    let fixture = |root: &std::path::Path| {
        AppState::build(
            StartupOptions {
                data_dir: Some(root.to_owned()),
                fixture: Some("F1".into()),
                ..Default::default()
            },
            || panic!("fixture must not resolve live directory"),
            || panic!("fixture must not resolve the home directory"),
        )
        .unwrap()
    };
    // Exact bytes, never reparsed floats. `fixture_mode.rs` holds the export
    // to these same default reads byte for byte.
    let pretty = |report: &DashboardMetrics| serde_json::to_string_pretty(report).unwrap();
    let root = tempfile::TempDir::new().unwrap();
    let state = fixture(root.path());
    assert_eq!(state.typing_speed().unwrap(), 40);
    let default = state.metrics_dashboard(7).unwrap();
    // The disposable database takes a speed like any other.
    assert_eq!(state.set_typing_speed(120).unwrap(), 120);
    let faster = state.metrics_dashboard(7).unwrap();
    assert_eq!(
        serde_json::to_string(&faster.tiles.agent_hours).unwrap(),
        serde_json::to_string(&default.tiles.agent_hours).unwrap()
    );
    // Back at the default, every number is the export's again.
    assert_eq!(state.set_typing_speed(40).unwrap(), 40);
    assert_eq!(
        pretty(&state.metrics_dashboard(7).unwrap()),
        pretty(&default)
    );
    state.shutdown();
    // A new fixture database starts from the default again.
    let root = tempfile::TempDir::new().unwrap();
    let state = fixture(root.path());
    assert_eq!(state.typing_speed().unwrap(), 40);
    state.shutdown();
}
