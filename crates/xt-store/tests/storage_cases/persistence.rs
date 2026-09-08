use super::support::*;
use rusqlite::{Connection, params};
use tempfile::TempDir;
use xt_store::{Error, Store};

#[test]
fn file_wal_migrations_and_readers() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("store.sqlite");
    let mut store = Store::open(&path).unwrap();
    let reader = Connection::open(&path).unwrap();
    assert_eq!(
        reader
            .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    reader
        .execute(
            "UPDATE schema_version SET applied_at='migration-once' WHERE version=1",
            [],
        )
        .unwrap();
    store.migrate().unwrap();
    let reopened = Store::open(&path).unwrap();
    let history: (i64, i64, String) = reader
        .query_row(
            "SELECT count(*), max(version), max(applied_at) FROM schema_version",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(history, (2, 2, "migration-once".into()));
    store.upsert_session(&session(SESSION), true).unwrap();
    let batch: Vec<_> = (0..100)
        .map(|n| record(&format!("synthetic-{n}")))
        .collect();
    assert_eq!(
        store
            .upsert_records(SESSION, &batch, true)
            .unwrap()
            .inserted,
        100
    );
    assert_eq!(reopened.counts().unwrap().records, 100);
    reader.execute_batch("BEGIN").unwrap();
    let count = || {
        reader
            .query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    assert_eq!(count(), 100);
    assert_eq!(
        store
            .upsert_records(SESSION, &[record("after-snapshot")], true)
            .unwrap()
            .inserted,
        1
    );
    assert_eq!(
        count(),
        100,
        "WAL reader keeps its snapshot while writer commits"
    );
    reader.execute_batch("COMMIT").unwrap();
    assert_eq!(count(), 101);
}

#[test]
fn incompatible_and_failed_migrations_do_not_partially_apply() {
    let directory = TempDir::new().unwrap();
    let future_path = directory.path().join("future.sqlite");
    let store = Store::open(&future_path).unwrap();
    drop(store);
    let connection = Connection::open(&future_path).unwrap();
    connection
        .execute("INSERT INTO schema_version VALUES (?1,'future')", [999])
        .unwrap();
    assert!(matches!(
        Store::open(future_path),
        Err(Error::IncompatibleSchema)
    ));
    let broken_path = directory.path().join("broken.sqlite");
    let broken = Connection::open(&broken_path).unwrap();
    broken
        .execute_batch("CREATE TABLE records(original_column TEXT)")
        .unwrap();
    assert!(Store::open(broken_path).is_err());
    assert_eq!(
        broken
            .query_row("SELECT count(*) FROM schema_version", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        broken
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='sessions'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn concurrent_writers_insert_each_uuid_once() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("writers.sqlite");
    let mut first = Store::open(&path).unwrap();
    first.upsert_session(&session(SESSION), true).unwrap();
    let second = Store::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = [first, second]
        .into_iter()
        .map(|mut store| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let batch: Vec<_> = (0..100).map(|n| record(&format!("shared-{n}"))).collect();
                barrier.wait();
                store.upsert_records(SESSION, &batch, false).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().map(|r| r.inserted).sum::<usize>(), 100);
    assert_eq!(results.iter().map(|r| r.ignored).sum::<usize>(), 100);
    assert_eq!(Store::open(path).unwrap().counts().unwrap().records, 100);
}

#[test]
fn file_database_constraints_protect_canonical_relations() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("constraints.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.upsert_session(&session(SESSION), true).unwrap();
    store
        .upsert_records(SESSION, &[record("constrained")], true)
        .unwrap();
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO usage(uuid,input_tokens) VALUES (?1,1)",
                ["missing-record"]
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO usage(uuid,input_tokens) VALUES (?1,-1)",
                ["constrained"]
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO tool_uses(uuid,session_id,block_index,name) VALUES (?1,?2,0,'Read')",
                params!["missing-record", SESSION]
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE records SET ts=?1,ts_ms=NULL WHERE uuid=?2",
                params!["2026-09-01T00:00:00Z", "constrained"]
            )
            .is_err()
    );
}
