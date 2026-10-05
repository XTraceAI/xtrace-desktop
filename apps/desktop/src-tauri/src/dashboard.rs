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
        typing_rate: TypingRate,
    ) -> xt_metrics::Result<Self> {
        Ok(Self {
            counts: db.counts(window, typing_rate)?,
            spans: db.active_spans(window)?,
            human: db.human_time(window, typing_rate, zone.clone())?,
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
        timezone: zone_name(zone, clock.clone()),
        clock,
    }
}
/// The zone a report names: its IANA name, `UTC` for the fixture clock, or
/// `system-local` when the OS reports none.
pub(crate) fn zone_name(zone: &TimeZone, clock: MetricClock) -> String {
    zone.iana_name()
        .unwrap_or(if clock == MetricClock::Fixture {
            "UTC"
        } else {
            "system-local"
        })
        .into()
}

/// The returned spans of the fixed lane window, how many there were before the
/// display cap, and one context row per distinct session the returned spans
/// name: its repository, branch, saved title, start (the recorded start, or a
/// Claude session's earliest message) and recorded
/// pull-request link counts, read in one bounded statement for every named
/// session at once.
struct Lanes {
    spans: Vec<xt_metrics::ActiveSpan>,
    /// Spans in the window before the display cap; never a capped count.
    total: u64,
    sessions: Vec<DashboardLaneSession>,
}

// Every session the capped lanes name is priced in one bounded read, so the
// cap can never exceed what that read accepts: raising one alone fails here.
const _: () = assert!(
    LANE_LIMIT <= xt_metrics::MAX_SESSIONS,
    "LANE_LIMIT must not exceed the sessions one cost read accepts"
);

/// The DTO name of a pricing gap, field for field; exhaustive, so a new
/// reason in `xt_metrics` fails to compile here instead of drifting.
fn unpriced_reason(reason: xt_metrics::UnpricedReason) -> MetricUnpricedReason {
    use xt_metrics::UnpricedReason as R;
    match reason {
        R::MissingModel => MetricUnpricedReason::MissingModel,
        R::UnknownModel => MetricUnpricedReason::UnknownModel,
        R::MissingServiceTier => MetricUnpricedReason::MissingServiceTier,
        R::UnknownServiceTier => MetricUnpricedReason::UnknownServiceTier,
        R::MissingCounters => MetricUnpricedReason::MissingCounters,
        R::MissingPromptCounters => MetricUnpricedReason::MissingPromptCounters,
        R::MissingCacheSplit => MetricUnpricedReason::MissingCacheSplit,
        R::InconsistentCacheSplit => MetricUnpricedReason::InconsistentCacheSplit,
        R::MissingRate => MetricUnpricedReason::MissingRate,
    }
}

/// The returned spans of the fixed lane window, most recent first and capped
/// for display, and how many there were before the cap.
fn lane_spans(
    db: &MetricsDb,
    lane_window: Window,
) -> xt_metrics::Result<(Vec<xt_metrics::ActiveSpan>, u64)> {
    let mut spans = db.active_spans(lane_window)?.spans;
    // Most recent first; the display cap never feeds an aggregate metric.
    spans.sort_by(|a, b| {
        b.end_ms
            .cmp(&a.end_ms)
            .then_with(|| b.start_ms.cmp(&a.start_ms))
            .then_with(|| a.session_id.cmp(&b.session_id))
            .then_with(|| a.host.cmp(&b.host))
    });
    let total = spans.len() as u64;
    spans.truncate(LANE_LIMIT);
    Ok((spans, total))
}

/// Read the lane spans, cap them for display, then read the context and the
/// cost of exactly the sessions the capped list still names.
///
/// The cost is taken over the whole lane window, so a session whose earlier
/// spans the cap dropped still reports everything it spent in that window.
/// Ordering and the cap are unchanged: they decide which spans are drawn, and
/// no aggregate is derived from them.
fn lane_report(
    db: &MetricsDb,
    lane_window: Window,
    catalog: &PriceCatalog,
) -> xt_metrics::Result<Lanes> {
    let (spans, total) = lane_spans(db, lane_window)?;
    // At most one session per returned span, so this list is bounded by the
    // same cap and stays well inside what one measurement read accepts.
    let mut ids: Vec<&str> = spans.iter().map(|span| span.session_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut context = std::collections::BTreeMap::new();
    for row in db.session_context(&ids)? {
        context.insert(row.id.clone(), row);
    }
    let costs = db.session_costs(lane_window, &ids, catalog)?;
    // One row per distinct returned session, in identifier order, whether or
    // not the metadata read found context for it: a row is always addressable,
    // and an absent fact stays absent instead of being filled in.
    let sessions = ids
        .iter()
        .map(|id| {
            let found = context.get(*id);
            DashboardLaneSession {
                session_id: (*id).to_owned(),
                host: match found {
                    Some(row) => row.host.clone(),
                    // The span itself names the host that recorded it.
                    None => spans
                        .iter()
                        .find(|span| span.session_id == *id)
                        .map(|span| span.host.clone())
                        .unwrap_or_default(),
                },
                repo: found.and_then(|row| row.repo.clone()),
                branch: found.and_then(|row| row.branch.clone()),
                title: found.and_then(|row| row.title.clone()),
                automated_review: found.is_some_and(|row| row.automated_review),
                // The recorded start, or a Claude session's earliest message
                // when it has none; uncapped and unclipped by the lane window.
                // Another host's absent start stays absent even when spans exist.
                started_at_ms: found.and_then(|row| row.started_at_ms),
                // Measured over the stored index for an indexed session; an
                // absent context is unknown, never a zero.
                pr_links: found.map(|row| row.pr_links),
                inferred_pr_links: found.map(|row| row.inferred_pr_links),
                // Absent only when no indexed user session owns the
                // identifier: unknown, never a measured zero.
                cost: costs.get(*id).map(|cost| DashboardLaneCost {
                    total_usd: cost.total_usd,
                    priced_subtotal_usd: cost.priced_subtotal_usd,
                    selected_observations: cost.selected_observations,
                    priced_observations: cost.priced_observations,
                    unpriced_observations: cost.unpriced_observations,
                    assumed_tier_observations: cost.assumed_tier_observations,
                    unpriced: cost
                        .unpriced
                        .iter()
                        .map(|item| DashboardUnpriced {
                            model: item.model.clone(),
                            service_tier: item.service_tier.clone(),
                            reason: unpriced_reason(item.reason),
                            observations: item.observations,
                        })
                        .collect(),
                }),
                // Read with the context, in this snapshot; never a lane of its own.
                parent: found.and_then(|row| row.parent.clone()).map(Into::into),
            }
        })
        .collect();
    Ok(Lanes {
        spans,
        total,
        sessions,
    })
}

const PR_EFFORT_RULE: &str = "M-19";
/// The Dashboard reads M-19 with inferred links removed before anything else.
const CONFIRMED_ONLY: bool = true;

/// The merged-PR tile states the core tile and nothing more: a value only when
/// every retained pull request's cached facts can decide it, otherwise unknown
/// with the known subtotal named, never a zero. The delta compares two complete
/// counts, with the distinct merged pull requests of each window as n.
fn merged_prs_tile(
    current: &xt_metrics::MergedPrTile,
    prior: &xt_metrics::MergedPrTile,
) -> Result<MetricTile, StateError> {
    let mut tile = tile(
        current.merged.map(|n| n as f64),
        prior.merged.map(|n| n as f64),
        (current.merged, prior.merged, "prs"),
        "prs",
        PR_EFFORT_RULE,
        &format!(
            "{} known merged; {} linked pull {} no cached merge facts",
            current.known_merged,
            current.unknown_facts,
            if current.unknown_facts == 1 {
                "request has"
            } else {
                "requests have"
            }
        ),
    )?;
    tile.note = Some(
        "Confirmed links only: exact and SHA links count, inferred links are removed first. Cached gh pr view facts; nothing refreshes automatically."
            .into(),
    );
    Ok(tile)
}

/// Validate a range before opening storage, so an invalid request does no I/O.
pub fn validate_window(days: u32) -> Result<(), StateError> {
    if WINDOW_PRESETS.contains(&days) {
        Ok(())
    } else {
        Err(StateError::InvalidMetricWindow)
    }
}

/// The caller supplies one clock/zone/catalog policy and one typing rate.
/// Native and fixture paths use this same read-only assembler; no
/// source-history data is sampled here. The one rate is read for both the
/// current and the previous window, so a delta compares like with like.
pub fn assemble(
    db: &MetricsDb,
    days: u32,
    now_ms: i64,
    zone: TimeZone,
    clock: MetricClock,
    catalog: &PriceCatalog,
    typing_rate: TypingRate,
) -> Result<DashboardMetrics, StateError> {
    let window = selected_window(days, now_ms)?;
    let previous = window.previous()?;
    let lane_start = now_ms
        .checked_sub(LANE_WINDOW_MS)
        .ok_or(StateError::InvalidMetricWindow)?;
    let lane_window = Window::new(lane_start, now_ms)?;
    let (current, prior, coverage, lanes, untimed, pr_current, pr_prior) =
        db.read_snapshot(|db| {
            Ok((
                Period::read(db, window, zone.clone(), catalog, typing_rate)?,
                Period::read(db, previous, zone.clone(), catalog, typing_rate)?,
                db.coverage(window, now_ms, &[])?,
                // Spans, the context of the sessions they name and those
                // sessions' cost are read inside this one snapshot, so a row's
                // repository, identity and number all describe the same committed
                // state while the native writer keeps appending elsewhere.
                lane_report(db, lane_window, catalog)?,
                // Unwindowed on purpose: a record with no timestamp is in no day,
                // so this counts all indexed history. Reading it in the same
                // snapshot keeps it describing the state the windows describe.
                db.untimed_history()?,
                // M-19 over both equal windows, from cached refresh facts only:
                // no request is made here. Inferred links are removed before
                // eligibility and assignment, so the tile and the chart count
                // exact and SHA links only.
                db.pr_effort(window, zone.clone(), CONFIRMED_ONLY, catalog)?,
                db.pr_effort(previous, zone.clone(), CONFIRMED_ONLY, catalog)?,
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
    human.note = Some("Counted input characters divided by saved typing speed (five characters per word); includes selected or pasted text. Estimated input effort, not measured attention.".into());
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
        merged_prs: merged_prs_tile(&pr_current.tile, &pr_prior.tile)?,
        rule_fires: unavailable("Rule-fire data is unavailable", "fires", "R-05")?,
    };
    // The daily hours are the selected window's own measurements split at its
    // interior local-day boundaries, never a per-day re-measurement: the spans
    // this report already folded, and the one human union it already built.
    let agent_days = current.spans.by_day(window, zone.clone())?;
    let series = &current.tokens.by_day;
    if series.len() != current.cost.by_day.len()
        || series.len() != current.sessions.days.len()
        || series.len() != agent_days.len()
        || series.len() != current.human.by_day.len()
    {
        return Err(StateError::MetricEncoding);
    }
    let mut days_out = Vec::with_capacity(series.len());
    for ((((tokens, cost), sessions), agent), human_day) in series
        .iter()
        .zip(&current.cost.by_day)
        .zip(&current.sessions.days)
        .zip(&agent_days)
        .zip(&current.human.by_day)
    {
        // Every series must name the same day and the same bounds; a bucket
        // that drifted would silently pair one day's hours with another's.
        if tokens.date != cost.date
            || tokens.date != sessions.date
            || tokens.date != agent.date
            || tokens.date != human_day.date
            || (tokens.start_ms, tokens.end_ms) != (agent.start_ms, agent.end_ms)
            || (tokens.start_ms, tokens.end_ms) != (human_day.start_ms, human_day.end_ms)
        {
            return Err(StateError::MetricEncoding);
        }
        let agent_hours = agent.active_ms as f64 / 3_600_000.0;
        let human_hours_est = human_day.minutes_est.map(|minutes| minutes / 60.0);
        // Serialization turns a nonfinite float into null, which would read as
        // unknown; the same rule the tiles use rejects it instead.
        if !agent_hours.is_finite() || human_hours_est.is_some_and(|v| !v.is_finite()) {
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
            agent_hours,
            human_hours_est,
        });
    }
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
        lanes: convert(&lanes.spans)?,
        lane_sessions: lanes.sessions,
        lane_start_ms: lane_start,
        lane_end_ms: now_ms,
        lanes_total: lanes.total,
        lanes_truncated: lanes.total > LANE_LIMIT as u64,
        usage_coverage: convert(&coverage.usage)?,
        usage_gate_14d: convert(&coverage.gate_14d)?,
        // No runtime discovery inventory is supplied; receipts cannot establish it.
        capture_inventory: MetricInventory::Unknown,
        capture_coverage: convert(&coverage.capture)?,
        untimed_history: convert(&untimed)?,
        pr_effort: DashboardPrEffort {
            rule_id: PR_EFFORT_RULE.into(),
            current: convert(&pr_current)?,
            previous: convert(&pr_prior.tile)?,
        },
        cost: DashboardCost {
            price_version: current.cost.price_version,
            as_of: current.cost.price_as_of,
            basis: current.cost.basis,
            total_usd: current.cost.total.total_usd,
            priced_subtotal_usd: current.cost.total.priced_subtotal_usd,
            selected_observations: current.cost.total.selected_observations,
            priced_observations: current.cost.total.priced_observations,
            unpriced_observations: current.cost.total.unpriced_observations,
            assumed_tier_observations: current.cost.total.assumed_tier_observations,
            unpriced: convert(&current.cost.total.unpriced)?,
        },
        unavailable: [
            ("rule_fires", "Rule-fire data is unavailable"),
            ("environment", "Environment data is unavailable"),
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

/// The longest stored session identifier a span request names, in bytes. The
/// lanes only ever name identifiers they were sent; this bounds anything else.
const MAX_SESSION_ID_BYTES: usize = 512;

/// One lane span's detail as `xt_metrics` measures it: its most-used tool,
/// output tokens and the last message a person typed in it or before it.
///
/// The request names a span by its session and its two endpoints, exactly as
/// the lane report sent them. A span is folded inside the fixed lane window,
/// so a request that is longer than that window, inverted, or names no
/// session could not have come from a lane and is refused before storage is
/// read. The endpoints are not looked up again: a span whose events have
/// since grown simply describes the part it named.
pub fn span_detail(
    db: &MetricsDb,
    session_id: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<xt_metrics::SpanDetail, StateError> {
    if session_id.trim().is_empty()
        || session_id.len() > MAX_SESSION_ID_BYTES
        || start_ms > end_ms
        || end_ms
            .checked_sub(start_ms)
            .is_none_or(|length| length > LANE_WINDOW_MS)
    {
        return Err(StateError::InvalidSpanRequest);
    }
    Ok(db.span_detail(session_id, start_ms, end_ms)?)
}

/// One session's original source for a span's words, read at most once per
/// detail: loaded in memory, or why it was not.
pub enum SpanSource {
    Loaded(Box<xt_ingest::native::session_source::LoadedSession>),
    /// The source was read, but holds no record to look the words up in.
    NotFound,
    /// Every read slot was taken; nothing was read.
    Busy,
    /// The source could not be read, for the reason a transcript open gives.
    Unavailable(SessionSourceReason),
}

/// The span detail as it crosses to the view. Everything but the words is
/// the core report converted field for field. Words the index kept are used
/// as they are; only when it did not — for the last prompt, the automatic
/// line or both — is `read_source` asked, once, for the session's source, so
/// a source is opened only for words that are otherwise unknown and never
/// twice for one detail.
pub fn span_detail_dto(
    detail: xt_metrics::SpanDetail,
    read_source: impl FnOnce() -> Result<SpanSource, StateError>,
) -> Result<DashboardSpanDetail, StateError> {
    use xt_metrics::{AutomaticText, PromptText, SpanAutomatic, SpanDetail, SpanPrompt};
    let SpanDetail::Indexed {
        tool,
        output_tokens,
        prompt,
        automatic,
    } = detail
    else {
        return Ok(DashboardSpanDetail::Missing);
    };
    let needs_source = matches!(
        prompt,
        SpanPrompt::Found {
            text: PromptText::NotStored,
            ..
        }
    ) || matches!(
        automatic,
        SpanAutomatic::Found {
            text: AutomaticText::NotStored,
            ..
        }
    );
    let source = if needs_source {
        Some(read_source()?)
    } else {
        None
    };
    let prompt = match prompt {
        SpanPrompt::NoMessage => DashboardSpanPrompt::NoMessage,
        SpanPrompt::Unclassified => DashboardSpanPrompt::Unclassified,
        SpanPrompt::Found {
            at_ms,
            in_span,
            record_uuid,
            text,
        } => DashboardSpanPrompt::Found {
            at_ms,
            in_span,
            text: match (text, &source) {
                (PromptText::Stored { text, truncated }, _) => {
                    DashboardPromptText::Stored { text, truncated }
                }
                (PromptText::Wrapped, _) => DashboardPromptText::Wrapped,
                (PromptText::NotStored, Some(SpanSource::Loaded(session))) => {
                    prompt_words(session, &record_uuid)
                }
                (PromptText::NotStored, Some(SpanSource::NotFound)) => {
                    DashboardPromptText::NotFound
                }
                (PromptText::NotStored, Some(SpanSource::Busy)) => DashboardPromptText::Busy,
                (PromptText::NotStored, Some(SpanSource::Unavailable(reason))) => {
                    DashboardPromptText::Unavailable { reason: *reason }
                }
                // Asked for above whenever these words were not kept.
                (PromptText::NotStored, None) => unreachable!("the source was read"),
            },
        },
    };
    let automatic = match automatic {
        SpanAutomatic::NoNotification => DashboardSpanAutomatic::NoNotification,
        SpanAutomatic::Found {
            at_ms,
            record_uuid,
            text,
        } => DashboardSpanAutomatic::Found {
            at_ms,
            text: match (text, &source) {
                (AutomaticText::Stored { text, truncated }, _) => {
                    DashboardAutomaticText::Stored { text, truncated }
                }
                (AutomaticText::NoSummary, _) => DashboardAutomaticText::NoSummary,
                (AutomaticText::NotStored, Some(SpanSource::Loaded(session))) => {
                    automatic_words(session, &record_uuid)
                }
                (AutomaticText::NotStored, Some(SpanSource::NotFound)) => {
                    DashboardAutomaticText::NotFound
                }
                (AutomaticText::NotStored, Some(SpanSource::Busy)) => DashboardAutomaticText::Busy,
                (AutomaticText::NotStored, Some(SpanSource::Unavailable(reason))) => {
                    DashboardAutomaticText::Unavailable { reason: *reason }
                }
                (AutomaticText::NotStored, None) => unreachable!("the source was read"),
            },
        },
    };
    let detail = DashboardSpanDetail::Indexed {
        tool: convert(&tool)?,
        output_tokens,
        prompt,
        automatic,
    };
    checked_value(&detail)?;
    Ok(detail)
}

/// The joined text of every record in one loaded source that carries this
/// saved identity, or `None` when none does; `Some(Err(()))` when several do
/// and they do not say the same thing.
fn record_words(
    session: &xt_ingest::native::session_source::LoadedSession,
    record_uuid: &str,
) -> Option<Result<String, ()>> {
    let mut found = session
        .records
        .iter()
        .filter(|record| record.canonical.uuid.as_deref() == Some(record_uuid))
        .map(|record| {
            xt_store::record_text::joined(
                record.canonical.message.content.as_deref().unwrap_or(&[]),
            )
        });
    let text = found.next()?;
    Some(if found.any(|other| other != text) {
        Err(())
    } else {
        Ok(text)
    })
}

/// The summary of exactly the task notification with this saved identity in
/// one session's loaded source, by the same record match as
/// [`prompt_words`]: a source that holds no such record, or differing copies
/// of it, says so and never stands in another record.
pub fn automatic_words(
    session: &xt_ingest::native::session_source::LoadedSession,
    record_uuid: &str,
) -> DashboardAutomaticText {
    match record_words(session, record_uuid) {
        None => DashboardAutomaticText::NotFound,
        Some(Err(())) => DashboardAutomaticText::Ambiguous,
        Some(Ok(text)) => match xt_metrics::notification_summary(&text) {
            xt_metrics::AutomaticText::Stored { text, truncated } => {
                DashboardAutomaticText::Stored { text, truncated }
            }
            xt_metrics::AutomaticText::NoSummary | xt_metrics::AutomaticText::NotStored => {
                DashboardAutomaticText::NoSummary
            }
        },
    }
}

/// The words of exactly the record with this saved identity in one session's
/// loaded source: the same text blocks the human-input classifier joined at
/// ingest, as one bounded line. A source that holds no such record says so,
/// and one that holds several under the identity (a fork copies records) is
/// shown only when they all say the same thing; it never stands in another
/// message.
pub fn prompt_words(
    session: &xt_ingest::native::session_source::LoadedSession,
    record_uuid: &str,
) -> DashboardPromptText {
    let text = match record_words(session, record_uuid) {
        None => return DashboardPromptText::NotFound,
        Some(Err(())) => return DashboardPromptText::Ambiguous,
        Some(Ok(text)) => text,
    };
    match xt_metrics::prompt_excerpt(&text) {
        xt_metrics::PromptText::Stored { text, truncated } => {
            DashboardPromptText::Stored { text, truncated }
        }
        // An excerpt of text that was read is always text.
        xt_metrics::PromptText::NotStored | xt_metrics::PromptText::Wrapped => {
            DashboardPromptText::NotFound
        }
    }
}

/// The cancel tokens of the span detail reads running now. Separate from the
/// transcript opens so pointing at bars never takes a detail view's place.
#[derive(Default)]
pub struct SpanDetailReads(crate::transcript_reads::TranscriptReads);
impl std::ops::Deref for SpanDetailReads {
    type Target = crate::transcript_reads::TranscriptReads;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// The detail of every span a fixture's Dashboard lanes return, read through
/// the same command path and pinned clock as its reports. The lane axis is
/// fixed, so every range preset returns the same spans; they are read once.
/// A fixture has no local history, so words the fixture did not keep are
/// unavailable exactly as fixture startup answers.
pub fn fixture_span_details(
    path: &std::path::Path,
    now_ms: i64,
) -> Result<Vec<FixtureSpanDetail>, StateError> {
    let metrics = MetricsDb::open(path)?;
    let lane_start = now_ms
        .checked_sub(LANE_WINDOW_MS)
        .ok_or(StateError::InvalidMetricWindow)?;
    let (spans, _) = lane_spans(&metrics, Window::new(lane_start, now_ms)?)?;
    spans
        .into_iter()
        .map(|span| {
            let detail = span_detail(&metrics, &span.session_id, span.start_ms, span.end_ms)?;
            Ok(FixtureSpanDetail {
                detail: span_detail_dto(detail, || {
                    Ok(SpanSource::Unavailable(SessionSourceReason::NotIndexed))
                })?,
                session_id: span.session_id,
                start_ms: span.start_ms,
                end_ms: span.end_ms,
            })
        })
        .collect()
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
                // The export describes a fresh database: no saved speed.
                TypingRate::default(),
            )
        })
        .collect()
}

/// Each range's M-19 section, read by the same assembler as the Dashboard. The
/// fixture exporter calls it after its synthetic refresh, so the browser
/// preview can show what that refresh produced once it has made the same
/// request itself; nothing here is a second calculation.
pub fn fixture_pr_effort(
    path: &std::path::Path,
    now_ms: i64,
    prices: Option<&serde_json::Value>,
) -> Result<Vec<FixtureRefreshedPrEffort>, StateError> {
    Ok(fixture_reports(path, now_ms, prices)?
        .into_iter()
        .map(|report| FixtureRefreshedPrEffort {
            days: report.window.days,
            merged_prs: report.tiles.merged_prs,
            pr_effort: report.pr_effort,
        })
        .collect())
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

#[cfg(test)]
mod prompt_words_tests {
    use super::*;
    use serde_json::json;
    use xt_ingest::canonical::{Parsed, parse_line};
    use xt_ingest::native::session_source::{GenerationBasis, LoadedSession};

    fn loaded(texts: &[(&str, &str)]) -> LoadedSession {
        LoadedSession {
            native_session_id: "s".into(),
            host: xt_store::Host::Claude,
            generation: GenerationBasis::Unrecorded,
            sources: Vec::new(),
            records: texts
                .iter()
                .map(|(uuid, text)| {
                    let line = json!({"uuid": uuid, "type": "user",
                        "timestamp": "2026-09-07T12:00:00.000Z",
                        "message": {"role": "user", "content": [{"type": "text", "text": text}]}});
                    match parse_line(&line.to_string()).unwrap() {
                        Parsed::Record(record) => *record,
                        other => panic!("{other:?}"),
                    }
                })
                .collect(),
            other_lines: 0,
            dropped_records: 0,
            gaps: Vec::new(),
        }
    }

    #[test]
    fn a_record_is_picked_by_its_identity_alone() {
        let session = loaded(&[("a", "first"), ("b", "second")]);
        assert_eq!(
            prompt_words(&session, "b"),
            DashboardPromptText::Stored {
                text: "second".into(),
                truncated: false
            }
        );
        assert_eq!(prompt_words(&session, "c"), DashboardPromptText::NotFound);
    }

    #[test]
    fn copies_under_one_identity_are_shown_only_when_they_agree() {
        // A fork copies a record: the same words twice are still one message.
        let agreeing = loaded(&[("a", "same words"), ("a", "same words")]);
        assert_eq!(
            prompt_words(&agreeing, "a"),
            DashboardPromptText::Stored {
                text: "same words".into(),
                truncated: false
            }
        );
        let disagreeing = loaded(&[("a", "one thing"), ("a", "another")]);
        assert_eq!(
            prompt_words(&disagreeing, "a"),
            DashboardPromptText::Ambiguous
        );
    }

    const NOTE: &str = "<task-notification><task-id>t1</task-id><status>completed</status>\
        <summary>Agent \"Map prompt classification sources\" finished</summary>\
        <result>synthetic</result></task-notification>";

    #[test]
    fn a_notifications_summary_is_read_by_its_identity_alone() {
        let session = loaded(&[("p", "the request"), ("n", NOTE)]);
        assert_eq!(
            automatic_words(&session, "n"),
            DashboardAutomaticText::Stored {
                text: "Agent \"Map prompt classification sources\" finished".into(),
                truncated: false
            }
        );
        assert_eq!(
            automatic_words(&session, "x"),
            DashboardAutomaticText::NotFound
        );
        // A record with no summary of a notification has none to show.
        assert_eq!(
            automatic_words(&session, "p"),
            DashboardAutomaticText::NoSummary
        );
        let disagreeing = loaded(&[("n", NOTE), ("n", "another")]);
        assert_eq!(
            automatic_words(&disagreeing, "n"),
            DashboardAutomaticText::Ambiguous
        );
    }

    fn indexed(
        prompt: xt_metrics::PromptText,
        automatic: xt_metrics::AutomaticText,
    ) -> xt_metrics::SpanDetail {
        xt_metrics::SpanDetail::Indexed {
            tool: xt_metrics::SpanTool::NoCalls,
            output_tokens: None,
            prompt: xt_metrics::SpanPrompt::Found {
                at_ms: 1,
                in_span: true,
                record_uuid: "p".into(),
                text: prompt,
            },
            automatic: xt_metrics::SpanAutomatic::Found {
                at_ms: 2,
                record_uuid: "n".into(),
                text: automatic,
            },
        }
    }

    fn texts(detail: DashboardSpanDetail) -> (DashboardPromptText, DashboardAutomaticText) {
        match detail {
            DashboardSpanDetail::Indexed {
                prompt: DashboardSpanPrompt::Found { text: prompt, .. },
                automatic:
                    DashboardSpanAutomatic::Found {
                        text: automatic, ..
                    },
                ..
            } => (prompt, automatic),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_source_is_read_once_for_both_lines_and_only_when_words_were_not_kept() {
        use xt_metrics::{AutomaticText, PromptText};
        let mut reads = 0;
        let detail = span_detail_dto(
            indexed(PromptText::NotStored, AutomaticText::NotStored),
            || {
                reads += 1;
                Ok(SpanSource::Loaded(Box::new(loaded(&[
                    ("p", "the request"),
                    ("n", NOTE),
                ]))))
            },
        )
        .unwrap();
        assert_eq!(reads, 1);
        let (prompt, automatic) = texts(detail);
        assert_eq!(
            prompt,
            DashboardPromptText::Stored {
                text: "the request".into(),
                truncated: false
            }
        );
        assert!(matches!(automatic, DashboardAutomaticText::Stored { .. }));
        // Kept words need no source.
        let kept = span_detail_dto(
            indexed(
                PromptText::Stored {
                    text: "kept".into(),
                    truncated: false,
                },
                AutomaticText::NoSummary,
            ),
            || panic!("no source is read for words the index kept"),
        )
        .unwrap();
        assert_eq!(texts(kept).1, DashboardAutomaticText::NoSummary);
        // One busy answer covers both lines.
        let busy = span_detail_dto(
            indexed(PromptText::NotStored, AutomaticText::NotStored),
            || Ok(SpanSource::Busy),
        )
        .unwrap();
        assert_eq!(
            texts(busy),
            (DashboardPromptText::Busy, DashboardAutomaticText::Busy)
        );
    }
}
