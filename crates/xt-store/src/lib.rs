//! Canonical SQLite storage. All imports share the UUID write boundary here.
//!
//! Opening a store configures its connection and applies embedded migrations.
//! Callers supply the canonical conversation ID unchanged, upsert its session,
//! and write records with the same explicit `keep_content` restriction. The
//! persisted policy may restrict them further. Disabling content retention affects
//! future writes; explicit purge is a separate transaction.
//! The connection is private so consumers cannot bypass the canonical writer.

pub mod batch;
pub mod ingest;
pub mod measurement;
mod migrations;
pub mod model;
mod native_order;
mod read;
pub mod retention;
mod server_settings;
pub mod session_list;
pub mod timestamp;
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
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::Store;

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
