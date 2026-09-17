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
fn native_order_combines_overlapping_copies_in_either_arrival_order() {
    let child = "00000000-0000-4000-8000-000000000010";
    for child_first in [false, true] {
        for overlap_only in [false, true] {
            let (_temp, project, file, mut store, c) = setup();
            append(&file, &row(NEW, 20));
            let copy = project.join(format!("{child}.jsonl"));
            // Shift the copy's offsets. In the overlap case neither source alone
            // proves OLD -> LAST: the shared NEW joins their evidence.
            let prefix = if overlap_only {
                String::new()
            } else {
                row(OLD, 10)
            };
            fs::write(
                &copy,
                format!(
                    "{{\"type\":\"attachment\"}}\n{prefix}{}{}",
                    row(NEW, 20),
                    row(LAST, 30)
                ),
            )
            .unwrap();
            let order = if child_first {
                [child, SID]
            } else {
                [SID, child]
            };
            complete(scan(&mut store, &project, order[0], ScanMode::Resume));
            if overlap_only {
                assert!(!precedes(&c, OLD, LAST));
            }
            complete(scan(&mut store, &project, order[1], ScanMode::Resume));
            assert!(precedes(&c, OLD, NEW));
            assert!(precedes(&c, NEW, LAST));
            assert!(precedes(&c, OLD, LAST));
            assert!(!precedes(&c, LAST, OLD));
            assert_eq!(edges(&c), 2);
            assert_eq!(store.counts().unwrap().records, 3);
            for id in order {
                complete(scan(&mut store, &project, id, ScanMode::Replay));
                assert_eq!(edges(&c), 2);
                assert_eq!(store.counts().unwrap().records, 3);
                assert!(precedes(&c, OLD, LAST));
            }
        }
    }
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

fn try_write_native(
    store: &mut Store,
    session: &str,
    ids: &[&str],
    observe_order: bool,
) -> xt_store::Result<xt_ingest::writer::BatchOutcome> {
    use xt_ingest::{
        canonical::{Parsed, SourceContext, parse_line},
        writer::{WriteBatch, write_batch},
    };
    let context = SourceContext {
        conversation_id: Some(session.into()),
        native_session_id: Some(session.into()),
        source: Some(xt_store::SessionSource::Transcript),
        source_surface: Some(format!("surface-{session}")),
        ..Default::default()
    };
    let records: Vec<_> = ids
        .iter()
        .map(|id| {
            let mut value: serde_json::Value = serde_json::from_str(&row(id, 10)).unwrap();
            value.as_object_mut().unwrap().remove("sessionId");
            if id.starts_with("middle-") {
                value["timestamp"] = json!("2026-09-07T11:00:00Z");
            }
            if *id == "other-api" {
                value["message"]["id"] = json!("other-response");
            }
            if *id == "other-request" {
                value["requestId"] = json!("other-request");
            }
            match parse_line(&value.to_string()).unwrap() {
                Parsed::Record(record) => *record,
                _ => panic!("expected record"),
            }
        })
        .collect();
    let discovery = xt_store::ingest::DiscoveredSession {
        host: xt_store::Host::Claude,
        native_session_id: session.into(),
        conversation_id: Some(session.into()),
        surface: context.source_surface.clone(),
        started_at_ms: None,
        last_observed_at: 100,
        discovery_complete: true,
    };
    let order = xt_store::batch::NativeOrderSource {
        key: session.into(),
        reset: true,
    };
    write_batch(
        store,
        &WriteBatch {
            context: &context,
            declared_host: Some(xt_store::Host::Claude),
            records: &records,
            title: None,
            cwd: None,
            git_branch: None,
            namespace: None,
            keep_content: false,
            observed_at: 100,
            receipt: None,
            cursor: None,
            discovery: Some(&discovery),
            checkpoint: None,
            native_order: observe_order.then_some(&order),
        },
    )
}

fn write_native(
    store: &mut Store,
    session: &str,
    ids: &[&str],
    observe_order: bool,
) -> xt_ingest::writer::BatchOutcome {
    try_write_native(store, session, ids, observe_order).unwrap()
}

fn assert_invalidated(output: &xt_ingest::writer::BatchOutcome, expected: &[&str]) {
    let mut ids = output
        .events
        .iter()
        .map(|event| event.conversation_id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(ids, expected);
    for event in &output.events {
        assert!(event.invalidate_measurements && event.invalidate_cost);
        assert_eq!((event.records_new, event.records_enriched), (0, 0));
        assert_eq!(
            event.surface,
            Some(format!("surface-{}", event.conversation_id))
        );
    }
}

#[test]
fn native_order_new_edge_invalidates_existing_copied_context() {
    let (_temp, _project, _file, mut store, c) = setup();
    write_native(&mut store, "owner", &[OLD, NEW], false);
    write_native(&mut store, "copy", &[OLD, NEW], false);
    write_native(&mut store, "importer", &[OLD, NEW], false);
    write_native(&mut store, "other-api", &["other-api"], false);
    write_native(&mut store, "other-request", &["other-request"], false);
    let before = store.counts().unwrap();
    c.execute_batch(&format!("CREATE TRIGGER fail_order BEFORE INSERT ON native_response_heads WHEN NEW.record_uuid='{NEW}' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;")).unwrap();
    assert!(try_write_native(&mut store, "importer", &[OLD, NEW], true).is_err());
    assert_eq!(edges(&c), 0);
    assert_eq!(store.counts().unwrap(), before);
    assert_eq!(
        c.query_row("SELECT count(*) FROM native_response_heads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER fail_order").unwrap();
    let output = write_native(&mut store, "importer", &[OLD, NEW, NEW], true);
    assert!(precedes(&c, OLD, NEW));
    assert_invalidated(&output, &["copy", "importer", "owner"]);
    let replay = write_native(&mut store, "importer", &[OLD, NEW], true);
    assert_eq!(replay.events.len(), 1);
    assert!(!replay.events[0].invalidate_measurements && !replay.events[0].invalidate_cost);
}

#[test]
fn native_order_bridge_invalidates_context_without_either_endpoint() {
    const FIRST: &str = "middle-first";
    const MIDDLE: &str = "middle-second";
    let (_temp, _project, _file, mut store, c) = setup();
    write_native(&mut store, "owner", &[OLD, FIRST, MIDDLE, LAST], false);
    write_native(&mut store, "copy", &[OLD, LAST], false);
    write_native(&mut store, "left", &[OLD, FIRST], true);
    write_native(&mut store, "right", &[MIDDLE, LAST], true);
    write_native(&mut store, "bridge", &[FIRST, MIDDLE], false);
    assert!(!precedes(&c, OLD, LAST));
    let output = write_native(&mut store, "bridge", &[FIRST, MIDDLE], true);
    assert!(precedes(&c, OLD, LAST));
    assert_eq!((output.records_new, output.records_enriched), (0, 0));
    let endpoints: i64 = c.query_row("SELECT count(*) FROM session_work_records WHERE session_id='copy' AND record_uuid IN (?1,?2)", [FIRST, MIDDLE], |r| r.get(0)).unwrap();
    assert_eq!(endpoints, 0);
    assert_invalidated(&output, &["bridge", "copy", "left", "owner", "right"]);
}
