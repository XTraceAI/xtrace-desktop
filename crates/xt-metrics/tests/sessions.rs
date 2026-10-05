use jiff::{Timestamp, tz::TimeZone};
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{Delta, MetricsDb, Window};
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
fn seed(db: &mut TempDb, id: &str, times: &[&str]) {
    let mut session = fixture("F1").sessions()[0].metadata.clone();
    session.session_id = id.into();
    db.store_mut().upsert_session(&session, false).unwrap();
    let rows:Vec<CanonicalRecord>=times.iter().enumerate().map(|(i,ts)|serde_json::from_value(json!({"uuid":format!("{id}-{i}"),"type":"assistant","timestamp":ts,"message":{"role":"assistant"}})).unwrap()).collect();
    db.store_mut().upsert_records(id, &rows, false).unwrap();
}
#[test]
fn sessions_per_day_f3_first_event_and_f2_zero_dates() {
    let db = fixture("F3").build_db(false).unwrap();
    let report = MetricsDb::open(db.path())
        .unwrap()
        .sessions_per_day(window(), TimeZone::UTC)
        .unwrap();
    assert_eq!(report.total_sessions, 1);
    assert_eq!(report.max_per_day, 1);
    assert_eq!(report.mean_per_day, 1.0 / 7.0);
    assert_eq!(report.days[0].date, "2026-09-01");
    assert_eq!(report.days[0].sessions, 1);
    assert!(report.days[1..].iter().all(|day| day.sessions == 0));
    let f = fixture("F2");
    let mut db = TempDb::empty().unwrap();
    for lane in f.snapshots()["spans"]["lanes"].as_array().unwrap() {
        let id = lane["session_id"].as_str().unwrap();
        let mut session = f.sessions()[0].metadata.clone();
        session.session_id = id.into();
        db.store_mut().upsert_session(&session, false).unwrap();
        let rows: Vec<CanonicalRecord> = serde_json::from_value(lane["records"].clone()).unwrap();
        db.store_mut().upsert_records(id, &rows, false).unwrap();
    }
    let report = MetricsDb::open(db.path())
        .unwrap()
        .sessions_per_day(window(), TimeZone::UTC)
        .unwrap();
    assert_eq!(report.total_sessions, 3);
    assert_eq!(report.max_per_day, 3);
    assert_eq!(report.mean_per_day, 3.0 / 7.0);
    assert_eq!(report.days[6].sessions, 3);
}
#[test]
fn sessions_per_day_partial_dst_days_use_reported_local_dates() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "one",
        &["2026-03-08T08:00:00Z", "2026-03-08T01:30:00-08:00"],
    );
    seed(&mut db, "two", &["2026-03-09T07:00:00Z"]);
    let w = Window::new(
        ms("2026-03-07T12:00:00-08:00"),
        ms("2026-03-10T12:00:00-07:00"),
    )
    .unwrap();
    assert_eq!(w.end_ms() - w.start_ms(), 71 * 60 * 60 * 1000);
    let r = MetricsDb::open(db.path())
        .unwrap()
        .sessions_per_day(w, TimeZone::get("America/Los_Angeles").unwrap())
        .unwrap();
    assert_eq!(
        r.days.iter().map(|d| d.sessions).collect::<Vec<_>>(),
        vec![0, 1, 1, 0]
    );
    assert_eq!(r.mean_per_day, 0.5);
}
#[test]
fn sessions_per_day_precise_leap_and_offset_boundaries() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "leap",
        &["2026-09-01T23:59:60.9Z", "2026-09-02T00:00:00Z"],
    );
    seed(
        &mut db,
        "offset",
        &[
            "2026-09-01T17:00:00-07:00",
            "2026-09-02T00:00:00.00000000001Z",
        ],
    );
    let w = Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-03T00:00:00Z")).unwrap();
    let r = MetricsDb::open(db.path())
        .unwrap()
        .sessions_per_day(w, TimeZone::UTC)
        .unwrap();
    assert_eq!(
        r.days.iter().map(|d| d.sessions).collect::<Vec<_>>(),
        vec![1, 1]
    );
}
#[test]
fn delta_low_samples_unknown_and_zero_baselines_remain_honest() {
    for (current_n, previous_n) in [(4, 5), (5, 4), (4, 4)] {
        let d = Delta::new(Some(20.0), Some(10.0), current_n, previous_n);
        assert!(d.suppressed);
        assert_eq!(d.pct, None);
        assert_eq!((d.current_n, d.previous_n), (current_n, previous_n));
    }
    assert_eq!(Delta::new(Some(20.0), Some(10.0), 5, 5).pct, Some(100.0));
    for (current, previous) in [
        (None, Some(1.0)),
        (Some(1.0), None),
        (Some(1.0), Some(0.0)),
        (Some(f64::NAN), Some(1.0)),
        (Some(1.0), Some(f64::INFINITY)),
        (Some(f64::MAX), Some(-f64::MAX)),
    ] {
        let d = Delta::new(current, previous, 5, 5);
        assert!(d.suppressed);
        assert_eq!(d.pct, None);
        assert!(d.current.is_none_or(f64::is_finite));
        assert!(d.previous.is_none_or(f64::is_finite));
    }
    let w = window();
    assert_eq!(w.previous().unwrap().end_ms(), w.start_ms());
    assert_eq!(
        w.previous().unwrap().end_ms() - w.previous().unwrap().start_ms(),
        w.end_ms() - w.start_ms()
    );
}
#[test]
fn favorite_iso_week_calendar_uses_explicit_zone_and_monday_boundaries() {
    let zone = TimeZone::get("America/Los_Angeles").unwrap();
    let w = Window::iso_week(ms("2026-03-08T12:00:00-07:00"), zone).unwrap();
    assert_eq!(w.start_ms(), ms("2026-03-02T00:00:00-08:00"));
    assert_eq!(w.end_ms(), ms("2026-03-09T00:00:00-07:00"));
    assert_eq!(w.end_ms() - w.start_ms(), 167 * 60 * 60 * 1000);
    let w = Window::iso_week(ms("2027-01-01T00:00:00Z"), TimeZone::UTC).unwrap();
    assert_eq!(w.start_ms(), ms("2026-12-28T00:00:00Z"));
    assert_eq!(w.end_ms(), ms("2027-01-04T00:00:00Z"));
}

#[test]
fn sessions_per_day_metadata_parity_and_empty_days() {
    let mut reports = Vec::new();
    for keep in [false, true] {
        let db = fixture("F1").build_db(keep).unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        reports.push(metrics.sessions_per_day(window(), TimeZone::UTC).unwrap());
        let empty = metrics
            .sessions_per_day(window().previous().unwrap(), TimeZone::UTC)
            .unwrap();
        assert_eq!(empty.total_sessions, 0);
        assert_eq!(empty.mean_per_day, 0.0);
        assert_eq!(empty.max_per_day, 0);
        assert_eq!(empty.days.len(), 7);
    }
    assert_eq!(reports[0], reports[1]);
}
