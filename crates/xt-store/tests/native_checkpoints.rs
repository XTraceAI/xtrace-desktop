//! Native resume checkpoints: opaque generation plus position per source key,
//! replaced as a whole (a new generation may move the position backwards),
//! committed only with the batch rows they cover, and removable. Migrations 5,
//! 6 and 7 each invalidate transcript checkpoints once and nothing else.
use rusqlite::Connection;
use xt_fixtures::TempDb;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store,
    batch::{IngestBatch, NativeCheckpoint},
    ingest::{CaptureReceipt, RecordCoverage},
    pr_link::{PrConfidence, PrIdentity, PrLinkObservation},
};

/// Leave the exact history an older build wrote: versions from `version` on
/// are unapplied and what migrations 8 to 13 added is gone again.
fn rewind_before(sql: &Connection, version: u32) {
    sql.execute_batch(&format!(
        "DELETE FROM schema_version WHERE version>={version};
         ALTER TABLE pull_requests DROP COLUMN refresh_error;
         ALTER TABLE pull_requests DROP COLUMN last_attempted_at;
         ALTER TABLE tool_uses DROP COLUMN group_key;
         ALTER TABLE tool_uses DROP COLUMN group_version;
         ALTER TABLE tool_uses DROP COLUMN group_conflict;
         DROP TABLE confirmed_automated_inputs;
         DROP TABLE guardian_turn_inputs;
         DROP TABLE injected_context_inputs; DROP TABLE IF EXISTS tool_sent_inputs; DROP TABLE IF EXISTS task_notification_inputs; DROP TABLE IF EXISTS record_previews; DROP TABLE IF EXISTS session_child_checks; DROP TABLE IF EXISTS session_child_facts; DROP TABLE IF EXISTS human_input_adjustments; DROP TABLE IF EXISTS human_session_origins;
         DROP TABLE session_creation_relations;
         DROP TABLE session_creation_bootstrap; DROP TABLE cli_artifact_launch_owners; DROP TABLE claude_launch_groups; DROP TABLE claude_launch_group_members; DROP TABLE claude_launch_candidates; DROP TABLE claude_launch_staged_candidates;
         DROP INDEX sessions_host_native; DROP INDEX source_cursors_tail;"
    ))
    .unwrap();
}

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
    rewind_before(&sql, 5);
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
    assert_eq!(store.schema_version().unwrap(), 22);
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
        assert_eq!(store.schema_version().unwrap(), 22);
        assert_eq!(
            store
                .native_checkpoint(SessionSource::Transcript, transcript_key)
                .unwrap(),
            Some(recreated.clone())
        );
        assert_eq!(count("SELECT count(*) FROM schema_version"), 22);
    }
}

/// Migration 6 exists because tool calls now carry structural kinds and Claude
/// native scans now keep hook summaries: unchanged transcripts must be proven
/// again once. It may reset nothing else, and only once.
#[test]
fn migration_6_resets_transcripts_once_and_keeps_everything_else() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("v5.sqlite");
    let transcript_key = "claude:/home/.claude/projects/p/s.jsonl";
    let reader_key = "codex:/home/.codex";
    {
        let mut store = Store::open(&path).unwrap();
        let session = SessionMeta::new("s", "claude", SessionSource::Transcript);
        let records: [CanonicalRecord; 1] = [serde_json::from_value(serde_json::json!({
            "uuid":"u1","type":"user","timestamp":"2026-09-07T12:00:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Human turn."}]}
        }))
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
        // Migration 5's work: the classification derived before content discard.
        assert_eq!(
            store.records("s").unwrap()[0].classification.is_human,
            Some(true)
        );
    }
    // Exact schema-5 history: migration 6 has not been applied yet.
    let sql = Connection::open(&path).unwrap();
    rewind_before(&sql, 6);
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
        "tool_uses",
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
    assert_eq!(store.schema_version().unwrap(), 22);
    assert!(
        store
            .native_checkpoint(SessionSource::Transcript, transcript_key)
            .unwrap()
            .is_none()
    );
    assert_eq!(snapshot("native_checkpoints"), reader_before);
    assert_eq!(tables.map(snapshot), before);
    assert_eq!(
        store.records("s").unwrap()[0].classification.is_human,
        Some(true)
    );

    // A checkpoint recreated after the upgrade survives every later open.
    let recreated = checkpoint(transcript_key, r#"{"kind":"file","ino":7}"#, 120, 20);
    store.record_native_checkpoint(&recreated).unwrap();
    drop(store);
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 22);
        assert_eq!(
            store
                .native_checkpoint(SessionSource::Transcript, transcript_key)
                .unwrap(),
            Some(recreated.clone())
        );
        assert_eq!(count("SELECT count(*) FROM schema_version"), 22);
    }
}

