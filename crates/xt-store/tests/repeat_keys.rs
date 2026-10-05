//! What the store persists for M-20: the version-1 comparison key beside each
//! canonical `tool_use`, derived before content retention decides about the
//! block, replayed without ever preferring a later observation to a stored one,
//! and added by migration 9 without asking any transcript to be read again.

use rusqlite::{Connection, params, types::Value as SqlValue};
use serde_json::{Value, json};
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store,
    batch::{IngestBatch, NativeCheckpoint},
    ingest::{ToolEvent, ToolKind},
    measurement::Projection,
    retention::{ContentRegistry, RetentionMode},
};

const SESSION: &str = "session-repeat-keys";

/// One assistant record whose single content block is a tool call with the
/// supplied input, or no input field at all when none is supplied.
fn call(uuid: &str, name: &str, input: Option<Value>) -> CanonicalRecord {
    let mut block = json!({"type": "tool_use", "name": name});
    if let Some(input) = input {
        block["input"] = input;
    }
    serde_json::from_value(json!({
        "uuid": uuid, "type": "assistant", "timestamp": "2026-09-07T12:00:00Z",
        "message": {"role": "assistant", "content": [block]}
    }))
    .unwrap()
}

fn open(directory: &TempDir, mode: RetentionMode) -> Store {
    let mut store = Store::open(directory.path().join("keys.sqlite")).unwrap();
    store.set_retention_mode(mode).unwrap();
    store
        .upsert_session(
            &SessionMeta::new(SESSION, "claude", SessionSource::Transcript),
            true,
        )
        .unwrap();
    store
}

