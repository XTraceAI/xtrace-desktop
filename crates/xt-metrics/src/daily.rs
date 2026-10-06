//! Day-by-day series of existing window metrics over the selected window's
//! local days. Nothing here defines a metric.
//!
//! Concurrency (M-06) measures each day as a shorter window. Hands-off (M-09)
//! is measured once over the whole window, exactly as its range number is:
//! one load, one timestamp-health judgement per surface, one fold into
//! stretches. Each stretch then belongs to the local day of the message that
//! starts it, so a stretch over midnight stays whole on its start day, and a
//! surface the range leaves out is left out of every day.
use crate::{MetricsDb, Result, Window, hands_off, stats};
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
            let ends: Vec<_> = days
                .iter()
                .map(|day| InstantKey::from_millisecond(day.window.end_ms()))
                .collect();
            let last = days.len().saturating_sub(1);
            let day_of =
                |instant: &InstantKey| ends.partition_point(|end| end <= instant).min(last);
            let mut durations = vec![Vec::new(); days.len()];
            let mut unknown = vec![false; days.len()];
            let surfaces = hands_off::load(&db.connection, window)?;
            for ((host, surface), sessions) in &surfaces {
                if hands_off::health(host, surface, sessions).is_some() {
                    continue;
                }
                for events in sessions.values() {
                    match hands_off::collect(events)? {
                        Some(stretches) => {
                            for stretch in stretches {
                                durations[day_of(stretch.start_instant())]
                                    .push(stretch.duration_ms());
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
        })
    }
}
