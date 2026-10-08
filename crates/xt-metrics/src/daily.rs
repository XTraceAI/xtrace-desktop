//! Day-by-day series of existing window metrics over the selected window's
//! local days. Nothing here defines a metric.
//!
//! Concurrency (M-06) measures each day as a shorter window. Hands-off (M-09)
//! is measured once over the whole window, exactly as its range number is:
//! one load, one timestamp-health judgement per surface, one fold into
//! stretches. Each stretch then belongs to the local day of the message that
//! starts it, so a stretch over midnight stays whole on its start day, and a
//! surface the range leaves out is left out of every day.
use crate::{DayBucket, HandsOff, MetricsDb, Result, Window, hands_off, stats};
use jiff::tz::TimeZone;
use serde::Serialize;
use xt_store::timestamp::InstantKey;

/// M-06 over one local day.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DayConcurrency {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub max: Option<u32>,
    pub mean: Option<f64>,
}

/// M-09 over one local day.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DayHandsOff {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub n: Option<u64>,
    pub median_min: Option<f64>,
    pub p90_min: Option<f64>,
}

impl MetricsDb {
    /// [`MetricsDb::concurrency`] for every local day of the window.
    pub fn concurrency_by_day(
        &self,
        window: Window,
        zone: TimeZone,
    ) -> Result<Vec<DayConcurrency>> {
        self.read_snapshot(|db| {
            window
                .local_days(zone)?
                .into_iter()
                .map(|day| {
                    let measured = db.concurrency(day.window)?;
                    Ok(DayConcurrency {
                        date: day.date.to_string(),
                        start_ms: day.window.start_ms(),
                        end_ms: day.window.end_ms(),
                        max: measured.max,
                        mean: measured.mean,
                    })
                })
                .collect()
        })
    }

    /// [`MetricsDb::hands_off`]'s stretches, grouped by the local day their
    /// first message is on. A session whose stretches cannot be measured (an
    /// unknown classification) makes every day from its first to its last
    /// event in the window unknown: such a message could start a stretch on
    /// any of them.
    pub fn hands_off_by_day(&self, window: Window, zone: TimeZone) -> Result<Vec<DayHandsOff>> {
        self.read_snapshot(|db| {
            let days = window.local_days(zone)?;
            let surfaces = hands_off::load(&db.connection, window)?;
            hands_off_days(&days, &surfaces)
        })
    }

    /// [`MetricsDb::hands_off`] and [`MetricsDb::hands_off_by_day`] over one
    /// window in one snapshot, from one load of its events. Each report is
    /// still worked out by its own method's code, so each equals that method.
    pub fn hands_off_with_days(
        &self,
        window: Window,
        zone: TimeZone,
    ) -> Result<(HandsOff, Vec<DayHandsOff>)> {
        self.read_snapshot(|db| {
            let days = window.local_days(zone)?;
            let surfaces = hands_off::load(&db.connection, window)?;
            Ok((
                hands_off::summary(&surfaces)?,
                hands_off_days(&days, &surfaces)?,
            ))
        })
    }
}

