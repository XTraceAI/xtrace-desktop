use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::Fixture;
use xt_metrics::{MetricsDb, Window};
use xt_store::Store;
fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}
#[test]
fn schema_contract_window_uses_events_not_session_age_or_declared_path_shape() {
    let f = fixture("F3");
    let db = f.build_db(false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let w = Window::new(
        f.window_start().timestamp_millis(),
        f.now().timestamp_millis(),
    )
    .unwrap();
    let raw = Connection::open_with_flags(db.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (input,output):(i64,i64)=raw.query_row("SELECT sum(input_tokens),sum(output_tokens) FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2",[w.start_ms(),w.end_ms()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    f.assert_expectation("M-01",&json!({"records":metrics.event_count(w).unwrap(),"input_tokens":input,"output_tokens":output,"previous_records":metrics.event_count(w.previous().unwrap()).unwrap()})).unwrap();
    assert_eq!(metrics.event_count(Window::new(0, 1).unwrap()).unwrap(), 0);
    assert_eq!(db.store().counts().unwrap().records, 5);
    assert_eq!(raw.query_row("SELECT count(*) FROM v_records WHERE ts_ms IS NULL AND model IS NULL AND surface IS NULL",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}
#[test]
fn schema_contract_f1_replay_and_live_reader_observe_committed_work_once() {
    let f = fixture("F1");
    let mut db = f.build_db(false).unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let w = Window::new(
        f.window_start().timestamp_millis(),
        f.now().timestamp_millis(),
    )
    .unwrap();
    assert_eq!(metrics.event_count(w).unwrap(), 25);
    let session = &f.sessions()[0];
    db.store_mut()
        .upsert_records(&session.metadata.session_id, &session.records, false)
        .unwrap();
    assert_eq!(metrics.event_count(w).unwrap(), 25);
    let mut added = session.records[0].clone();
    added.uuid = Some("00000000-0000-4000-8000-000000000999".into());
    db.store_mut()
        .upsert_records(&session.metadata.session_id, &[added], false)
        .unwrap();
    assert_eq!(metrics.event_count(w).unwrap(), 26);
}
#[test]
fn schema_contract_views_reload_without_touching_ingest_or_multiplying_context() {
    let f = fixture("F1");
    let db = f.build_db(false).unwrap();
    let conn = Connection::open(db.path()).unwrap();
    let count = db.store().counts().unwrap();
    conn.execute_batch(
        "DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT 1 AS obsolete;",
    )
    .unwrap();
    let reopened = Store::open(db.path()).unwrap();
    assert_eq!(reopened.counts().unwrap(), count);
    let cols: Vec<String> = conn
        .prepare("pragma table_info(v_records)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for column in [
        "uuid",
        "ts_ms",
        "host",
        "surface",
        "source",
        "input_tokens",
        "cache_creation_5m",
        "service_tier",
        "is_human",
    ] {
        assert!(cols.iter().any(|v| v == column), "{column}");
    }
    assert!(!cols.iter().any(|v| v == "content_json"));
    let session = &f.sessions()[0];
    let id = &session.metadata.session_id;
    conn.execute("INSERT INTO native_record_copies(session_id,record_uuid) SELECT session_id,uuid FROM records WHERE session_id=?1",[id]).unwrap();
    let w = Window::new(
        f.window_start().timestamp_millis(),
        f.now().timestamp_millis(),
    )
    .unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    assert_eq!(metrics.event_count(w).unwrap(), 25);
    conn.execute(
        "UPDATE records SET is_meta=1 WHERE uuid=?1",
        [session.records[0].uuid.as_ref().unwrap()],
    )
    .unwrap();
    assert_eq!(metrics.event_count(w).unwrap(), 24);
    conn.execute("UPDATE sessions SET kind='judge'", [])
        .unwrap();
    assert_eq!(metrics.event_count(w).unwrap(), 0);
    conn.execute("UPDATE sessions SET kind='user'", []).unwrap();
    conn.execute("UPDATE records SET model='<synthetic>'", [])
        .unwrap();
    assert_eq!(metrics.event_count(w).unwrap(), 0);
}
#[test]
fn schema_contract_reader_does_not_create_or_migrate_a_database() {
    let temp = tempfile::tempdir().unwrap();
    let absent = temp.path().join("absent.db");
    assert!(MetricsDb::open(&absent).is_err());
    assert!(!absent.exists());
    let empty = temp.path().join("empty.db");
    let conn = Connection::open(&empty).unwrap();
    assert!(MetricsDb::open(&empty).is_err());
    assert_eq!(
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
