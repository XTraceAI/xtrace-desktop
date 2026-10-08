//! "Your hours" over a real store, and the day-by-day series of existing
//! metrics. Every session identity and record is synthetic.
mod hands_off_support;

use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{BreakLength, MetricsDb, TypingRate, Window};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource};

fn ms(ts: &str) -> i64 {
    ts.parse::<Timestamp>().unwrap().as_millisecond()
}
fn event(id: &str, ts: &str, human: bool) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":id,"type":if human {"user"} else {"assistant"},"timestamp":ts,"message":{"role":if human {"user"} else {"assistant"},"content":[{"type":"text","text":"x"}]}})).unwrap()
}
fn seed(db: &mut TempDb, id: &str, records: &[CanonicalRecord]) {
    let fixture =
        Fixture::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1")).unwrap();
    let mut session = fixture.sessions()[0].metadata.clone();
    session.session_id = id.into();
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, records, false).unwrap();
}
/// Two conversations whose messages interleave, with agent replies between.
fn two_conversations() -> TempDb {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "a",
        &[
            event("a1", "2026-09-02T09:00:00Z", true),
            event("a1r", "2026-09-02T09:01:00Z", false),
            event("a2", "2026-09-02T09:50:00Z", true),
            event("a2r", "2026-09-02T09:52:00Z", false),
            event("a3", "2026-09-02T10:40:00Z", true),
            // Alone: more than 30 minutes from any other message.
            event("a4", "2026-09-02T15:00:00Z", true),
        ],
    );
    seed(
        &mut db,
        "b",
        &[
            // Agent work in b while a's first reply runs.
            event("b0", "2026-09-02T09:00:20Z", false),
            event("b0r", "2026-09-02T09:00:40Z", false),
            event("b1", "2026-09-02T09:25:00Z", true),
            event("b1r", "2026-09-02T09:30:00Z", false),
            event("b2", "2026-09-02T10:15:00Z", true),
        ],
    );
    db
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-04T00:00:00Z")).unwrap()
}

#[test]
fn messages_from_every_conversation_share_one_timeline() {
    let db = two_conversations();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let report = metrics
        .human_hours(window(), BreakLength::new(30).unwrap(), TimeZone::UTC)
        .unwrap()
        .current;
    assert_eq!(report.break_minutes, 30);
    assert_eq!(report.messages, Some(6));
    // Agent replies are not the person's messages and never extend a stretch.
    assert_eq!(report.active_ms, Some(100 * 60_000));
    let day = &report.by_day[1];
    assert_eq!(day.date, "2026-09-02");
    let stretches: Vec<_> = day
        .stretches
        .iter()
        .map(|s| (s.start_ms, s.end_ms))
        .collect();
    assert_eq!(
        stretches,
        [
            (ms("2026-09-02T09:00:00Z"), ms("2026-09-02T10:40:00Z")),
            (ms("2026-09-02T15:00:00Z"), ms("2026-09-02T15:00:00Z")),
        ]
    );
    assert_eq!(report.by_day[0].active_ms, Some(0));
    assert_eq!(report.by_day[2].active_ms, Some(0));
    // The same messages counted as Human messages.
    assert_eq!(
        metrics
            .counts(window(), TypingRate::default())
            .unwrap()
            .human_messages,
        report.messages
    );
}

#[test]
fn local_midnight_splits_a_stretch_in_the_selected_zone() {
    let mut db = TempDb::empty().unwrap();
    // 23:40 and 00:30 in Berlin (UTC+2 in September).
    seed(
        &mut db,
        "late",
        &[
            event("l1", "2026-09-01T21:40:00Z", true),
            event("l2", "2026-09-01T22:30:00Z", true),
        ],
    );
    let metrics = MetricsDb::open(db.path()).unwrap();
    let zone = TimeZone::get("Europe/Berlin").unwrap();
    let report = metrics
        .human_hours(window(), BreakLength::default(), zone)
        .unwrap()
        .current;
    let midnight = ms("2026-09-01T22:00:00Z");
    let day = |date: &str| report.by_day.iter().find(|d| d.date == date).unwrap();
    assert_eq!(day("2026-09-01").stretches[0].end_ms, midnight);
    assert_eq!(day("2026-09-01").active_ms, Some(20 * 60_000));
    assert_eq!(day("2026-09-02").stretches[0].start_ms, midnight);
    assert_eq!(day("2026-09-02").active_ms, Some(30 * 60_000));
    assert_eq!(report.active_ms, Some(50 * 60_000));
}

