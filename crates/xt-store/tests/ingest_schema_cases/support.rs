use rusqlite::{Connection, types::Value as SqlValue};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_store::{
    CanonicalRecord,
    ingest::{CaptureReceipt, RecordCoverage},
};

pub fn fixture(id: &str) -> Fixture {
    Fixture::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(id),
    )
    .unwrap()
}

#[derive(Deserialize)]
pub struct ReceiptCase {
    pub records: Vec<CanonicalRecord>,
    pub receipt: CaptureReceipt,
    pub coverage: Vec<RecordCoverage>,
    pub expected: Value,
}

pub fn receipt_db(keep_content: bool) -> (TempDb, ReceiptCase) {
    let fixture = fixture("F18");
    let case: ReceiptCase = serde_json::from_value(fixture.snapshots()["schema"].clone()).unwrap();
    let mut db = TempDb::empty().unwrap();
    let metadata = &fixture.sessions()[0].metadata;
    db.store_mut()
        .upsert_session(metadata, keep_content)
        .unwrap();
    db.store_mut()
        .upsert_records(&metadata.session_id, &case.records, keep_content)
        .unwrap();
    (db, case)
}

pub fn connection(path: &std::path::Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    connection
}

pub fn scalar(connection: &Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

pub fn columns(connection: &Connection, table: &str) -> Vec<String> {
    connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

pub fn rows(connection: &Connection, table: &str, columns: &[String]) -> Vec<Vec<SqlValue>> {
    connection
        .prepare(&format!(
            "SELECT {} FROM {table} ORDER BY 1",
            columns.join(",")
        ))
        .unwrap()
        .query_map([], |row| {
            (0..columns.len()).map(|index| row.get(index)).collect()
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
