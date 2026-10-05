//! The Sessions list's per-row tool calls and M-09 hands-off median, batched
//! over a page. Each is cross-checked against the rule it reuses: tools against
//! the Dashboard's global tool-call sum (`counts`), hands-off against the per-session detail
//! stretches and the shared median, so a row can never drift from either.
use jiff::Timestamp;
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{MetricsDb, SessionHandsOff, SessionStretches, SessionWindow, TypingRate, Window};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource};

fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn seed(db: &mut TempDb, id: &str, host: &str, surface: Option<&str>, rows: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, host, SessionSource::Fixture);
    session.surface = surface.map(str::to_owned);
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, rows, false).unwrap();
}
fn record(id: &str, ts: &str, human: bool, tools: usize) -> CanonicalRecord {
    let role = if human { "user" } else { "assistant" };
    let content: Vec<_> = if tools == 0 {
        vec![json!({"type":"text","text":"Synthetic"})]
    } else {
        (0..tools)
            .map(|_| json!({"type":"tool_use","name":"Read","input":{"path":"synthetic"}}))
            .collect()
    };
    serde_json::from_value(json!({"uuid":id,"type":role,"timestamp":ts,
        "message":{"role":role,"content":content}}))
    .unwrap()
}
fn tools(measured: &SessionWindow) -> Option<u64> {
    match measured {
        SessionWindow::Indexed { tool_calls, .. } => *tool_calls,
        SessionWindow::Missing => panic!("indexed"),
    }
}
/// The shared median, restated independently so parity cannot borrow the
/// implementation's arithmetic.
fn median(values: &mut [u64]) -> f64 {
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] as f64 + values[middle] as f64) / 2.0
    } else {
        values[middle] as f64
    }
}

#[test]
fn tools_are_the_global_m03_sum_sliced_by_session_with_unknown_and_zero_distinct() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "busy",
        "claude",
        Some("cli"),
        &[
            record("b-h", "2026-09-07T12:00:00Z", true, 0),
            record("b-a1", "2026-09-07T12:01:00Z", false, 2),
            record("b-a2", "2026-09-07T12:02:00Z", false, 3),
            // Outside the window: never counted.
            record("b-old", "2026-08-01T12:00:00Z", false, 7),
        ],
    );
    seed(
        &mut db,
        "idle",
        "claude",
        Some("cli"),
        &[
            record("i-h", "2026-09-07T13:00:00Z", true, 0),
            record("i-a", "2026-09-07T13:01:00Z", false, 0),
        ],
    );
    seed(
        &mut db,
        "unknown",
        "codex",
        None,
        &[
            record("u-h", "2026-09-07T14:00:00Z", true, 0),
            record("u-a", "2026-09-07T14:01:00Z", false, 4),
        ],
    );
    seed(&mut db, "empty", "cursor", None, &[]);
    let metrics = MetricsDb::open(db.path()).unwrap();
    let ids = ["busy", "idle", "empty", "unknown"];
    let measured = metrics.session_windows(window(), &ids).unwrap();
    assert_eq!(tools(&measured["busy"]), Some(5));
    // A measured zero stays zero, both with records and with none in window.
    assert_eq!(tools(&measured["idle"]), Some(0));
    assert_eq!(tools(&measured["empty"]), Some(0));
    assert_eq!(tools(&measured["unknown"]), Some(4));
    // The rows partition the Dashboard's own tool-call total.
    let global = metrics.counts(window(), TypingRate::default()).unwrap();
    assert_eq!(global.tool_calls, Some(9));

    // An unknown stated count makes that session unknown, and only that one.
    let c = Connection::open(db.path()).unwrap();
    c.execute(
        "UPDATE records SET tool_use_count=NULL WHERE uuid='u-a'",
        [],
    )
    .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let measured = metrics.session_windows(window(), &ids).unwrap();
    assert_eq!(tools(&measured["unknown"]), None);
    assert_eq!(tools(&measured["busy"]), Some(5));
    assert_eq!(
        metrics
            .counts(window(), TypingRate::default())
            .unwrap()
            .tool_calls,
        None
    );

    // A copied record inherited by a fork is counted once, under its owner.
    c.execute(
        "INSERT INTO native_record_copies(session_id,record_uuid) VALUES('idle','b-a1'),('idle','b-a2')",
        [],
    )
    .unwrap();
    let measured = MetricsDb::open(db.path())
        .unwrap()
        .session_windows(window(), &ids)
        .unwrap();
    assert_eq!(tools(&measured["busy"]), Some(5));
    assert_eq!(tools(&measured["idle"]), Some(0));
}

