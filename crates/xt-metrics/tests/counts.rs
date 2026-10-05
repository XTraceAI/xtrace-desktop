use jiff::Timestamp;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{Counts, MetricsDb, TypingRate, Window};
use xt_store::CanonicalRecord;

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
fn query(db: &TempDb) -> Counts {
    MetricsDb::open(db.path())
        .unwrap()
        .counts(window(), TypingRate::default())
        .unwrap()
}
fn record(id: &str, ts: &str, role: &str, content: Value) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":id,"type":if role=="assistant" {"assistant"} else {"user"},"timestamp":ts,"message":{"role":role,"content":content}})).unwrap()
}
// Known classifications come from the writer; explicit NULL simulates legacy
// unknown measurements only. Fixture acceptance below never patches stored flags.
fn seed(records: &[(CanonicalRecord, Option<bool>)]) -> TempDb {
    let mut db = TempDb::empty().unwrap();
    let f = fixture("F1");
    let session = &f.sessions()[0].metadata;
    db.store_mut().upsert_session(session, false).unwrap();
    let c = Connection::open(db.path()).unwrap();
    for (r, human) in records {
        db.store_mut()
            .upsert_records(&session.session_id, std::slice::from_ref(r), false)
            .unwrap();
        if human.is_none() {
            c.execute("UPDATE records SET is_human=NULL WHERE uuid=?1", [&r.uuid])
                .unwrap();
        } else {
            let stored: Option<bool> = c
                .query_row(
                    "SELECT is_human FROM records WHERE uuid=?1",
                    [&r.uuid],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stored, *human);
        }
    }
    db
}
#[test]
fn counts_empty_and_typing_rate_validation() {
    let db = TempDb::empty().unwrap();
    assert_eq!(
        query(&db),
        Counts {
            sessions: 0,
            human_messages: Some(0),
            assistant_turns: Some(0),
            assistant_records: 0,
            tool_calls: Some(0),
            human_chars: Some(0),
            typing_minutes_est: Some(0.0)
        }
    );
    assert!(TypingRate::new(0).is_err());
    assert_eq!(TypingRate::default().characters_per_minute(), 200);
}
#[test]
fn counts_unknown_classification_and_length_are_not_zero_or_inferred() {
    let db = seed(&[
        (
            record(
                "human",
                "2026-09-07T12:00:00Z",
                "user",
                json!([{"type":"text","text":"hi"}]),
            ),
            Some(true),
        ),
        (
            record("unknown", "2026-09-07T12:00:01Z", "assistant", json!([])),
            None,
        ),
    ]);
    let counts = query(&db);
    assert_eq!(counts.sessions, 1);
    assert_eq!(counts.assistant_records, 1);
    assert_eq!(counts.tool_calls, Some(0));
    assert_eq!(counts.human_messages, None);
    assert_eq!(counts.assistant_turns, None);
    assert_eq!(counts.human_chars, None);
    assert_eq!(counts.typing_minutes_est, None);
    let c = Connection::open(db.path()).unwrap();
    c.execute("UPDATE records SET is_human=0 WHERE uuid='unknown'", [])
        .unwrap();
    c.execute("UPDATE records SET text_len=NULL WHERE uuid='human'", [])
        .unwrap();
    let counts = query(&db);
    assert_eq!(counts.human_messages, Some(1));
    assert_eq!(counts.assistant_turns, Some(1));
    assert_eq!(counts.typing_minutes_est, None);
    c.execute(
        "UPDATE records SET tool_use_count=NULL WHERE uuid='unknown'",
        [],
    )
    .unwrap();
    assert_eq!(query(&db).tool_calls, None);
    // Absent length on an explicitly non-human record does not poison typing.
    c.execute("UPDATE records SET text_len=2 WHERE uuid='human'", [])
        .unwrap();
    c.execute("UPDATE records SET text_len=NULL WHERE uuid='unknown'", [])
        .unwrap();
    assert_eq!(query(&db).typing_minutes_est, Some(0.01));
}
#[test]
fn counts_filter_before_segments_and_sort_native_precision_then_uuid() {
    let db = seed(&[
        (
            record(
                "outside-human",
                "2026-08-31T23:59:59.99999999999Z",
                "user",
                json!([{"type":"text","text":"x"}]),
            ),
            Some(true),
        ),
        (
            record("leading", "2026-09-01T00:00:00Z", "assistant", json!([])),
            Some(false),
        ),
        (
            record(
                "z-human",
                "2026-09-07T12:00:00.00000000001Z",
                "user",
                json!([{"type":"text","text":"x"}]),
            ),
            Some(true),
        ),
        (
            record(
                "a-agent",
                "2026-09-07T12:00:00.00000000002Z",
                "assistant",
                json!([]),
            ),
            Some(false),
        ),
        (
            record(
                "a-human",
                "2026-09-07T12:00:01Z",
                "user",
                json!([{"type":"text","text":"x"}]),
            ),
            Some(true),
        ),
        (
            record(
                "z-agent",
                "2026-09-07T05:00:01-07:00",
                "assistant",
                json!([]),
            ),
            Some(false),
        ),
        (
            record(
                "trailing",
                "2026-09-07T23:59:59.99999999999Z",
                "user",
                json!([{"type":"text","text":"x"}]),
            ),
            Some(true),
        ),
        (
            record(
                "outside-agent",
                "2026-09-08T00:00:00Z",
                "assistant",
                json!([]),
            ),
            None,
        ),
        (
            record(
                "outside-leap",
                "2026-09-08T00:00:60Z",
                "user",
                json!([{"type":"text","text":"x"}]),
            ),
            None,
        ),
    ]);
    let counts = query(&db);
    assert_eq!(counts.human_messages, Some(3));
    assert_eq!(counts.assistant_turns, Some(2));
    assert_eq!(counts.assistant_records, 3);
}
#[test]
fn counts_leap_second_overlap_is_filtered_by_exact_native_instant() {
    let db = seed(&[
        (
            record(
                "h",
                "2026-09-07T23:59:59Z",
                "user",
                json!([{"type":"text","text":"x"}]),
            ),
            Some(true),
        ),
        (
            record("a", "2026-09-07T23:59:60.9Z", "assistant", json!([])),
            Some(false),
        ),
    ]);
    let counts = query(&db);
    assert_eq!(counts.human_messages, Some(1));
    assert_eq!(counts.assistant_turns, Some(1));
    let next = Window::new(window().end_ms(), window().end_ms() + 1000).unwrap();
    assert_eq!(
        MetricsDb::open(db.path())
            .unwrap()
            .counts(next, TypingRate::default())
            .unwrap()
            .sessions,
        0
    );
}
#[test]
fn counts_sessions_are_independent_and_empty_sessions_do_not_contribute() {
    let db = seed(&[
        (
            record(
                "h",
                "2026-09-07T12:00:00Z",
                "user",
                json!([{"type":"text","text":"x"}]),
            ),
            Some(true),
        ),
        (
            record("a", "2026-09-07T12:00:01Z", "assistant", json!([])),
            Some(false),
        ),
    ]);
    let c = Connection::open(db.path()).unwrap();
    c.execute("INSERT INTO sessions(session_id,host,source) VALUES ('other','claude','fixture'),('empty','claude','fixture')", []).unwrap();
    c.execute("UPDATE records SET session_id='other' WHERE uuid='a'", [])
        .unwrap();
    assert_eq!(query(&db).sessions, 2);
    assert_eq!(query(&db).assistant_turns, Some(0));
}

