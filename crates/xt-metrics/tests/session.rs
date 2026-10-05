//! Per-session M-02/M-04/M-05 over an explicit window, against the shared
//! fixtures. Every expectation is an exact value derived from the fixture
//! input, and each is cross-checked against the global report the Dashboard
//! already renders, so a per-session slice can never drift from the whole.
use jiff::Timestamp;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{MetricsDb, SessionWindow, TypingRate, Window};
use xt_store::CanonicalRecord;

fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
fn ms(ts: &str) -> i64 {
    ts.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
/// F2 declares its three overlapping sessions as a snapshot, not as canonical
/// session inputs; seed them the same way the existing span/sweep tests do.
fn f2_lanes() -> TempDb {
    let f = fixture("F2");
    let mut db = TempDb::empty().unwrap();
    for lane in f.snapshots()["spans"]["lanes"].as_array().unwrap() {
        let id = lane["session_id"].as_str().unwrap();
        let mut session = fixture("F1").sessions()[0].metadata.clone();
        session.session_id = id.into();
        db.store_mut().upsert_session(&session, false).unwrap();
        let records: Vec<CanonicalRecord> =
            serde_json::from_value(lane["records"].clone()).unwrap();
        db.store_mut().upsert_records(id, &records, false).unwrap();
    }
    db
}
fn value(window: &SessionWindow) -> serde_json::Value {
    serde_json::to_value(window).unwrap()
}

#[test]
fn f1_single_session_window_matches_its_golden_totals() {
    let f = fixture("F1");
    let id = f.sessions()[0].metadata.session_id.clone();
    let db = f.build_db(false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let measured = metrics.session_windows(window(), &[id.as_str()]).unwrap();
    // F1's goldens: five human messages, ten selected responses summing to
    // 1,100 tokens over four measured counters, and one 23-minute active span.
    assert_eq!(
        value(&measured[&id]),
        json!({
            "state": "indexed",
            "events": 25,
            "human_messages": 5,
            "tool_calls": 5,
            "agent_ms": 1_380_000,
            "tokens": {
                "selected_responses": 10,
                "measured_responses": 10,
                "sessions": 1,
                "measured_sessions": 1,
                "counters": {
                    "input_tokens": 750,
                    "output_tokens": 150,
                    "cache_read_tokens": 150,
                    "cache_creation_tokens": 50,
                    "total_tokens": 1100
                }
            }
        })
    );
    // The same rules the Dashboard reads, restricted to one session.
    let counts = metrics.counts(window(), TypingRate::default()).unwrap();
    let tokens = metrics.tokens(window(), jiff::tz::TimeZone::UTC).unwrap();
    let spans = metrics.active_spans(window()).unwrap();
    let SessionWindow::Indexed {
        events,
        human_messages,
        tool_calls,
        tokens: session_tokens,
        agent_ms,
    } = &measured[&id]
    else {
        panic!("F1's only session is indexed");
    };
    assert_eq!(*human_messages, counts.human_messages);
    assert_eq!(*tool_calls, counts.tool_calls);
    assert_eq!(*session_tokens, tokens.total);
    assert_eq!(*agent_ms, spans.active_ms);
    assert_eq!(u64::from(*events > 0), counts.sessions);
}

#[test]
fn f2_overlapping_sessions_partition_the_global_totals() {
    let db = f2_lanes();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let ids = ["f2-lane-a", "f2-lane-b", "f2-lane-c"];
    let measured = metrics.session_windows(window(), &ids).unwrap();
    // Lane A runs 12:00→12:50 with four human messages; B and C each run twenty
    // minutes with two. No lane records usage, so tokens stay unknown, not zero.
    let unmeasured = json!({
        "selected_responses": 0, "measured_responses": 0, "sessions": 0, "measured_sessions": 0,
        "counters": {"input_tokens": null, "output_tokens": null, "cache_read_tokens": null,
                     "cache_creation_tokens": null, "total_tokens": null}
    });
    for (id, events, humans, agent_ms) in [
        ("f2-lane-a", 6, 4, 3_000_000),
        ("f2-lane-b", 3, 2, 1_200_000),
        ("f2-lane-c", 3, 2, 1_200_000),
    ] {
        assert_eq!(
            value(&measured[id]),
            json!({
                "state": "indexed",
                "events": events,
                "human_messages": humans,
                // The lanes state no content blocks, so no tool count is known.
                "tool_calls": null,
                "agent_ms": agent_ms,
                "tokens": unmeasured
            }),
            "{id}"
        );
    }
    // Parallel sessions add by design (M-05); the fixture's own span snapshot
    // carries the same total.
    let global = metrics.active_spans(window()).unwrap();
    assert_eq!(global.active_ms, 3_000_000 + 1_200_000 + 1_200_000);
    assert_eq!(
        global.active_ms,
        fixture("F2").snapshots()["spans"]["active_ms"]
            .as_u64()
            .unwrap()
    );
    let counts = metrics.counts(window(), TypingRate::default()).unwrap();
    assert_eq!(counts.human_messages, Some(4 + 2 + 2));
}

#[test]
fn f3_window_boundary_and_empty_window_stay_distinct_from_a_missing_session() {
    let f = fixture("F3");
    let id = f.sessions()[0].metadata.session_id.clone();
    let db = f.build_db(false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    // Only the two in-window responses count (M-01); the earlier, the later and
    // the untimed one do not. Their instants are seven days apart, so the M-05
    // gap rule leaves two zero-length spans and no active time.
    assert_eq!(
        value(&metrics.session_windows(window(), &[id.as_str()]).unwrap()[&id]),
        json!({
            "state": "indexed",
            "events": 2,
            "human_messages": 0,
            "tool_calls": null,
            "agent_ms": 0,
            "tokens": {
                "selected_responses": 2,
                "measured_responses": 2,
                "sessions": 1,
                "measured_sessions": 1,
                "counters": {
                    "input_tokens": 50,
                    "output_tokens": 5,
                    "cache_read_tokens": 0,
                    "cache_creation_tokens": 0,
                    "total_tokens": 55
                }
            }
        })
    );
    // A window the session exists in but did nothing during is a measured zero
    // for events, human messages and agent time; M-04 has no selected response,
    // so its counters remain unknown rather than an invented zero.
    let quiet = Window::new(ms("2026-09-02T00:00:00Z"), ms("2026-09-03T00:00:00Z")).unwrap();
    assert_eq!(
        value(&metrics.session_windows(quiet, &[id.as_str()]).unwrap()[&id]),
        json!({
            "state": "indexed",
            "events": 0,
            "human_messages": 0,
            // Nothing in the window: a measured zero, not an unknown.
            "tool_calls": 0,
            "agent_ms": 0,
            "tokens": {
                "selected_responses": 0, "measured_responses": 0, "sessions": 0,
                "measured_sessions": 0,
                "counters": {"input_tokens": null, "output_tokens": null, "cache_read_tokens": null,
                             "cache_creation_tokens": null, "total_tokens": null}
            }
        })
    );
    // A session that is not indexed reports no measurement at all.
    let absent = "00000000-0000-4000-8000-00000000ffff";
    let mixed = metrics
        .session_windows(window(), &[id.as_str(), absent, absent])
        .unwrap();
    assert_eq!(mixed.len(), 2);
    assert_eq!(value(&mixed[absent]), json!({"state": "missing"}));
    assert!(matches!(mixed[&id], SessionWindow::Indexed { .. }));
}

#[test]
fn unclassified_records_keep_human_messages_unknown_without_hiding_other_measurements() {
    let f = fixture("F1");
    let id = f.sessions()[0].metadata.session_id.clone();
    let db = f.build_db(false).unwrap();
    {
        let connection = rusqlite::Connection::open(db.path()).unwrap();
        xt_store::timestamp::register_sqlite(&connection).unwrap();
        assert_eq!(
            connection
                .execute(
                    "UPDATE records SET is_human=NULL WHERE session_id=?1 AND type='user'
                     AND uuid=(SELECT min(uuid) FROM records WHERE session_id=?1 AND type='user')",
                    [&id],
                )
                .unwrap(),
            1
        );
    }
    let metrics = MetricsDb::open(db.path()).unwrap();
    let measured = metrics.session_windows(window(), &[id.as_str()]).unwrap();
    let SessionWindow::Indexed {
        human_messages,
        tokens,
        agent_ms,
        events,
        ..
    } = &measured[&id]
    else {
        panic!("the session stays indexed");
    };
    assert_eq!(*human_messages, None);
    assert_eq!(*events, 25);
    assert_eq!(*agent_ms, 1_380_000);
    assert_eq!(tokens.counters.total_tokens, Some(1100));
    assert_eq!(
        metrics
            .counts(window(), TypingRate::default())
            .unwrap()
            .human_messages,
        None
    );
}

#[test]
fn the_metadata_page_and_its_measurements_read_one_snapshot() {
    let db = f2_lanes();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (rows, measured) = metrics
        .read_snapshot(|db| {
            let rows = db.sessions_page("", None, None)?;
            let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
            let measured = db.session_windows(window(), &ids)?;
            Ok((rows, measured))
        })
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .all(|row| matches!(measured[&row.id], SessionWindow::Indexed { .. }))
    );
    // Search and host filtering remain the store's, and still select the rows
    // whose measurements this read returns.
    let filtered = metrics
        .sessions_page("lane-b", Some("claude"), None)
        .unwrap();
    assert_eq!(
        filtered
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["f2-lane-b"]
    );
    assert!(metrics.sessions_page("", Some("nope"), None).is_err());
}

#[test]
fn session_context_and_its_measurements_read_one_snapshot() {
    let f = fixture("F1");
    let id = f.sessions()[0].metadata.session_id.clone();
    let mut db = f.build_db(false).unwrap();
    // An indexed session carrying neither repository nor branch, beside F1's.
    let mut bare = f.sessions()[0].metadata.clone();
    bare.session_id = "bare-session".into();
    bare.cwd = None;
    bare.git_branch = None;
    db.store_mut().upsert_session(&bare, false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let ids = [id.as_str(), "bare-session", id.as_str(), "absent-session"];
    let (context, measured) = metrics
        .read_snapshot(|db| {
            Ok((
                db.session_context(&ids)?,
                db.session_windows(window(), &ids)?,
            ))
        })
        .unwrap();
    // Duplicates collapse; an identifier no indexed session owns is absent
    // from the context and explicitly missing from the measurements.
    assert_eq!(
        context
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        [id.as_str(), "bare-session"]
    );
    assert_eq!(measured["absent-session"], SessionWindow::Missing);
    let bare = context.iter().find(|row| row.id == "bare-session").unwrap();
    assert_eq!(
        (bare.host.as_str(), &bare.repo, &bare.branch),
        ("claude", &None, &None)
    );
    // The measured session's output is the same globally selected M-04 output
    // the whole-window token report carries for this one session.
    let SessionWindow::Indexed { tokens, .. } = &measured[&id] else {
        panic!("F1's session is indexed");
    };
    assert_eq!(
        tokens.counters.output_tokens,
        metrics
            .tokens(window(), jiff::tz::TimeZone::UTC)
            .unwrap()
            .total
            .counters
            .output_tokens
    );
    let many: Vec<String> = (0..=xt_metrics::MAX_SESSIONS)
        .map(|index| format!("s{index}"))
        .collect();
    let overflow: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!(metrics.session_context(&overflow).is_err());
    assert!(metrics.session_context(&overflow[1..]).is_ok());
    assert!(metrics.session_context(&[]).unwrap().is_empty());
}
