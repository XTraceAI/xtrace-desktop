use crate::support::*;
use rusqlite::{Connection, params};
use serde_json::json;
use tempfile::TempDir;
use xt_store::Store;

fn legacy_database(path: &std::path::Path) -> Connection {
    let connection = connection(path);
    connection
        .execute_batch(include_str!("../../migrations/0001_canonical.sql"))
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
        INSERT INTO schema_version VALUES(1,'base-schema-fixture');",
        )
        .unwrap();
    let fixture = fixture("F1");
    let session = &fixture.sessions()[0];
    let record = session
        .records
        .iter()
        .find(|r| {
            r.message
                .content
                .as_ref()
                .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_use"))
        })
        .unwrap();
    let metadata = &session.metadata;
    connection.execute("INSERT INTO sessions(session_id,host,source_platform,source,title,surface,native_session_id,started_at_ms)
        VALUES(?1,?2,?3,?4,'Synthetic old title',?5,?6,?7)", params![metadata.session_id,metadata.host,metadata.source_platform,metadata.source,metadata.surface,metadata.native_session_id,metadata.started_at_ms]).unwrap();
    connection.execute("INSERT INTO records(uuid,session_id,type,ts,ts_ms,api_message_id,request_id,is_meta,is_sidechain,role,model,content_json)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", params![record.uuid,metadata.session_id,record.record_type,record.timestamp,
            chrono::DateTime::parse_from_rfc3339(record.timestamp.as_ref().unwrap()).unwrap().timestamp_millis(),record.message.id,record.request_id,record.is_meta,record.is_sidechain,record.message.role,record.message.model,serde_json::to_string(&record.message.content).unwrap()]).unwrap();
    connection.execute("INSERT INTO usage(uuid,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,cache_creation_5m,cache_creation_1h,service_tier)
        VALUES(?1,100,10,0,20,8,12,'fixture-tier')", [&record.uuid]).unwrap();
    // IDs, arbitrary valid names and nullable inputs are all legal in 0001.
    connection
        .execute(
            "INSERT INTO tool_uses(id,uuid,block_index,name,input_json) VALUES(41,?1,0,'Read',?2)",
            params![record.uuid, json!({"path":"old-fixture.txt"}).to_string()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO tool_uses(id,uuid,block_index,name,input_json) VALUES(99,?1,2,'',NULL)",
            [&record.uuid],
        )
        .unwrap();
    connection
}

#[test]
fn upgrade_real_base_schema() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("legacy.sqlite");
    let old = legacy_database(&path);
    let baseline = [
        "sessions",
        "records",
        "usage",
        "tool_uses",
        "meta",
        "schema_version",
    ]
    .map(|table| {
        let columns = columns(&old, table);
        (table, rows(&old, table, &columns), columns)
    });
    drop(old);
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        let current = connection(&path);
        for (table, expected, old_columns) in &baseline {
            let actual = rows(&current, table, old_columns);
            if *table == "schema_version" {
                assert_eq!(&actual[..1], expected);
            } else {
                assert_eq!(&actual, expected, "0001 data changed in {table}");
            }
            let names = columns(&current, table);
            assert_eq!(
                names
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                names.len()
            );
        }
        assert_eq!(scalar(&current, "SELECT count(*) FROM schema_version"), 2);
        assert_eq!(scalar(&current, "SELECT record_count FROM sessions"), 1);
        assert_eq!(
            scalar(&current, "SELECT count(*) FROM pragma_foreign_key_check"),
            0
        );
        assert_eq!(
            current
                .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        let session_id: String = current
            .query_row("SELECT session_id FROM sessions", [], |r| r.get(0))
            .unwrap();
        let stored = store.records(&session_id).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].tool_uses.iter().map(|t| t.id).collect::<Vec<_>>(),
            [41, 99]
        );
        assert_eq!(
            stored[0].usage.as_ref().unwrap().cache_read_input_tokens,
            Some(0)
        );
        assert_eq!(
            stored[0]
                .usage
                .as_ref()
                .unwrap()
                .cache_creation
                .as_ref()
                .unwrap()
                .ephemeral_1h_input_tokens,
            Some(12)
        );
    }
}

#[test]
fn failed_upgrade_preserves_original_schema_and_tool_rows() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("invalid-legacy.sqlite");
    let old = legacy_database(&path);
    old.pragma_update(None, "foreign_keys", "OFF").unwrap();
    old.execute(
        "INSERT INTO tool_uses(id,uuid,block_index,name) VALUES(100,'missing',0,'orphan')",
        [],
    )
    .unwrap();
    let original_columns = columns(&old, "tool_uses");
    let original_rows = rows(&old, "tool_uses", &original_columns);
    assert!(Store::open(&path).is_err());
    assert_eq!(columns(&old, "tool_uses"), original_columns);
    assert_eq!(rows(&old, "tool_uses", &original_columns), original_rows);
    assert!(!columns(&old, "sessions").contains(&"record_count".into()));
    assert_eq!(scalar(&old, "SELECT count(*) FROM schema_version"), 1);
    assert_eq!(
        scalar(
            &old,
            "SELECT count(*) FROM sqlite_master WHERE name IN ('capture_receipts','tool_uses_ingest')"
        ),
        0
    );
}
