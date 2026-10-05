//! A lane span's last prompt, through the app's own command path.
//!
//! Under the default metadata-only retention the index keeps only a short
//! preview of a person's message, and the span detail shows it without
//! opening anything. Where no preview is kept (an index written before
//! previews, or one whose stored text was deleted) it reads the words from
//! the session's original source — the same resolution and bounded reader a
//! transcript open uses — and picks the record by its saved identity alone.
//! These cases prove what the bubble may say: the words when the record is
//! there, an honest state when it is not or the source changed, the index's
//! own words first when it kept them, and that nothing read is ever written.
//!
//! Every transcript here is written by the test: short synthetic lines in the
//! canonical shape, never a real conversation.
#![cfg(unix)]
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use xt_ingest::native::{ImportRequest, ProducerSource, import_native, readers_cli::read_pin};
use xt_store::{CanonicalRecord, Host, Store, retention::RetentionMode};
use xtrace_desktop::{
    dto::{
        DashboardPromptText, DashboardSpanDetail, DashboardSpanPrompt, DashboardSpanTool,
        SessionSourceReason,
    },
    native_index::DetailReaders,
    state::{AppState, StartupOptions},
    transcript_reads::{MAX_ACTIVE_READS, TranscriptReads},
};

const OBSERVED_AT: i64 = 1_788_782_400_000;
const SESSION: &str = "00000000-0000-4000-8000-00000000bbbb";
const PROJECT: &str = "-repo-fixture";
/// Unique enough that finding it in a database file means it was written.
const WORDS: &str = "summarize zebra-quartz notes";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}
fn uuid(index: usize) -> String {
    format!("22222222-2222-4222-8222-{index:012}")
}
fn ms(ts: &str) -> i64 {
    ts.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

/// A person's question, then an answer that calls a tool.
fn transcript() -> String {
    [
        json!({
            "uuid": uuid(0), "type": "user", "sessionId": SESSION, "cwd": "/repo/fixture",
            "timestamp": "2026-09-07T12:00:00.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": format!("please\n  {WORDS}")}]}
        }),
        json!({
            "uuid": uuid(1), "type": "assistant", "sessionId": SESSION, "cwd": "/repo/fixture",
            "timestamp": "2026-09-07T12:00:02.000Z",
            "message": {"role": "assistant", "model": "claude-fixture", "content": [
                {"type": "tool_use", "id": "toolu_0001", "name": "Read",
                 "input": {"file_path": "/repo/fixture/a.txt"}}]}
        }),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect()
}

fn source_path(root: &Path) -> PathBuf {
    root.join("home/.claude/projects")
        .join(PROJECT)
        .join(format!("{SESSION}.jsonl"))
}

/// Write the home and index it exactly as the app does, under `retention`.
fn indexed(retention: RetentionMode) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let source = source_path(root.path());
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(&source, transcript()).unwrap();
    let db = root.path().join("data/xtrace.db");
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    let mut store = Store::open(&db).unwrap();
    store.set_retention_mode(retention).unwrap();
    let report = import_native(
        &mut store,
        &ImportRequest {
            home: &root.path().join("home"),
            hosts: &[Host::Claude],
            producer: &ProducerSource::Bundle {
                pin: read_pin(&repo().join(".plugin-pin")).unwrap(),
                root: repo().join("vendor/agent-plugins"),
            },
            python: None,
            observed_at: OBSERVED_AT,
            cancel: None,
        },
    );
    assert!(report.complete(), "{report:?}");
    root
}

