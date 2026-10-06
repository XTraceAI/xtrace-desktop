use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{DayHuman, HumanTime, MetricsDb, TypingRate, Window};
use xt_store::{CanonicalRecord, retention::RetentionMode};
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
fn event(id: &str, ts: &str, human: bool) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":id,"type":if human {"user"} else {"assistant"},"timestamp":ts,"message":{"role":if human {"user"} else {"assistant"},"content":[{"type":"text","text":"x"}]}})).unwrap()
}
fn seed(db: &mut TempDb, id: &str, records: &[CanonicalRecord], keep: bool) {
    let mut session = fixture("F1").sessions()[0].metadata.clone();
    session.session_id = id.into();
    db.store_mut().upsert_session(&session, keep).unwrap();
    db.store_mut().upsert_records(id, records, keep).unwrap();
}
fn query(db: &TempDb, window: Window) -> HumanTime {
    MetricsDb::open(db.path())
        .unwrap()
        .human_time(window, TypingRate::default(), TimeZone::UTC)
        .unwrap()
}
#[test]
fn human_characters_f1_ratio_and_metadata_replay_parity() {
    let f = fixture("F1");
    let mut results = Vec::new();
    for keep in [false, true] {
        let mut db = f.build_db(keep).unwrap();
        let report = query(&db, window());
        assert_eq!(report.human_minutes_est, Some(0.675));
        assert_eq!(report.summed_session_minutes_est, Some(0.675));
        assert_eq!(report.agent_minutes, 23.0);
        for session in f.sessions() {
            seed(
                &mut db,
                &session.metadata.session_id,
                &session.records,
                keep,
            );
        }
        assert_eq!(query(&db, window()), report);
        results.push(report);
    }
    assert_eq!(results[0], results[1]);
}
#[test]
fn human_characters_f2_are_additive_across_sessions() {
    let f = fixture("F2");
    let snapshot = &f.snapshots()["human_time"];
    for keep in [false, true] {
        for reversed in [false, true] {
            let mut db = TempDb::empty().unwrap();
            if keep {
                db.store_mut()
                    .set_retention_mode(RetentionMode::FullContent)
                    .unwrap();
            }
            for session in f.snapshots()["spans"]["lanes"].as_array().unwrap() {
                let mut rows: Vec<CanonicalRecord> =
                    serde_json::from_value(session["records"].clone()).unwrap();
                if reversed {
                    rows.reverse();
                }
                seed(
                    &mut db,
                    session["session_id"].as_str().unwrap(),
                    &rows,
                    keep,
                );
            }
            let report = query(&db, window());
            let expected = snapshot["expected"].clone();
            assert_eq!(serde_json::to_value(report).unwrap(), expected);
        }
    }
}
#[test]
fn human_characters_ignore_agent_gaps_and_tool_result_text() {
    let mut db = TempDb::empty().unwrap();
    let mut tool = event("tool", "2026-09-07T12:50:00Z", true);
    tool.message.content = Some(vec![json!({"type":"tool_result","content":"Synthetic"})]);
    seed(
        &mut db,
        "s",
        &[
            event("agent", "2026-09-07T12:00:00Z", false),
            event("cap", "2026-09-07T12:45:00Z", true),
            tool,
            event("h1", "2026-09-07T12:55:00Z", true),
            event("h2", "2026-09-07T13:00:00Z", true),
            event("z-agent", "2026-09-07T13:00:00Z", false),
            event("h3", "2026-09-07T13:05:00Z", true),
        ],
        false,
    );
    let result = query(&db, window());
    assert_eq!(result.human_minutes_est, Some(0.02));
    assert_eq!(result.summed_session_minutes_est, Some(0.02));
}
#[test]
fn human_characters_include_full_length_at_window_start() {
    let mut db = fixture("F3").build_db(false).unwrap();
    assert_eq!(query(&db, window()).human_minutes_est, Some(0.0));
    let session = fixture("F3").sessions()[0].metadata.session_id.clone();
    db.store_mut()
        .upsert_records(
            &session,
            &[event("first", "2026-09-01T00:00:02Z", true)],
            false,
        )
        .unwrap();
    let w = Window::new(ms("2026-09-01T00:00:01.900Z"), ms("2026-09-01T00:00:03Z")).unwrap();
    assert_eq!(query(&db, w).human_minutes_est, Some(0.005));
    assert_eq!(query(&db, w).agent_minutes, 0.0);
    let w = Window::new(ms("2026-09-01T00:00:02Z"), ms("2026-09-01T00:00:03Z")).unwrap();
    assert_eq!(query(&db, w).human_minutes_est, Some(0.005));
    assert_eq!(query(&db, w).agent_minutes, 0.0);
}
#[test]
fn human_intervals_tiny_typing_at_large_epoch_and_configurable_rate() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[event("human", "9999-01-01T00:00:01Z", true)],
        false,
    );
    let w = Window::new(ms("9999-01-01T00:00:00Z"), ms("9999-01-01T00:00:02Z")).unwrap();
    let metric = MetricsDb::open(db.path()).unwrap();
    let tiny = metric
        .human_time(w, TypingRate::new(u32::MAX).unwrap(), TimeZone::UTC)
        .unwrap();
    assert_eq!(tiny.human_minutes_est, Some(1.0 / f64::from(u32::MAX)));
    assert!(tiny.human_minutes_est.unwrap() > 0.0);
    assert_eq!(
        metric
            .human_time(w, TypingRate::new(400).unwrap(), TimeZone::UTC)
            .unwrap()
            .human_minutes_est,
        Some(1.0 / 400.0)
    );
}
#[test]
fn human_characters_count_same_instant_inputs_and_leap_boundaries() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            event("z-human", "2026-09-07T12:00:00.00000000001Z", true),
            event("a-agent", "2026-09-07T12:00:00.00000000002Z", false),
            event("later-human", "2026-09-07T12:00:00.00000000003Z", true),
        ],
        false,
    );
    assert_eq!(query(&db, window()).human_minutes_est, Some(2.0 / 200.0));
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            event("leap-agent", "2026-09-07T23:59:60.9Z", false),
            event("next-human", "2026-09-08T00:00:00Z", true),
        ],
        false,
    );
    let w = Window::new(ms("2026-09-07T23:59:59Z"), ms("2026-09-08T00:00:01Z")).unwrap();
    assert_eq!(query(&db, w).human_minutes_est, Some(0.005));
    assert_eq!(query(&db, w).agent_minutes, 900.0 / 60_000.0);
}
#[test]
fn human_characters_require_every_eligible_length() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[event("human", "2026-09-07T12:00:01Z", true)],
        false,
    );
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE records SET text_len=NULL", []).unwrap();
    assert_eq!(query(&db, window()).human_minutes_est, None);
    seed(
        &mut db,
        "s",
        &[event("agent", "2026-09-07T12:00:00Z", false)],
        false,
    );
    assert_eq!(query(&db, window()).human_minutes_est, None);
    c.execute("UPDATE records SET is_human=NULL WHERE uuid='agent'", [])
        .unwrap();
    let result = query(&db, window());
    assert_eq!(result.human_minutes_est, None);
    assert_eq!(result.summed_session_minutes_est, None);
    assert_eq!(result.agent_minutes, 1.0 / 60.0);
    let w = Window::new(ms("2026-09-07T12:00:00.500Z"), ms("2026-09-07T12:00:02Z")).unwrap();
    c.execute("UPDATE records SET text_len=1 WHERE uuid='human'", [])
        .unwrap();
    assert_eq!(query(&db, w).human_minutes_est, Some(1.0 / 200.0));
}
#[test]
fn human_intervals_empty_ratio_and_serialization_preserve_unknowns() {
    let db = TempDb::empty().unwrap();
    assert_eq!(
        query(&db, window()),
        HumanTime {
            human_minutes_est: Some(0.0),
            summed_session_minutes_est: Some(0.0),
            agent_minutes: 0.0,
            by_day: (1..=7)
                .map(|day| DayHuman {
                    date: format!("2026-09-0{day}"),
                    start_ms: ms(&format!("2026-09-0{day}T00:00:00Z")),
                    end_ms: ms(&format!("2026-09-0{}T00:00:00Z", day + 1)),
                    minutes_est: Some(0.0),
                })
                .collect(),
        }
    );
}

