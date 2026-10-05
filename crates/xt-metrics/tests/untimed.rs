use rusqlite::Connection;
use serde_json::{Value, json};
use xt_fixtures::TempDb;
use xt_metrics::{MetricsDb, UntimedHistory, UntimedSurface};
use xt_store::{CanonicalRecord, SessionMeta, SessionSource};

/// One assistant record, with a timestamp only when the source stated one.
fn record(uuid: &str, ts: Option<&str>) -> CanonicalRecord {
    let mut value: Value = json!({"uuid":uuid,"type":"assistant",
        "message":{"role":"assistant","model":"test-claude","content":[]}});
    if let Some(ts) = ts {
        value["timestamp"] = json!(ts);
    }
    serde_json::from_value(value).unwrap()
}
fn seed(
    db: &mut TempDb,
    session_id: &str,
    platform: &str,
    surface: Option<&str>,
    rows: &[CanonicalRecord],
) {
    let mut session = SessionMeta::new(session_id, platform, SessionSource::Fixture);
    session.surface = surface.map(str::to_owned);
    db.store_mut().upsert_session(&session, false).unwrap();
    db.store_mut()
        .upsert_records(session_id, rows, false)
        .unwrap();
}
fn read(db: &TempDb) -> UntimedHistory {
    MetricsDb::open(db.path())
        .unwrap()
        .untimed_history()
        .unwrap()
}
fn surface(host: &str, surface: Option<&str>, records: u64) -> UntimedSurface {
    UntimedSurface {
        host: host.to_owned(),
        surface: surface.map(str::to_owned),
        records,
    }
}
fn sql(db: &TempDb) -> Connection {
    let connection = Connection::open(db.path()).unwrap();
    xt_store::timestamp::register_sqlite(&connection).unwrap();
    connection
}

#[test]
fn untimed_history_is_zero_and_empty_when_every_indexed_record_states_its_time() {
    let db = TempDb::empty().unwrap();
    assert_eq!(read(&db), UntimedHistory::default());
    let mut db = db;
    seed(
        &mut db,
        "timed",
        "claude",
        Some("cli"),
        &[
            record(
                "11111111-1111-4111-8111-111111111111",
                Some("2026-09-07T12:00:00Z"),
            ),
            record(
                "22222222-2222-4222-8222-222222222222",
                Some("2026-09-07T12:01:00Z"),
            ),
        ],
    );
    // A measured zero, stated as an empty grouping rather than as a missing fact.
    assert_eq!(
        read(&db),
        UntimedHistory {
            records: 0,
            by_surface: vec![]
        }
    );
}

#[test]
fn untimed_history_counts_only_the_records_whose_own_timestamp_is_absent() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "mixed",
        "claude",
        Some("cli"),
        &[
            record(
                "11111111-1111-4111-8111-111111111111",
                Some("2026-09-07T12:00:00Z"),
            ),
            record("22222222-2222-4222-8222-222222222222", None),
            record("33333333-3333-4333-8333-333333333333", None),
        ],
    );
    assert_eq!(
        read(&db),
        UntimedHistory {
            records: 2,
            by_surface: vec![surface("claude", Some("cli"), 2)]
        }
    );
    // The same rows the timed projection cannot hold: no window contains them,
    // whatever range a consumer selects.
    let count: i64 = sql(&db)
        .query_row("SELECT count(*) FROM v_session_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn untimed_history_groups_every_host_and_raw_surface_and_keeps_an_unknown_surface_unknown() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "claude-cli",
        "claude",
        Some("cli"),
        &[record("11111111-1111-4111-8111-111111111111", None)],
    );
    seed(
        &mut db,
        "claude-desktop",
        "claude",
        Some("desktop"),
        &[
            record("22222222-2222-4222-8222-222222222222", None),
            record("33333333-3333-4333-8333-333333333333", None),
        ],
    );
    seed(
        &mut db,
        "cursor-unknown",
        "cursor",
        None,
        &[record("44444444-4444-4444-8444-444444444444", None)],
    );
    let report = read(&db);
    assert_eq!(report.records, 4);
    // Host then surface order, with the unknown surface first inside its host
    // and never aliased to a name the source did not state.
    assert_eq!(
        report.by_surface,
        vec![
            surface("claude", Some("cli"), 1),
            surface("claude", Some("desktop"), 2),
            surface("cursor", None, 1),
        ]
    );
    assert_eq!(
        report.records,
        report.by_surface.iter().map(|row| row.records).sum::<u64>()
    );
}

#[test]
fn untimed_history_does_not_multiply_copies_and_never_reclaims_other_exclusions() {
    let mut db = TempDb::empty().unwrap();
    seed(
        &mut db,
        "owner",
        "claude",
        Some("cli"),
        &[
            record("11111111-1111-4111-8111-111111111111", None),
            record("22222222-2222-4222-8222-222222222222", None),
        ],
    );
    let expected = UntimedHistory {
        records: 2,
        by_surface: vec![surface("claude", Some("cli"), 2)],
    };
    assert_eq!(read(&db), expected);
    let c = sql(&db);
    // A fork repeats the same immutable records in another session file. Work
    // totals count distinct UUIDs, and so does this disclosure.
    c.execute(
        "INSERT INTO native_record_copies(session_id,record_uuid) SELECT session_id,uuid FROM records",
        [],
    )
    .unwrap();
    assert_eq!(read(&db), expected);
    // Rows already excluded for another reason are not reported as records
    // that merely lack a timestamp.
    c.execute("UPDATE sessions SET kind='judge'", []).unwrap();
    assert_eq!(read(&db), UntimedHistory::default());
    c.execute("UPDATE sessions SET kind='user'", []).unwrap();
    c.execute("UPDATE records SET is_meta=1", []).unwrap();
    assert_eq!(read(&db), UntimedHistory::default());
    c.execute("UPDATE records SET is_meta=0,model='<synthetic>'", [])
        .unwrap();
    assert_eq!(read(&db), UntimedHistory::default());
    c.execute("UPDATE records SET model='test-claude'", [])
        .unwrap();
    assert_eq!(read(&db), expected);
}
