//! The tool-sent input proof table: metadata only, closed kinds, immutable,
//! and bound only through the ingest batch to a stored human input.
use rusqlite::Connection;
use serde_json::json;
use xt_fixtures::TempDb;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, batch::IngestBatch, tool_sent::ToolSentKind,
};

const SESSION: &str = "codex-019a0000-0000-7000-8000-000000000001";

fn input(uuid: &str) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":uuid,"type":"user","timestamp":"2026-09-30T10:00:00Z",
        "message":{"role":"user","content":[{"type":"text","text":"<turn_aborted>x</turn_aborted>"}]}}))
    .unwrap()
}

#[test]
fn proofs_are_closed_metadata_only_and_immutable() {
    let mut db = TempDb::empty().unwrap();
    let mut meta = SessionMeta::new(SESSION, "codex", SessionSource::ReadersCli);
    meta.native_session_id = Some("019a0000-0000-7000-8000-000000000001".into());
    let records = [input("u1"), input("u2")];
    let kinds = [Some(ToolSentKind::CodexTurnAborted), None];
    let mut batch = IngestBatch::new(&meta, &records, false);
    batch.tool_sent = &kinds;
    let outcome = db.store_mut().apply_ingest_batch(&batch).unwrap();
    assert_eq!(
        outcome
            .affected_owners
            .iter()
            .map(|owner| owner.session_id.as_str())
            .collect::<Vec<_>>(),
        [SESSION],
        "the proven session's measurements are read again"
    );
    // Misaligned markers refuse the batch.
    let mut misaligned = IngestBatch::new(&meta, &records, false);
    misaligned.tool_sent = &kinds[..1];
    assert!(db.store_mut().apply_ingest_batch(&misaligned).is_err());

    let connection = Connection::open(db.path()).unwrap();
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('tool_sent_inputs') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        columns,
        ["record_uuid", "session_id", "evidence_kind", "rule_version"]
    );
    let row: (String, String, String, i64) = connection
        .query_row("SELECT * FROM tool_sent_inputs", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap();
    assert_eq!(
        row,
        ("u1".into(), SESSION.into(), "codex_turn_aborted".into(), 1)
    );
    for statement in [
        "UPDATE tool_sent_inputs SET evidence_kind='codex_apps_open_page'",
        "DELETE FROM tool_sent_inputs",
        // A kind outside the closed set, another rule version, or a record
        // the index does not hold.
        "INSERT INTO tool_sent_inputs VALUES ('u2','codex-019a0000-0000-7000-8000-000000000001','claude_task_notification',1)",
        "INSERT INTO tool_sent_inputs VALUES ('u2','codex-019a0000-0000-7000-8000-000000000001','codex_turn_aborted',2)",
        "INSERT INTO tool_sent_inputs VALUES ('missing','codex-019a0000-0000-7000-8000-000000000001','codex_turn_aborted',1)",
    ] {
        connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        assert!(connection.execute_batch(statement).is_err(), "{statement}");
    }
}
