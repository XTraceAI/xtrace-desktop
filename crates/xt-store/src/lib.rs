//! Canonical SQLite storage. All imports share the UUID write boundary here.
//!
//! Opening a store configures its connection and applies embedded migrations.
//! Callers supply the canonical conversation ID unchanged, upsert its session,
//! and write records with the same explicit `keep_content` restriction. The
//! persisted policy may restrict them further. Disabling content retention affects
//! future writes; explicit purge is a separate transaction.
//! The connection is private so consumers cannot bypass the canonical writer.

// The read rules the shared views and per-session reads both use. Each is
// written once here; `views.rs` builds `v_records`, `v_usage_records` and
// `v_response_usage` from them and `session_model.rs` reads the same text.
// Macros, not constants, because SQL text is assembled with `concat!`.

/// The work events every metric reads: a user session's non-meta record that
/// is not a `<synthetic>` row, for a record aliased `r` joined to its session
/// aliased `s`, or for the aliases given (the same alias twice for a row that
/// carries both). It is `v_records`' `WHERE` (and so `v_session_events`',
/// which adds only a stored timestamp).
macro_rules! work_record_sql {
    () => {
        work_record_sql!(r, s)
    };
    ($record:ident, $session:ident) => {
        concat!(
            stringify!($record),
            ".is_meta=0 AND ",
            stringify!($session),
            ".kind='user' AND (",
            stringify!($record),
            ".model IS NULL OR ",
            stringify!($record),
            ".model<>'<synthetic>')"
        )
    };
}

/// A common table `whitespace(chars)`: Unicode White_Space, matching Rust
/// `str::trim` used for blank input.
macro_rules! whitespace_sql {
    () => {
        "whitespace(chars) AS (
    -- Unicode White_Space, matching Rust str::trim used for blank input.
    SELECT char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)
)"
    };
}

/// Whether a usage record is a keyed Claude response: non-blank message and
/// request IDs. Needs `host`, `api_message_id`, `request_id` and
/// [`whitespace_sql`]'s `chars` in scope.
macro_rules! response_keyed_sql {
    () => {
        "host='claude' AND trim(api_message_id,chars)<>''
    AND trim(request_id,chars)<>''"
    };
}

/// Every assistant work record on the base tables, aliased `r`, `s` and `u`,
/// with its usage observation when one exists (`u.uuid` is `NULL` when none
/// does). Records with one are `v_usage_records`' rows, without that view's
/// classification joins, which never drop or repeat a record.
macro_rules! assistant_work_rows_sql {
    () => {
        concat!(
            "records r JOIN sessions s ON s.session_id=r.session_id LEFT JOIN usage u ON u.uuid=r.uuid
    WHERE r.type='assistant' AND ",
            work_record_sql!()
        )
    };
}

/// Whether the usage record aliased `t` (with `host`, `api_message_id`,
/// `request_id`, `ts`, `uuid` and `response_keyed`) is a selected response
/// (M-04): an unkeyed record is its own response; a keyed response counts
/// only its latest snapshot, judged against every stored snapshot of the same
/// response across all history, by exact instant and then UUID.
macro_rules! response_selected_sql {
    () => {
        concat!(
            "coalesce(t.response_keyed,0)=0 OR NOT EXISTS (
    -- The caller can restrict candidate timestamps through records_ts, while
    -- successors are checked across all history through records_response.
    SELECT 1 FROM ",
            assistant_work_rows_sql!(),
            "
      AND u.uuid IS NOT NULL AND s.host=t.host AND r.api_message_id=t.api_message_id AND r.request_id=t.request_id
      -- Later by exact instant, then by UUID. One spelling is one instant, so
      -- the comparator runs only when the spellings differ.
      AND (CASE WHEN r.ts IS t.ts THEN 0 ELSE xt_timestamp_cmp(r.ts,t.ts) END,r.uuid)>(0,t.uuid)
)"
        )
    };
}

pub mod batch;
pub mod child_check;
pub mod child_fact;
pub mod claude_launch;
pub mod confirmation;
pub mod creation;
pub mod human_break;
pub mod human_input;
pub mod identity_reader;
pub mod ingest;
pub mod injected;
pub mod measurement;
mod migrations;
pub mod model;
pub mod pr_link;
mod read;
pub mod record_preview;
pub mod record_text;
pub mod repeat_key;
pub mod retention;
mod server_settings;
pub mod session_list;
pub mod session_model;
mod task_notification;
pub mod timestamp;
pub mod tool_sent;
pub mod tool_use;
pub mod typing_speed;
pub mod views;
mod write;

