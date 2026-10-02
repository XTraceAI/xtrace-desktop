use jiff::Timestamp;
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{ActiveSpanReport, MetricsDb, Window};
use xt_store::CanonicalRecord;

fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn seed(db: &mut TempDb, id: &str, records: &[CanonicalRecord]) {
    let mut session = fixture("F1").sessions()[0].metadata.clone();
    session.session_id = id.into();
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, records, false).unwrap();
}
fn record(id: &str, ts: &str) -> CanonicalRecord {
    serde_json::from_value(
        json!({"uuid":id,"type":"assistant","timestamp":ts,"message":{"role":"assistant"}}),
    )
    .unwrap()
}
fn query(db: &TempDb, w: Window) -> ActiveSpanReport {
    let report = MetricsDb::open(db.path()).unwrap().active_spans(w).unwrap();
    // Independent SQLite lag/sum oracle. Exact comparison uses native boundary
    // strings in these ordinary-year cases; production supports numeric bounds.
    let c = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&c).unwrap();
    let start = Timestamp::from_millisecond(w.start_ms())
        .unwrap()
        .to_string();
    let end = Timestamp::from_millisecond(w.end_ms()).unwrap().to_string();
    let sum: i64 = c.query_row(
        "WITH selected AS (SELECT * FROM v_session_events WHERE xt_timestamp_cmp(ts,?1)>=0 AND xt_timestamp_cmp(ts,?2)<0),
         gaps AS (SELECT ts_ms-lag(ts_ms) OVER(PARTITION BY session_id ORDER BY ts_ms,uuid) AS gap FROM selected)
         SELECT coalesce(sum(CASE WHEN gap<=1200000 THEN gap ELSE 0 END),0) FROM gaps",
        [start,end], |r|r.get(0)).unwrap();
    assert_eq!(report.active_ms, sum as u64);
    assert_eq!(
        report.active_ms,
        report
            .spans
            .iter()
            .map(|s| u64::try_from(s.end_ms - s.start_ms).unwrap())
            .sum::<u64>()
    );
    report
}

#[test]
fn spans_f1_baseline_and_f2_parallel_lanes() {
    let f = fixture("F1");
    let db = f.build_db(false).unwrap();
    f.assert_expectation("M-05", &json!({"active_ms":query(&db,window()).active_ms}))
        .unwrap();
    let f = fixture("F2");
    let snapshot = &f.snapshots()["spans"];
    let mut db = TempDb::empty().unwrap();
    for lane in snapshot["lanes"].as_array().unwrap() {
        seed(
            &mut db,
            lane["session_id"].as_str().unwrap(),
            &serde_json::from_value::<Vec<CanonicalRecord>>(lane["records"].clone()).unwrap(),
        );
    }
    let report = query(&db, window());
    assert_eq!(report.active_ms, snapshot["active_ms"].as_u64().unwrap());
    assert_eq!(report.spans.len(), 3);
    for (span, lane) in report
        .spans
        .iter()
        .zip(snapshot["lanes"].as_array().unwrap())
    {
        assert_eq!(span.session_id, lane["session_id"].as_str().unwrap());
        assert_eq!(span.host, "claude");
        assert_eq!(span.start_ms, lane["start_ms"].as_i64().unwrap());
        assert_eq!(span.end_ms, lane["end_ms"].as_i64().unwrap());
    }
}

#[test]
fn spans_active_gap_boundaries_and_structural_classes() {
    let mut db = TempDb::empty().unwrap();
    for (id, seconds, expected) in [
        ("under", 1199, 1199000),
        ("equal", 1200, 1200000),
        ("over", 1201, 0),
        ("tie", 0, 0),
    ] {
        let base = ms("2026-09-07T12:00:00Z");
        let end = Timestamp::from_millisecond(base + seconds * 1000)
            .unwrap()
            .to_string();
        seed(
            &mut db,
            id,
            &[
                record(&format!("{id}-a"), "2026-09-07T12:00:00Z"),
                record(&format!("{id}-b"), &end),
            ],
        );
        let report = query(&db, window());
        let spans: Vec<_> = report.spans.iter().filter(|s| s.session_id == id).collect();
        assert_eq!(spans.len(), if id == "over" { 2 } else { 1 });
        assert_eq!(
            spans.iter().map(|s| s.end_ms - s.start_ms).sum::<i64>(),
            expected
        );
    }
    seed(
        &mut db,
        "single",
        &[record("single", "2026-09-07T12:00:00Z")],
    );
    let rows:Vec<CanonicalRecord> = serde_json::from_value(json!([
        {"uuid":"human","type":"user","timestamp":"2026-09-07T10:00:00Z","message":{"role":"user","content":[{"type":"text","text":"hi"}]}},
        {"uuid":"assistant","type":"assistant","timestamp":"2026-09-07T10:20:00Z","message":{"role":"assistant"}},
        {"uuid":"tool","type":"user","timestamp":"2026-09-07T10:40:00Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}},
        {"uuid":"side","type":"assistant","isSidechain":true,"timestamp":"2026-09-07T11:00:00Z","message":{"role":"assistant"}}
    ])).unwrap();
    seed(&mut db, "classes", &rows);
    let report = query(&db, window());
    assert_eq!(
        report
            .spans
            .iter()
            .find(|s| s.session_id == "single")
            .map(|s| s.end_ms - s.start_ms),
        Some(0)
    );
    assert_eq!(
        report
            .spans
            .iter()
            .find(|s| s.session_id == "classes")
            .map(|s| s.end_ms - s.start_ms),
        Some(3600000)
    );
}

#[test]
fn spans_f3_only_in_window_no_interpolation() {
    let db = fixture("F3").build_db(false).unwrap();
    let report = query(&db, window());
    assert_eq!(report.active_ms, 0);
    assert_eq!(report.spans.len(), 2);
    assert_eq!(report.spans[0].start_ms, window().start_ms());
    assert_eq!(report.spans[1].start_ms, window().end_ms() - 1);
    assert_eq!(query(&db, window().previous().unwrap()).spans.len(), 1);
}

#[test]
fn spans_leap_projection_reversal_and_precise_membership() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "leap",
        &[
            record("a", "2016-12-31T23:59:60.9Z"),
            record("b", "2017-01-01T00:00:00.1Z"),
        ],
    );
    let boundary = ms("2017-01-01T00:00:00Z");
    let combined = query(&db, Window::new(boundary - 1000, boundary + 1000).unwrap());
    assert_eq!(combined.active_ms, 800);
    assert_eq!(
        (combined.spans[0].start_ms, combined.spans[0].end_ms),
        (boundary + 100, boundary + 900)
    );
    for w in [
        Window::new(boundary - 1000, boundary).unwrap(),
        Window::new(boundary, boundary + 1000).unwrap(),
    ] {
        let report = query(&db, w);
        assert_eq!(report.spans.len(), 1);
        assert_eq!(report.active_ms, 0);
    }
    seed(
        &mut db,
        "precise",
        &[
            record("p1", "2017-01-01T00:00:00.0000000001Z"),
            record("p2", "2017-01-01T00:00:00.0009999999Z"),
            record("p3", "2017-01-01T00:00:00.001Z"),
        ],
    );
    let report = query(&db, Window::new(boundary, boundary + 1).unwrap());
    assert_eq!(report.spans.len(), 1);
    assert_eq!(report.active_ms, 0);
}

