//! Read-only metrics over canonical work records and explicit event windows.
mod counts;
mod hands_off;
mod human;
mod spans;
mod stats;
mod sweep;
mod tokens;
mod window;
pub use counts::{Counts, TypingRate};
pub use hands_off::{ExcludedSurface, HandsOff};
pub use human::HumanTime;
pub use spans::{ActiveSpan, ActiveSpanReport};
pub use sweep::Concurrency;
pub use tokens::{
    DayTokens, HostTokens, ModelTokens, SurfaceTokens, TokenCounters, TokenReport, TokenSummary,
};
pub use window::{DayBucket, Window};

use rusqlite::{Connection, OpenFlags};
use std::{path::Path, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid metric window")]
    InvalidWindow,
    #[error("Metric counter overflow")]
    CounterOverflow,
    #[error("Typing rate must be positive characters per minute")]
    InvalidTypingRate,
    #[error(transparent)]
    Time(#[from] jiff::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

/// Opens an existing app-owned database, never creates or migrates one.
/// The canonical writer installs the shared views after its migrations.
pub struct MetricsDb {
    connection: Connection,
}
impl MetricsDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        xt_store::timestamp::register_sqlite(&connection)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "query_only", true)?;
        // Fail at open if the writer has not installed the current projection.
        connection.prepare("SELECT uuid,session_id,ts_ms FROM v_session_events LIMIT 0")?;
        connection.prepare(&format!("{} LIMIT 0", tokens::QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", spans::QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", counts::QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", human::QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", hands_off::QUERY))?;
        Ok(Self { connection })
    }

    /// Global work-event count: copied contexts do not multiply canonical UUIDs.
    /// Missing timestamps cannot be assigned to a window.
    pub fn event_count(&self, window: Window) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT count(*) FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2",
            [window.start_ms(), window.end_ms()],
            |row| {
                let count: i64 = row.get(0)?;
                u64::try_from(count).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, count))
            },
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_contract_connection_rejects_writes() {
        let db = xt_fixtures::TempDb::empty().unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        assert!(
            metrics
                .connection
                .execute("DELETE FROM records", [])
                .is_err()
        );
        assert!(
            metrics
                .connection
                .execute_batch("CREATE TABLE forbidden(x)")
                .is_err()
        );
        assert_eq!(db.store().counts().unwrap().records, 0);
    }
}