#[test]
fn counts_f1_and_f8_real_writer_segments_and_configurable_typing() {
    for (id, humans, turns, assistants, tools, chars) in
        [("F1", 5, 5, 15, 5, 135), ("F8", 1, 1, 3, 1, 6)]
    {
        let f = fixture(id);
        let mut measured = Vec::new();
        for keep_content in [false, true] {
            let db = f.build_db(keep_content).unwrap();
            let result = query(&db);
            f.assert_expectation("M-02", &json!({"human_messages":result.human_messages}))
                .unwrap();
            assert_eq!(
                result,
                Counts {
                    sessions: 1,
                    human_messages: Some(humans),
                    assistant_turns: Some(turns),
                    assistant_records: assistants,
                    tool_calls: Some(tools),
                    human_chars: Some(chars),
                    typing_minutes_est: Some(chars as f64 / 200.0),
                }
            );
            let faster = MetricsDb::open(db.path())
                .unwrap()
                .counts(window(), TypingRate::new(400).unwrap())
                .unwrap();
            assert_eq!(faster.typing_minutes_est, Some(chars as f64 / 400.0));
            assert_eq!(
                Counts {
                    typing_minutes_est: result.typing_minutes_est,
                    ..faster
                },
                result
            );
            measured.push(result);
        }
        assert_eq!(measured[0], measured[1]);
    }
}

#[test]
fn counts_f8_leading_blocks_and_trailing_unanswered_human_do_not_add_turns() {
    let f = fixture("F8");
    let mut db = f.build_db(false).unwrap();
    let session = &f.sessions()[0].metadata.session_id;
    let trailing = record(
        "trailing",
        "2026-09-07T12:01:00Z",
        "user",
        json!([{"type":"text","text":"Next"}]),
    );
    db.store_mut()
        .upsert_records(session, &[trailing], false)
        .unwrap();
    assert_eq!(query(&db).human_messages, Some(2));
    assert_eq!(query(&db).assistant_turns, Some(1));
    // Two consecutive human messages do not manufacture a responding segment.
    let next = record(
        "consecutive",
        "2026-09-07T12:02:00Z",
        "user",
        json!([{"type":"text","text":"Again"}]),
    );
    db.store_mut()
        .upsert_records(session, &[next], false)
        .unwrap();
    assert_eq!(query(&db).human_messages, Some(3));
    assert_eq!(query(&db).assistant_turns, Some(1));
}

