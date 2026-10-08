//! Naming exactly one session, instead of searching for it.
//!
//! The detail screen is about one session, and the identity it holds is exact.
//! Reaching that row through the list's substring search was wrong in a way
//! that only shows up at scale: the search matches any identity that *contains*
//! the text, returns a bounded page of the matches, and orders them by time —
//! so a session whose identity is a substring of fifty others simply could not
//! be found. These cases pin the exact lookup, and the shapes that would make a
//! search plausible-looking and still wrong.
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use std::path::Path;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store};
use xtrace_desktop::{
    dto::{MetricSessionWindow, SessionRow},
    state::{AppState, StartupOptions},
};

/// The fixtures' clock, read once per test.
///
/// A selected window is `[now − days, now]` against the system clock at the
/// moment of the query, so a fixture written as a fixed calendar date is only
/// inside a window until that date falls out of it — a suite written that way
/// stops testing what it says and then starts failing on its own. Every
/// instant below is placed relative to one captured reading instead, with
/// margins measured in days, so neither a slow run nor the passage of time can
/// move a record across a boundary.
fn anchor() -> Timestamp {
    Timestamp::now()
}

fn ago(anchor: Timestamp, days: i64) -> String {
    (anchor - SignedDuration::from_hours(days * 24)).to_string()
}

/// Inside every window the app offers, and never in the future: six days clear
/// of the shortest window's start and a day clear of its end.
fn recent(anchor: Timestamp) -> String {
    ago(anchor, 1)
}

/// Outside the shortest window by thirteen days and inside the longest by ten,
/// so exactly one of the two windows counts it.
fn older(anchor: Timestamp) -> String {
    ago(anchor, 20)
}

fn record(uuid: &str, timestamp: &str) -> CanonicalRecord {
    serde_json::from_value(json!({
        "uuid": uuid, "type": "assistant", "timestamp": timestamp,
        "message": {"role": "assistant", "model": "test-claude", "content": []}
    }))
    .unwrap()
}

fn seed(store: &mut Store, id: &str, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
    session.cwd = Some("/Users/example/code/atlas".into());
    session.git_branch = Some("feat/detail".into());
    store.upsert_session(&session, false).unwrap();
    store.upsert_records(id, rows, false).unwrap();
}

