//! How much indexed history states no timestamp at all.
//!
//! Every date-based measurement in this crate reads `v_session_events`, which
//! is `v_records` with the timestamp requirement applied. A record the source
//! stored without a timestamp therefore belongs to no window: it is not a zero
//! on some day, it is absent from every day. That absence is silent in every
//! other report, so this one states it out loud.
//!
//! The projection is deliberately `v_records`, the eligible canonical metric
//! records *before* the timestamp requirement. It carries the exclusions the
//! rest of the crate applies — meta records, judge sessions and `<synthetic>`
//! model rows are already gone — so a row counted here is a row that every
//! other metric would have measured if it had stated when it happened, and a
//! row excluded for one of those other reasons is never reported as missing
//! only a timestamp. `records` has one row per canonical UUID and
//! `native_record_copies` is not joined, so a record repeated in several
//! native files is counted once, exactly as the work totals count it.
//!
//! The count has no window, because none of these rows could be placed in one.
//! It describes all indexed history and is independent of any selected range
//! and of any filter a consumer's own table applies. It is grouped by host and
//! the raw source surface, preserved verbatim with `None` left unknown, so a
//! reader can see where the untimed history sits without any record, identifier
//! or content leaving this summary.
//!
//! Nothing here infers, repairs or substitutes a timestamp, and a count above
//! zero is not a statement that a host is broken: a source is free to record
//! history without stating when it happened.

use crate::{Error, MetricsDb, Result};
use serde::Serialize;
use std::collections::BTreeMap;

/// Eligible canonical records that state no timestamp, by host and raw surface.
pub(crate) const QUERY: &str =
    "SELECT host,surface,count(*) FROM v_records WHERE ts_ms IS NULL GROUP BY host,surface";

/// Untimed records observed at one host and one raw source surface. The surface
/// is the value the source stated, preserved verbatim; `None` stays unknown and
/// is never aliased to an empty string or to the host name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UntimedSurface {
    pub host: String,
    pub surface: Option<String>,
    pub records: u64,
}

/// All indexed history that no date-based measurement can hold. `records` is
/// the sum of `by_surface`, which carries one row per observed host/surface
/// pair in host then surface order and no row for a pair with no such record.
/// An empty report is a measured zero: nothing indexed is missing a timestamp.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct UntimedHistory {
    pub records: u64,
    pub by_surface: Vec<UntimedSurface>,
}

impl MetricsDb {
    /// Count the eligible canonical records that state no timestamp, grouped by
    /// host and raw surface. Read-only, unwindowed and bounded by the number of
    /// host/surface pairs the history holds; no record row is returned.
    pub fn untimed_history(&self) -> Result<UntimedHistory> {
        let mut by_surface = BTreeMap::<(String, Option<String>), u64>::new();
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let count: i64 = row.get(2)?;
            let count = u64::try_from(count)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, count))?;
            // Grouped by the same pair, so one key is written once; summing
            // keeps the total correct if SQLite ever splits a group.
            let total = by_surface.entry((row.get(0)?, row.get(1)?)).or_default();
            *total = total.checked_add(count).ok_or(Error::CounterOverflow)?;
        }
        let mut records: u64 = 0;
        for count in by_surface.values() {
            records = records.checked_add(*count).ok_or(Error::CounterOverflow)?;
        }
        Ok(UntimedHistory {
            records,
            by_surface: by_surface
                .into_iter()
                .map(|((host, surface), records)| UntimedSurface {
                    host,
                    surface,
                    records,
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untimed_candidates_use_the_record_timestamp_index() {
        let db = xt_fixtures::TempDb::empty().unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let plan: Vec<String> = metrics
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {QUERY}"))
            .unwrap()
            .query_map([], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            plan.iter().any(|line| line.contains("records_ts")),
            "{plan:?}"
        );
    }
}
