use jiff::Timestamp;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{HumanTime, MetricsDb, TypingRate, Window};
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
        .human_time(window, TypingRate::default())
        .unwrap()
}
#[test]
fn human_intervals_f1_ratio_and_metadata_replay_parity() {
    let f = fixture("F1");
    let mut results = Vec::new();
    for keep in [false, true] {
        let mut db = f.build_db(keep).unwrap();
        let report = query(&db, window());
        assert_eq!(report.human_minutes_est, Some(8.135));
        assert_eq!(report.summed_session_minutes_est, Some(8.135));
        assert_eq!(report.agent_minutes, 23.0);
        assert_eq!(report.agent_to_human_ratio, Some(23.0 / 8.135));
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
fn human_intervals_f2_union_ratio_uses_each_session_union_not_raw_sum() {
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
            assert_eq!(serde_json::to_value(report).unwrap(), snapshot["expected"]);
        }
    }
}
#[test]
fn human_intervals_cap_tool_result_consecutive_and_adjacent_union() {
    let mut db = TempDb::empty().unwrap();
    let mut tool = event("tool", "2026-09-07T12:50:00Z", true);
    tool.message.content = Some(vec![json!({"type":"tool_result","content":"Synthetic"})]);
    seed(
        &mut db,
        "s",
        &[
            event("agent", "2026-09-07T12:00:00Z", false),
            event("cap", "2026-09-07T12:45:00Z", true), // [15,45]
            tool,
            event("h1", "2026-09-07T12:55:00Z", true), // [50,55]
            event("h2", "2026-09-07T13:00:00Z", true), // [50,60], contains prior
            event("z-agent", "2026-09-07T13:00:00Z", false),
            event("h3", "2026-09-07T13:05:00Z", true), // [60,65], adjacent
        ],
        false,
    );
    let result = query(&db, window());
    assert_eq!(result.human_minutes_est, Some(45.0));
    assert_eq!(result.summed_session_minutes_est, Some(45.0));
}
#[test]
fn human_intervals_f3_first_in_window_ignores_outside_agent_and_clips_typing() {
    let mut db = fixture("F3").build_db(false).unwrap();
    assert_eq!(query(&db, window()).human_minutes_est, Some(0.0));
    assert_eq!(query(&db, window()).agent_to_human_ratio, None);
    let session = fixture("F3").sessions()[0].metadata.session_id.clone();
    db.store_mut()
        .upsert_records(
            &session,
            &[event("first", "2026-09-01T00:00:02Z", true)],
            false,
        )
        .unwrap();
    let w = Window::new(ms("2026-09-01T00:00:01.900Z"), ms("2026-09-01T00:00:03Z")).unwrap();
    assert_eq!(query(&db, w).human_minutes_est, Some(100.0 / 60000.0));
    assert_eq!(query(&db, w).agent_to_human_ratio, Some(0.0));
    let w = Window::new(ms("2026-09-01T00:00:02Z"), ms("2026-09-01T00:00:03Z")).unwrap();
    assert_eq!(query(&db, w).human_minutes_est, Some(0.0));
    assert_eq!(query(&db, w).agent_to_human_ratio, None);
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
        .human_time(w, TypingRate::new(u32::MAX).unwrap())
        .unwrap();
    assert_eq!(tiny.human_minutes_est, Some(1.0 / f64::from(u32::MAX)));
    assert!(tiny.human_minutes_est.unwrap() > 0.0);
    assert_eq!(
        metric
            .human_time(w, TypingRate::new(400).unwrap())
            .unwrap()
            .human_minutes_est,
        Some(1.0 / 400.0)
    );
}
#[test]
fn human_intervals_precise_order_precedes_uuid_and_negative_leap_gap_is_zero() {
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
    assert_eq!(query(&db, window()).human_minutes_est, Some(1.0 / 200.0));
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
    assert_eq!(query(&db, w).human_minutes_est, Some(0.0));
    assert_eq!(query(&db, w).agent_to_human_ratio, None);
}
#[test]
fn human_intervals_unknowns_only_require_typing_length_without_prior_agent() {
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
    assert_eq!(query(&db, window()).human_minutes_est, Some(1.0 / 60.0));
    c.execute("UPDATE records SET is_human=NULL WHERE uuid='agent'", [])
        .unwrap();
    let result = query(&db, window());
    assert_eq!(result.human_minutes_est, None);
    assert_eq!(result.summed_session_minutes_est, None);
    assert_eq!(result.agent_to_human_ratio, None);
    assert_eq!(result.agent_minutes, 1.0 / 60.0);
    // Unknown classification outside the window cannot poison the selected work.
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
            agent_to_human_ratio: None
        }
    );
    let value: Value = serde_json::to_value(query(&db, window())).unwrap();
    assert_eq!(value["agent_to_human_ratio"], Value::Null);
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
    assert_eq!(query(&db, w).human_minutes_est, Some(500.0 / 60000.0));
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
