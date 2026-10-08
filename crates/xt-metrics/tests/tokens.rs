use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{MetricsDb, PriceCatalog, TokenReport, TokenSummary, Window};
use xt_store::{CanonicalRecord, Host, SessionMeta, SessionSource};
mod report_support;
fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}
fn ms(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_millisecond()
}
fn value(t: &TokenSummary) -> Value {
    json!({"selected_responses":t.selected_responses,"input_tokens":t.counters.input_tokens,"output_tokens":t.counters.output_tokens,"cache_read_tokens":t.counters.cache_read_tokens,"cache_creation_tokens":t.counters.cache_creation_tokens,"total_tokens":t.counters.total_tokens})
}
fn rows(v: &Value) -> Vec<CanonicalRecord> {
    serde_json::from_value(v.clone()).unwrap()
}
fn seed(id: &str) -> (Fixture, TempDb) {
    let f = fixture(id);
    let mut db = TempDb::empty().unwrap();
    let session = &f.sessions()[0].metadata;
    db.store_mut().upsert_session(session, false).unwrap();
    db.store_mut()
        .upsert_records(
            &session.session_id,
            &rows(&f.snapshots()["tokens"]["records"]),
            false,
        )
        .unwrap();
    (f, db)
}
fn query(db: &TempDb) -> TokenReport {
    tokens_in(
        &MetricsDb::open(db.path()).unwrap(),
        window(),
        TimeZone::UTC,
    )
    .unwrap()
}
/// The token report, checked against the combined report read.
fn tokens_in(
    metrics: &MetricsDb,
    window: Window,
    zone: TimeZone,
) -> xt_metrics::Result<TokenReport> {
    let catalog = PriceCatalog::bundled().unwrap();
    report_support::matches_standalone(metrics, window, window.end_ms(), &[], zone, &catalog)
        .current
        .tokens
}
fn breakdowns_match(r: &TokenReport) {
    let total = r.total.counters.total_tokens.unwrap();
    assert_eq!(
        r.by_host
            .iter()
            .map(|v| v.tokens.counters.total_tokens.unwrap_or(0))
            .sum::<u64>(),
        total
    );
    assert_eq!(
        r.by_model
            .iter()
            .map(|v| v.tokens.counters.total_tokens.unwrap_or(0))
            .sum::<u64>(),
        total
    );
    assert_eq!(
        r.by_surface
            .iter()
            .map(|v| v.tokens.counters.total_tokens.unwrap_or(0))
            .sum::<u64>(),
        total
    );
    assert_eq!(
        r.by_day
            .iter()
            .map(|v| v.tokens.counters.total_tokens.unwrap_or(0))
            .sum::<u64>(),
        total
    );
}
#[test]
fn tokens_f1_and_host_goldens_share_all_breakdowns() {
    let f = fixture("F1");
    let db = f.build_db(false).unwrap();
    let r = query(&db);
    let c = &r.total.counters;
    f.assert_expectation("M-04",&json!({"selected_responses":r.total.selected_responses,"input_tokens":c.input_tokens,"output_tokens":c.output_tokens,"cache_read_input_tokens":c.cache_read_tokens,"cache_creation_input_tokens":c.cache_creation_tokens,"total_tokens":c.total_tokens})).unwrap();
    breakdowns_match(&r);
    for id in ["F5", "F6"] {
        let (f, db) = seed(id);
        let r = query(&db);
        assert_eq!(value(&r.total), f.snapshots()["tokens"]["expected"]);
        breakdowns_match(&r);
    }
}
#[test]
fn tokens_f17_selects_before_window_and_day_without_deleting_content_rows() {
    let (f, db) = seed("F17");
    let r = query(&db);
    let golden = &f.snapshots()["tokens"];
    assert_eq!(value(&r.total), golden["expected"]);
    breakdowns_match(&r);
    let c = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&c).unwrap();
    let selected: Vec<String> = c
        .prepare("SELECT uuid FROM v_response_usage WHERE ts_ms>=?1 AND ts_ms<?2 ORDER BY uuid")
        .unwrap()
        .query_map([window().start_ms(), window().end_ms()], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(json!(selected), golden["selected_uuids"]);
    assert_eq!(
        db.store().counts().unwrap().records,
        golden["stored_records"].as_u64().unwrap()
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM tool_uses", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        golden["tool_calls"].as_i64().unwrap()
    );
    assert_eq!(r.by_day[0].tokens.selected_responses, 0);
    assert_eq!(r.by_day[1].tokens.selected_responses, 1);
    let next = Window::new(window().end_ms(), ms("2026-09-15T00:00:00Z")).unwrap();
    let r = MetricsDb::open(db.path())
        .unwrap()
        .tokens(next, TimeZone::UTC)
        .unwrap();
    assert_eq!(r.total.selected_responses, 1);
    assert_eq!(r.total.counters.total_tokens, Some(44));
    assert!(
        query(&db)
            .by_model
            .iter()
            .any(|m| m.model.is_none() && m.tokens.counters.total_tokens == Some(6))
    );
}
#[test]
fn tokens_f18_enrichment_converges_in_both_orders_and_replays() {
    let f = fixture("F18");
    let data = &f.snapshots()["tokens"];
    for order in [["native", "hook"], ["hook", "native"]] {
        let mut db = TempDb::empty().unwrap();
        let session = &f.sessions()[0].metadata;
        db.store_mut().upsert_session(session, false).unwrap();
        let metric = MetricsDb::open(db.path()).unwrap();
        for source in order {
            db.store_mut()
                .upsert_records(&session.session_id, &rows(&data[source]), false)
                .unwrap();
        }
        let stable_uuid = rows(&data["native"])[0].uuid.clone().unwrap();
        let expected = metric.tokens(window(), TimeZone::UTC).unwrap();
        assert_eq!(value(&expected.total), data["expected"]);
        for source in order {
            db.store_mut()
                .upsert_records(&session.session_id, &rows(&data[source]), false)
                .unwrap();
            assert_eq!(metric.tokens(window(), TimeZone::UTC).unwrap(), expected);
            let stored = db.store().records(&session.session_id).unwrap();
            assert_eq!(stored.len(), 1);
            assert_eq!(stored[0].uuid, stable_uuid);
        }
        assert_eq!(db.store().counts().unwrap().records, 1);
        assert_eq!(
            query(&db).by_surface[0].surface.as_deref(),
            Some("desktop.next")
        );
    }
}
fn one(usage: Value) -> (TempDb, SessionMeta, CanonicalRecord) {
    let mut db = TempDb::empty().unwrap();
    let session = SessionMeta {
        session_id: "test".into(),
        host: Host::Claude,
        source: SessionSource::Fixture,
        ..fixture("F1").sessions()[0].metadata.clone()
    };
    db.store_mut().upsert_session(&session, false).unwrap();
    let record:CanonicalRecord=serde_json::from_value(json!({"uuid":"row","type":"assistant","timestamp":"2026-09-07T12:00:00Z","message":{"role":"assistant","model":"known-model","usage":usage}})).unwrap();
    db.store_mut()
        .upsert_records(&session.session_id, std::slice::from_ref(&record), false)
        .unwrap();
    (db, session, record)
}
#[test]
fn tokens_unknown_partial_and_zero_are_distinct_and_enrichment_is_visible() {
    for usage in [
        Value::Null,
        json!({"input_tokens":100}),
        json!({"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
    ] {
        let (db, _, _) = one(usage.clone());
        let r = query(&db);
        let zero = usage.get("output_tokens").is_some();
        assert_eq!(r.total.measured_sessions, u64::from(zero));
        assert_eq!(r.total.counters.total_tokens, zero.then_some(0));
        assert_eq!(
            r.by_model
                .first()
                .and_then(|m| m.tokens.counters.output_tokens),
            zero.then_some(0)
        );
    }
    let (mut db, s, mut row) = one(json!({"input_tokens":100}));
    let reader = MetricsDb::open(db.path()).unwrap();
    assert_eq!(
        reader
            .tokens(window(), TimeZone::UTC)
            .unwrap()
            .total
            .counters
            .total_tokens,
        None
    );
    row.message.usage=Some(serde_json::from_value(json!({"input_tokens":100,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0})).unwrap());
    db.store_mut()
        .upsert_records(&s.session_id, &[row], false)
        .unwrap();
    assert_eq!(
        reader
            .tokens(window(), TimeZone::UTC)
            .unwrap()
            .total
            .counters
            .total_tokens,
        Some(105)
    );
}
#[test]
fn tokens_excludes_metadata_judges_synthetic_and_does_not_multiply_copies() {
    let (f, db) = seed("F17");
    let c = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&c).unwrap();
    let before = query(&db);
    c.execute("INSERT INTO native_record_copies(session_id,record_uuid) SELECT session_id,uuid FROM records",[]).unwrap();
    assert_eq!(query(&db), before);
    c.execute("UPDATE sessions SET kind='judge'", []).unwrap();
    assert_eq!(query(&db).total.selected_responses, 0);
    c.execute("UPDATE sessions SET kind='user'", []).unwrap();
    c.execute("UPDATE records SET is_meta=1", []).unwrap();
    assert_eq!(query(&db).total.selected_responses, 0);
    c.execute("UPDATE records SET is_meta=0,model='<synthetic>'", [])
        .unwrap();
    assert_eq!(query(&db).total.selected_responses, 0);
    assert_eq!(
        db.store()
            .records(&f.sessions()[0].metadata.session_id)
            .unwrap()
            .len(),
        8
    );
}
#[test]
fn tokens_empty_and_overflow_are_explicit() {
    let db = TempDb::empty().unwrap();
    assert_eq!(query(&db).total.counters.total_tokens, None);
    let (mut db, s, row) = one(
        json!({"input_tokens":i64::MAX,"output_tokens":i64::MAX,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
    );
    let mut second = row;
    second.uuid = Some("another".into());
    db.store_mut()
        .upsert_records(&s.session_id, &[second], false)
        .unwrap();
    assert!(matches!(
        MetricsDb::open(db.path())
            .unwrap()
            .tokens(window(), TimeZone::UTC),
        Err(xt_metrics::Error::CounterOverflow)
    ));
}

#[test]
fn tokens_usage_snapshot_wins_over_later_content_only_and_partial_is_not_zero() {
    let (mut db, s, mut first) = one(
        json!({"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
    );
    first.uuid = Some("usage-1".into());
    first.api_message_id = Some("response".into());
    first.request_id = Some("request".into());
    let mut later = first.clone();
    later.uuid = Some("usage-2".into());
    later.timestamp = Some("2026-09-08T00:00:00Z".into());
    later.message.usage = None;
    db.store_mut()
        .upsert_records(&s.session_id, &[first.clone(), later.clone()], false)
        .unwrap();
    assert_eq!(query(&db).total.selected_responses, 2); // original fallback row + one response
    assert_eq!(query(&db).total.counters.total_tokens, Some(24));
    later.message.usage = Some(serde_json::from_value(json!({"input_tokens":20})).unwrap());
    db.store_mut()
        .upsert_records(&s.session_id, &[later], false)
        .unwrap();
    assert_eq!(query(&db).total.selected_responses, 1); // partial snapshot moved beyond the window
    let next = Window::new(window().end_ms(), ms("2026-09-09T00:00:00Z")).unwrap();
    let report = MetricsDb::open(db.path())
        .unwrap()
        .tokens(next, TimeZone::UTC)
        .unwrap();
    assert_eq!(report.total.counters.input_tokens, Some(20));
    assert_eq!(report.total.counters.total_tokens, None);
}
#[test]
fn tokens_keys_are_host_scoped_and_missing_ids_never_form_a_shared_group() {
    let (mut db, s, row) = one(
        json!({"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
    );
    for (i, (api, request)) in [
        (Some(""), Some("r")),
        (Some(" "), Some("r")),
        (Some("a"), None),
        (None, Some("r")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut r = row.clone();
        r.uuid = Some(format!("missing-{i}"));
        r.api_message_id = api.map(str::to_owned);
        r.request_id = request.map(str::to_owned);
        db.store_mut()
            .upsert_records(&s.session_id, &[r], false)
            .unwrap();
    }
    let other = SessionMeta {
        session_id: "codex".into(),
        host: Host::Codex,
        ..s.clone()
    };
    db.store_mut().upsert_session(&other, false).unwrap();
    for i in 0..2 {
        let mut r = row.clone();
        r.uuid = Some(format!("codex-{i}"));
        r.api_message_id = Some("same".into());
        r.request_id = Some("same".into());
        db.store_mut()
            .upsert_records(&other.session_id, &[r], false)
            .unwrap();
    }
    let report = query(&db);
    assert_eq!(report.total.selected_responses, 7);
    assert_eq!(report.total.counters.total_tokens, Some(14));
    assert_eq!(report.by_host.len(), 2);
    breakdowns_match(&report);
}

#[test]
fn tokens_local_day_buckets_use_the_same_selected_responses() {
    let (_, db) = seed("F17");
    let report = tokens_in(
        &MetricsDb::open(db.path()).unwrap(),
        window(),
        TimeZone::get("America/Los_Angeles").unwrap(),
    )
    .unwrap();
    assert_eq!(report.by_day.len(), 8);
    assert_eq!(
        report
            .by_day
            .iter()
            .find(|d| d.date == "2026-09-01")
            .unwrap()
            .tokens
            .selected_responses,
        1
    );
    assert_eq!(report.by_day[0].start_ms, window().start_ms());
    assert_eq!(report.by_day.last().unwrap().end_ms, window().end_ms());
    breakdowns_match(&report);
}

#[test]
fn tokens_open_rejects_missing_or_incomplete_response_projection() {
    let db = TempDb::empty().unwrap();
    let c = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&c).unwrap();
    c.execute_batch("DROP VIEW v_response_usage;").unwrap();
    assert!(MetricsDb::open(db.path()).is_err());
    c.execute_batch("CREATE VIEW v_response_usage AS SELECT session_id,host,model,surface,ts_ms,input_tokens,output_tokens,cache_read_tokens FROM v_usage_records;").unwrap();
    assert!(MetricsDb::open(db.path()).is_err());
    let _writer = xt_store::Store::open(db.path()).unwrap();
    assert!(MetricsDb::open(db.path()).is_ok());
}
#[test]
fn tokens_all_rust_whitespace_ids_fall_back_without_collapsing_observations() {
    let (mut db, s, row) = one(
        json!({"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
    );
    let mut inputs = Vec::new();
    for ch in (0..=0x10ffff)
        .filter_map(char::from_u32)
        .filter(|c| c.is_whitespace())
    {
        for blank_api in [false, true] {
            for repeat in 0..2 {
                let mut r = row.clone();
                r.uuid = Some(format!("space-{}-{blank_api}-{repeat}", u32::from(ch)));
                r.api_message_id = Some(if blank_api {
                    ch.to_string()
                } else {
                    "response".into()
                });
                r.request_id = Some(if blank_api {
                    "request".into()
                } else {
                    ch.to_string()
                });
                inputs.push(r);
            }
        }
    }
    db.store_mut()
        .upsert_records(&s.session_id, &inputs, false)
        .unwrap();
    let report = query(&db);
    assert_eq!(report.total.selected_responses, inputs.len() as u64 + 1);
    assert_eq!(
        report.total.counters.total_tokens,
        Some((inputs.len() as u64 + 1) * 2)
    );
}

#[test]
fn tokens_timestamp_then_uuid_selection_is_independent_of_arrival_order() {
    let f = fixture("F17");
    let session = &f.sessions()[0].metadata;
    for reverse in [false, true] {
        let mut db = TempDb::empty().unwrap();
        db.store_mut().upsert_session(session, false).unwrap();
        let mut inputs = rows(&f.snapshots()["tokens"]["records"]);
        if reverse {
            inputs.reverse();
        }
        db.store_mut()
            .upsert_records(&session.session_id, &inputs, false)
            .unwrap();
        // The greater UUID wins equal timestamps, even when it arrived first
        // and the lower UUID carries a larger counter.
        assert_eq!(query(&db).total.counters.total_tokens, Some(43));
        let c = Connection::open(db.path()).unwrap();
        xt_store::timestamp::register_sqlite(&c).unwrap();
        let lower = "03000000-0000-4000-8000-000000001707";
        c.execute(
            "UPDATE records SET ts='2026-09-03T13:00:00.001Z',ts_ms=?1 WHERE uuid=?2",
            rusqlite::params![ms("2026-09-03T13:00:00.001Z"), lower],
        )
        .unwrap();
        assert_eq!(query(&db).total.counters.total_tokens, Some(44));
    }
}

const RANKED_USAGE_ORACLE: &str = "CREATE TEMP VIEW ranked_usage_oracle AS
SELECT * FROM (
    SELECT *, row_number() OVER (
        PARTITION BY host,
          CASE WHEN response_keyed THEN 1 ELSE 0 END,
          CASE WHEN response_keyed THEN api_message_id ELSE uuid END,
          CASE WHEN response_keyed THEN request_id ELSE '' END
        ORDER BY ts_ms DESC, uuid DESC
    ) AS response_rank FROM v_usage_records
) WHERE response_rank=1;";

#[test]
fn tokens_indexed_selection_matches_ranked_whole_row_oracle() {
    let (mut db, base, row) = one(
        json!({"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
    );
    for (label, host) in [
        ("claude", Host::Claude),
        ("codex", Host::Codex),
        ("cursor", Host::Cursor),
    ] {
        let session = SessionMeta {
            session_id: label.into(),
            host,
            surface: Some(label.into()),
            ..base.clone()
        };
        db.store_mut().upsert_session(&session, false).unwrap();
        let mut inputs = Vec::new();
        for (key, api, request) in [
            ("keyed", Some("response"), Some("request")),
            ("null-keyed", Some("null-response"), Some("request")),
            ("missing-api", None, Some("request")),
            ("missing-request", Some("response"), None),
            ("blank-api", Some("\u{2003}"), Some("request")),
            ("blank-request", Some("response"), Some("\t")),
        ] {
            for (time, timestamp) in [
                None,
                Some("2026-09-01T00:00:00Z"),
                Some("2026-09-07T12:00:00Z"),
                Some("2026-09-08T00:00:00Z"),
            ]
            .into_iter()
            .enumerate()
            {
                for variant in 0..3 {
                    let mut record = row.clone();
                    record.uuid = Some(format!("{label}-{key}-{time}-{variant}"));
                    record.api_message_id = api.map(str::to_owned);
                    record.request_id = request.map(str::to_owned);
                    record.timestamp = if key == "null-keyed" {
                        None
                    } else {
                        timestamp.map(str::to_owned)
                    };
                    record.message.model = Some(format!("model-{variant}"));
                    record.message.usage = match variant {
                        0 => None,
                        1 => Some(serde_json::from_value(json!({"input_tokens":100})).unwrap()),
                        _ => Some(serde_json::from_value(json!({"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0})).unwrap()),
                    };
                    inputs.push(record);
                }
            }
        }
        db.store_mut()
            .upsert_records(&session.session_id, &inputs, false)
            .unwrap();
    }
    let c = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&c).unwrap();
    c.execute_batch(RANKED_USAGE_ORACLE).unwrap();
    let selected = |view: &str, predicate: &str| {
        c.prepare(&format!("SELECT * FROM {view} {predicate} ORDER BY uuid"))
            .unwrap()
            .query_map([], |row| {
                (0..row.as_ref().column_count())
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    for mutation in [
        "SELECT 1;",
        "UPDATE records SET is_meta=1 WHERE uuid LIKE '%-3-2';",
        "UPDATE records SET model='<synthetic>' WHERE uuid LIKE '%-2-2';",
        "UPDATE sessions SET kind='judge' WHERE session_id='claude';",
    ] {
        c.execute_batch(mutation).unwrap();
        for predicate in [
            "",
            "WHERE ts_ms IS NULL",
            "WHERE ts_ms<1788220800000",
            "WHERE ts_ms>=1788220800000 AND ts_ms<1788825600000",
            "WHERE ts_ms>=1788825600000",
        ] {
            assert_eq!(
                selected("v_response_usage", predicate),
                selected("ranked_usage_oracle", predicate),
                "{predicate}"
            );
        }
    }
}

#[test]
fn tokens_precise_instants_precede_uuid_including_leap_seconds() {
    for (name, a, z, expected) in [
        (
            "submillisecond",
            Some("2026-09-07T12:00:00.0009Z"),
            Some("2026-09-07T12:00:00.0001Z"),
            "a",
        ),
        (
            "beyond-nanoseconds",
            Some("2026-09-07T12:00:00.0000000009Z"),
            Some("2026-09-07T12:00:00.0000000001Z"),
            "a",
        ),
        (
            "offset",
            Some("2026-09-07T14:00:00+02:00"),
            Some("2026-09-07T12:00:00Z"),
            "z",
        ),
        (
            "zero-tail",
            Some("2026-09-07T12:00:00.10000000000Z"),
            Some("2026-09-07T12:00:00.1Z"),
            "z",
        ),
        (
            "equal",
            Some("2026-09-07T12:00:00Z"),
            Some("2026-09-07T12:00:00Z"),
            "z",
        ),
        ("known", Some("2026-09-07T12:00:00Z"), None, "a"),
        ("unknown", None, None, "z"),
        (
            "leap-overlap",
            Some("2017-01-01T00:00:00.1Z"),
            Some("2016-12-31T23:59:60.9Z"),
            "a",
        ),
    ] {
        let (mut db, session, row) = one(
            json!({"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
        );
        for (uuid, timestamp, input) in [("a", a, 9), ("z", z, 3)] {
            let mut record = row.clone();
            record.uuid = Some(uuid.into());
            record.timestamp = timestamp.map(str::to_owned);
            record.api_message_id = Some("response".into());
            record.request_id = Some("request".into());
            record.message.usage.as_mut().unwrap().input_tokens = Some(input);
            db.store_mut()
                .upsert_records(&session.session_id, &[record], false)
                .unwrap();
        }
        let c = Connection::open_with_flags(db.path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
        assert!(c.prepare("SELECT * FROM v_response_usage").is_err());
        xt_store::timestamp::register_sqlite(&c).unwrap();
        assert!(c.execute("DELETE FROM records", []).is_err());
        let selected: String = c
            .query_row(
                "SELECT uuid FROM v_response_usage WHERE response_keyed",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(selected, expected, "{name}");
        let w = if name == "leap-overlap" {
            Window::new(ms("2016-12-31T00:00:00Z"), ms("2017-01-02T00:00:00Z")).unwrap()
        } else {
            window()
        };
        let report = tokens_in(&MetricsDb::open(db.path()).unwrap(), w, TimeZone::UTC).unwrap();
        let expected_input = if name == "unknown" {
            1
        } else {
            (if expected == "a" { 9 } else { 3 }) + i64::from(name != "leap-overlap")
        };
        assert_eq!(
            report.total.counters.input_tokens,
            Some(expected_input as u64),
            "{name}"
        );
        if name == "submillisecond" {
            Connection::open(db.path())
                .unwrap()
                .execute("UPDATE records SET ts='invalid' WHERE uuid='a'", [])
                .unwrap();
            assert!(tokens_in(&MetricsDb::open(db.path()).unwrap(), w, TimeZone::UTC).is_err());
        }
    }
}

#[test]
fn tokens_precise_window_and_day_membership_keeps_leaps_before_midnight() {
    let (mut db, session, row) = one(
        json!({"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}),
    );
    for (id, ts, input, response) in [
        ("leap", "2016-12-31T23:59:60.5Z", 7, None),
        ("start", "2017-01-01T00:00:00Z", 11, None),
        ("subms", "2017-01-01T00:00:00.0009Z", 13, None),
        ("millisecond-end", "2017-01-01T00:00:00.001Z", 29, None),
        ("end", "2017-01-02T00:00:00Z", 17, None),
        ("suppressed", "2016-12-31T23:59:60.9Z", 19, Some("crossing")),
        (
            "successor",
            "2017-01-01T00:00:00.0001Z",
            23,
            Some("crossing"),
        ),
    ] {
        let mut r = row.clone();
        r.uuid = Some(id.into());
        r.timestamp = Some(ts.into());
        r.api_message_id = response.map(str::to_owned);
        r.request_id = response.map(str::to_owned);
        r.message.usage.as_mut().unwrap().input_tokens = Some(input);
        db.store_mut()
            .upsert_records(&session.session_id, &[r], false)
            .unwrap();
    }
    let metrics = MetricsDb::open(db.path()).unwrap();
    let boundary = ms("2017-01-01T00:00:00Z");
    let end = ms("2017-01-02T00:00:00Z");
    let left = tokens_in(
        &metrics,
        Window::new(boundary - 86_400_000, boundary).unwrap(),
        TimeZone::UTC,
    )
    .unwrap();
    let right = tokens_in(&metrics, Window::new(boundary, end).unwrap(), TimeZone::UTC).unwrap();
    let combined = tokens_in(
        &metrics,
        Window::new(boundary - 86_400_000, end).unwrap(),
        TimeZone::UTC,
    )
    .unwrap();
    assert_eq!(left.total.counters.input_tokens, Some(7));
    assert_eq!(right.total.counters.input_tokens, Some(76));
    assert_eq!(combined.total.counters.input_tokens, Some(83));
    assert_eq!(combined.by_day[0].tokens, left.total);
    assert_eq!(combined.by_day[1].tokens, right.total);
    assert_eq!(
        left.total.selected_responses + right.total.selected_responses,
        combined.total.selected_responses
    );
    breakdowns_match(&left);
    breakdowns_match(&right);
    breakdowns_match(&combined);
    let millisecond = tokens_in(
        &metrics,
        Window::new(boundary, boundary + 1).unwrap(),
        TimeZone::UTC,
    )
    .unwrap();
    assert_eq!(millisecond.total.counters.input_tokens, Some(47));
    let after = tokens_in(
        &metrics,
        Window::new(boundary + 1, boundary + 2).unwrap(),
        TimeZone::UTC,
    )
    .unwrap();
    assert_eq!(after.total.counters.input_tokens, Some(29));
}

#[test]
fn tokens_empty_windows_preserve_numeric_boundary_domain() {
    let db = TempDb::empty().unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    for start in [
        ms("-000001-01-01T00:00:00Z"),
        Timestamp::MAX.as_second() * 1000 - 1,
    ] {
        let report = tokens_in(
            &metrics,
            Window::new(start, start + 1).unwrap(),
            TimeZone::UTC,
        )
        .unwrap();
        assert_eq!(report.total.selected_responses, 0);
    }
}
