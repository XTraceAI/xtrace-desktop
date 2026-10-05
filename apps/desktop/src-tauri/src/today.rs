//! The tray's "today": local midnight up to one captured instant, read in one
//! snapshot through the existing M-04 token and cost reports and the M-05
//! spans. Nothing here defines a metric; it only states which of their answers
//! are a measured zero, unknown, or a partial.
use crate::{
    dashboard::checked_value,
    dto::{DashboardUnavailable, MetricClock},
    state::StateError,
};
use jiff::{Timestamp, tz::TimeZone};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xt_metrics::{MetricsDb, PriceCatalog, Window};

/// What the output counter says for today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TodayOutputState {
    /// Every selected response carries an output counter.
    Recorded,
    /// No response is recorded today: nothing to count, not an unknown count.
    NoneRecorded,
    /// At least one selected response lacks its output counter, so the sum is unknown.
    Incomplete,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TodayOutput {
    pub state: TodayOutputState,
    /// Present only when `state` is `recorded`.
    #[ts(type = "number | null")]
    pub output_tokens: Option<u64>,
    #[ts(type = "number")]
    pub selected_responses: u64,
    /// Distinct sessions with a selected response.
    #[ts(type = "number")]
    pub sessions: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TodayCostState {
    /// Every selected response was priced: `total_usd` is the total.
    Priced,
    /// Some were priced: `priced_subtotal_usd` covers only those, and is not a total.
    Partial,
    /// Responses were selected and none could be priced.
    Unpriced,
    /// No response is recorded today.
    NoneRecorded,
}

/// API-equivalent cost under the bundled (or fixture) price catalog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TodayCost {
    pub state: TodayCostState,
    pub total_usd: Option<f64>,
    pub priced_subtotal_usd: f64,
    #[ts(type = "number")]
    pub selected_observations: u64,
    #[ts(type = "number")]
    pub priced_observations: u64,
    pub price_version: String,
    pub basis: String,
}

/// M-05 agent time from today's events only: a span begun before midnight
/// contributes from its first event after midnight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TodayAgent {
    #[ts(type = "number")]
    pub active_ms: u64,
    /// Distinct sessions with at least one event today.
    #[ts(type = "number")]
    pub sessions: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TodaySummary {
    /// The local calendar date of `observed_ms` in `timezone`.
    pub date: String,
    /// An IANA name, `UTC` for fixtures, or `system-local` when the OS names none.
    pub timezone: String,
    pub clock: MetricClock,
    /// Local midnight: the inclusive start.
    #[ts(type = "number")]
    pub start_ms: i64,
    /// The one captured instant every figure was read at: the exclusive end.
    #[ts(type = "number")]
    pub observed_ms: i64,
    /// The next local midnight, when this date ends.
    #[ts(type = "number")]
    pub next_midnight_ms: i64,
    /// True when `observed_ms` is exactly local midnight: the half-open day
    /// holds no instant yet, so nothing was read and every count is zero.
    pub empty: bool,
    pub output: TodayOutput,
    pub cost: TodayCost,
    pub agent: TodayAgent,
    /// Tray sections existing data cannot prove.
    pub unavailable: Vec<DashboardUnavailable>,
}

/// Local midnight of `now_ms`'s date and the next one, in `zone`. Where
/// midnight itself is skipped by a transition, the day starts at its first
/// existing instant.
pub fn day_bounds(now_ms: i64, zone: &TimeZone) -> Result<(String, i64, i64), StateError> {
    let local = Timestamp::from_millisecond(now_ms)
        .map_err(|_| StateError::InvalidMetricWindow)?
        .to_zoned(zone.clone());
    let start = local
        .start_of_day()
        .map_err(|_| StateError::InvalidMetricWindow)?;
    let next = start
        .tomorrow()
        .and_then(|day| day.start_of_day())
        .map_err(|_| StateError::InvalidMetricWindow)?;
    Ok((
        local.date().to_string(),
        start.timestamp().as_millisecond(),
        next.timestamp().as_millisecond(),
    ))
}

fn unavailable() -> Vec<DashboardUnavailable> {
    [
        (
            "active_now",
            "Live session state is not recorded; indexed events cannot show what is running now",
        ),
        ("last_rule_fire", "Rule-fire data is unavailable"),
    ]
    .into_iter()
    .map(|(key, reason)| DashboardUnavailable {
        key: key.into(),
        reason: reason.into(),
    })
    .collect()
}