fn state(root: &Path) -> AppState {
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

fn home(root: &Path) {
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
}

fn store(root: &Path) -> Store {
    Store::open(root.join("data/xtrace.db")).unwrap()
}

fn indexed(row: &SessionRow) -> (u64, u64) {
    match &row.metrics {
        MetricSessionWindow::Indexed {
            events, agent_ms, ..
        } => (*events, *agent_ms),
        MetricSessionWindow::Missing => panic!("a listed row is always indexed"),
    }
}

#[test]
fn a_session_named_among_many_that_merely_contain_its_identity_is_found() {
    // The failure the exact lookup exists for. `SESSION` is a substring of
    // every other identity here, and there are more of them than one page of
    // search results holds — so a search for it returns fifty other sessions
    // and not the one that was asked for.
    const SESSION: &str = "00000000-0000-4000-8000-00000000aaaa";
    let now = anchor();
    let root = tempfile::tempdir().unwrap();
    home(root.path());
    {
        let mut store = store(root.path());
        seed(
            &mut store,
            SESSION,
            &[record("11111111-1111-4111-8111-000000000000", &recent(now))],
        );
        for index in 0..60 {
            // Later in time than the target, so they fill the first page.
            seed(
                &mut store,
                &format!("{SESSION}-fork-{index:03}"),
                &[record(
                    &format!("22222222-2222-4222-8222-{index:012}"),
                    &recent(now),
                )],
            );
        }
    }
    let state = state(root.path());
    // The search the detail screen used to rely on: 51 rows, none of them the
    // session that was asked for.
    let searched = state.sessions_list(SESSION, None, None, 7).unwrap();
    assert_eq!(searched.rows.len(), 50);
    assert!(searched.next.is_some());
    assert!(
        !searched.rows.iter().any(|row| row.id == SESSION),
        "the search cannot reach the target, which is why the exact lookup exists"
    );
    // Named exactly, it is one row, and it is the right one.
    let row = state.session_row(SESSION, 7).unwrap().expect("the session");
    assert_eq!(row.id, SESSION);
    assert_eq!(row.repo.as_deref(), Some("/Users/example/code/atlas"));
    assert_eq!(row.branch.as_deref(), Some("feat/detail"));
    assert_eq!(row.record_count, 1);
}

#[test]
fn a_prefix_of_another_identity_is_a_different_session() {
    let now = anchor();
    let root = tempfile::tempdir().unwrap();
    home(root.path());
    {
        let mut store = store(root.path());
        seed(
            &mut store,
            "session-one-extended",
            &[record("11111111-1111-4111-8111-000000000000", &recent(now))],
        );
    }
    let state = state(root.path());
    // Exact, not "starts with" and not "contains".
    assert!(state.session_row("session-one", 7).unwrap().is_none());
    assert!(state.session_row("one-extended", 7).unwrap().is_none());
    assert!(
        state
            .session_row("SESSION-ONE-EXTENDED", 7)
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .session_row("session-one-extended", 7)
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_session_this_index_does_not_hold_is_nothing_rather_than_an_empty_row() {
    let now = anchor();
    let root = tempfile::tempdir().unwrap();
    home(root.path());
    {
        let mut store = store(root.path());
        seed(
            &mut store,
            "session-one",
            &[record("11111111-1111-4111-8111-000000000000", &recent(now))],
        );
    }
    let state = state(root.path());
    // Nothing, so a caller says the index holds no such session rather than
    // showing a row of zeroes for one.
    assert!(state.session_row("no-such-session", 7).unwrap().is_none());
    assert!(state.session_row("", 7).unwrap().is_none());
}

#[test]
fn the_selected_window_is_the_window_the_row_reports() {
    // One session with work inside every window and work only inside the
    // longest, so the numbers must differ by the window that was asked for.
    const SESSION: &str = "00000000-0000-4000-8000-00000000bbbb";
    let now = anchor();
    let root = tempfile::tempdir().unwrap();
    home(root.path());
    {
        let mut store = store(root.path());
        seed(
            &mut store,
            SESSION,
            &[
                record("11111111-1111-4111-8111-000000000000", &recent(now)),
                record("11111111-1111-4111-8111-000000000001", &older(now)),
            ],
        );
    }
    let state = state(root.path());
    let inside = indexed(&state.session_row(SESSION, 7).unwrap().unwrap());
    let both = indexed(&state.session_row(SESSION, 30).unwrap().unwrap());
    // Exactly which records each window counts, not merely that one counts
    // more: an assertion that only compares the two would pass just as well
    // on a pair of windows that both counted nothing.
    assert_eq!(
        (inside.0, both.0),
        (1, 2),
        "the shorter window counts only the recent work: {inside:?} vs {both:?}"
    );
    // The metadata is the session's own and does not move with the window.
    for days in [7, 14, 30] {
        let row = state.session_row(SESSION, days).unwrap().unwrap();
        assert_eq!(row.record_count, 2);
        assert_eq!(row.id, SESSION);
    }
    // An unsupported window is refused rather than quietly answered.
    assert!(state.session_row(SESSION, 99).is_err());
}

#[test]
fn the_exact_row_and_the_listed_row_agree() {
    // A detail screen and the list it was opened from must not disagree about
    // what a session measured: same read, same rules, same snapshot.
    const SESSION: &str = "00000000-0000-4000-8000-00000000cccc";
    let now = anchor();
    let root = tempfile::tempdir().unwrap();
    home(root.path());
    {
        let mut store = store(root.path());
        seed(
            &mut store,
            SESSION,
            &[
                record("11111111-1111-4111-8111-000000000000", &recent(now)),
                record("11111111-1111-4111-8111-000000000001", &recent(now)),
            ],
        );
    }
    let state = state(root.path());
    for days in [7, 14, 30] {
        let listed = state.sessions_list(SESSION, None, None, days).unwrap();
        let from_list = listed.rows.iter().find(|row| row.id == SESSION).unwrap();
        let exact = state.session_row(SESSION, days).unwrap().unwrap();
        assert_eq!(&exact, from_list, "over {days} days");
    }
}

#[test]
fn an_identity_that_differs_only_in_whitespace_is_a_different_session() {
    let now = anchor();
    let root = tempfile::tempdir().unwrap();
    home(root.path());
    {
        let mut store = store(root.path());
        seed(
            &mut store,
            "session-ws",
            &[record("11111111-1111-4111-8111-000000000000", &recent(now))],
        );
        seed(
            &mut store,
            " session-ws",
            &[record("11111111-1111-4111-8111-000000000001", &recent(now))],
        );
    }
    let state = state(root.path());
    assert_eq!(
        state.session_row("session-ws", 7).unwrap().unwrap().id,
        "session-ws"
    );
    assert_eq!(
        state.session_row(" session-ws", 7).unwrap().unwrap().id,
        " session-ws"
    );
}

#[test]
fn whole_session_cost_matches_dashboard_and_never_shrinks_with_the_window() {
    let fixture = xt_fixtures::Fixture::load(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/F1"),
    )
    .unwrap();
    let catalog =
        xt_metrics::PriceCatalog::from_json(&fixture.snapshots()["prices"].to_string()).unwrap();
    let mut db = xt_fixtures::TempDb::empty().unwrap();
    let now = fixture.now().timestamp_millis();
    let response = |id: &str, timestamp: &str, model: &str| {
        serde_json::from_value(json!({"uuid": id, "type": "assistant", "timestamp": timestamp,
            "message": {"model": model, "usage": {"input_tokens": 0, "output_tokens": 1,
            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0, "service_tier": "standard"}}})).unwrap()
    };
    seed(
        db.store_mut(),
        "whole-cost",
        &[
            response("old", "2020-01-01T00:00:00Z", "test-flat"),
            response("new", "2026-09-07T12:00:00Z", "test-flat"),
            response("unknown", "2026-09-07T12:01:00Z", "not-in-catalog"),
        ],
    );
    let metrics = xt_metrics::MetricsDb::open(db.path()).unwrap();
    let reports = xtrace_desktop::dashboard::fixture_reports(
        db.path(),
        now,
        fixture.snapshots().get("prices"),
    )
    .unwrap();
    let dashboard_cost = &reports[0]
        .lane_sessions
        .iter()
        .find(|row| row.session_id == "whole-cost")
        .unwrap()
        .cost;
    for days in [7, 14, 30] {
        let page = xtrace_desktop::dto::session_page(
            &metrics,
            days,
            now,
            jiff::tz::TimeZone::UTC,
            xtrace_desktop::dto::MetricClock::Fixture,
            Default::default(),
            &catalog,
        )
        .unwrap();
        let listed = page.rows.iter().find(|row| row.id == "whole-cost").unwrap();
        let exact = xtrace_desktop::dto::session_row(&metrics, days, now, "whole-cost", &catalog)
            .unwrap()
            .unwrap();
        assert_eq!(&listed.cost, dashboard_cost);
        assert_eq!(listed.cost, exact.cost);
        let cost = listed.cost.as_ref().unwrap();
        assert_eq!(
            (
                cost.selected_observations,
                cost.priced_observations,
                cost.unpriced_observations
            ),
            (3, 2, 1)
        );
        assert_eq!(cost.total_usd, None);
        assert_eq!(cost.priced_subtotal_usd, 0.00002);
    }
}
