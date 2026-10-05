use jiff::{Timestamp, tz::TimeZone};
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

/// Allocation is only ever a split of what the window already measured, so the
/// buckets must add up to `active_ms` exactly, in integer milliseconds.
fn by_day(report: &ActiveSpanReport, w: Window, zone: TimeZone) -> Vec<(String, u64)> {
    let days = report.by_day(w, zone.clone()).unwrap();
    assert_eq!(
        days.iter().map(|d| d.active_ms).sum::<u64>(),
        report.active_ms
    );
    let buckets = w.local_days(zone).unwrap();
    assert_eq!(days.len(), buckets.len());
    for (day, bucket) in days.iter().zip(&buckets) {
        assert_eq!(
            (day.date.as_str(), day.start_ms, day.end_ms),
            (
                bucket.date.to_string().as_str(),
                bucket.window.start_ms(),
                bucket.window.end_ms()
            )
        );
    }
    days.into_iter()
        .map(|day| (day.date, day.active_ms))
        .collect()
}

#[test]
fn spans_by_day_splits_one_span_at_local_midnight_without_re_folding_it() {
    let mut db = TempDb::empty().unwrap();
    // One uninterrupted span either side of local midnight: the gap rule already
    // kept it whole, and the split must not turn it into two shorter spans.
    seed(
        &mut db,
        "crossing",
        &[
            record("a", "2026-09-02T23:50:00Z"),
            record("b", "2026-09-03T00:05:00Z"),
        ],
    );
    let report = query(&db, window());
    assert_eq!(report.spans.len(), 1);
    assert_eq!(report.active_ms, 15 * 60_000);
    assert_eq!(
        by_day(&report, window(), TimeZone::UTC),
        [
            ("2026-09-01".into(), 0),
            ("2026-09-02".into(), 10 * 60_000),
            ("2026-09-03".into(), 5 * 60_000),
            ("2026-09-04".into(), 0),
            ("2026-09-05".into(), 0),
            ("2026-09-06".into(), 0),
            ("2026-09-07".into(), 0),
        ]
    );
    // The same span, cut in a zone whose midnight falls elsewhere, lands whole
    // on one local date: the boundaries move, the measurement does not.
    let pacific = TimeZone::get("America/Los_Angeles").unwrap();
    let cut = by_day(&report, window(), pacific);
    assert_eq!(
        cut.iter()
            .filter(|(_, ms)| *ms > 0)
            .cloned()
            .collect::<Vec<_>>(),
        [("2026-09-02".into(), 15 * 60_000)]
    );
}

#[test]
fn spans_by_day_adds_parallel_sessions_and_reports_empty_and_partial_days() {
    let mut db = TempDb::empty().unwrap();
    // Two sessions overlapping the same wall-clock hour: M-05 adds them, and a
    // day bucket must add them too rather than unioning the shared time away.
    for (id, minutes) in [("a", [0, 10, 20]), ("b", [5, 15, 25])] {
        let rows: Vec<_> = minutes
            .iter()
            .map(|minute| {
                record(
                    &format!("{id}{minute}"),
                    &format!("2026-09-04T09:{minute:02}:00Z"),
                )
            })
            .collect();
        seed(&mut db, id, &rows);
    }
    let whole = query(&db, window());
    assert_eq!(whole.active_ms, 40 * 60_000);
    assert_eq!(
        by_day(&whole, window(), TimeZone::UTC),
        [
            ("2026-09-01".into(), 0),
            ("2026-09-02".into(), 0),
            ("2026-09-03".into(), 0),
            ("2026-09-04".into(), 40 * 60_000),
            ("2026-09-05".into(), 0),
            ("2026-09-06".into(), 0),
            ("2026-09-07".into(), 0),
        ]
    );
    // A clipped first day and a wholly empty last one: both are reported, and
    // the clipped day carries only the part of each span the window selected.
    let partial = Window::new(ms("2026-09-04T09:07:00Z"), ms("2026-09-06T00:00:00Z")).unwrap();
    let report = query(&db, partial);
    assert_eq!(report.active_ms, 20 * 60_000);
    assert_eq!(
        by_day(&report, partial, TimeZone::UTC),
        [("2026-09-04".into(), 20 * 60_000), ("2026-09-05".into(), 0)]
    );
}