#[test]
fn human_intervals_leap_overlap_clips_end_after_exact_membership() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            event("agent", "2026-09-07T23:59:59.5Z", false),
            event("human", "2026-09-07T23:59:60.9Z", true),
            event("outside", "2026-09-08T00:00:00Z", true),
        ],
        false,
    );
    let w = Window::new(ms("2026-09-07T23:59:59Z"), ms("2026-09-08T00:00:00Z")).unwrap();
    assert_eq!(query(&db, w).human_minutes_est, Some(0.005));
}

#[test]
fn human_intervals_open_rejects_missing_projection_and_never_repairs_it() {
    let db = TempDb::empty().unwrap();
    let c = Connection::open(db.path()).unwrap();
    c.execute_batch("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT uuid,session_id,host,ts,ts_ms,role,is_human,tool_use_count FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
    let schema = || {
        c.query_row(
            "SELECT sql FROM sqlite_master WHERE name='v_session_events'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
    };
    let before = schema();
    assert!(MetricsDb::open(db.path()).is_err());
    assert_eq!(schema(), before);
    xt_store::Store::open(db.path()).unwrap();
    assert!(MetricsDb::open(db.path()).is_ok());
}

fn typed(id: &str, ts: &str, characters: usize) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":id,"type":"user","timestamp":ts,
        "message":{"role":"user","content":[{"type":"text","text":"x".repeat(characters)}]}}))
    .unwrap()
}
fn by_day(report: &HumanTime, w: Window, zone: TimeZone) -> Vec<(String, Option<f64>)> {
    let buckets = w.local_days(zone).unwrap();
    assert_eq!(report.by_day.len(), buckets.len());
    for (day, bucket) in report.by_day.iter().zip(&buckets) {
        assert_eq!(
            (day.date.as_str(), day.start_ms, day.end_ms),
            (
                bucket.date.to_string().as_str(),
                bucket.window.start_ms(),
                bucket.window.end_ms()
            )
        );
    }
    match report.human_minutes_est {
        Some(total) => {
            let summed: f64 = report.by_day.iter().filter_map(|day| day.minutes_est).sum();
            assert_eq!(
                report
                    .by_day
                    .iter()
                    .filter(|d| d.minutes_est.is_none())
                    .count(),
                0
            );
            assert!((summed - total).abs() < 1e-9, "{summed} vs {total}");
        }
        None => assert!(report.by_day.iter().all(|day| day.minutes_est.is_none())),
    }
    report
        .by_day
        .iter()
        .map(|day| (day.date.clone(), day.minutes_est))
        .collect()
}

