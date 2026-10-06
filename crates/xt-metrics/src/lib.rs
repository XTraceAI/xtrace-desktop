//! Read-only metrics over canonical work records and explicit event windows.
mod cost;
mod counts;
mod coverage;
mod daily;
mod environment;
mod favorite;
mod hands_off;
mod human;
mod human_hours;
mod pr_analytics;
mod pr_effort;
mod prices;
mod repeats;
mod session;
mod session_hands_off;
mod session_stretches;
mod sessions;
mod span_detail;
mod spans;
mod stats;
mod sweep;
mod tokens;
mod untimed;
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
pub use daily::{DayConcurrency, DayHandsOff};
pub use environment::{
    DayCalls, EnvUsage, HostEnvironment, HostInventory, IdentityCalls, Inventory, InventoryJoin,
    SurfaceCalls, ToolIdentity, UnresolvedCalls, UnresolvedReason,
};
pub use favorite::{FavoriteComparison, FavoriteModel, FavoriteUnknown};
pub use hands_off::{ExcludedSurface, HandsOff};
pub use human::{DayHuman, HumanTime};
pub use human_hours::{BreakLength, DayHumanHours, HumanHours, HumanHoursPeriods, HumanStretch};
pub use pr_analytics::{
    LinkEvidence, PrAnalyticsReport, PrEligibility, PrHandsOff, PrMedian, PrRow, PrSummary,
    PrTokens, TokenMedian, TokenWithheld, TypeSummary,
};
pub use pr_effort::{
    AssignmentEffort, DayEffort, EffortAssignment, EffortTotals, MergedPrTile, ModelDayEffort,
    PrEffortReport, PrFreshness, PrFreshnessSummary, PrMarker,
};
pub use prices::{COST_BASIS, PriceCatalog};
pub use repeats::{
    DEFAULT_ACTIVE_MS, DEFAULT_REPEATS, RepeatDensity, RepeatGroup, RepeatThresholds,
    SessionRepeats, StretchRepeats, UnknownRepeats,
};
pub use session::{MAX_SESSIONS, SUB_SESSION_WALK, SessionWindow, SubSessions};
pub use session_hands_off::SessionHandsOff;
pub use session_stretches::{SessionStretch, SessionStretches, ToolBlock};
pub use sessions::{DaySessions, SessionsPerDay};
pub use span_detail::{
    AutomaticText, MAX_PROMPT_CHARS, PromptText, SpanAutomatic, SpanDetail, SpanPrompt, SpanTool,
    notification_summary, prompt_excerpt,
};
pub use spans::{ActiveSpan, ActiveSpanReport, DayActive};
pub use stats::Delta;
pub use sweep::Concurrency;
pub use tokens::{
    DayTokens, HostTokens, ModelTokens, SurfaceTokens, TokenCounters, TokenReport, TokenSummary,
};
pub use untimed::{UntimedHistory, UntimedSurface};
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
    #[error("Break length must be positive whole minutes")]
    InvalidBreakLength,
    #[error("Duplicate discovery health context for a host/surface")]
    InvalidCoverageContext,
    #[error("Duplicate supplied inventory for a host")]
    DuplicateInventoryHost,
    #[error("Too many sessions requested for one measurement read")]
    TooManySessions,
    #[error("Repeat thresholds must be positive active milliseconds and repeats")]
    InvalidRepeatThresholds,
    #[error("Invalid price catalog")]
    InvalidPriceCatalog,
    #[error(transparent)]
    Time(#[from] jiff::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Store(#[from] xt_store::Error),
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
        connection.prepare(&format!("{} LIMIT 0", human_hours::QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", hands_off::QUERY))?;
        for query in [
            coverage::EVENTS_QUERY,
            coverage::USAGE_QUERY,
            coverage::CAPTURE_QUERY,
        ] {
            connection.prepare(&format!("{query} LIMIT 0"))?;
        }
        connection.prepare(&format!("{} LIMIT 0", sessions::QUERY))?;
        for query in [
            session::EXISTS_QUERY,
            session::EVENTS_QUERY,
            xt_store::session_list::CONTEXT_SQL,
            xt_store::session_list::PARENT_SQL,
        ] {
            connection.prepare(&format!("{query} LIMIT 0"))?;
        }
        connection.prepare(&format!("{} LIMIT 0", session_stretches::TOOL_QUERY))?;
        for query in [
            span_detail::EXISTS_QUERY,
            span_detail::TOOL_QUERY,
            span_detail::STATED_QUERY,
            span_detail::PROMPT_QUERY,
            span_detail::AUTOMATIC_QUERY,
        ] {
            connection.prepare(&format!("{query} LIMIT 0"))?;
        }
        connection.prepare(&format!("{} LIMIT 0", repeats::BLOCK_QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", favorite::OUTPUT_QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", favorite::TURN_QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", cost::QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", untimed::QUERY))?;
        connection.prepare(&format!("{} LIMIT 0", pr_effort::LINK_QUERY))?;
        for query in [
            environment::RECORD_QUERY,
            environment::RECORD_UNTIMED_QUERY,
            environment::STRUCTURAL_QUERY,
        ] {
            connection.prepare(&format!("{query} LIMIT 0"))?;
        }
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
