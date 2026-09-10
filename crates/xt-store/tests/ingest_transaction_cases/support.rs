use rusqlite::{Connection, types::Value as SqlValue};
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource,
    batch::SourceCursor,
    ingest::{CaptureReceipt, RecordCoverage},
};

pub fn fixture() -> Fixture {
    Fixture::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1")).unwrap()
}

pub fn record(uuid: &str) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":uuid,"type":"assistant","message":{}})).unwrap()
}

pub fn rich(uuid: &str) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":uuid,"type":"assistant","timestamp":"2026-09-01T01:00:00.000000000001Z","message":{
        "model":"synthetic-model","usage":{"input_tokens":10,"cache_read_input_tokens":0},
        "content":[{"type":"tool_use","name":"Read","input":{"synthetic":"content"}}]}})).unwrap()
}

pub fn session(id: &str) -> SessionMeta {
    SessionMeta::new(id, "claude", SessionSource::Fixture)
}
pub fn cursor(source: SessionSource, key: &str, position: i64) -> SourceCursor {
    SourceCursor {
        source,
        cursor_key: key.into(),
        position,
        updated_at: 100,
    }
}
pub fn receipt_case(session: &str, uuid: &str) -> (CaptureReceipt, Vec<RecordCoverage>) {
    (
        CaptureReceipt {
            receipt_id: "synthetic-receipt".into(),
            session_id: session.into(),
            surface: None,
            received_at: 100,
        },
        vec![RecordCoverage {
            record_uuid: uuid.into(),
            metric_field_mask: 1,
            measurement_revision: "a".repeat(64),
            digest_schema_version: 1,
        }],
    )
}
pub fn sql(db: &TempDb) -> Connection {
    Connection::open(db.path()).unwrap()
}

pub const TABLES: &[&str] = &[
    "sessions",
    "records",
    "usage",
    "tool_uses",
    "session_sources",
    "record_sources",
    "capture_receipts",
    "capture_record_coverage",
    "source_cursors",
];
pub fn snapshot(connection: &Connection) -> Vec<Vec<Vec<SqlValue>>> {
    TABLES
        .iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let count = statement.column_count();
            statement
                .query_map([], |row| (0..count).map(|n| row.get(n)).collect())
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        })
        .collect()
}