#[test]
fn an_unknown_classification_is_unknown_never_a_smaller_number() {
    let db = two_conversations();
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE records SET is_human=NULL WHERE uuid='b1'", [])
        .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    assert_eq!(
        metrics
            .counts(window(), TypingRate::default())
            .unwrap()
            .human_messages,
        None
    );
    let report = metrics
        .human_hours(window(), BreakLength::default(), TimeZone::UTC)
        .unwrap()
        .current;
    assert_eq!(report.active_ms, None);
    assert_eq!(report.messages, None);
    assert!(
        report
            .by_day
            .iter()
            .all(|day| day.active_ms.is_none() && day.stretches.is_empty())
    );
}

#[test]
fn an_empty_window_is_a_measured_zero() {
    let db = TempDb::empty().unwrap();
    let report = MetricsDb::open(db.path())
        .unwrap()
        .human_hours(window(), BreakLength::default(), TimeZone::UTC)
        .unwrap()
        .current;
    assert_eq!(report.active_ms, Some(0));
    assert_eq!(report.messages, Some(0));
    assert_eq!(report.by_day.len(), 3);
}

#[test]
fn daily_series_name_the_ranges_days() {
    let db = two_conversations();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let zone = TimeZone::UTC;
    let days = window().local_days(zone.clone()).unwrap();
    let concurrency = metrics.concurrency_by_day(window(), zone.clone()).unwrap();
    let (range, hands_off) = hands_off_support::matches_standalone(&metrics, window(), zone);
    assert_eq!(concurrency.len(), days.len());
    assert_eq!(hands_off.len(), days.len());
    for ((bucket, c), h) in days.iter().zip(&concurrency).zip(&hands_off) {
        assert_eq!(c.date, bucket.date.to_string());
        assert_eq!(h.date, bucket.date.to_string());
        assert_eq!(
            (c.start_ms, c.end_ms),
            (bucket.window.start_ms(), bucket.window.end_ms())
        );
        let whole = metrics.concurrency(bucket.window).unwrap();
        assert_eq!((c.max, c.mean), (whole.max, whole.mean));
    }
    // Hands-off is measured once over the range: the days' stretches are the
    // range's own, so their counts add up to the range's.
    assert_eq!(
        hands_off.iter().map(|day| day.n.unwrap()).sum::<u64>(),
        range.n.unwrap()
    );
    // Both conversations ran at once on the second day only.
    assert_eq!(concurrency[0].max, None);
    assert_eq!(concurrency[1].max, Some(2));
    assert_eq!(concurrency[2].max, None);
}

/// A rolling range from 10:00 on Sep 2 to 10:00 on Sep 5 touches four local
/// days, so human time covers Sep 2-5 whole and the previous period Aug 29 to
/// Sep 1 whole.
fn rolling() -> Window {
    Window::new(ms("2026-09-02T10:00:00Z"), ms("2026-09-05T10:00:00Z")).unwrap()
}
fn night_owl() -> TempDb {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "night",
        &[
            // Before the previous period starts: only its part after the
            // previous period's first midnight counts there.
            event("p0", "2026-08-28T23:30:00Z", true),
            event("p1", "2026-08-29T00:10:00Z", true),
            // Across the previous/current boundary.
            event("b0", "2026-09-01T23:50:00Z", true),
            event("b1", "2026-09-02T00:20:00Z", true),
        ],
    );
    db
}

#[test]
fn whole_days_keep_stretches_that_cross_a_period_start() {
    let db = night_owl();
    let report = MetricsDb::open(db.path())
        .unwrap()
        .human_hours(rolling(), BreakLength::default(), TimeZone::UTC)
        .unwrap();
    let (current, previous) = (&report.current, &report.previous);
    assert_eq!(
        (current.start_ms, current.end_ms),
        (ms("2026-09-02T00:00:00Z"), ms("2026-09-06T00:00:00Z"))
    );
    assert_eq!(
        (previous.start_ms, previous.end_ms),
        (ms("2026-08-29T00:00:00Z"), ms("2026-09-02T00:00:00Z"))
    );
    // Every row is a whole day, the first one included.
    assert_eq!(current.by_day.len(), 4);
    for day in &current.by_day {
        assert_eq!(day.end_ms - day.start_ms, 86_400_000, "{}", day.date);
    }
    // 23:50 → 00:20 over the boundary: 20 minutes on the first current day,
    // 10 on the last previous day; nothing lost from both.
    let first = &current.by_day[0];
    assert_eq!(first.date, "2026-09-02");
    assert_eq!(
        first
            .stretches
            .iter()
            .map(|s| (s.start_ms, s.end_ms))
            .collect::<Vec<_>>(),
        [(ms("2026-09-02T00:00:00Z"), ms("2026-09-02T00:20:00Z"))]
    );
    assert_eq!(current.active_ms, Some(20 * 60_000));
    assert_eq!(current.messages, Some(1));
    // 23:30 → 00:10 over the previous period's start keeps its 10 minutes,
    // and the boundary stretch gives the previous period its other 10.
    assert_eq!(previous.by_day[0].active_ms, Some(10 * 60_000));
    assert_eq!(previous.by_day[3].active_ms, Some(10 * 60_000));
    assert_eq!(previous.active_ms, Some(20 * 60_000));
    assert_eq!(previous.messages, Some(2));
}

