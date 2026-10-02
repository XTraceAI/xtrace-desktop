//! Read-only metrics over canonical work records and explicit event windows.
mod cost;
mod counts;
mod coverage;
mod favorite;
mod hands_off;
mod human;
mod prices;
mod sessions;
mod spans;
mod stats;
mod sweep;
mod tokens;
mod window;
pub use cost::{
    CostReport, CostSummary, DayCost, HostCost, ModelCost, SurfaceCost, UnpricedCost,
    UnpricedReason,
};
pub use counts::{Counts, TypingRate};
pub use coverage::{
    CaptureGap, Coverage, CoverageSurface, DiscoveryHealth, HostUsageCoverage, InventoryState,
    SurfaceCapture, SurfaceUsageCoverage, UsageCoverage, UsageCoverageSummary, UsageGap, UsageGate,
};
pub use favorite::{FavoriteComparison, FavoriteModel, FavoriteUnknown};
pub use hands_off::{ExcludedSurface, HandsOff};
pub use human::HumanTime;
pub use prices::{COST_BASIS, PriceCatalog};
pub use sessions::{DaySessions, SessionsPerDay};
pub use spans::{ActiveSpan, ActiveSpanReport};
pub use stats::Delta;
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
    #[error("Duplicate discovery health context for a host/surface")]
    InvalidCoverageContext,
    #[error("Invalid price catalog")]
    InvalidPriceCatalog,
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
        for query in [
            coverage::EVENTS_QUERY,
            coverage::USAGE_QUERY,
            coverage::CAPTURE_QUERY,
        ] {
            connection.prepare(&format!("{query} LIMIT 0"))?;
        }
        connection.prepare(&format!("{} LIMIT 0", sessions::QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", favorite::OUTPUT_QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", favorite::TURN_QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", cost::QUERY))?;
        Ok(Self { connection })
    }

    /// Compose read-only reports against one snapshot. Nested metric methods
    /// reuse it; an existing caller transaction remains owned by that caller.
    pub fn read_snapshot<T>(&self, read: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let result = read(self)?;
        if let Some(snapshot) = snapshot {
            snapshot.commit()?;
        }
        Ok(result)
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