#[test]
fn human_by_day_uses_message_timestamp() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            event("agent", "2026-09-02T23:50:00Z", false),
            event("human", "2026-09-03T00:05:00Z", true),
        ],
        false,
    );
    let report = query(&db, window());
    assert_eq!(report.human_minutes_est, Some(0.005));
    let days = by_day(&report, window(), TimeZone::UTC);
    assert_eq!(days[1], ("2026-09-02".into(), Some(0.0)));
    assert_eq!(days[2], ("2026-09-03".into(), Some(0.005)));
    assert!(
        days.iter()
            .enumerate()
            .all(|(i, (_, m))| i == 1 || i == 2 || *m == Some(0.0))
    );
    let pacific = TimeZone::get("America/Los_Angeles").unwrap();
    let recut = MetricsDb::open(db.path())
        .unwrap()
        .human_time(window(), TypingRate::default(), pacific.clone())
        .unwrap();
    assert_eq!(recut.human_minutes_est, report.human_minutes_est);
    assert_eq!(
        recut.summed_session_minutes_est,
        report.summed_session_minutes_est
    );
    assert_eq!(recut.agent_minutes, report.agent_minutes);
    assert_eq!(
        by_day(&recut, window(), pacific)
            .into_iter()
            .filter(|(_, m)| *m != Some(0.0))
            .collect::<Vec<_>>(),
        [("2026-09-02".into(), Some(0.005))]
    );
}