#[test]
fn an_unknown_message_within_a_break_before_a_period_makes_only_that_period_unknown() {
    let db = night_owl();
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE records SET is_human=NULL WHERE uuid='p0'", [])
        .unwrap();
    let report = MetricsDb::open(db.path())
        .unwrap()
        .human_hours(rolling(), BreakLength::default(), TimeZone::UTC)
        .unwrap();
    assert_eq!(report.previous.active_ms, None);
    assert_eq!(report.previous.messages, None);
    assert_eq!(report.current.active_ms, Some(20 * 60_000));
    // More than a break length before the period, it reads nothing there.
    let report = MetricsDb::open(db.path())
        .unwrap()
        .human_hours(rolling(), BreakLength::new(20).unwrap(), TimeZone::UTC)
        .unwrap();
    assert_eq!(report.previous.active_ms, Some(0));
}

/// A record on one surface, with a tool call when `tools`.
fn work(id: &str, ts: &str, human: bool, tools: bool) -> CanonicalRecord {
    let role = if human { "user" } else { "assistant" };
    serde_json::from_value(json!({"uuid":id,"type":role,"timestamp":ts,"message":{"role":role,"content":if tools {json!([{"type":"tool_use","name":"Read","input":{"path":"synthetic"}}])} else {json!([{"type":"text","text":"Synthetic"}])}}})).unwrap()
}
fn seed_on(db: &mut TempDb, id: &str, surface: &str, records: &[CanonicalRecord]) {
    let mut session = SessionMeta::new(id, "claude", SessionSource::Fixture);
    session.surface = Some(surface.into());
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut().upsert_records(id, records, false).unwrap();
}

#[test]
fn an_overnight_hands_off_stretch_stays_whole_on_its_start_day() {
    let mut db = TempDb::empty().unwrap();
    seed_on(
        &mut db,
        "late",
        "cli",
        &[
            work("h", "2026-09-01T23:50:00Z", true, false),
            work("a1", "2026-09-02T00:10:00Z", false, true),
            work("a2", "2026-09-02T00:30:00Z", false, false),
        ],
    );
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (range, days) = hands_off_support::matches_standalone(&metrics, window(), TimeZone::UTC);
    assert_eq!(days[0].date, "2026-09-01");
    assert_eq!(days[0].n, Some(1));
    assert_eq!(days[0].median_min, Some(40.0));
    assert_eq!(days[1].n, Some(0));
    assert_eq!(days[1].median_min, None);
    assert_eq!(range.median_min, Some(40.0));
}