/// [`indexed`], then the database as an index without previews leaves it:
/// the rows removed and the file rewritten, so not one byte of the words is
/// left in it or beside it.
fn indexed_without_previews() -> tempfile::TempDir {
    let root = indexed(RetentionMode::MetadataOnly);
    let connection = rusqlite::Connection::open(root.path().join("data/xtrace.db")).unwrap();
    connection
        .execute_batch("DELETE FROM record_previews; VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    drop(connection);
    assert!(!contains(&database_bytes(root.path()), WORDS));
    root
}

fn state(root: &Path) -> AppState {
    AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(root.join("home")),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap()
}

fn canonical_id(root: &Path) -> String {
    Store::open(root.join("data/xtrace.db"))
        .unwrap()
        .session(SESSION)
        .unwrap()
        .expect("indexed")
        .meta
        .session_id
}

/// The prompt's words for the span 12:00:00 → `end`.
fn words(root: &Path, end: &str, reads: &TranscriptReads) -> (bool, DashboardPromptText) {
    let (in_span, text, _) = detail(root, end, reads);
    (in_span, text)
}

/// The whole detail for the span 12:00:00 → `end`, read under `READ`.
fn detail(
    root: &Path,
    end: &str,
    reads: &TranscriptReads,
) -> (bool, DashboardPromptText, DashboardSpanTool) {
    let state = state(root);
    let detail = state
        .span_detail(
            &canonical_id(root),
            ms("2026-09-07T12:00:00Z"),
            ms(end),
            reads,
            READ,
            &DetailReaders::disabled(),
        )
        .unwrap();
    let DashboardSpanDetail::Indexed {
        tool,
        prompt: DashboardSpanPrompt::Found { in_span, text, .. },
        ..
    } = detail
    else {
        panic!("the span has a person's message: {detail:?}");
    };
    (in_span, text, tool)
}
const READ: &str = "span-read";

fn stored(text: &str) -> DashboardPromptText {
    DashboardPromptText::Stored {
        text: text.into(),
        truncated: false,
    }
}

/// Every byte of the database and its sidecars.
fn database_bytes(root: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    for entry in fs::read_dir(root.join("data")).unwrap() {
        bytes.extend(fs::read(entry.unwrap().path()).unwrap());
    }
    bytes
}

#[test]
fn unkept_words_are_read_from_the_source_and_never_written() {
    let root = indexed_without_previews();
    let db = root.path().join("data/xtrace.db");
    let rows = |db: &Path| -> Vec<(String, Option<String>)> {
        let connection = rusqlite::Connection::open(db).unwrap();
        let mut statement = connection
            .prepare("SELECT uuid,content_json FROM records ORDER BY uuid")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let before = (rows(&db), fs::read(source_path(root.path())).unwrap());
    assert!(before.0.iter().all(|(_, content)| content.is_none()));
    assert!(!contains(&database_bytes(root.path()), WORDS));

    let (in_span, text) = words(
        root.path(),
        "2026-09-07T12:00:02Z",
        &TranscriptReads::default(),
    );
    assert!(in_span);
    // The same joined text the classifier saw, as one line.
    assert_eq!(text, stored(&format!("please {WORDS}")));

    // Nothing was written: no row changed, the source is untouched, and the
    // words appear nowhere in the database or beside it.
    assert_eq!(
        before,
        (rows(&db), fs::read(source_path(root.path())).unwrap())
    );
    assert!(!contains(&database_bytes(root.path()), WORDS));
    let stray: Vec<_> = fs::read_dir(root.path().join("data"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| !name.to_string_lossy().starts_with("xtrace.db"))
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn a_source_without_the_saved_record_says_so_rather_than_showing_another() {
    let root = indexed_without_previews();
    // A later person's message the index saved but the source does not hold.
    let later: CanonicalRecord = serde_json::from_value(json!({
        "uuid": uuid(9), "type": "user", "timestamp": "2026-09-07T12:00:03.000Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "not in the file"}]}
    }))
    .unwrap();
    Store::open(root.path().join("data/xtrace.db"))
        .unwrap()
        .upsert_records(&canonical_id(root.path()), &[later], false)
        .unwrap();
    let (_, text) = words(
        root.path(),
        "2026-09-07T12:00:03Z",
        &TranscriptReads::default(),
    );
    assert_eq!(text, DashboardPromptText::NotFound);
}

#[test]
fn a_changed_source_or_a_cancel_returns_no_words() {
    let root = indexed_without_previews();
    // Cancelled before it began: the registry remembers it.
    let (_, text) = words(root.path(), "2026-09-07T12:00:02Z", &{
        let reads = TranscriptReads::default();
        reads.cancel(READ);
        reads
    });
    assert_eq!(
        text,
        DashboardPromptText::Unavailable {
            reason: SessionSourceReason::Cancelled
        }
    );
    // Same length, different words: the source is no longer what was measured.
    let rewritten = transcript().replace("zebra", "otter");
    fs::write(source_path(root.path()), rewritten).unwrap();
    let (_, text) = words(
        root.path(),
        "2026-09-07T12:00:02Z",
        &TranscriptReads::default(),
    );
    assert_eq!(
        text,
        DashboardPromptText::Unavailable {
            reason: SessionSourceReason::Replaced
        }
    );
}

#[test]
fn words_the_index_kept_are_used_without_opening_the_source() {
    let root = indexed(RetentionMode::FullContent);
    // With the source gone, only the index's own words can answer.
    fs::remove_file(source_path(root.path())).unwrap();
    let (_, text) = words(
        root.path(),
        "2026-09-07T12:00:02Z",
        &TranscriptReads::default(),
    );
    assert_eq!(text, stored(&format!("please {WORDS}")));
}

#[test]
fn every_read_slot_taken_still_answers_the_measured_detail() {
    let root = indexed_without_previews();
    let reads = TranscriptReads::default();
    let held: Vec<_> = (0..MAX_ACTIVE_READS)
        .map(|index| reads.begin(&format!("other-{index}")).unwrap())
        .collect();
    let (in_span, text, tool) = detail(root.path(), "2026-09-07T12:00:02Z", &reads);
    // The words wait for a slot; the tool and the rest do not.
    assert!(in_span);
    assert_eq!(text, DashboardPromptText::Busy);
    assert_eq!(
        tool,
        DashboardSpanTool::Called {
            name: "Read".into(),
            calls: 1
        }
    );
    drop(held);
    let (_, text, _) = detail(root.path(), "2026-09-07T12:00:02Z", &reads);
    assert_eq!(text, stored(&format!("please {WORDS}")));
}

/// The default index keeps the person's message's short preview, and the
/// bubble answers from it alone: with the source gone and every read slot
/// taken, the words are still there, so no source was opened and no slot was
/// asked for. Deleting stored content takes the preview away, and the words
/// are then read from the source again.
#[test]
fn a_kept_preview_answers_without_opening_the_source() {
    let root = indexed(RetentionMode::MetadataOnly);
    let db = root.path().join("data/xtrace.db");
    let content: i64 = rusqlite::Connection::open(&db)
        .unwrap()
        .query_row("SELECT count(content_json) FROM records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        content, 0,
        "metadata-only retention keeps no record content"
    );
    let source = fs::read(source_path(root.path())).unwrap();
    fs::remove_file(source_path(root.path())).unwrap();
    let reads = TranscriptReads::default();
    let held: Vec<_> = (0..MAX_ACTIVE_READS)
        .map(|index| reads.begin(&format!("other-{index}")).unwrap())
        .collect();
    let (in_span, text) = words(root.path(), "2026-09-07T12:00:02Z", &reads);
    assert!(in_span);
    assert_eq!(text, stored(&format!("please {WORDS}")));
    drop(held);

    // "Delete stored content" removes the preview; the source answers again.
    fs::write(source_path(root.path()), source).unwrap();
    let state = state(root.path());
    let purge = state.purge_stored_content(|| {}).unwrap();
    assert!(purge.invalidate_content);
    assert!(
        purge
            .tables
            .iter()
            .any(|table| table.table == "record_previews" && table.rows == 1),
        "{purge:?}"
    );
    state.shutdown();
    fs::remove_file(source_path(root.path())).unwrap();
    let (_, text) = words(
        root.path(),
        "2026-09-07T12:00:02Z",
        &TranscriptReads::default(),
    );
    assert!(
        matches!(
            text,
            DashboardPromptText::Unavailable { .. } | DashboardPromptText::NotFound
        ),
        "{text:?}"
    );
}