/// Every stored `(block_index, group_version, group_key, group_conflict)` of
/// one record, read as raw columns: the digest is not readable through the
/// typed row, which is the point of it being private.
fn columns(path: &std::path::Path, uuid: &str) -> Vec<(i64, Option<i64>, Option<String>, bool)> {
    Connection::open(path)
        .unwrap()
        .prepare(
            "SELECT block_index,group_version,group_key,group_conflict FROM tool_uses
             WHERE uuid=?1 ORDER BY block_index",
        )
        .unwrap()
        .query_map([uuid], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn key(path: &std::path::Path, uuid: &str) -> String {
    columns(path, uuid)[0].2.clone().expect("a stored key")
}

#[test]
fn the_same_call_derives_the_same_key_whether_or_not_its_content_is_kept() {
    let mut digests = Vec::new();
    for mode in [RetentionMode::MetadataOnly, RetentionMode::FullContent] {
        let directory = TempDir::new().unwrap();
        let mut store = open(&directory, mode);
        let path = directory.path().join("keys.sqlite");
        let records = [
            call("bash", "Bash", Some(json!({"command": "cargo test"}))),
            call("edit", "Edit", Some(json!({"file_path": "/tmp/a.rs"}))),
            // An object naming none of the identifying fields is an answer.
            call("todo", "TodoWrite", Some(json!({"todos": []}))),
            // No input at all cannot say which call this was.
            call("bare", "Bash", None),
        ];
        store.upsert_records(SESSION, &records, true).unwrap();

        let retained = mode == RetentionMode::FullContent;
        for uuid in ["bash", "edit", "todo"] {
            let stored = columns(&path, uuid);
            let [(index, version, digest, conflict)] = &stored[..] else {
                panic!("one stored block for {uuid}");
            };
            assert_eq!((*index, *version, *conflict), (0, Some(1), false), "{uuid}");
            let digest = digest.clone().expect("a derived key");
            assert_eq!(digest.len(), 64, "{uuid}");
            digests.push(digest);
        }
        assert_eq!(columns(&path, "bare"), [(0, None, None, false)]);
        // Retention decided only the copy of the argument, never the key.
        let stored = store.records(SESSION).unwrap();
        for record in &stored {
            assert_eq!(
                record.tool_uses[0].input_json.is_some(),
                retained && record.uuid != "bare"
            );
            assert_eq!(
                record.tool_uses[0].group.is_some(),
                record.uuid != "bare",
                "{}",
                record.uuid
            );
        }
        // An explicit purge clears content. The key is derived metadata beside
        // the structural columns, not a copy of what the call said, so it is
        // not registered as content and survives with them.
        let purged = store.purge_content(&ContentRegistry::default()).unwrap();
        assert_eq!(purged.invalidate_content, retained);
        assert!(
            store
                .records(SESSION)
                .unwrap()
                .iter()
                .all(|record| record.tool_uses[0].input_json.is_none())
        );
        for uuid in ["bash", "edit", "todo"] {
            assert_eq!(columns(&path, uuid)[0].1, Some(1), "{uuid}");
        }
    }
    let (metadata_only, full_content) = digests.split_at(3);
    assert_eq!(metadata_only, full_content);
    // The three calls are three different groups.
    assert_eq!(
        metadata_only
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}

#[test]
fn replay_fills_an_unknown_key_agrees_with_itself_and_marks_a_disagreement() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::MetadataOnly);
    let path = directory.path().join("keys.sqlite");
    let first = [call("one", "Bash", Some(json!({"command": "cargo test"})))];
    store.upsert_records(SESSION, &first, true).unwrap();
    let derived = key(&path, "one");

    // The same observation again changes nothing at all.
    let stats = store.upsert_records(SESSION, &first, true).unwrap();
    assert_eq!((stats.inserted, stats.enriched, stats.ignored), (0, 0, 1));
    assert_eq!(
        columns(&path, "one"),
        [(0, Some(1), Some(derived.clone()), false)]
    );

    // An observation that cannot say which call this was never erases it.
    store
        .upsert_records(SESSION, &[call("one", "Bash", None)], true)
        .unwrap();
    assert_eq!(
        columns(&path, "one"),
        [(0, Some(1), Some(derived.clone()), false)]
    );

    // A disagreeing observation keeps the stored key and marks the call. The
    // record's own conflict flag is untouched: the key is private local
    // metadata outside the C-08 measurement projection, so it may not blank
    // facts the same observation is otherwise entitled to state.
    let mut moved = call("one", "Bash", Some(json!({"command": "cargo build"})));
    moved.message.model = Some("model-later".into());
    store.upsert_records(SESSION, &[moved], true).unwrap();
    assert_eq!(
        columns(&path, "one"),
        [(0, Some(1), Some(derived.clone()), true)]
    );
    let stored = &store.records(SESSION).unwrap()[0];
    assert!(!stored.has_conflict);
    assert_eq!(stored.model.as_deref(), Some("model-later"));
    assert!(stored.tool_uses[0].group_conflict);

    // A later agreeing observation does not clear the mark.
    store.upsert_records(SESSION, &first, true).unwrap();
    assert_eq!(columns(&path, "one"), [(0, Some(1), Some(derived), true)]);

    // A row an older build left without a key acquires one from a compatible
    // observation of the same call, and only of the same call.
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "UPDATE tool_uses SET group_version=NULL,group_key=NULL,group_conflict=0 WHERE uuid='one'",
        [],
    )
    .unwrap();
    store
        .upsert_records(
            SESSION,
            &[call("one", "Bash", Some(json!({"command": "cargo test"})))],
            true,
        )
        .unwrap();
    let refilled = columns(&path, "one");
    let [(_, Some(1), Some(filled), false)] = &refilled[..] else {
        panic!("the unknown key is filled once");
    };
    let filled = filled.clone();
    raw.execute(
        "UPDATE tool_uses SET group_version=NULL,group_key=NULL WHERE uuid='one'",
        [],
    )
    .unwrap();
    // A renamed call at the same position is a different call: nothing fills.
    store
        .upsert_records(
            SESSION,
            &[call("one", "Write", Some(json!({"command": "cargo test"})))],
            true,
        )
        .unwrap();
    assert_eq!(columns(&path, "one"), [(0, None, None, false)]);
    assert_ne!(filled, String::new());
}

