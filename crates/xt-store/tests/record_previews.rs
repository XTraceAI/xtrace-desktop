//! Short previews of a person's messages and of Claude Code task
//! notifications' summaries: kept by the ingest batch, in its transaction,
//! whatever the retention mode, only for inputs whose whole text the current
//! classification calls a person's (or a proven notification's summary), and
//! cleared by "Delete stored content".

use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store,
    batch::IngestBatch,
    human_input::InputAdjustment,
    record_preview::{self, MAX_PREVIEW_CHARS, Preview, PreviewKind},
    retention::{ContentRegistry, RetentionMode},
};

fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn text(uuid: &str, minute: u32, words: &str) -> CanonicalRecord {
    record(
        json!({"uuid":uuid,"type":"user","timestamp":format!("2026-09-07T12:{minute:02}:00Z"),
        "message":{"role":"user","content":[{"type":"text","text":words}]}}),
    )
}
fn session(id: &str, host: &str) -> SessionMeta {
    let mut meta = SessionMeta::new(
        id,
        host,
        if host == "claude" {
            SessionSource::Transcript
        } else {
            SessionSource::ReadersCli
        },
    );
    meta.native_session_id = Some(id.trim_start_matches("codex-").into());
    meta
}

/// Every kind of user-role input, and an assistant reply, in one session.
fn inputs() -> Vec<CanonicalRecord> {
    let long = format!("start {}\n\n end", "word ".repeat(100));
    vec![
        text("person", 0, "  Make the   PR\nlink exact  "),
        text("long", 1, &long),
        text(
            "wrapped",
            2,
            "<send_user_message_question_reply>[]</send_user_message_question_reply>",
        ),
        text(
            "note",
            3,
            "<task-notification><task-id>t</task-id><summary>Agent  finished</summary><result>quoted <summary>x</summary></result></task-notification>",
        ),
        record(
            json!({"uuid":"reply","type":"assistant","timestamp":"2026-09-07T12:04:00Z",
            "message":{"id":"m","role":"assistant","model":"synthetic-model",
                "content":[{"type":"text","text":"Synthetic reply"}]}}),
        ),
        record(
            json!({"uuid":"result","type":"user","timestamp":"2026-09-07T12:05:00Z",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"synthetic output"}]}}),
        ),
        record(
            json!({"uuid":"meta","type":"user","isMeta":true,"timestamp":"2026-09-07T12:06:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic context"}]}}),
        ),
        record(
            json!({"uuid":"side","type":"user","isSidechain":true,"timestamp":"2026-09-07T12:07:00Z",
            "message":{"role":"user","content":[{"type":"text","text":"Synthetic sidechain"}]}}),
        ),
        text("command", 8, "<command-name>/synthetic</command-name>"),
        text(
            "reminder",
            9,
            "<system-reminder>synthetic</system-reminder>",
        ),
        text("interrupted", 10, "[Request interrupted by user]"),
        record(
            json!({"uuid":"unknown","type":"user","timestamp":"2026-09-07T12:11:00Z",
            "message":{"role":"user"}}),
        ),
    ]
}

fn write(
    store: &mut Store,
    meta: &SessionMeta,
    rows: &[CanonicalRecord],
    notes: &[&str],
    withheld: &[&str],
) {
    let notes = rows
        .iter()
        .map(|row| notes.contains(&row.uuid.as_deref().unwrap_or("")))
        .collect::<Vec<_>>();
    let withheld = rows
        .iter()
        .map(|row| withheld.contains(&row.uuid.as_deref().unwrap_or("")))
        .collect::<Vec<_>>();
    let mut batch = IngestBatch::new(meta, rows, true);
    batch.task_notifications = &notes;
    batch.withheld_previews = &withheld;
    store.apply_ingest_batch(&batch).unwrap();
}

fn previews(path: &std::path::Path) -> Vec<(String, String, String, Option<String>, bool)> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT record_uuid,session_id,kind,text,truncated FROM record_previews ORDER BY record_uuid",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn previews_are_kept_only_for_a_persons_whole_message_and_a_notifications_summary() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("previews.sqlite");
    let mut store = Store::open(&path).unwrap();
    // The default mode keeps no content; previews are kept all the same.
    assert_eq!(store.retention_mode().unwrap(), RetentionMode::MetadataOnly);
    let meta = session("claude-previews", "claude");
    write(&mut store, &meta, &inputs(), &["note"], &["wrapped"]);
    let kept = previews(&path);
    let names: Vec<(&str, &str)> = kept
        .iter()
        .map(|row| (row.0.as_str(), row.2.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("long", "person"),
            ("note", "automatic"),
            ("person", "person")
        ]
    );
    let row = |uuid: &str| kept.iter().find(|row| row.0 == uuid).unwrap();
    assert_eq!(row("person").3.as_deref(), Some("Make the PR link exact"));
    assert!(!row("person").4);
    assert_eq!(row("person").1, "claude-previews");
    let long = row("long");
    assert!(long.4, "more followed");
    assert_eq!(long.3.as_ref().unwrap().chars().count(), MAX_PREVIEW_CHARS);
    assert!(long.3.as_ref().unwrap().starts_with("start word word"));
    // The notification's own summary, never its result or its whole text.
    assert_eq!(row("note").3.as_deref(), Some("Agent finished"));
    // No content was kept anywhere else.
    let content: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(content_json) FROM records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(content, 0);
    // The reader returns them by kind only.
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        record_preview::read(&connection, "person", PreviewKind::Person).unwrap(),
        Some(Preview {
            text: "Make the PR link exact".into(),
            truncated: false
        })
    );
    assert_eq!(
        record_preview::read(&connection, "person", PreviewKind::Automatic).unwrap(),
        None
    );
    assert_eq!(
        record_preview::read(&connection, "wrapped", PreviewKind::Person).unwrap(),
        None
    );
    // A replay keeps exactly the same rows.
    write(&mut store, &meta, &inputs(), &["note"], &["wrapped"]);
    assert_eq!(previews(&path), kept);
}