#[test]
fn spans_by_day_gives_a_leap_projection_past_the_window_end_to_the_last_day() {
    let mut db = TempDb::empty().unwrap();
    // Both events are inside the leap second of 2016-12-31, so both are members
    // of a window that ends at the following midnight, while the POSIX axis
    // projects them after it. The duration is M-05's, unchanged; only its
    // allocation is decided here.
    seed(
        &mut db,
        "leap",
        &[
            record("a", "2016-12-31T23:59:60.1Z"),
            record("b", "2016-12-31T23:59:60.9Z"),
        ],
    );
    let boundary = ms("2017-01-01T00:00:00Z");
    let w = Window::new(boundary - 2 * 86_400_000, boundary).unwrap();
    let report = query(&db, w);
    assert_eq!(report.active_ms, 800);
    assert_eq!(
        (report.spans[0].start_ms, report.spans[0].end_ms),
        (boundary + 100, boundary + 900)
    );
    assert!(report.spans[0].start_ms > w.end_ms());
    assert_eq!(
        by_day(&report, w, TimeZone::UTC),
        [("2016-12-30".into(), 0), ("2016-12-31".into(), 800)]
    );
}

#[test]
fn spans_by_day_treats_a_short_and_a_long_dst_day_as_one_bucket_each() {
    let zone = TimeZone::get("America/Los_Angeles").unwrap();
    for (start, end, date, local) in [
        (
            "2026-03-08T08:00:00Z",
            "2026-03-09T07:00:00Z",
            "2026-03-08",
            "2026-03-08T18:00:00Z",
        ),
        (
            "2026-11-01T07:00:00Z",
            "2026-11-02T08:00:00Z",
            "2026-11-01",
            "2026-11-01T18:00:00Z",
        ),
    ] {
        let mut db = TempDb::empty().unwrap();
        let after = Timestamp::from_millisecond(ms(local) + 60_000)
            .unwrap()
            .to_string();
        seed(&mut db, "dst", &[record("a", local), record("b", &after)]);
        let w = Window::new(ms(start), ms(end)).unwrap();
        let report = query(&db, w);
        assert_eq!(report.active_ms, 60_000);
        // A 23- or 25-hour local date is still one reported day holding all of it.
        assert_eq!(by_day(&report, w, zone.clone()), [(date.into(), 60_000)]);
    }
}

#[test]
fn spans_by_day_keeps_sub_millisecond_and_zero_duration_days_measured() {
    let mut db = TempDb::empty().unwrap();
    // Distinct instants inside one millisecond project onto the same POSIX
    // millisecond: a real, measured zero for that day, not an unknown.
    seed(
        &mut db,
        "sub",
        &[
            record("a", "2026-09-05T12:00:00.0000001Z"),
            record("b", "2026-09-05T12:00:00.0009999Z"),
        ],
    );
    let report = query(&db, window());
    assert_eq!(report.active_ms, 0);
    assert_eq!(report.spans.len(), 1);
    let days = by_day(&report, window(), TimeZone::UTC);
    assert_eq!(days.len(), 7);
    assert!(days.iter().all(|(_, ms)| *ms == 0));
    // One recorded millisecond is a tiny positive the day keeps as one.
    seed(
        &mut db,
        "tiny",
        &[
            record("c", "2026-09-05T13:00:00.000Z"),
            record("d", "2026-09-05T13:00:00.001Z"),
        ],
    );
    let report = query(&db, window());
    assert_eq!(
        by_day(&report, window(), TimeZone::UTC)[4],
        ("2026-09-05".into(), 1)
    );
}