pub use model::{
    CanonicalMessage, CanonicalRecord, Host, SessionMeta, SessionSource, SurfaceEvidence, Usage,
    WriteStats,
};
pub use read::{StoreCounts, StoredRecord, StoredSession, StoredToolUse};

use rusqlite::{Connection, OpenFlags};
use std::{path::Path, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Stored JSON could not be decoded: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid canonical input: {0}")]
    InvalidInput(&'static str),
    #[error("The database migration history is incompatible with this build")]
    IncompatibleSchema,
    /// The database applied migrations this build does not have under the
    /// same numbers. The message names what differs.
    #[error("The database migration history does not match this build: {0}")]
    MigrationHistory(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One SQLite connection. Share ownership through the application's writer;
/// independent readers can open the file without blocking committed WAL reads.
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Open a file-backed database. The parent directory must already exist.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        // URI interpretation is deliberately disabled: a supplied path cannot
        // silently switch this file-backed API to a shared in-memory database.
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(path, flags)?;
        Self::configure(connection, true)
    }

    /// Single-connection test storage. SQLite keeps its MEMORY journal mode.
    pub fn open_in_memory() -> Result<Self> {
        Self::configure(Connection::open_in_memory()?, false)
    }

    fn configure(connection: Connection, file_backed: bool) -> Result<Self> {
        Self::prepare_connection(&connection, file_backed)?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    /// Connection settings every store connection uses before migrating. The
    /// migration runner reuses them for its in-memory reference database.
    fn prepare_connection(connection: &Connection, file_backed: bool) -> Result<()> {
        timestamp::register_sqlite(connection)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        if file_backed {
            let mode: String =
                connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") {
                return Err(Error::InvalidInput("file database did not enable WAL"));
            }
        }
        // WAL NORMAL can lose acknowledged commits after an OS crash. A file
        // writer must sync the WAL before reporting a successful transaction.
        connection.pragma_update(
            None,
            "synchronous",
            if file_backed { "FULL" } else { "NORMAL" },
        )?;
        #[cfg(target_os = "macos")]
        if file_backed {
            connection.pragma_update(None, "fullfsync", "ON")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Store;

    #[test]
    fn store_connections_register_precise_response_timestamp_comparison() {
        let directory = tempfile::TempDir::new().unwrap();
        for store in [
            Store::open_in_memory().unwrap(),
            Store::open(directory.path().join("timestamps.sqlite")).unwrap(),
        ] {
            let comparison: i64 = store.connection.query_row(
                "SELECT xt_timestamp_cmp('2026-09-07T12:00:00.0009Z','2026-09-07T12:00:00.0001Z')",
                [], |row| row.get(0),
            ).unwrap();
            assert_eq!(comparison, 1);
            store
                .connection
                .prepare("SELECT * FROM v_response_usage LIMIT 0")
                .unwrap();
            assert!(
                store
                    .connection
                    .query_row("SELECT xt_timestamp_cmp('invalid',NULL)", [], |row| row
                        .get::<_, i64>(0))
                    .is_err()
            );
        }
    }

    #[test]
    fn connection_settings_are_explicit_for_file_and_memory() {
        let directory = tempfile::TempDir::new().unwrap();
        let file = Store::open(directory.path().join("settings.sqlite")).unwrap();
        let memory = Store::open_in_memory().unwrap();
        let reopened = Store::open(directory.path().join("settings.sqlite")).unwrap();
        for (store, journal, synchronous) in [
            (&file, "wal", 2),
            (&reopened, "wal", 2),
            (&memory, "memory", 1),
        ] {
            for (pragma, expected) in [
                ("foreign_keys", 1),
                ("synchronous", synchronous),
                ("busy_timeout", 5000),
            ] {
                let value: i64 = store
                    .connection
                    .pragma_query_value(None, pragma, |row| row.get(0))
                    .unwrap();
                assert_eq!(value, expected, "{pragma}");
            }
            let mode: String = store
                .connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            assert_eq!(mode, journal);
            #[cfg(target_os = "macos")]
            {
                let fullfsync: i64 = store
                    .connection
                    .pragma_query_value(None, "fullfsync", |row| row.get(0))
                    .unwrap();
                assert_eq!(fullfsync, i64::from(journal == "wal"));
            }
        }
    }
}
