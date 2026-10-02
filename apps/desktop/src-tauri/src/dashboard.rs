//! Thin application composition: metric definitions remain in xt-metrics.
use crate::{dto::*, state::StateError};
use jiff::tz::TimeZone;
use serde::{Serialize, de::DeserializeOwned};
use xt_metrics::{MetricsDb, PriceCatalog, TypingRate, Window};

pub const WINDOW_PRESETS: [u32; 3] = [7, 14, 30];
const DAY_MS: i64 = 86_400_000;
/// Activity lanes use this fixed recent axis, independent of the selected range.
pub const LANE_WINDOW_MS: i64 = 2 * DAY_MS;
pub const LANE_LIMIT: usize = 200;

#[derive(Serialize)]
struct Period {
    counts: xt_metrics::Counts,
    spans: xt_metrics::ActiveSpanReport,
    human: xt_metrics::HumanTime,
    concurrency: xt_metrics::Concurrency,
    hands_off: xt_metrics::HandsOff,
    tokens: xt_metrics::TokenReport,
    favorite: xt_metrics::FavoriteModel,
    sessions: xt_metrics::SessionsPerDay,
    cost: xt_metrics::CostReport,
}
impl Period {
    fn read(
        db: &MetricsDb,
        window: Window,
        zone: TimeZone,
        catalog: &PriceCatalog,
    ) -> xt_metrics::Result<Self> {
        Ok(Self {
            counts: db.counts(window, TypingRate::default())?,
            spans: db.active_spans(window)?,
            human: db.human_time(window, TypingRate::default())?,
            concurrency: db.concurrency(window)?,
            hands_off: db.hands_off(window)?,
            tokens: db.tokens(window, zone.clone())?,
            favorite: db.favorite_model(window)?,
            sessions: db.sessions_per_day(window, zone.clone())?,
            cost: db.cost(window, zone, catalog)?,
        })
    }
}