#[test]
fn spans_canonical_exclusions_copies_empty_and_invalid_time() {
    let db = fixture("F1").build_db(false).unwrap();
    let before = query(&db, window());
    let c = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&c).unwrap();
    c.execute("INSERT INTO native_record_copies(session_id,record_uuid) SELECT session_id,uuid FROM records",[]).unwrap();
    assert_eq!(query(&db, window()), before);
    for mutation in [
        "UPDATE sessions SET kind='judge'",
        "UPDATE sessions SET kind='user'; UPDATE records SET is_meta=1",
        "UPDATE records SET is_meta=0,model='<synthetic>'",
    ] {
        c.execute_batch(mutation).unwrap();
        assert!(query(&db, window()).spans.is_empty());
    }
    assert_eq!(query(&TempDb::empty().unwrap(), window()).active_ms, 0);
    c.execute("UPDATE records SET model=NULL,ts='bad'", [])
        .unwrap();
    assert!(
        MetricsDb::open(db.path())
            .unwrap()
            .active_spans(window())
            .is_err()
    );
}

#[test]
fn spans_arrival_order_replay_and_canonical_session_ownership() {
    let events = [
        record("later", "2026-09-07T12:20:00Z"),
        record("earlier", "2026-09-07T12:00:00Z"),
        record("tie-z", "2026-09-07T12:10:00.0001Z"),
        record("tie-a", "2026-09-07T12:10:00.0009Z"),
    ];
    let mut baseline = None;
    for reverse in [false, true] {
        let mut db = TempDb::empty().unwrap();
        let mut input = events.to_vec();
        if reverse {
            input.reverse();
        }
        seed(&mut db, "owner", &input);
        let report = query(&db, window());
        assert_eq!(report.active_ms, 1200000);
        if let Some(expected) = &baseline {
            assert_eq!(&report, expected);
        }
        baseline = Some(report.clone());
        seed(&mut db, "owner", &input);
        seed(&mut db, "copy", &input);
        assert_eq!(query(&db, window()), report);
        assert_eq!(report.spans.len(), 1);
        assert_eq!(report.spans[0].session_id, "owner");
    }
}

#[test]
fn spans_empty_windows_preserve_numeric_boundary_domain() {
    let db = TempDb::empty().unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    for start in [
        ms("-000001-01-01T00:00:00Z"),
        Timestamp::MAX.as_second() * 1000 - 1,
    ] {
        let report = metrics
            .active_spans(Window::new(start, start + 1).unwrap())
            .unwrap();
        assert!(report.spans.is_empty());
        assert_eq!(report.active_ms, 0);
    }
}

#[test]
fn spans_open_rejects_incomplete_projection_until_writer_restores_it() {
    let db = fixture("F1").build_db(false).unwrap();
    let expected = query(&db, window());
    let c = Connection::open(db.path()).unwrap();
    for columns in ["uuid,session_id,ts_ms,ts", "uuid,session_id,ts_ms,host"] {
        c.execute_batch(&format!(
            "DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT {columns} FROM v_records WHERE ts_ms IS NOT NULL;"
        )).unwrap();
        assert!(MetricsDb::open(db.path()).is_err());
        // Opening the read-only metrics reader must not repair the stale view.
        let actual: String = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='v_session_events'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(actual.contains(columns));
        let _writer = xt_store::Store::open(db.path()).unwrap();
        assert_eq!(query(&db, window()), expected);
    }
}
