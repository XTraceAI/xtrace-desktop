//! Native resume checkpoints: opaque generation plus position per source key,
//! replaced as a whole (a new generation may move the position backwards),
//! committed only with the batch rows they cover, and removable. Migration 5
//! invalidates transcript checkpoints once and nothing else.
use rusqlite::Connection;
use xt_fixtures::TempDb;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store,
    batch::{IngestBatch, NativeCheckpoint},
    ingest::{CaptureReceipt, RecordCoverage},
};

fn checkpoint(key: &str, generation: &str, position: i64, updated_at: i64) -> NativeCheckpoint {
    NativeCheckpoint {
        source: SessionSource::Transcript,
        cursor_key: key.into(),
        generation: generation.into(),
        position,
        updated_at,
    }
}

#[test]
fn checkpoints_replace_as_a_whole_and_may_move_backwards_with_a_new_generation() {
    let mut db = TempDb::empty().unwrap();
    let key = "claude:/home/.claude/projects/p/s.jsonl";
    assert!(
        db.store()
            .native_checkpoint(SessionSource::Transcript, key)
            .unwrap()
            .is_none()
    );
    let first = checkpoint(key, r#"{"kind":"file","ino":7}"#, 4096, 10);
    db.store_mut().record_native_checkpoint(&first).unwrap();
    assert_eq!(
        db.store()
            .native_checkpoint(SessionSource::Transcript, key)
            .unwrap(),
        Some(first.clone())
    );
    // A replaced source starts over: a smaller position under another generation.
    let replaced = checkpoint(key, r#"{"kind":"file","ino":8}"#, 512, 11);
    db.store_mut().record_native_checkpoint(&replaced).unwrap();
    assert_eq!(
        db.store()
            .native_checkpoint(SessionSource::Transcript, key)
            .unwrap(),
        Some(replaced)
    );
    // Other keys and sources are untouched; the plugin source is refused.
    assert!(
        db.store()
            .native_checkpoint(SessionSource::ReadersCli, key)
            .unwrap()
            .is_none()
    );
    let mut plugin = first.clone();
    plugin.source = SessionSource::Plugin;
    assert!(db.store_mut().record_native_checkpoint(&plugin).is_err());
    let mut malformed = first.clone();
    malformed.generation = "not-json".into();
    assert!(db.store_mut().record_native_checkpoint(&malformed).is_err());
    let mut negative = first;
    negative.position = -1;
    assert!(db.store_mut().record_native_checkpoint(&negative).is_err());
    db.store_mut()
        .clear_native_checkpoint(SessionSource::Transcript, key)
        .unwrap();
    assert!(
        db.store()
            .native_checkpoint(SessionSource::Transcript, key)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_checkpoint_commits_only_with_the_rows_it_covers() {
    let mut db = TempDb::empty().unwrap();
    let session = SessionMeta::new("s", "claude", SessionSource::Transcript);
    let records: [CanonicalRecord; 1] =
        [
            serde_json::from_value(
                serde_json::json!({"uuid":"u1","type":"assistant","message":{}}),
            )
            .unwrap(),
        ];
    let key = "claude:/home/.claude/projects/p/s.jsonl";
    let progress = checkpoint(key, r#"{"kind":"file","ino":7}"#, 120, 10);
    let mut batch = IngestBatch::new(&session, &records, true);
    batch.checkpoint = Some(&progress);
    db.store_mut().apply_ingest_batch(&batch).unwrap();
    assert_eq!(
        db.store()
            .native_checkpoint(SessionSource::Transcript, key)
            .unwrap(),
        Some(progress.clone())
    );
    // A later batch whose rows fail leaves the earlier checkpoint in place.
    let connection = Connection::open(db.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE INSERT ON records BEGIN
                 SELECT RAISE(ABORT, 'synthetic transaction failure'); END;",
        )
        .unwrap();
    let later: [CanonicalRecord; 1] =
        [
            serde_json::from_value(
                serde_json::json!({"uuid":"u2","type":"assistant","message":{}}),
            )
            .unwrap(),
        ];
    let advanced = checkpoint(key, r#"{"kind":"file","ino":7}"#, 240, 11);
    let mut failing = IngestBatch::new(&session, &later, true);
    failing.checkpoint = Some(&advanced);
    assert!(db.store_mut().apply_ingest_batch(&failing).is_err());
    assert_eq!(
        db.store()
            .native_checkpoint(SessionSource::Transcript, key)
            .unwrap(),
        Some(progress)
    );
    assert_eq!(db.store().records("s").unwrap().len(), 1);
    connection
        .execute_batch("DROP TRIGGER inject_failure")
        .unwrap();
    db.store_mut().apply_ingest_batch(&failing).unwrap();
    assert_eq!(
        db.store()
            .native_checkpoint(SessionSource::Transcript, key)
            .unwrap(),
        Some(advanced)
    );
    assert_eq!(db.store().records("s").unwrap().len(), 2);
}

#[test]
fn a_batch_without_records_cannot_carry_a_checkpoint() {
    let mut db = TempDb::empty().unwrap();
    let session = SessionMeta::new("s", "claude", SessionSource::Transcript);
    let key = "claude:/home/.claude/projects/p/s.jsonl";
    let progress = checkpoint(key, r#"{"kind":"file","ino":7}"#, 120, 10);
    let mut empty = IngestBatch::new(&session, &[], true);
    empty.checkpoint = Some(&progress);
    assert!(db.store_mut().apply_ingest_batch(&empty).is_err());
    assert!(
        db.store()
            .native_checkpoint(SessionSource::Transcript, key)
            .unwrap()
            .is_none(),
        "no row was stored, so no progress was recorded"
    );
    assert!(db.store().session("s").unwrap().is_none());
}

#[test]
fn migration_5_clears_only_transcript_checkpoints_once() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("v4.sqlite");
    let transcript_key = "claude:/home/.claude/projects/p/s.jsonl";
    let reader_key = "codex:/home/.codex";
    {
        let mut store = Store::open(&path).unwrap();
        let session = SessionMeta::new("s", "claude", SessionSource::Transcript);
        let records: [CanonicalRecord; 1] = [serde_json::from_value(
            serde_json::json!({"uuid":"u1","type":"user","message":{"role":"user"}}),
        )
        .unwrap()];
        let progress = checkpoint(transcript_key, r#"{"kind":"file","ino":7}"#, 120, 10);
        let mut batch = IngestBatch::new(&session, &records, false);
        batch.checkpoint = Some(&progress);
        store.apply_ingest_batch(&batch).unwrap();
        let mut reader = checkpoint(reader_key, r#"{"kind":"scan","at":10}"#, 0, 10);
        reader.source = SessionSource::ReadersCli;
        store.record_native_checkpoint(&reader).unwrap();
        store
            .insert_capture_receipt(
                &CaptureReceipt {
                    receipt_id: "r1".into(),
                    session_id: "s".into(),
                    surface: None,
                    received_at: 10,
                },
                &[RecordCoverage {
                    record_uuid: "u1".into(),
                    metric_field_mask: 3,
                    measurement_revision: "a".repeat(64),
                    digest_schema_version: 1,
                }],
            )
            .unwrap();
    }
    // Exact schema-4 history: the reset has not been applied yet.
    let sql = Connection::open(&path).unwrap();
    sql.execute("DELETE FROM schema_version WHERE version=5", [])
        .unwrap();
    let count = |query: &str| sql.query_row(query, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count("SELECT count(*) FROM native_checkpoints"), 2);
    let snapshot = |table: &str| {
        let mut statement = sql
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |row| {
                (0..width)
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let tables = [
        "sessions",
        "records",
        "source_cursors",
        "capture_receipts",
        "capture_record_coverage",
    ];
    let before = tables.map(snapshot);
    let reader_before = snapshot("native_checkpoints")
        .into_iter()
        .filter(|row| row[0] == rusqlite::types::Value::Text("readers_cli".into()))
        .collect::<Vec<_>>();

    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 5);
    assert!(
        store
            .native_checkpoint(SessionSource::Transcript, transcript_key)
            .unwrap()
            .is_none()
    );
    assert_eq!(snapshot("native_checkpoints"), reader_before);
    assert_eq!(tables.map(snapshot), before);

    // A checkpoint recreated after the upgrade survives every later open.
    let recreated = checkpoint(transcript_key, r#"{"kind":"file","ino":7}"#, 120, 20);
    store.record_native_checkpoint(&recreated).unwrap();
    drop(store);
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 5);
        assert_eq!(
            store
                .native_checkpoint(SessionSource::Transcript, transcript_key)
                .unwrap(),
            Some(recreated.clone())
        );
        assert_eq!(count("SELECT count(*) FROM schema_version"), 5);
    }
}