#[test]
fn a_rejected_input_or_batch_keeps_no_preview() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("rejected.sqlite");
    let mut store = Store::open(&path).unwrap();
    let owner = session("claude-owner", "claude");
    write(
        &mut store,
        &owner,
        &[text("owned", 0, "the owner's words")],
        &[],
        &[],
    );
    // The same identity read under another session is rejected: the owner's
    // preview stays the owner's, and the rejected input adds nothing.
    let other = session("claude-other", "claude");
    write(
        &mut store,
        &other,
        &[
            text("owned", 0, "different words"),
            text("fresh", 1, "fresh words"),
        ],
        &[],
        &[],
    );
    let kept = previews(&path);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[1].0, "owned");
    assert_eq!(kept[1].3.as_deref(), Some("the owner's words"));
    // A batch that fails after its rows were written rolls everything back,
    // previews included.
    let rows = [text("lost", 2, "never kept")];
    let mut batch = IngestBatch::new(&other, &rows, true);
    let misaligned = [false, false];
    batch.withheld_previews = &misaligned;
    assert!(store.apply_ingest_batch(&batch).is_err());
    assert_eq!(previews(&path), kept);
    let records: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM records WHERE uuid='lost'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(records, 0);
}

#[test]
fn deleting_stored_content_clears_previews() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("purge.sqlite");
    let mut store = Store::open(&path).unwrap();
    write(
        &mut store,
        &session("claude-purge", "claude"),
        &inputs(),
        &["note"],
        &[],
    );
    let outcome = store.purge_content(&ContentRegistry::default()).unwrap();
    assert!(outcome.invalidate_content);
    assert!(
        outcome
            .tables
            .iter()
            .any(|table| table.table == "record_previews" && table.rows == 4),
        "{outcome:?}"
    );
    assert!(previews(&path).iter().all(|row| row.3.is_none()));
    let connection = Connection::open(&path).unwrap();
    for (uuid, kind) in [
        ("person", PreviewKind::Person),
        ("note", PreviewKind::Automatic),
    ] {
        assert_eq!(record_preview::read(&connection, uuid, kind).unwrap(), None);
    }
    // A replay does not bring a deleted preview back for the same record.
    write(
        &mut store,
        &session("claude-purge", "claude"),
        &inputs(),
        &["note"],
        &[],
    );
    assert!(previews(&path).iter().all(|row| row.3.is_none()));
    // A second purge has nothing left to clear.
    assert!(
        !store
            .purge_content(&ContentRegistry::default())
            .unwrap()
            .invalidate_content
    );
}

#[test]
fn a_human_input_adjustment_withdraws_a_person_preview() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("adjusted.sqlite");
    let mut store = Store::open(&path).unwrap();
    let meta = session("codex-0199aaaa-0000-7000-8000-000000000001", "codex");
    let words = "<send_user_message_question_reply>[]</send_user_message_question_reply>";
    write(
        &mut store,
        &meta,
        &[text("reply", 0, words), text("plain", 1, "plain words")],
        &[],
        &[],
    );
    assert_eq!(previews(&path).len(), 2);
    let report = store
        .apply_human_input_adjustments(&[InputAdjustment {
            record_uuid: "reply".into(),
            session_id: meta.session_id.clone(),
            original_ts: "2026-09-07T12:00:00Z".into(),
            original_length: words.chars().count() as i64,
            retained_length: Some(2),
            reason: "question_reply".into(),
            native_item_id: None,
        }])
        .unwrap();
    assert_eq!(report.applied, 1);
    let kept = previews(&path);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].0, "plain");
    // Read again, the adjusted record's counted length is not its own, so no
    // preview comes back even without the caller's marker.
    write(&mut store, &meta, &[text("reply", 0, words)], &[], &[]);
    assert_eq!(previews(&path), kept);
}

/// A record whose identifier is longer than the preview table holds is still
/// saved, with the rest of its batch; it only gets no preview.
#[test]
fn an_identifier_too_long_for_a_preview_never_refuses_the_batch() {
    let mut store = Store::open_in_memory().unwrap();
    let meta = session("claude-long-id", "claude");
    let long = "x".repeat(300);
    let rows = [
        text(&long, 0, "a person's words"),
        text("short", 1, "more words"),
    ];
    write(&mut store, &meta, &rows, &[], &[]);
    assert_eq!(store.counts().unwrap().records, 2);
    assert_eq!(
        store.record_preview(&long, PreviewKind::Person).unwrap(),
        None
    );
    assert!(
        store
            .record_preview("short", PreviewKind::Person)
            .unwrap()
            .is_some()
    );
}