// Match DbCounts' exact JSON-integer boundary across all nested transport fields.
// Core float reports already enforce finite arithmetic; tile values are checked
// separately before serialization so a nonfinite value cannot turn into null.
fn check_integers(value: &serde_json::Value) -> Result<(), StateError> {
    match value {
        serde_json::Value::Number(number) => {
            if number.as_u64().is_some_and(|n| n >= 1_u64 << 53)
                || number
                    .as_i64()
                    .is_some_and(|n| n.unsigned_abs() >= 1_u64 << 53)
            {
                return Err(StateError::CountRange);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                check_integers(value)?;
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                check_integers(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn checked_value(value: &impl Serialize) -> Result<serde_json::Value, StateError> {
    let value = serde_json::to_value(value).map_err(|_| StateError::MetricEncoding)?;
    check_integers(&value)?;
    Ok(value)
}
/// Generated DTOs mirror core report shapes. The conversion must reproduce the
/// core JSON exactly, so a renamed or removed core field fails instead of
/// silently becoming an unknown (`null`) value.
pub(crate) fn convert<T: Serialize + DeserializeOwned>(
    value: &impl Serialize,
) -> Result<T, StateError> {
    let source = checked_value(value)?;
    let converted: T =
        serde_json::from_value(source.clone()).map_err(|_| StateError::MetricEncoding)?;
    if serde_json::to_value(&converted).map_err(|_| StateError::MetricEncoding)? != source {
        return Err(StateError::MetricEncoding);
    }
    Ok(converted)
}
fn tile(
    value: Option<f64>,
    previous: Option<f64>,
    samples: (Option<u64>, Option<u64>, &str),
    unit: &str,
    rule: &str,
    reason: &str,
) -> Result<MetricTile, StateError> {
    if [value, previous]
        .into_iter()
        .flatten()
        .any(|v| !v.is_finite())
    {
        return Err(StateError::MetricEncoding);
    }
    let pct = match (samples.0, samples.1) {
        (Some(current), Some(previous_n)) => {
            xt_metrics::Delta::new(value, previous, current, previous_n).pct
        }
        _ => None,
    };
    Ok(MetricTile {
        value,
        unit: unit.into(),
        rule_id: rule.into(),
        reason: value.is_none().then(|| reason.into()),
        note: None,
        current_n: samples.0,
        previous_n: samples.1,
        sample_unit: samples.2.into(),
        delta: MetricDelta {
            previous,
            pct,
            suppressed: pct.is_none(),
        },
    })
}
fn unavailable(reason: &str, unit: &str, rule: &str) -> Result<MetricTile, StateError> {
    tile(None, None, (None, None, "unavailable"), unit, rule, reason)
}
pub(crate) fn selected_window(days: u32, now_ms: i64) -> Result<Window, StateError> {
    validate_window(days)?;
    let start = now_ms
        .checked_sub(i64::from(days) * DAY_MS)
        .ok_or(StateError::InvalidMetricWindow)?;
    Ok(Window::new(start, now_ms)?)
}
pub(crate) fn dashboard_window(
    days: u32,
    window: Window,
    zone: &TimeZone,
    clock: MetricClock,
) -> DashboardWindow {
    DashboardWindow {
        days,
        start_ms: window.start_ms(),
        end_ms: window.end_ms(),
        timezone: zone
            .iana_name()
            .unwrap_or(if clock == MetricClock::Fixture {
                "UTC"
            } else {
                "system-local"
            })
            .into(),
        clock,
    }
}

/// Validate a range before opening storage, so an invalid request does no I/O.
pub fn validate_window(days: u32) -> Result<(), StateError> {
    if WINDOW_PRESETS.contains(&days) {
        Ok(())
    } else {
        Err(StateError::InvalidMetricWindow)
    }
}

/// The caller supplies one clock/zone/catalog policy. Native and fixture paths
/// use this same read-only assembler; no source-history data is sampled here.
pub fn assemble(
    db: &MetricsDb,
    days: u32,
    now_ms: i64,
    zone: TimeZone,
    clock: MetricClock,
    catalog: &PriceCatalog,
) -> Result<DashboardMetrics, StateError> {
    let window = selected_window(days, now_ms)?;
    let previous = window.previous()?;
    let lane_start = now_ms
        .checked_sub(LANE_WINDOW_MS)
        .ok_or(StateError::InvalidMetricWindow)?;
    let lane_window = Window::new(lane_start, now_ms)?;
    let (current, prior, coverage, mut lanes) = db.read_snapshot(|db| {
        Ok((
            Period::read(db, window, zone.clone(), catalog)?,
            Period::read(db, previous, zone.clone(), catalog)?,
            db.coverage(window, now_ms, &[])?,
            db.active_spans(lane_window)?.spans,
        ))
    })?;
    checked_value(&current)?;
    checked_value(&prior)?;
    let session_n = (
        Some(current.counts.sessions),
        Some(prior.counts.sessions),
        "sessions",
    );
    let stretch_n = (current.hands_off.n, prior.hands_off.n, "stretches");
    let mut human = tile(
        current.human.human_minutes_est.map(|v| v / 60.0),
        prior.human.human_minutes_est.map(|v| v / 60.0),
        session_n,
        "hours_est",
        "M-07",
        "Human classification or required typing length is unmeasured",
    )?;
    human.note = Some(match current.human.summed_session_minutes_est {
        Some(value) => format!(
            "Wall-clock union; summed session unions: {:.3} hours est.",
            value / 60.0
        ),
        None => "Wall-clock union; summed session unions are unmeasured".into(),
    });
    // Same divisor convention as sessions/day: every reported local date bucket,
    // including empty and partial days. A DST day is one bucket, not 23/25 hours.
    let mut agent_per_day = tile(
        Some(current.spans.active_ms as f64 / 3_600_000.0 / current.sessions.days.len() as f64),
        Some(prior.spans.active_ms as f64 / 3_600_000.0 / prior.sessions.days.len() as f64),
        session_n,
        "hours_per_day",
        "M-05",
        "Agent time is unmeasured",
    )?;
    agent_per_day.note = Some(format!(
        "Selected agent hours divided by {} reported local date buckets, including zero and partial days; previous denominator: {}.",
        current.sessions.days.len(),
        prior.sessions.days.len()
    ));
    let tiles = DashboardTiles {
        agent_hours_per_day: agent_per_day,
        agent_hours: tile(
            Some(current.spans.active_ms as f64 / 3_600_000.0),
            Some(prior.spans.active_ms as f64 / 3_600_000.0),
            session_n,
            "hours",
            "M-05",
            "Agent time is unmeasured",
        )?,
        human_hours_est: human,
        ratio: tile(
            current.human.agent_to_human_ratio,
            prior.human.agent_to_human_ratio,
            session_n,
            "ratio",
            "M-08",
            "Human time is zero or unmeasured",
        )?,
        concurrency_max: tile(
            current.concurrency.max.map(f64::from),
            prior.concurrency.max.map(f64::from),
            session_n,
            "sessions",
            "M-06",
            "No positive-duration active spans",
        )?,
        concurrency_mean: tile(
            current.concurrency.mean,
            prior.concurrency.mean,
            session_n,
            "sessions",
            "M-06",
            "No positive-duration active spans",
        )?,
        hands_off_median: tile(
            current.hands_off.median_min,
            prior.hands_off.median_min,
            stretch_n,
            "minutes",
            "M-09",
            "No measurable eligible hands-off stretches",
        )?,
        hands_off_p90: tile(
            current.hands_off.p90_min,
            prior.hands_off.p90_min,
            stretch_n,
            "minutes",
            "M-09",
            "No measurable eligible hands-off stretches",
        )?,
        sessions: tile(
            Some(current.counts.sessions as f64),
            Some(prior.counts.sessions as f64),
            session_n,
            "sessions",
            "M-16",
            "Sessions are unmeasured",
        )?,
        human_messages: tile(
            current.counts.human_messages.map(|v| v as f64),
            prior.counts.human_messages.map(|v| v as f64),
            session_n,
            "messages",
            "M-02",
            "Human classification is unmeasured",
        )?,
        assistant_turns: tile(
            current.counts.assistant_turns.map(|v| v as f64),
            prior.counts.assistant_turns.map(|v| v as f64),
            session_n,
            "turns",
            "M-03",
            "Human classification is unmeasured",
        )?,
        tool_calls: tile(
            current.counts.tool_calls.map(|v| v as f64),
            prior.counts.tool_calls.map(|v| v as f64),
            session_n,
            "calls",
            "M-17",
            "Tool count is unmeasured",
        )?,
        sessions_per_day: tile(
            Some(current.sessions.mean_per_day),
            Some(prior.sessions.mean_per_day),
            session_n,
            "sessions_per_day",
            "M-16",
            "Sessions are unmeasured",
        )?,
        tokens: tile(
            current.tokens.total.counters.total_tokens.map(|v| v as f64),
            prior.tokens.total.counters.total_tokens.map(|v| v as f64),
            (
                Some(current.tokens.total.sessions),
                Some(prior.tokens.total.sessions),
                "sessions",
            ),
            "tokens",
            "M-04",
            "Selected usage counters are absent or incomplete",
        )?,
        // Session-derived delta: n is distinct sessions contributing selected
        // usage (the same token report cost prices), never response counts.
        cost: tile(
            current.cost.total.total_usd,
            prior.cost.total.total_usd,
            (
                Some(current.tokens.total.sessions),
                Some(prior.tokens.total.sessions),
                "sessions",
            ),
            "usd_api_equivalent",
            "M-04",
            "Selected usage is absent or unpriced; see cost reasons",
        )?,
        merged_prs: unavailable("Merged PR data is unavailable", "prs", "M-14")?,
        rule_fires: unavailable("Rule-fire data is unavailable", "fires", "R-05")?,
    };
    let series = &current.tokens.by_day;
    if series.len() != current.cost.by_day.len() || series.len() != current.sessions.days.len() {
        return Err(StateError::MetricEncoding);
    }
    let mut days_out = Vec::with_capacity(series.len());
    for ((tokens, cost), sessions) in series
        .iter()
        .zip(&current.cost.by_day)
        .zip(&current.sessions.days)
    {
        if tokens.date != cost.date || tokens.date != sessions.date {
            return Err(StateError::MetricEncoding);
        }
        days_out.push(DashboardDay {
            date: tokens.date.clone(),
            start_ms: tokens.start_ms,
            end_ms: tokens.end_ms,
            tokens: convert(&tokens.tokens)?,
            cost_usd: cost.cost.total_usd,
            priced_subtotal_usd: cost.cost.priced_subtotal_usd,
            sessions: sessions.sessions,
        });
    }
    // Most recent first; the display cap never feeds an aggregate metric.
    lanes.sort_by(|a, b| {
        b.end_ms
            .cmp(&a.end_ms)
            .then_with(|| b.start_ms.cmp(&a.start_ms))
            .then_with(|| a.session_id.cmp(&b.session_id))
            .then_with(|| a.host.cmp(&b.host))
    });
    let lanes_total = lanes.len() as u64;
    lanes.truncate(LANE_LIMIT);
    let report = DashboardMetrics {
        window: dashboard_window(days, window, &zone, clock),
        tiles,
        hands_off_excluded_surfaces: convert(&current.hands_off.excluded_surfaces)?,
        favorite: DashboardFavorite {
            current: convert(&current.favorite)?,
            previous: convert(&prior.favorite)?,
            rule_id: "M-10".into(),
        },
        tokens: convert(&current.tokens.total)?,
        tokens_by_host: convert(&current.tokens.by_host)?,
        days: days_out,
        lanes: convert(&lanes)?,
        lane_start_ms: lane_start,
        lane_end_ms: now_ms,
        lanes_total,
        lanes_truncated: lanes_total > LANE_LIMIT as u64,
        usage_coverage: convert(&coverage.usage)?,
        usage_gate_14d: convert(&coverage.gate_14d)?,
        // No runtime discovery inventory is supplied; receipts cannot establish it.
        capture_inventory: MetricInventory::Unknown,
        capture_coverage: convert(&coverage.capture)?,
        cost: DashboardCost {
            price_version: current.cost.price_version,
            as_of: current.cost.price_as_of,
            basis: current.cost.basis,
            total_usd: current.cost.total.total_usd,
            priced_subtotal_usd: current.cost.total.priced_subtotal_usd,
            selected_observations: current.cost.total.selected_observations,
            priced_observations: current.cost.total.priced_observations,
            unpriced_observations: current.cost.total.unpriced_observations,
            unpriced: convert(&current.cost.total.unpriced)?,
        },
        unavailable: [
            ("merged_prs", "Merged PR data is unavailable"),
            ("rule_fires", "Rule-fire data is unavailable"),
            ("environment", "Environment data is unavailable"),
            ("work_type", "Work-type data is unavailable"),
        ]
        .into_iter()
        .map(|(key, reason)| DashboardUnavailable {
            key: key.into(),
            reason: reason.into(),
        })
        .collect(),
    };
    checked_value(&report)?;
    Ok(report)
}

/// Recorded host token totals for the selected range: the same core token
/// report the Dashboard uses, without assembling the other metrics.
pub fn tokens_by_host(
    db: &MetricsDb,
    days: u32,
    now_ms: i64,
    zone: TimeZone,
    clock: MetricClock,
) -> Result<TokensByHost, StateError> {
    let window = selected_window(days, now_ms)?;
    let report = db.read_snapshot(|db| db.tokens(window, zone.clone()))?;
    Ok(TokensByHost {
        window: dashboard_window(days, window, &zone, clock),
        hosts: convert(&report.by_host)?,
    })
}

/// A fixture's explicit synthetic catalog, or the bundled one when it has none.
pub fn fixture_catalog(prices: Option<&serde_json::Value>) -> Result<PriceCatalog, StateError> {
    Ok(match prices {
        Some(prices) => PriceCatalog::from_json(&prices.to_string())?,
        None => PriceCatalog::bundled()?,
    })
}

/// Existing fixture export/startup paths supply their pinned clock and explicit
/// synthetic catalog. This never selects native history or changes stored rows.
pub fn fixture_reports(
    path: &std::path::Path,
    now_ms: i64,
    prices: Option<&serde_json::Value>,
) -> Result<Vec<DashboardMetrics>, StateError> {
    let metrics = MetricsDb::open(path)?;
    let catalog = fixture_catalog(prices)?;
    WINDOW_PRESETS
        .into_iter()
        .map(|days| {
            assemble(
                &metrics,
                days,
                now_ms,
                TimeZone::UTC,
                MetricClock::Fixture,
                &catalog,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dashboard_json_integer_and_nonfinite_tile_boundaries() {
        assert!(check_integers(&serde_json::json!({"nested":[(1_u64<<53)-1]})).is_ok());
        assert!(check_integers(&serde_json::json!(-((1_i64 << 53) - 1))).is_ok());
        for value in [1_u64 << 53, u64::MAX] {
            assert!(check_integers(&serde_json::json!({"nested":[value]})).is_err());
        }
        assert!(check_integers(&serde_json::json!(-(1_i64 << 53))).is_err());
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for (value, previous) in [(Some(bad), None), (Some(1.0), Some(bad))] {
                assert!(
                    tile(
                        value,
                        previous,
                        (None, None, "sessions"),
                        "hours",
                        "M-05",
                        ""
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn dashboard_delta_is_percentage_points_and_suppresses_low_samples() {
        let shown = tile(
            Some(15.0),
            Some(10.0),
            (Some(5), Some(5), "sessions"),
            "h",
            "M-05",
            "",
        )
        .unwrap();
        assert_eq!(
            (shown.delta.pct, shown.delta.suppressed),
            (Some(50.0), false)
        );
        for n in [(Some(4), Some(5)), (Some(5), Some(4)), (None, Some(5))] {
            let low = tile(
                Some(15.0),
                Some(10.0),
                (n.0, n.1, "sessions"),
                "h",
                "M-05",
                "",
            )
            .unwrap();
            assert_eq!((low.delta.previous, low.delta.pct), (Some(10.0), None));
            assert!(low.delta.suppressed);
        }
        let zero = tile(
            Some(0.0),
            Some(0.0),
            (Some(9), Some(9), "sessions"),
            "h",
            "M-05",
            "x",
        )
        .unwrap();
        assert_eq!(
            (zero.value, zero.reason, zero.delta.pct),
            (Some(0.0), None, None)
        );
        let unknown = tile(
            None,
            Some(1.0),
            (Some(9), Some(9), "sessions"),
            "h",
            "M-05",
            "why",
        )
        .unwrap();
        assert_eq!(unknown.reason.as_deref(), Some("why"));
        assert!(unknown.delta.suppressed);
    }

    #[test]
    fn dashboard_conversion_rejects_core_shape_drift() {
        #[derive(Serialize)]
        struct Renamed {
            host: String,
            renamed: u64,
        }
        // A removed core field would otherwise deserialize as an unknown `null`.
        #[derive(Serialize)]
        struct Missing {
            model: Option<String>,
        }
        #[derive(Serialize)]
        struct Exact {
            model: Option<String>,
            output_tokens: Option<u64>,
            unknown_reason: Option<&'static str>,
        }
        let renamed = Renamed {
            host: "claude".into(),
            renamed: 1,
        };
        assert!(convert::<DashboardLane>(&renamed).is_err());
        let missing = Missing {
            model: Some("synthetic-model".into()),
        };
        assert!(convert::<MetricFavorite>(&missing).is_err());
        let exact = Exact {
            model: None,
            output_tokens: None,
            unknown_reason: Some("no_measured_output"),
        };
        assert_eq!(
            convert::<MetricFavorite>(&exact).unwrap().unknown_reason,
            Some(MetricFavoriteUnknown::NoMeasuredOutput)
        );
        assert!(validate_window(15).is_err());
        assert!(
            WINDOW_PRESETS
                .into_iter()
                .all(|days| validate_window(days).is_ok())
        );
    }
}