#[test]
fn f1_tools_match_the_global_count() {
    let f = fixture("F1");
    let id = f.sessions()[0].metadata.session_id.clone();
    let db = f.build_db(false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let measured = metrics.session_windows(window(), &[id.as_str()]).unwrap();
    assert_eq!(
        tools(&measured[&id]),
        metrics
            .counts(window(), TypingRate::default())
            .unwrap()
            .tool_calls
    );
    assert_eq!(tools(&measured[&id]), Some(5));
}

#[test]
fn hands_off_is_the_median_of_the_detail_stretches_with_n() {
    let mut db = TempDb::empty().unwrap();
    // Three stretches of 1, 4 and 2 minutes, then one of 6: median 3.
    let mut rows = Vec::new();
    for (turn, (start, end)) in [
        ("12:00", "12:01"),
        ("12:10", "12:14"),
        ("12:20", "12:22"),
        ("12:30", "12:36"),
    ]
    .iter()
    .enumerate()
    {
        rows.push(record(
            &format!("h{turn}"),
            &format!("2026-09-07T{start}:00Z"),
            true,
            0,
        ));
        rows.push(record(
            &format!("a{turn}"),
            &format!("2026-09-07T{end}:00Z"),
            false,
            1,
        ));
    }
    seed(&mut db, "four", "claude", Some("cli"), &rows);
    // Records in window but no stretch that called a tool: measured, n=0.
    seed(
        &mut db,
        "talk",
        "claude",
        Some("cli"),
        &[
            record("t-h", "2026-09-07T15:00:00Z", true, 0),
            record("t-a", "2026-09-07T15:05:00Z", false, 0),
        ],
    );
    // Nothing in the window at all: also measured, n=0 and no median.
    seed(
        &mut db,
        "quiet",
        "claude",
        Some("cli"),
        &[
            record("q-h", "2026-08-01T15:00:00Z", true, 0),
            record("q-a", "2026-08-01T15:05:00Z", false, 1),
        ],
    );
    seed(
        &mut db,
        "blind",
        "claude",
        Some("cli"),
        &[
            record("x-h", "2026-09-07T16:00:00Z", true, 0),
            record("x-a", "2026-09-07T16:05:00Z", false, 1),
        ],
    );
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE records SET is_human=NULL WHERE uuid='x-a'", [])
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let ids = ["four", "talk", "quiet", "blind", "absent", "four"];
    let answers = metrics.session_hands_off(window(), &ids).unwrap();
    assert_eq!(answers.len(), 5);
    let SessionStretches::Measured { stretches } =
        metrics.session_stretches(window(), "four").unwrap()
    else {
        panic!("measured");
    };
    let mut durations: Vec<u64> = stretches.iter().map(|s| s.duration_ms).collect();
    assert_eq!(
        answers["four"],
        SessionHandsOff::Measured {
            n: 4,
            median_min: Some(median(&mut durations) / 60000.0)
        }
    );
    assert_eq!(
        serde_json::to_value(&answers["four"]).unwrap(),
        json!({"state":"measured","n":4,"median_min":3.0})
    );
    for empty in ["talk", "quiet"] {
        assert_eq!(
            serde_json::to_value(&answers[empty]).unwrap(),
            json!({"state":"measured","n":0,"median_min":null}),
            "{empty}"
        );
    }
    assert_eq!(
        answers["blind"],
        SessionHandsOff::Unmeasured {
            excluded_surface: None
        }
    );
    assert_eq!(answers["absent"], SessionHandsOff::Missing);
    // Every state agrees with the detail read of the same session.
    for id in ["talk", "quiet", "blind", "absent"] {
        let detail = metrics.session_stretches(window(), id).unwrap();
        let agrees = match (&answers[id], &detail) {
            (SessionHandsOff::Missing, SessionStretches::Missing) => true,
            (
                SessionHandsOff::Unmeasured {
                    excluded_surface: a,
                },
                SessionStretches::Unmeasured {
                    excluded_surface: b,
                },
            ) => a == b,
            (SessionHandsOff::Measured { n, .. }, SessionStretches::Measured { stretches }) => {
                *n == stretches.len() as u64
            }
            _ => false,
        };
        assert!(agrees, "{id}: {:?} vs {detail:?}", answers[id]);
    }
}

#[test]
fn hands_off_names_a_whole_surface_exclusion_judged_beyond_the_page() {
    let f = fixture("F9");
    let data = &f.snapshots()["hands_off"];
    for case in data["cases"].as_array().unwrap() {
        let mut db = TempDb::empty().unwrap();
        let names: Vec<&str> = case["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        for name in &names {
            let session = data["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["session_id"] == *name)
                .unwrap();
            let rows: Vec<CanonicalRecord> =
                serde_json::from_value(session["records"].clone()).unwrap();
            seed(&mut db, name, "claude", Some("raw-batched"), &rows);
        }
        seed(
            &mut db,
            "sibling",
            "codex",
            Some("raw-batched"),
            &[
                record("sib-h", "2026-09-07T12:00:00Z", true, 0),
                record("sib-a", "2026-09-07T12:01:00Z", false, 1),
            ],
        );
        let metrics = MetricsDb::open(db.path()).unwrap();
        // Ask about one session of the surface only: health is still judged
        // over every session that surface has in the window.
        let answers = metrics
            .session_hands_off(window(), &[names[0], "sibling"])
            .unwrap();
        let detail = metrics.session_stretches(window(), names[0]).unwrap();
        match (&answers[names[0]], detail) {
            (
                SessionHandsOff::Unmeasured {
                    excluded_surface: Some(a),
                },
                SessionStretches::Unmeasured {
                    excluded_surface: Some(b),
                },
            ) => {
                assert!(case["excluded"].as_bool().unwrap(), "{}", case["name"]);
                assert_eq!(*a, b);
            }
            (SessionHandsOff::Measured { n, .. }, SessionStretches::Measured { stretches }) => {
                assert!(!case["excluded"].as_bool().unwrap(), "{}", case["name"]);
                assert_eq!(*n, stretches.len() as u64);
            }
            (answer, detail) => panic!("{}: {answer:?} vs {detail:?}", case["name"]),
        }
        assert!(matches!(
            answers["sibling"],
            SessionHandsOff::Measured { n: 1, .. }
        ));
    }
}