#[test]
fn a_structural_event_with_no_record_and_no_input_states_no_key() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::MetadataOnly);
    store
        .insert_tool_event(&ToolEvent {
            session_id: SESSION.into(),
            source: SessionSource::Transcript,
            source_event_id: "hook-1".into(),
            name: "stop_hook_summary".into(),
            kind: ToolKind::Hook,
            server: None,
            tool: None,
            skill: None,
            timestamp: Some("2026-09-07T12:00:00Z".into()),
        })
        .unwrap();
    let row: (Option<i64>, Option<String>, bool) =
        Connection::open(directory.path().join("keys.sqlite"))
            .unwrap()
            .query_row(
                "SELECT group_version,group_key,group_conflict FROM tool_uses WHERE uuid IS NULL",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
    assert_eq!(row, (None, None, false));
}

#[test]
fn migration_9_leaves_older_rows_unknown_and_asks_for_no_replay() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("keys.sqlite");
    let cursor = "claude:/home/.claude/projects/p/s.jsonl";
    {
        let mut store = open(&directory, RetentionMode::MetadataOnly);
        let session = SessionMeta::new(SESSION, "claude", SessionSource::Transcript);
        let records = [call("kept", "Bash", Some(json!({"command": "cargo test"})))];
        let checkpoint = NativeCheckpoint {
            source: SessionSource::Transcript,
            cursor_key: cursor.into(),
            generation: r#"{"kind":"file","ino":7}"#.into(),
            position: 120,
            updated_at: 10,
        };
        let mut batch = IngestBatch::new(&session, &records, false);
        batch.checkpoint = Some(&checkpoint);
        store.apply_ingest_batch(&batch).unwrap();
    }
    // Exactly the history a schema-8 build left: the rows, the checkpoint and
    // a retained tool input that migration 9 must not read.
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch(
        "DELETE FROM schema_version WHERE version>=9;
         ALTER TABLE tool_uses DROP COLUMN group_key;
         ALTER TABLE tool_uses DROP COLUMN group_version;
         ALTER TABLE tool_uses DROP COLUMN group_conflict;
         DROP TABLE confirmed_automated_inputs;
         DROP TABLE guardian_turn_inputs;
         DROP TABLE injected_context_inputs; DROP TABLE IF EXISTS task_notification_inputs; DROP TABLE IF EXISTS record_previews; DROP TABLE IF EXISTS human_input_adjustments; DROP TABLE IF EXISTS human_session_origins;
         DROP TABLE session_creation_relations;
         DROP TABLE session_creation_bootstrap; DROP TABLE cli_artifact_launch_owners; DROP TABLE claude_launch_groups; DROP TABLE claude_launch_group_members; DROP TABLE claude_launch_candidates; DROP TABLE claude_launch_staged_candidates;
         DROP INDEX sessions_host_native; DROP INDEX source_cursors_tail;",
    )
    .unwrap();
    sql.execute(
        "UPDATE tool_uses SET input_json=?1 WHERE uuid='kept'",
        params![json!({"command": "cargo test"}).to_string()],
    )
    .unwrap();
    let snapshot = |table: &str, sql: &Connection| -> Vec<Vec<SqlValue>> {
        let mut statement = sql
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |row| {
                (0..width)
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let before = [
        "sessions",
        "records",
        "native_checkpoints",
        "source_cursors",
    ]
    .map(|table| snapshot(table, &sql));

    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 17);
        // Migration 9 asks for nothing to be read again: the records, the
        // session and the cursors are exactly as they were.
        assert_eq!(
            ["sessions", "records", "source_cursors"].map(|table| snapshot(table, &sql)),
            [&before[0], &before[1], &before[3]].map(Clone::clone)
        );
        // The same upgrade also applies migration 16, whose one-time
        // transcript replay is the only thing that drops this checkpoint;
        // migration 9 itself never names the checkpoints.
        assert!(
            !include_str!("../migrations/0009_repeat_group_keys.sql")
                .contains("native_checkpoints")
        );
        assert!(snapshot("native_checkpoints", &sql).is_empty());
        assert!(
            store
                .native_checkpoint(SessionSource::Transcript, cursor)
                .unwrap()
                .is_none()
        );
        // The older row is unknown, and stays unknown: a retained copy of the
        // argument is not a fresh observation and is never derived from.
        assert_eq!(columns(&path, "kept"), [(0, None, None, false)]);
        assert!(
            store.records(SESSION).unwrap()[0].tool_uses[0]
                .group
                .is_none()
        );
    }

    // It becomes known only when that call is written again.
    let mut store = Store::open(&path).unwrap();
    store
        .upsert_records(
            SESSION,
            &[call("kept", "Bash", Some(json!({"command": "cargo test"})))],
            true,
        )
        .unwrap();
    assert_eq!(columns(&path, "kept")[0].1, Some(1));
}