/// One clock and zone from the caller, one snapshot of `db`.
pub fn today(
    db: &MetricsDb,
    now_ms: i64,
    zone: TimeZone,
    clock: MetricClock,
    catalog: &PriceCatalog,
) -> Result<TodaySummary, StateError> {
    let (date, start_ms, next_midnight_ms) = day_bounds(now_ms, &zone)?;
    let timezone = crate::dashboard::zone_name(&zone, clock.clone());
    let none = |price_version: String, basis: String| {
        (
            TodayOutput {
                state: TodayOutputState::NoneRecorded,
                output_tokens: None,
                selected_responses: 0,
                sessions: 0,
            },
            TodayCost {
                state: TodayCostState::NoneRecorded,
                total_usd: None,
                priced_subtotal_usd: 0.0,
                selected_observations: 0,
                priced_observations: 0,
                price_version,
                basis,
            },
            TodayAgent {
                active_ms: 0,
                sessions: 0,
            },
        )
    };
    // [midnight, midnight) holds no instant: say so rather than read an
    // interval the metric windows reject.
    let empty = now_ms == start_ms;
    let (output, cost, agent) = if empty {
        none(catalog.version().into(), xt_metrics::COST_BASIS.into())
    } else {
        let window = Window::new(start_ms, now_ms)?;
        let (tokens, cost, spans) = db.read_snapshot(|db| {
            Ok((
                db.tokens(window, zone.clone())?,
                db.cost(window, zone.clone(), catalog)?,
                db.active_spans(window)?,
            ))
        })?;
        let total = &tokens.total;
        let output = TodayOutput {
            state: match total.counters.output_tokens {
                _ if total.selected_responses == 0 => TodayOutputState::NoneRecorded,
                Some(_) => TodayOutputState::Recorded,
                None => TodayOutputState::Incomplete,
            },
            output_tokens: total.counters.output_tokens,
            selected_responses: total.selected_responses,
            sessions: total.sessions,
        };
        let summary = &cost.total;
        let cost = TodayCost {
            state: match summary.total_usd {
                _ if summary.selected_observations == 0 => TodayCostState::NoneRecorded,
                Some(_) => TodayCostState::Priced,
                None if summary.priced_observations > 0 => TodayCostState::Partial,
                None => TodayCostState::Unpriced,
            },
            total_usd: summary.total_usd,
            priced_subtotal_usd: summary.priced_subtotal_usd,
            selected_observations: summary.selected_observations,
            priced_observations: summary.priced_observations,
            price_version: cost.price_version,
            basis: cost.basis,
        };
        let mut sessions: Vec<(&str, &str)> = spans
            .spans
            .iter()
            .map(|span| (span.host.as_str(), span.session_id.as_str()))
            .collect();
        sessions.sort_unstable();
        sessions.dedup();
        let agent = TodayAgent {
            active_ms: spans.active_ms,
            sessions: sessions.len() as u64,
        };
        (output, cost, agent)
    };
    if cost.total_usd.is_some_and(|v| !v.is_finite()) || !cost.priced_subtotal_usd.is_finite() {
        return Err(StateError::MetricEncoding);
    }
    let summary = TodaySummary {
        date,
        timezone,
        clock,
        start_ms,
        observed_ms: now_ms,
        next_midnight_ms,
        empty,
        output,
        cost,
        agent,
        unavailable: unavailable(),
    };
    checked_value(&summary)?;
    Ok(summary)
}

/// Existing fixture export paths supply their pinned clock (UTC) and catalog.
pub fn fixture_today(
    path: &std::path::Path,
    now_ms: i64,
    prices: Option<&serde_json::Value>,
) -> Result<TodaySummary, StateError> {
    today(
        &MetricsDb::open(path)?,
        now_ms,
        TimeZone::UTC,
        MetricClock::Fixture,
        &crate::dashboard::fixture_catalog(prices)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }

    #[test]
    fn today_bounds_follow_local_dst_days_and_skipped_midnights() {
        let eastern = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
        // The 23-hour spring-forward date and the 25-hour fall-back date.
        let (date, start, next) = day_bounds(ms("2026-03-08T20:00:00Z"), &eastern).unwrap();
        assert_eq!(date, "2026-03-08");
        assert_eq!(
            (start, next - start),
            (ms("2026-03-08T05:00:00Z"), 23 * 3_600_000)
        );
        let (date, start, next) = day_bounds(ms("2026-11-01T20:00:00Z"), &eastern).unwrap();
        assert_eq!(date, "2026-11-01");
        assert_eq!(
            (start, next - start),
            (ms("2026-11-01T04:00:00Z"), 25 * 3_600_000)
        );
        // A zone whose transition skips midnight itself: the day begins at
        // 01:00, its first existing instant.
        let skipped = TimeZone::posix("<-04>4<-03>,M9.1.6/24,M4.1.6/24").unwrap();
        let (date, start, _) = day_bounds(ms("2026-09-06T12:00:00Z"), &skipped).unwrap();
        assert_eq!(date, "2026-09-06");
        assert_eq!(start, ms("2026-09-06T04:00:00Z"));
        // Exact midnight is its own date's start, never the previous day.
        let (date, start, _) = day_bounds(ms("2026-09-08T00:00:00Z"), &TimeZone::UTC).unwrap();
        assert_eq!(
            (date.as_str(), start),
            ("2026-09-08", ms("2026-09-08T00:00:00Z"))
        );
    }
}