/// Migration 7 exists because Claude native scans now link the exact `pr-link`
/// witnesses they used to ignore: unchanged transcripts must be proven again
/// once. It may reset nothing else, including pull-request rows, and only once.
#[test]
fn migration_7_resets_transcripts_once_and_keeps_everything_else() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("v6.sqlite");
    let transcript_key = "claude:/home/.claude/projects/p/s.jsonl";
    let reader_key = "codex:/home/.codex";
    {
        let mut store = Store::open(&path).unwrap();
        let session = SessionMeta::new("s", "claude", SessionSource::Transcript);
        let records: [CanonicalRecord; 1] = [serde_json::from_value(serde_json::json!({
            "uuid":"u1","type":"user","timestamp":"2026-09-07T12:00:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Human turn."}]}
        }))
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
        store
            .record_pr_link(&PrLinkObservation {
                session_id: "s".into(),
                pull_request: PrIdentity::from_url("https://github.com/example/fixture/pull/42")
                    .unwrap(),
                confidence: PrConfidence::Exact,
                first_seen_at: 5,
                last_seen_at: 9,
            })
            .unwrap();
    }
    // Exact schema-6 history: migration 7 has not been applied yet.
    let sql = Connection::open(&path).unwrap();
    rewind_before(&sql, 7);
    let count = |query: &str| sql.query_row(query, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count("SELECT count(*) FROM native_checkpoints"), 2);
    // Migration 8 then only appends NULL refresh-status columns, checked below.
    let snapshot = |table: &str| {
        let columns = if table == "pull_requests" {
            "id,repo,number,url,title,state,merged_at,additions,deletions,head_ref_name,refreshed_at"
        } else {
            "*"
        };
        let mut statement = sql
            .prepare(&format!("SELECT {columns} FROM {table} ORDER BY 1"))
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
        "tool_uses",
        "source_cursors",
        "capture_receipts",
        "capture_record_coverage",
        "pull_requests",
        "pr_links",
        "discovered_sessions",
    ];
    let before = tables.map(snapshot);
    assert_eq!(before[6].len(), 1);
    assert_eq!(before[7].len(), 1);
    let reader_before = snapshot("native_checkpoints")
        .into_iter()
        .filter(|row| row[0] == rusqlite::types::Value::Text("readers_cli".into()))
        .collect::<Vec<_>>();

    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 22);
    assert!(
        store
            .native_checkpoint(SessionSource::Transcript, transcript_key)
            .unwrap()
            .is_none()
    );
    assert_eq!(snapshot("native_checkpoints"), reader_before);
    assert_eq!(tables.map(snapshot), before);
    assert_eq!(
        count(
            "SELECT count(*) FROM pull_requests WHERE last_attempted_at IS NULL AND refresh_error IS NULL"
        ),
        1
    );
    assert_eq!(
        store.records("s").unwrap()[0].classification.is_human,
        Some(true)
    );

    // A checkpoint recreated after the upgrade survives every later open.
    let recreated = checkpoint(transcript_key, r#"{"kind":"file","ino":7}"#, 120, 20);
    store.record_native_checkpoint(&recreated).unwrap();
    drop(store);
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 22);
        assert_eq!(
            store
                .native_checkpoint(SessionSource::Transcript, transcript_key)
                .unwrap(),
            Some(recreated.clone())
        );
        assert_eq!(count("SELECT count(*) FROM schema_version"), 22);
        assert_eq!(tables.map(snapshot), before);
    }
}