#[test]
fn the_schema_refuses_a_half_stated_or_misspelled_key() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::MetadataOnly);
    store
        .upsert_records(
            SESSION,
            &[call("one", "Bash", Some(json!({"command": "cargo test"})))],
            true,
        )
        .unwrap();
    let sql = Connection::open(directory.path().join("keys.sqlite")).unwrap();
    for statement in [
        "UPDATE tool_uses SET group_key=NULL",
        "UPDATE tool_uses SET group_version=NULL",
        "UPDATE tool_uses SET group_key='abc'",
        "UPDATE tool_uses SET group_key=upper(group_key)",
        "UPDATE tool_uses SET group_version=0",
        "UPDATE tool_uses SET group_version='one'",
        "UPDATE tool_uses SET group_conflict=2",
    ] {
        assert!(sql.execute(statement, []).is_err(), "{statement}");
    }
}

#[test]
fn the_measurement_projection_every_receipt_publishes_carries_no_key() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::MetadataOnly);
    let path = directory.path().join("keys.sqlite");
    store
        .upsert_records(
            SESSION,
            &[call("one", "Bash", Some(json!({"command": "cargo test"})))],
            true,
        )
        .unwrap();
    let digest = key(&path, "one");
    let stored = &store.records(SESSION).unwrap()[0];
    assert!(stored.tool_uses[0].group.is_some());

    // C-08's content-free projection describes the same call by position and
    // name. The comparison key is private local metadata and is deliberately
    // not one of its fields: nothing a receipt publishes can carry the digest,
    // and a call whose key later changes cannot change a sealed revision.
    let projection = Projection::from_stored(stored).unwrap();
    let bytes = String::from_utf8(projection.canonical_bytes()).unwrap();
    assert!(!bytes.contains(&digest));
    assert!(bytes.contains("Bash"));
    let mut conflicted = stored.clone();
    conflicted.tool_uses[0].group = None;
    conflicted.tool_uses[0].group_conflict = true;
    let unkeyed = Projection::from_stored(&conflicted).unwrap();
    assert_eq!(unkeyed.canonical_bytes(), projection.canonical_bytes());
    assert_eq!(unkeyed.field_mask(), projection.field_mask());
    assert_eq!(unkeyed.conflicting_fields(&projection), 0);
}

/// One assistant record with exactly these content blocks.
fn blocks(uuid: &str, content: Value) -> CanonicalRecord {
    serde_json::from_value(json!({
        "uuid": uuid, "type": "assistant", "timestamp": "2026-09-07T12:00:00Z",
        "message": {"role": "assistant", "content": content}
    }))
    .unwrap()
}