/// The days' reports from the window's loaded events:
/// [`MetricsDb::hands_off_by_day`]'s body.
fn hands_off_days(
    days: &[DayBucket],
    surfaces: &hands_off::SurfaceSessions,
) -> Result<Vec<DayHandsOff>> {
    let ends: Vec<_> = days
        .iter()
        .map(|day| InstantKey::from_millisecond(day.window.end_ms()))
        .collect();
    let last = days.len().saturating_sub(1);
    let day_of = |instant: &InstantKey| ends.partition_point(|end| end <= instant).min(last);
    let mut durations = vec![Vec::new(); days.len()];
    let mut unknown = vec![false; days.len()];
    for ((host, surface), sessions) in surfaces {
        if hands_off::health(host, surface, sessions).is_some() {
            continue;
        }
        for events in sessions.values() {
            match hands_off::collect(events)? {
                Some(stretches) => {
                    for stretch in stretches {
                        durations[day_of(stretch.start_instant())].push(stretch.duration_ms());
                    }
                }
                None => {
                    if let (Some(first), Some(end)) = (events.first(), events.last()) {
                        unknown[day_of(first.instant())..=day_of(end.instant())].fill(true);
                    }
                }
            }
        }
    }
    Ok(days
        .iter()
        .zip(durations)
        .zip(unknown)
        .map(|((day, mut durations), unknown)| {
            let percentiles = (!unknown)
                .then(|| stats::median_p90(&mut durations))
                .flatten();
            DayHandsOff {
                date: day.date.to_string(),
                start_ms: day.window.start_ms(),
                end_ms: day.window.end_ms(),
                n: (!unknown).then_some(durations.len() as u64),
                median_min: percentiles.map(|(median, _)| median / 60000.0),
                p90_min: percentiles.map(|(_, p90)| p90 as f64 / 60000.0),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    fn fixture() -> xt_fixtures::TempDb {
        xt_fixtures::Fixture::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"),
        )
        .unwrap()
        .build_db(false)
        .unwrap()
    }
    fn window() -> Window {
        Window::new(1788220800000, 1788825600000).unwrap()
    }
    fn combined(metrics: &MetricsDb) -> (HandsOff, Vec<DayHandsOff>) {
        metrics
            .hands_off_with_days(window(), TimeZone::UTC)
            .unwrap()
    }
    fn standalone(metrics: &MetricsDb) -> (HandsOff, Vec<DayHandsOff>) {
        (
            metrics.hands_off(window()).unwrap(),
            metrics.hands_off_by_day(window(), TimeZone::UTC).unwrap(),
        )
    }
    /// Only the hands-off load reads these reports' events, so each call of
    /// `on_row` is one row that load read.
    fn watch_event_rows(
        metrics: &MetricsDb,
        path: &std::path::Path,
        on_row: impl Fn() -> rusqlite::Result<()> + Send + 'static,
    ) {
        Connection::open(path).unwrap().execute_batch("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT session_id,host,surface,ts_ms,ts,event_row(uuid) AS uuid,type,is_human,text_len,human_is_eligible,human_text_len,human_excluded,role,tool_use_count,confirmed_automated_input FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
        metrics
            .connection
            .create_scalar_function("event_row", 1, FunctionFlags::SQLITE_UTF8, move |context| {
                on_row()?;
                context.get::<String>(0)
            })
            .unwrap();
    }

    #[test]
    fn hands_off_with_days_reads_the_events_once_for_both_reports() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let rows = Arc::new(AtomicUsize::new(0));
        let counted = rows.clone();
        watch_event_rows(&metrics, db.path(), move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        metrics.hands_off(window()).unwrap();
        let one_read = rows.swap(0, Ordering::SeqCst);
        assert!(one_read > 0);
        let shared = combined(&metrics);
        assert_eq!(rows.swap(0, Ordering::SeqCst), one_read);
        // Standalone, the range and its days each read the events.
        assert_eq!(standalone(&metrics), shared);
        assert_eq!(rows.load(Ordering::SeqCst), 2 * one_read);
        assert_eq!(shared.0.n, Some(5));
    }

    #[test]
    fn hands_off_with_days_holds_through_a_writer_commit_then_rereads() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let armed = Arc::new(AtomicBool::new(false));
        let trigger = armed.clone();
        let path = db.path().to_owned();
        watch_event_rows(&metrics, db.path(), move || {
            if trigger.swap(false, Ordering::SeqCst) {
                Connection::open(&path)?.execute_batch("UPDATE records SET is_human=0;")?;
            }
            Ok(())
        });
        let before = standalone(&metrics);
        assert_eq!(before.0.n, Some(5));
        armed.store(true, Ordering::SeqCst);
        assert_eq!(combined(&metrics), before);
        assert!(!armed.load(Ordering::SeqCst));
        assert!(metrics.connection.is_autocommit());
        // The next call is a fresh read that sees the commit.
        let after = combined(&metrics);
        assert_eq!(after, standalone(&metrics));
        assert_eq!(after.0.n, Some(0));
    }

    #[test]
    fn hands_off_with_days_leaves_a_caller_snapshot_open() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let transaction = metrics.connection.unchecked_transaction().unwrap();
        let before = combined(&metrics);
        Connection::open(db.path())
            .unwrap()
            .execute_batch("UPDATE records SET is_human=0;")
            .unwrap();
        assert_eq!(combined(&metrics), before);
        assert!(!metrics.connection.is_autocommit());
        transaction.commit().unwrap();
        let after = combined(&metrics);
        assert_eq!(after.0.n, Some(0));
        assert_ne!(after, before);
    }
}
