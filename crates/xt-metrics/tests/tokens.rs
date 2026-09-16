use jiff::{Timestamp, tz::TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{MetricsDb, TokenReport, TokenSummary, Window};
use xt_store::{CanonicalRecord, Host, SessionMeta, SessionSource};
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
    MetricsDb::open(db.path())
        .unwrap()
        .tokens(window(), TimeZone::UTC)
        .unwrap()
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
        for _ in 0..2 {
            assert_eq!(
                value(&metric.tokens(window(), TimeZone::UTC).unwrap().total),
                data["expected"]
            );
            db.store_mut()
                .upsert_records(&session.session_id, &rows(&data["native"]), false)
                .unwrap();
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
    let report = MetricsDb::open(db.path())
        .unwrap()
        .tokens(window(), TimeZone::get("America/Los_Angeles").unwrap())
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