#[test]
fn human_by_day_ignores_prior_agent_context() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            event("agent", "2026-09-02T23:40:00Z", false),
            event("h1", "2026-09-02T23:55:00Z", true),
            event("h2", "2026-09-03T00:10:00Z", true),
        ],
        false,
    );
    let report = query(&db, window());
    assert_eq!(report.human_minutes_est, Some(0.01));
    let days = by_day(&report, window(), TimeZone::UTC);
    assert_eq!(days[1], ("2026-09-02".into(), Some(0.005)));
    assert_eq!(days[2], ("2026-09-03".into(), Some(0.005)));
}

#[test]
fn human_by_day_adds_concurrent_sessions() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "a",
        &[
            event("a-agent", "2026-09-02T23:30:00Z", false),
            event("a-human", "2026-09-03T00:00:00Z", true),
        ],
        false,
    );
    seed(
        &mut db,
        "b",
        &[
            event("b-agent", "2026-09-02T23:45:00Z", false),
            event("b-human", "2026-09-03T00:10:00Z", true),
        ],
        false,
    );
    let report = query(&db, window());
    assert_eq!(report.human_minutes_est, Some(0.01));
    assert_eq!(report.summed_session_minutes_est, Some(0.01));
    let days = by_day(&report, window(), TimeZone::UTC);
    assert_eq!(days[1], ("2026-09-02".into(), Some(0.0)));
    assert_eq!(days[2], ("2026-09-03".into(), Some(0.01)));
}

#[test]
fn human_by_day_keeps_whole_message_length_on_its_day() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[typed("h", "2026-09-03T00:04:00Z", 2000)],
        false,
    );
    let report = query(&db, window());
    assert_eq!(report.human_minutes_est, Some(10.0));
    let days = by_day(&report, window(), TimeZone::UTC);
    assert_eq!(days[1], ("2026-09-02".into(), Some(0.0)));
    assert_eq!(days[2], ("2026-09-03".into(), Some(10.0)));
}

#[test]
fn human_by_day_reports_a_window_wide_unknown_on_every_day() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            event("agent", "2026-09-05T12:00:00Z", false),
            event("human", "2026-09-05T12:10:00Z", true),
            event("later", "2026-09-05T12:20:00Z", false),
        ],
        false,
    );
    let measured = query(&db, window());
    assert_eq!(measured.human_minutes_est, Some(0.005));
    assert_eq!(by_day(&measured, window(), TimeZone::UTC)[4].1, Some(0.005));
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE records SET is_human=NULL WHERE uuid='agent'", [])
        .unwrap();
    let unknown = query(&db, window());
    assert_eq!(unknown.human_minutes_est, None);
    assert_eq!(unknown.agent_minutes, measured.agent_minutes);
    let days = by_day(&unknown, window(), TimeZone::UTC);
    assert_eq!(days.len(), 7);
    assert!(days.iter().all(|(_, minutes)| minutes.is_none()));
}

#[test]
fn human_by_day_keeps_a_tiny_positive_estimate_apart_from_a_measured_zero() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[typed("h", "2026-09-05T12:00:00Z", 1)],
        false,
    );
    let report = query(&db, window());
    let days = by_day(&report, window(), TimeZone::UTC);
    assert_eq!(days[4].1, Some(0.005));
    assert!(days[4].1.is_some_and(|minutes| minutes > 0.0));
    assert_eq!(days[3].1, Some(0.0));
}

#[test]
fn human_by_day_holds_a_short_and_a_long_dst_day_in_one_bucket_each() {
    let zone = TimeZone::get("America/Los_Angeles").unwrap();
    for (start, end, date, agent, human) in [
        (
            "2026-03-08T08:00:00Z",
            "2026-03-09T07:00:00Z",
            "2026-03-08",
            "2026-03-08T18:00:00Z",
            "2026-03-08T18:05:00Z",
        ),
        (
            "2026-11-01T07:00:00Z",
            "2026-11-02T08:00:00Z",
            "2026-11-01",
            "2026-11-01T18:00:00Z",
            "2026-11-01T18:05:00Z",
        ),
    ] {
        let mut db = TempDb::empty().unwrap();
        seed(
            &mut db,
            "s",
            &[event("agent", agent, false), event("human", human, true)],
            false,
        );
        let w = Window::new(ms(start), ms(end)).unwrap();
        let report = MetricsDb::open(db.path())
            .unwrap()
            .human_time(w, TypingRate::default(), zone.clone())
            .unwrap();
        assert_eq!(report.human_minutes_est, Some(0.005));
        assert_eq!(
            by_day(&report, w, zone.clone()),
            [(date.into(), Some(0.005))]
        );
    }
}