/// A session that is measured early and unmeasured later publishes none of
/// its stretches: the range is unknown, and so is every day from its first to
/// its last event, its earlier measured day included. The other days keep
/// their own stretches, through a partial first day and the 25-hour day the
/// clocks go back on.
#[test]
fn a_later_unmeasured_stretch_hides_the_sessions_earlier_days() {
    let mut db = TempDb::empty().unwrap();
    // Rolling 10:00 PDT on Oct 31 to 09:00 PST on Nov 3, in Los Angeles.
    let window = Window::new(ms("2026-10-31T17:00:00Z"), ms("2026-11-03T17:00:00Z")).unwrap();
    seed_on(
        &mut db,
        "known",
        "cli",
        &[
            // Before the window opens: not this range's.
            work("early-h", "2026-10-31T16:00:00Z", true, false),
            work("early-a", "2026-10-31T16:10:00Z", false, true),
            // 23:50 PDT to 00:20 PDT: stays whole on Oct 31.
            work("night-h", "2026-11-01T06:50:00Z", true, false),
            work("night-a", "2026-11-01T07:20:00Z", false, true),
            // 01:30 PDT to 01:30 PST: one hour on Nov 1.
            work("fall-h", "2026-11-01T08:30:00Z", true, false),
            work("fall-a", "2026-11-01T09:30:00Z", false, true),
        ],
    );
    seed_on(
        &mut db,
        "later-unknown",
        "cli",
        &[
            work("u1-h", "2026-11-02T20:00:00Z", true, false),
            work("u1-a", "2026-11-02T20:05:00Z", false, true),
            work("u2-h", "2026-11-03T15:00:00Z", true, false),
            work("u2-a", "2026-11-03T15:10:00Z", false, false),
        ],
    );
    let zone = TimeZone::get("America/Los_Angeles").unwrap();
    let read = |db: &TempDb| {
        let metrics = MetricsDb::open(db.path()).unwrap();
        hands_off_support::matches_standalone(&metrics, window, zone.clone())
    };
    // The last reply calls no tool, so the second stretch is measured as none.
    let (range, days) = read(&db);
    assert_eq!(range.n, Some(3));
    assert_eq!(
        days.iter().map(|day| day.n).collect::<Vec<_>>(),
        [Some(1), Some(1), Some(1), Some(0)]
    );
    // Its tool evidence becomes unknown.
    Connection::open(db.path())
        .unwrap()
        .execute(
            "UPDATE records SET tool_use_count=NULL WHERE uuid='u2-a'",
            [],
        )
        .unwrap();
    let (range, days) = read(&db);
    assert_eq!(
        (range.n, range.median_min, range.p90_min),
        (None, None, None)
    );
    let summary: Vec<_> = days
        .iter()
        .map(|day| {
            (
                day.date.as_str(),
                day.end_ms - day.start_ms,
                day.n,
                day.median_min,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("2026-10-31", 14 * 3_600_000, Some(1), Some(30.0)),
            ("2026-11-01", 25 * 3_600_000, Some(1), Some(60.0)),
            ("2026-11-02", 24 * 3_600_000, None, None),
            ("2026-11-03", 9 * 3_600_000, None, None),
        ]
    );
}

#[test]
fn per_day_hands_off_uses_the_ranges_timestamp_exclusions() {
    let mut db = TempDb::empty().unwrap();
    // Three sessions on Sep 1 whose six records share one instant: alone,
    // that day's surface would be excluded (3 of 4 qualifying sessions).
    for n in 0..3 {
        let records: Vec<_> = (0..6)
            .map(|i| {
                work(
                    &format!("d{n}-{i}"),
                    "2026-09-01T08:00:00Z",
                    i % 2 == 0,
                    i % 2 == 1,
                )
            })
            .collect();
        seed_on(&mut db, &format!("degenerate-{n}"), "cli", &records);
    }
    let healthy = |id: &str, day: &str| -> Vec<CanonicalRecord> {
        (0..6)
            .map(|i| {
                work(
                    &format!("{id}-{i}"),
                    &format!("2026-09-{day}T1{i}:00:00Z"),
                    i % 2 == 0,
                    i % 2 == 1,
                )
            })
            .collect()
    };
    seed_on(&mut db, "healthy-1", "cli", &healthy("h1", "01"));
    // Seven more healthy sessions on Sep 2: over the range, 3 of 11 is not
    // more than 30%, so nothing is excluded.
    for n in 0..7 {
        seed_on(
            &mut db,
            &format!("healthy-2-{n}"),
            "cli",
            &healthy(&format!("h2-{n}"), "02"),
        );
    }
    let metrics = MetricsDb::open(db.path()).unwrap();
    let (range, days) = hands_off_support::matches_standalone(&metrics, window(), TimeZone::UTC);
    assert!(range.excluded_surfaces.is_empty());
    // Sep 1 keeps the healthy session's three stretches, as the range does.
    assert_eq!(days[0].n, Some(3));
    assert_eq!(days[1].n, Some(21));
    assert_eq!(
        days.iter().map(|day| day.n.unwrap()).sum::<u64>(),
        range.n.unwrap()
    );
    // Measuring Sep 1 alone would have excluded it.
    let alone = metrics
        .hands_off(Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-02T00:00:00Z")).unwrap())
        .unwrap();
    assert_eq!(alone.excluded_surfaces.len(), 1);
}

#[test]
fn an_unknown_message_within_a_break_after_a_period_makes_it_unknown() {
    // 23:40 on the previous period's last day is known; 00:10 on the next is
    // unknown and could extend that stretch back into the previous period.
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "late",
        &[
            event("k", "2026-09-01T23:40:00Z", true),
            event("u", "2026-09-02T00:10:00Z", true),
        ],
    );
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE records SET is_human=NULL WHERE uuid='u'", [])
        .unwrap();
    let report = MetricsDb::open(db.path())
        .unwrap()
        .human_hours(rolling(), BreakLength::default(), TimeZone::UTC)
        .unwrap();
    assert_eq!(report.previous.active_ms, None);
    assert_eq!(report.previous.messages, None);
    assert_eq!(report.current.active_ms, None);
}
