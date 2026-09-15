//! Native resume checkpoints: opaque generation plus position per source key,
//! replaced as a whole (a new generation may move the position backwards),
//! committed only with the batch rows they cover, and removable.
use rusqlite::Connection;
use xt_fixtures::TempDb;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource,
    batch::{IngestBatch, NativeCheckpoint},
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