#[test]
fn counts_f18_source_orders_replays_and_both_retention_modes_converge() {
    use xt_store::retention::RetentionMode;
    let f = fixture("F18");
    let data = &f.snapshots()["counts"];
    let mut results = Vec::new();
    for keep_content in [false, true] {
        for order in [["native", "hook"], ["hook", "native"]] {
            let mut db = TempDb::empty().unwrap();
            if keep_content {
                db.store_mut()
                    .set_retention_mode(RetentionMode::FullContent)
                    .unwrap();
            }
            let session = &f.sessions()[0].metadata;
            db.store_mut()
                .upsert_session(session, keep_content)
                .unwrap();
            let metric = MetricsDb::open(db.path()).unwrap();
            for source in order {
                let rows: Vec<CanonicalRecord> =
                    serde_json::from_value(data[source].clone()).unwrap();
                db.store_mut()
                    .upsert_records(&session.session_id, &rows, keep_content)
                    .unwrap();
            }
            let result = metric.counts(window(), TypingRate::default()).unwrap();
            assert_eq!(serde_json::to_value(&result).unwrap(), data["expected"]);
            for source in order {
                let rows: Vec<CanonicalRecord> =
                    serde_json::from_value(data[source].clone()).unwrap();
                let stats = db
                    .store_mut()
                    .upsert_records(&session.session_id, &rows, keep_content)
                    .unwrap();
                assert_eq!((stats.inserted, stats.enriched), (0, 0));
                assert_eq!(
                    metric.counts(window(), TypingRate::default()).unwrap(),
                    result
                );
            }
            let stored = db.store().records(&session.session_id).unwrap();
            assert_eq!(stored.len(), 3);
            assert!(stored.iter().all(|row| !row.has_conflict));
            assert!(
                stored
                    .iter()
                    .all(|row| row.content_json.is_some() == keep_content)
            );
            assert!(
                stored
                    .iter()
                    .flat_map(|row| &row.tool_uses)
                    .all(|tool| tool.input_json.is_some() == keep_content)
            );
            results.push(result);
        }
    }
    assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn counts_open_rejects_each_missing_measurement_without_repairing_writer_views() {
    for missing in ["ts", "role", "is_human", "text_len", "tool_use_count"] {
        let db = fixture("F1").build_db(false).unwrap();
        let c = Connection::open(db.path()).unwrap();
        let columns = [
            "uuid",
            "session_id",
            "ts_ms",
            "ts",
            "role",
            "is_human",
            "text_len",
            "tool_use_count",
        ];
        let selected = columns
            .into_iter()
            .filter(|column| *column != missing)
            .collect::<Vec<_>>()
            .join(",");
        c.execute_batch(&format!("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT {selected} FROM v_records WHERE ts_ms IS NOT NULL")).unwrap();
        let schema = || {
            c.query_row(
                "SELECT sql FROM sqlite_master WHERE name='v_session_events'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        let before = schema();
        assert!(MetricsDb::open(db.path()).is_err(), "missing {missing}");
        assert_eq!(schema(), before);
        let writer = xt_store::Store::open(db.path()).unwrap();
        assert_eq!(writer.counts().unwrap().records, 25);
        assert_eq!(query(&db).human_messages, Some(5));
    }
}

#[test]
fn counts_shared_view_exclusions_and_copied_context_do_not_multiply_work() {
    let db = fixture("F1").build_db(false).unwrap();
    let before = query(&db);
    let c = Connection::open(db.path()).unwrap();
    c.execute("INSERT INTO native_record_copies(session_id,record_uuid) SELECT session_id,uuid FROM records", []).unwrap();
    assert_eq!(query(&db), before);
    for change in [
        "UPDATE records SET is_meta=1",
        "UPDATE records SET model='<synthetic>'",
        "UPDATE sessions SET kind='judge'",
    ] {
        let db = fixture("F1").build_db(false).unwrap();
        Connection::open(db.path())
            .unwrap()
            .execute(change, [])
            .unwrap();
        assert_eq!(query(&db).sessions, 0);
        assert_eq!(query(&db).assistant_turns, Some(0));
    }
}

#[test]
fn counts_checked_measured_counter_overflow_is_an_error() {
    let db = fixture("F1").build_db(false).unwrap();
    let c = Connection::open(db.path()).unwrap();
    c.execute(
        "UPDATE records SET text_len=?1 WHERE is_human=1",
        [i64::MAX],
    )
    .unwrap();
    assert!(matches!(
        MetricsDb::open(db.path())
            .unwrap()
            .counts(window(), TypingRate::default()),
        Err(xt_metrics::Error::CounterOverflow)
    ));
    c.execute(
        "UPDATE records SET text_len=1,tool_use_count=?1",
        [i64::MAX],
    )
    .unwrap();
    assert!(matches!(
        MetricsDb::open(db.path())
            .unwrap()
            .counts(window(), TypingRate::default()),
        Err(xt_metrics::Error::CounterOverflow)
    ));
}
