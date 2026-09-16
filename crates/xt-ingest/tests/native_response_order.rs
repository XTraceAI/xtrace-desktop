use rusqlite::Connection;
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use xt_ingest::native::{ScanMode, SessionOutcome, claude_fs};
use xt_store::Store;
const SID: &str = "00000000-0000-4000-8000-000000000001";
const OLD: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";
const NEW: &str = "00000000-0000-4000-8000-000000000002";
const LAST: &str = "00000000-0000-4000-8000-000000000003";
fn row(id: &str, usage: i64) -> String {
    json!({"uuid":id,"type":"assistant","sessionId":SID,"requestId":"request","timestamp":"2026-09-07T12:00:00Z","message":{"role":"assistant","id":"response","usage":{"input_tokens":usage,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}).to_string()+"\n"
}
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, Store, Connection) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let file = project.join(format!("{SID}.jsonl"));
    fs::write(&file, row(OLD, 10)).unwrap();
    let db = temp.path().join("index.sqlite");
    let store = Store::open(&db).unwrap();
    let c = Connection::open(&db).unwrap();
    (temp, project, file, store, c)
}
fn scan(store: &mut Store, project: &Path, id: &str, mode: ScanMode) -> SessionOutcome {
    let file = claude_fs::enumerate(project.parent().unwrap())
        .unwrap()
        .0
        .into_iter()
        .find(|f| f.session_id == id)
        .unwrap();
    claude_fs::import_file(store, &file, 1_800_000_000_000, mode)
        .unwrap()
        .outcome
}
fn complete(result: SessionOutcome) {
    assert!(
        matches!(result, SessionOutcome::Imported { .. }),
        "{result:?}"
    );
}
fn append(file: &Path, text: &str) {
    fs::OpenOptions::new()
        .append(true)
        .open(file)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}
fn precedes(c: &Connection, a: &str, b: &str) -> bool {
    c.query_row("WITH RECURSIVE later(uuid) AS (SELECT after_uuid FROM native_response_order WHERE before_uuid=?1 UNION SELECT e.after_uuid FROM native_response_order e JOIN later ON e.before_uuid=later.uuid) SELECT EXISTS(SELECT 1 FROM later WHERE uuid=?2)",[a,b],|r|r.get(0)).unwrap()
}
fn edges(c: &Connection) -> i64 {
    c.query_row("SELECT count(*) FROM native_response_order", [], |r| {
        r.get(0)
    })
    .unwrap()
}
#[test]
fn native_order_preserves_ties_across_append_replay_and_copied_prefixes() {
    let (_temp, project, file, mut store, c) = setup();
    append(&file, &row(NEW, 20));
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    assert!(precedes(&c, OLD, NEW));
    append(&file, &row(LAST, 30));
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    assert!(precedes(&c, OLD, LAST));
    assert_eq!(edges(&c), 2);
    complete(scan(&mut store, &project, SID, ScanMode::Replay));
    assert_eq!(edges(&c), 2);
    let child = "00000000-0000-4000-8000-000000000010";
    let copy = project.join(format!("{child}.jsonl"));
    fs::write(
        &copy,
        format!(
            "{{\"type\":\"attachment\"}}\n{}{}{}",
            row(OLD, 10),
            row(NEW, 20),
            row(LAST, 30)
        ),
    )
    .unwrap();
    complete(scan(&mut store, &project, child, ScanMode::Resume));
    assert_eq!(edges(&c), 2);
    assert_eq!(store.counts().unwrap().records, 3);
    append(&file, &row(OLD, 10));
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    assert_eq!(edges(&c), 2);
    assert!(!precedes(&c, LAST, OLD));
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM records WHERE content_json IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn native_order_and_continuation_roll_back_with_failed_checkpoint() {
    let (_temp, project, file, mut store, c) = setup();
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    append(&file, &row(NEW, 20));
    c.execute_batch("CREATE TRIGGER reject_checkpoint BEFORE UPDATE ON native_checkpoints BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(matches!(
        scan(&mut store, &project, SID, ScanMode::Resume),
        SessionOutcome::Skipped { .. }
    ));
    assert_eq!(edges(&c), 0);
    assert_eq!(store.counts().unwrap().records, 1);
    assert_eq!(
        c.query_row("SELECT record_uuid FROM native_response_heads", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        OLD
    );
    c.execute_batch("DROP TRIGGER reject_checkpoint;").unwrap();
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    assert!(precedes(&c, OLD, NEW));
}
#[test]
fn native_order_resets_inert_rewrites_and_keeps_proven_historical_edges() {
    let (_temp, project, file, mut store, c) = setup();
    append(&file, &row(NEW, 20));
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    fs::write(&file, "{\"type\":\"attachment\"}\n").unwrap();
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    assert_eq!(
        c.query_row("SELECT count(*) FROM native_response_heads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(precedes(&c, OLD, NEW));
    append(&file, &row(LAST, 30));
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    assert_eq!(edges(&c), 1);
    assert!(!precedes(&c, NEW, LAST));
}
#[test]
fn native_order_upgrade_revisits_unchanged_files_without_changing_records() {
    let (temp, project, file, mut store, c) = setup();
    append(&file, &row(NEW, 20));
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    let before = store.counts().unwrap();
    drop(store);
    c.execute_batch("DROP TABLE native_response_heads; DROP TABLE native_response_order; DELETE FROM schema_version WHERE version=5;").unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    assert_eq!(store.counts().unwrap(), before);
    assert_eq!(
        c.query_row("SELECT count(*) FROM native_checkpoints", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    complete(scan(&mut store, &project, SID, ScanMode::Resume));
    assert!(precedes(&c, OLD, NEW));
    assert_eq!(store.counts().unwrap(), before);
}
#[test]
fn native_order_is_unavailable_to_generic_batches() {
    let (_temp, _project, _file, mut store, c) = setup();
    let fixture = xt_store::SessionMeta {
        session_id: SID.into(),
        host: xt_store::Host::Claude,
        source: xt_store::SessionSource::Fixture,
        cwd: None,
        git_branch: None,
        title: None,
        source_platform: None,
        surface: None,
        surface_evidence: None,
        native_session_id: None,
        started_at_ms: None,
    };
    let records: Vec<xt_store::CanonicalRecord> = vec![
        serde_json::from_str(&row(OLD, 10)).unwrap(),
        serde_json::from_str(&row(NEW, 20)).unwrap(),
    ];
    let order = xt_store::batch::NativeOrderSource {
        key: "synthetic".into(),
        reset: true,
    };
    let mut batch = xt_store::batch::IngestBatch::new(&fixture, &records, false);
    batch.native_order = Some(&order);
    assert!(store.apply_ingest_batch(&batch).is_err());
    assert_eq!(edges(&c), 0);
    assert_eq!(store.counts().unwrap().records, 0);
}