#[test]
fn a_renamed_call_at_the_same_position_disputes_the_stored_key() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::MetadataOnly);
    let path = directory.path().join("keys.sqlite");
    store
        .upsert_records(
            SESSION,
            &[call("one", "Edit", Some(json!({"file_path": "/a.rs"})))],
            true,
        )
        .unwrap();
    let stored = key(&path, "one");
    // The same record and block index now name another tool. The name is part
    // of what the key compares, so the stored key no longer describes a call
    // this position is agreed to hold: it stands, and it is disputed.
    store
        .upsert_records(
            SESSION,
            &[call("one", "Write", Some(json!({"file_path": "/a.rs"})))],
            true,
        )
        .unwrap();
    assert_eq!(
        columns(&path, "one"),
        [(0, Some(1), Some(stored.clone()), true)]
    );
    // ... and so it is when the renaming observation could derive no key.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::MetadataOnly);
    let path = directory.path().join("keys.sqlite");
    store
        .upsert_records(
            SESSION,
            &[call("one", "Edit", Some(json!({"file_path": "/a.rs"})))],
            true,
        )
        .unwrap();
    store
        .upsert_records(SESSION, &[call("one", "Write", None)], true)
        .unwrap();
    assert_eq!(columns(&path, "one"), [(0, Some(1), Some(stored), true)]);
}

#[test]
fn blocks_are_compared_by_position_so_an_inserted_block_cannot_hide_a_change() {
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::MetadataOnly);
    let path = directory.path().join("keys.sqlite");
    store
        .upsert_records(
            SESSION,
            &[blocks(
                "one",
                json!([
                    {"type":"tool_use","name":"Edit","input":{"file_path":"/a.rs"}},
                    {"type":"tool_use","name":"Edit","input":{"file_path":"/b.rs"}}
                ]),
            )],
            true,
        )
        .unwrap();
    let before = columns(&path, "one");
    // A later observation of the same record puts text at block 0, so every
    // call moves one position. Paired by array order, the stored block 0 met
    // the observed block 1 and the stored block 1 met the observed block 2;
    // their indices differed, both pairs were skipped, and the change at
    // block 1 — `/b.rs` became `/a.rs` — went unnoticed.
    store
        .upsert_records(
            SESSION,
            &[blocks(
                "one",
                json!([
                    {"type":"text","text":"Synthetic"},
                    {"type":"tool_use","name":"Edit","input":{"file_path":"/a.rs"}},
                    {"type":"tool_use","name":"Edit","input":{"file_path":"/c.rs"}}
                ]),
            )],
            true,
        )
        .unwrap();
    let after = columns(&path, "one");
    // Every stored key stands. Block 0 is observed as no call at all, and
    // block 1 as a different call: both are disputed.
    assert_eq!(
        after,
        [
            (0, before[0].1, before[0].2.clone(), true),
            (1, before[1].1, before[1].2.clone(), true),
        ]
    );
}

#[test]
fn a_record_conflict_that_is_not_about_the_call_leaves_its_key_trusted() {
    // Retained inputs that disagree only in a field the key does not read: the
    // record conflicts, the call at each position is still the same call, and
    // its grouping evidence is not in dispute.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::FullContent);
    let path = directory.path().join("keys.sqlite");
    store
        .upsert_records(
            SESSION,
            &[call("one", "Bash", Some(json!({"command": "cargo test"})))],
            true,
        )
        .unwrap();
    let stored = key(&path, "one");
    store
        .upsert_records(
            SESSION,
            &[call(
                "one",
                "Bash",
                Some(json!({"command": "cargo test", "timeout": 60})),
            )],
            true,
        )
        .unwrap();
    assert!(store.records(SESSION).unwrap()[0].has_conflict);
    assert_eq!(columns(&path, "one"), [(0, Some(1), Some(stored), false)]);

    // A less complete observation that saw no content enumerates no block and
    // disputes nothing.
    let directory = TempDir::new().unwrap();
    let mut store = open(&directory, RetentionMode::MetadataOnly);
    let path = directory.path().join("keys.sqlite");
    store
        .upsert_records(
            SESSION,
            &[call("one", "Bash", Some(json!({"command": "cargo test"})))],
            true,
        )
        .unwrap();
    let stored = key(&path, "one");
    let silent: CanonicalRecord = serde_json::from_value(json!({
        "uuid": "one", "type": "assistant", "timestamp": "2026-09-07T12:00:00Z",
        "message": {"role": "assistant"}
    }))
    .unwrap();
    store.upsert_records(SESSION, &[silent], true).unwrap();
    assert_eq!(columns(&path, "one"), [(0, Some(1), Some(stored), false)]);
}
