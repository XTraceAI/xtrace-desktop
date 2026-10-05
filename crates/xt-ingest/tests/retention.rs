use rusqlite::Connection;
use serde_json::json;
use xt_fixtures::TempDb;
use xt_ingest::{
    canonical::{Parsed, ParsedRecord, SourceContext, parse_line},
    writer::{WriteBatch, coverage, matches_current, write_batch},
};
use xt_store::{
    SessionMeta, SessionSource, Store,
    retention::{ContentRegistry, RetentionMode},
};

fn row(uuid: &str) -> ParsedRecord {
    let Parsed::Record(record) = parse_line(&json!({"uuid":uuid,"type":"assistant","parentUuid":"parent","timestamp":"2026-09-10T00:00:00Z","message":{"role":"assistant","model":"synthetic-model","content":[{"type":"text","text":"Synthetic transcript"},{"type":"tool_use","name":"Read","input":{"path":"synthetic.txt"}},{"type":"tool_result","content":"Synthetic result"}],"usage":{"input_tokens":7,"output_tokens":3}}}).to_string()).unwrap() else { panic!("synthetic record"); };
    *record
}
fn context() -> SourceContext {
    SourceContext {
        conversation_id: Some("retained".into()),
        source: Some(SessionSource::Transcript),
        ..Default::default()
    }
}
fn batch<'a>(context: &'a SourceContext, records: &'a [ParsedRecord]) -> WriteBatch<'a> {
    WriteBatch {
        namespace: None,
        context,
        declared_host: None,
        records,
        hook_summaries: &[],
        pr_witnesses: &[],
        title: Some("Synthetic title"),
        cwd: None,
        git_branch: None,
        keep_content: true,
        observed_at: 10,
        receipt: None,
        cursor: None,
        discovery: None,
        checkpoint: None,
    }
}
fn content(sql: &Connection) -> (Option<String>, Option<String>, Option<String>) {
    sql.query_row("SELECT s.title,r.content_json,t.input_json FROM sessions s JOIN records r ON r.session_id=s.session_id JOIN tool_uses t ON t.uuid=r.uuid WHERE r.uuid='saved'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
}

// Compare every persisted non-content column independently of the production DTOs.
fn structural_snapshot(
    sql: &Connection,
    registry: &ContentRegistry,
) -> std::collections::BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let tables = sql.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |row| row.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    tables
        .into_iter()
        .map(|table| {
            let columns = sql
                .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
                .unwrap()
                .query_map([&table], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let structural = columns
                .iter()
                .filter(|column| {
                    !registry
                        .columns()
                        .any(|(owner, content)| owner == table && content == *column)
                })
                .collect::<Vec<_>>();
            let selected = structural
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(",");
            let rows = sql
                .prepare(&format!(
                    "SELECT {selected} FROM \"{table}\" ORDER BY {selected}"
                ))
                .unwrap()
                .query_map([], |row| {
                    (0..structural.len())
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

#[test]
fn unconfigured_storage_defaults_to_metrics_for_every_write_entrypoint() {
    let mut db = TempDb::empty().unwrap();
    assert_eq!(
        db.store().retention_mode().unwrap(),
        RetentionMode::MetadataOnly
    );
    assert_eq!(
        Store::open_in_memory().unwrap().retention_mode().unwrap(),
        RetentionMode::MetadataOnly
    );
    let context = context();
    let mut initial = row("saved");
    initial.canonical.message.usage = None;
    write_batch(db.store_mut(), &batch(&context, &[initial])).unwrap();
    let result = write_batch(db.store_mut(), &batch(&context, &[row("saved")])).unwrap();
    assert_eq!((result.records_new, result.records_enriched), (0, 1));
    let sql = Connection::open(db.path()).unwrap();
    assert_eq!(content(&sql), (None, None, None));
    let saved = db.store().records("retained").unwrap().remove(0);
    assert_eq!(
        saved.text_len,
        Some("Synthetic transcript".chars().count() as i64)
    );
    assert_eq!(saved.tool_use_count, Some(1));
    assert_eq!(saved.usage.unwrap().output_tokens, Some(3));
    let mut legacy = SessionMeta::new("legacy", "claude", SessionSource::Transcript);
    legacy.title = Some("Synthetic title that must not persist".into());
    db.store_mut().upsert_session(&legacy, true).unwrap();
    db.store_mut()
        .upsert_records("legacy", &[row("legacy").canonical], true)
        .unwrap();
    let reopened = Store::open(db.path()).unwrap();
    assert_eq!(
        reopened.retention_mode().unwrap(),
        RetentionMode::MetadataOnly
    );
    assert!(
        reopened
            .session("legacy")
            .unwrap()
            .unwrap()
            .meta
            .title
            .is_none()
    );
    let legacy = reopened.records("legacy").unwrap().remove(0);
    assert!(legacy.content_json.is_none());
    assert!(
        legacy
            .tool_uses
            .iter()
            .all(|tool| tool.input_json.is_none())
    );
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM settings WHERE key='content_retention'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn absent_policy_on_an_existing_database_preserves_old_content_without_acquiring_more() {
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    let context = context();
    write_batch(db.store_mut(), &batch(&context, &[row("saved")])).unwrap();
    let sql = Connection::open(db.path()).unwrap();
    let previous = content(&sql);
    assert!(previous.0.is_some() && previous.1.is_some() && previous.2.is_some());
    // Represents a database written before the persisted policy existed.
    sql.execute("DELETE FROM settings WHERE key='content_retention'", [])
        .unwrap();
    let mut reopened = Store::open(db.path()).unwrap();
    assert_eq!(
        reopened.retention_mode().unwrap(),
        RetentionMode::MetadataOnly
    );
    for store in [db.store_mut(), &mut reopened] {
        write_batch(store, &batch(&context, &[row("fresh")])).unwrap();
    }
    assert_eq!(content(&sql), previous);
    assert!(
        reopened
            .records("retained")
            .unwrap()
            .iter()
            .find(|row| row.uuid == "fresh")
            .unwrap()
            .content_json
            .is_none()
    );
    reopened
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    assert_eq!(
        Store::open(db.path()).unwrap().retention_mode().unwrap(),
        RetentionMode::FullContent
    );
}

#[test]
fn retention_policy_is_persisted_and_restricts_every_write_entrypoint() {
    let mut db = TempDb::empty().unwrap();
    assert_eq!(
        db.store().retention_mode().unwrap(),
        RetentionMode::MetadataOnly
    );
    db.store_mut()
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    let context = context();
    let rows = [row("saved")];
    write_batch(db.store_mut(), &batch(&context, &rows)).unwrap();
    let sql = Connection::open(db.path()).unwrap();
    let previous = content(&sql);
    assert!(previous.0.is_some() && previous.1.is_some() && previous.2.is_some());
    // This connection predates the mode change; writes must not use a cached mode.
    let mut earlier = Store::open(db.path()).unwrap();
    db.store_mut()
        .set_retention_mode(RetentionMode::MetadataOnly)
        .unwrap();
    assert_eq!(content(&sql), previous);
    assert_eq!(
        earlier.retention_mode().unwrap(),
        RetentionMode::MetadataOnly
    );
    let mut fresh = row("fresh");
    fresh.canonical.message.usage = None;
    fresh.canonical.message.model = None;
    write_batch(&mut earlier, &batch(&context, &[fresh])).unwrap();
    let output = write_batch(&mut earlier, &batch(&context, &[row("fresh")])).unwrap();
    assert_eq!((output.records_new, output.records_enriched), (0, 1));
    let fresh = earlier
        .records("retained")
        .unwrap()
        .into_iter()
        .find(|r| r.uuid == "fresh")
        .unwrap();
    assert!(
        fresh.content_json.is_none()
            && fresh.tool_uses.iter().all(|tool| tool.input_json.is_none())
    );
    assert_eq!(fresh.usage.unwrap().output_tokens, Some(3));
    let mut meta = SessionMeta::new("legacy", "claude", SessionSource::Transcript);
    meta.title = Some("Do not acquire".into());
    earlier.upsert_session(&meta, true).unwrap();
    earlier
        .upsert_records("legacy", &[row("legacy-row").canonical], true)
        .unwrap();
    assert!(
        earlier
            .session("legacy")
            .unwrap()
            .unwrap()
            .meta
            .title
            .is_none()
    );
    assert!(earlier.records("legacy").unwrap()[0].content_json.is_none());
    assert_eq!(content(&sql), previous);
    earlier
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    // An explicit per-call restriction remains restrictive in full-content mode.
    let rows = [row("restricted")];
    let mut request = batch(&context, &rows);
    request.keep_content = false;
    write_batch(&mut earlier, &request).unwrap();
    assert!(
        earlier
            .records("retained")
            .unwrap()
            .iter()
            .find(|r| r.uuid == "restricted")
            .unwrap()
            .content_json
            .is_none()
    );
}

fn register_owners(sql: &Connection) -> ContentRegistry {
    sql.execute_batch("CREATE TABLE test_fires(id TEXT PRIMARY KEY, excerpt TEXT, outcome TEXT, count INTEGER NOT NULL); INSERT INTO test_fires VALUES('fire','Synthetic excerpt','Synthetic answer',2); CREATE TABLE zz_judge(id TEXT PRIMARY KEY, prompt TEXT, result TEXT, tokens INTEGER); INSERT INTO zz_judge VALUES('judge','Synthetic prompt','Synthetic result',12);").unwrap();
    let mut registry = ContentRegistry::default();
    registry
        .register("test_fires", &["excerpt", "outcome"])
        .unwrap();
    registry
        .register("zz_judge", &["prompt", "result"])
        .unwrap();
    registry
}

#[test]
fn purge_content_preserves_measurements_receipts_and_original_host_files() {
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    let native = context();
    let rows = [row("saved")];
    write_batch(db.store_mut(), &batch(&native, &rows)).unwrap();
    let plugin = SourceContext {
        source: Some(SessionSource::Plugin),
        ..native.clone()
    };
    let receipt = xt_store::ingest::CaptureReceipt {
        receipt_id: "capture".into(),
        session_id: "retained".into(),
        surface: None,
        received_at: 10,
    };
    let mut request = batch(&plugin, &rows);
    request.receipt = Some(&receipt);
    write_batch(db.store_mut(), &request).unwrap();
    let sql = Connection::open(db.path()).unwrap();
    let registry = register_owners(&sql);
    let host = db.path().with_file_name("synthetic-host.jsonl");
    let host_bytes = b"{\"synthetic\":\"original host file\"}\n";
    std::fs::write(&host, host_bytes).unwrap();
    let before = db.store().records("retained").unwrap();
    let digest = coverage("retained", &rows[0]).unwrap();
    let source = db.store().record_sources("saved").unwrap();
    let receipts = db.store().capture_coverage("capture").unwrap();
    db.store_mut()
        .set_retention_mode(RetentionMode::MetadataOnly)
        .unwrap();
    let structural = structural_snapshot(&sql, &registry);
    let output = db.store_mut().purge_content(&registry).unwrap();
    assert_eq!(structural_snapshot(&sql, &registry), structural);
    assert!(output.invalidate_content);
    assert_eq!(
        output.tables.iter().map(|table| table.rows).sum::<usize>(),
        5
    );
    for (table, column) in registry.columns() {
        assert_eq!(
            sql.query_row(
                &format!("SELECT count(*) FROM {table} WHERE {column} IS NOT NULL"),
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    let mut expected = before[0].clone();
    expected.content_json = None;
    for tool in &mut expected.tool_uses {
        tool.input_json = None;
    }
    assert_eq!(db.store().records("retained").unwrap(), [expected.clone()]);
    assert!(matches_current(&digest, &expected).unwrap());
    assert_eq!(db.store().capture_coverage("capture").unwrap(), receipts);
    assert_eq!(db.store().record_sources("saved").unwrap(), source);
    assert_eq!(
        sql.query_row("SELECT count FROM test_fires", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        sql.query_row("SELECT tokens FROM zz_judge", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(std::fs::read(&host).unwrap(), host_bytes);
    let mut reopened = Store::open(db.path()).unwrap();
    assert_eq!(
        reopened.retention_mode().unwrap(),
        RetentionMode::MetadataOnly
    );
    let retry = write_batch(&mut reopened, &request).unwrap();
    assert_eq!(retry.ack_through.as_deref(), Some("saved"));
    assert_eq!((retry.records_new, retry.records_enriched), (0, 0));
    assert_eq!(reopened.records("retained").unwrap(), [expected]);
    assert!(
        reopened
            .session("retained")
            .unwrap()
            .unwrap()
            .meta
            .title
            .is_none()
    );
    assert!(
        !reopened
            .purge_content(&registry)
            .unwrap()
            .invalidate_content
    );
}

#[test]
fn purge_content_hook_and_final_commit_failure_roll_back_all_owners() {
    for final_commit in [false, true] {
        let mut db = TempDb::empty().unwrap();
        db.store_mut()
            .set_retention_mode(RetentionMode::FullContent)
            .unwrap();
        let context = context();
        write_batch(db.store_mut(), &batch(&context, &[row("saved")])).unwrap();
        let sql = Connection::open(db.path()).unwrap();
        let registry = register_owners(&sql);
        let previous = content(&sql);
        if final_commit {
            sql.execute_batch("CREATE TABLE deferred_failure(session TEXT REFERENCES sessions(session_id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER injected AFTER UPDATE ON zz_judge BEGIN INSERT INTO deferred_failure VALUES('missing'); END;").unwrap();
        } else {
            sql.execute_batch("CREATE TRIGGER injected BEFORE UPDATE ON zz_judge BEGIN SELECT RAISE(ABORT, 'synthetic purge failure'); END;").unwrap();
        }
        db.store_mut()
            .set_retention_mode(RetentionMode::MetadataOnly)
            .unwrap();
        let structural = structural_snapshot(&sql, &registry);
        assert!(db.store_mut().purge_content(&registry).is_err());
        assert_eq!(structural_snapshot(&sql, &registry), structural);
        assert_eq!(content(&sql), previous);
        for (table, column) in registry.columns() {
            assert_eq!(
                sql.query_row(
                    &format!("SELECT count(*) FROM {table} WHERE {column} IS NOT NULL"),
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                // The saved row is an assistant reply: no input, no preview.
                i64::from(table != "record_previews")
            );
        }
        let mut reopened = Store::open(db.path()).unwrap();
        assert_eq!(
            reopened.retention_mode().unwrap(),
            RetentionMode::MetadataOnly
        );
        sql.execute_batch("DROP TRIGGER injected").unwrap();
        assert!(
            reopened
                .purge_content(&registry)
                .unwrap()
                .invalidate_content
        );
        assert_eq!(content(&sql), (None, None, None));
    }
}

#[test]
fn retention_invalid_setting_and_purge_inventory_fail_closed() {
    let mut db = TempDb::empty().unwrap();
    db.store_mut()
        .set_retention_mode(RetentionMode::FullContent)
        .unwrap();
    let context = context();
    write_batch(db.store_mut(), &batch(&context, &[row("saved")])).unwrap();
    let sql = Connection::open(db.path()).unwrap();
    let before = content(&sql);
    for value in ["null", "true", "0", "{}", "\"future-mode\""] {
        sql.execute(
            "INSERT OR REPLACE INTO settings VALUES('content_retention',?1)",
            [value],
        )
        .unwrap();
        assert!(db.store().retention_mode().is_err());
        assert!(write_batch(db.store_mut(), &batch(&context, &[row("new")])).is_err());
        assert_eq!(db.store().counts().unwrap().records, 1);
        assert_eq!(content(&sql), before);
    }
    db.store_mut()
        .set_retention_mode(RetentionMode::MetadataOnly)
        .unwrap();
    for (table, columns) in [
        ("missing_table", &["content"][..]),
        ("records", &["missing_column"][..]),
        ("records", &["uuid"][..]),
        ("records", &["is_meta"][..]),
    ] {
        let mut registry = ContentRegistry::default();
        registry.register(table, columns).unwrap();
        assert!(db.store_mut().purge_content(&registry).is_err());
        assert_eq!(content(&sql), before);
    }
    let mut registry = ContentRegistry::default();
    assert!(
        registry
            .register("records;DROP TABLE sessions", &["content_json"])
            .is_err()
    );
    assert!(registry.register("records", &["content_json"]).is_err());
    assert!(registry.register("empty", &[]).is_err());
    assert_eq!(registry.columns().count(), 4);
}