#[test]
fn human_by_day_keeps_a_leap_clipped_estimate_on_the_last_reported_day() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "s",
        &[
            event("agent", "2016-12-31T23:59:00Z", false),
            event("human", "2016-12-31T23:59:60.9Z", true),
        ],
        false,
    );
    let boundary = ms("2017-01-01T00:00:00Z");
    let w = Window::new(boundary - 2 * 86_400_000, boundary).unwrap();
    let report = MetricsDb::open(db.path())
        .unwrap()
        .human_time(w, TypingRate::default(), TimeZone::UTC)
        .unwrap();
    assert_eq!(report.human_minutes_est, Some(0.005));
    assert_eq!(
        by_day(&report, w, TimeZone::UTC),
        [
            ("2016-12-30".into(), Some(0.0)),
            ("2016-12-31".into(), Some(0.005))
        ]
    );
}

#[test]
fn human_characters_twelve_thousand_at_saved_rates_without_cap_or_overlap() {
    let mut db = TempDb::empty().unwrap();
    for id in ["a", "b"] {
        seed(
            &mut db,
            id,
            &[typed(id, "2026-09-01T00:00:00Z", 6000)],
            false,
        );
    }
    let metrics = MetricsDb::open(db.path()).unwrap();
    for (rate, minutes) in [(200, 60.0), (400, 30.0)] {
        let report = metrics
            .human_time(window(), TypingRate::new(rate).unwrap(), TimeZone::UTC)
            .unwrap();
        assert_eq!(report.human_minutes_est, Some(minutes));
        assert_eq!(report.summed_session_minutes_est, Some(minutes));
        assert_eq!(report.by_day[0].minutes_est, Some(minutes));
        assert_eq!(
            metrics
                .counts(window(), TypingRate::new(rate).unwrap())
                .unwrap()
                .typing_minutes_est,
            Some(minutes)
        );
    }
}

#[test]
fn excluded_missing_length_does_not_poison_human_estimate_or_change_agent_work() {
    let mut db = TempDb::empty().unwrap();
    let mut meta = fixture("F1").sessions()[0].metadata.clone();
    meta.session_id = "worker".into();
    meta.native_session_id = Some("worker-native".into());
    db.store_mut().upsert_session(&meta, false).unwrap();
    db.store_mut()
        .upsert_records(
            "worker",
            &[
                event("worker-input", "2026-09-01T00:00:00Z", true),
                event("worker-agent", "2026-09-01T00:01:00Z", false),
            ],
            false,
        )
        .unwrap();
    let sql = Connection::open(db.path()).unwrap();
    sql.execute(
        "UPDATE records SET text_len=NULL WHERE uuid='worker-input'",
        [],
    )
    .unwrap();
    let before = query(&db, window());
    assert_eq!(before.human_minutes_est, None);
    db.store_mut()
        .import_human_session_origins(&xt_store::human_input::OriginManifest {
            version: 1,
            sessions: vec![xt_store::human_input::SessionOrigin {
                session_id: "worker".into(),
                host: meta.host,
                native_session_id: "worker-native".into(),
                parent_host: meta.host,
                parent_native_session_id: "parent-native".into(),
                method: "explicit_session_id".into(),
                evidence_id: "audit".into(),
                launch_id: "call".into(),
            }],
        })
        .unwrap();
    let after = query(&db, window());
    assert_eq!(after.human_minutes_est, Some(0.0));
    assert_eq!(after.agent_minutes, before.agent_minutes);
    let metrics = MetricsDb::open(db.path()).unwrap();
    assert_eq!(
        metrics
            .counts(window(), TypingRate::default())
            .unwrap()
            .human_messages,
        Some(0)
    );
    let raw: Option<i64> = sql
        .query_row(
            "SELECT text_len FROM records WHERE uuid='worker-input'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, None);
}
