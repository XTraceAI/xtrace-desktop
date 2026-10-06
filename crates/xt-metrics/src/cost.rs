use crate::{
    Error, MetricsDb, PriceCatalog, Result, Window,
    prices::{COST_BASIS, CacheWriteMode},
};
use jiff::tz::TimeZone;
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str = "SELECT host,model,surface,ts,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,cache_creation_5m,cache_creation_1h,service_tier FROM v_response_usage WHERE ts_ms>=?1 AND ts_ms<?2";
/// The same selection and columns as [`QUERY`], plus the response's session
/// (column 11), for callers that group the priced responses by session.
pub(crate) const SESSION_QUERY: &str = "SELECT host,model,surface,ts,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,cache_creation_5m,cache_creation_1h,service_tier,session_id FROM v_response_usage WHERE ts_ms>=?1 AND ts_ms<?2";
/// [`SESSION_QUERY`]'s selection and columns with no window: every response
/// of the named sessions, whenever it was, and those with no timestamp too,
/// which no window holds and which are counted, unpriced, rather than dropped.
pub(crate) const WHOLE_SESSION_QUERY: &str = "SELECT host,model,surface,ts,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,cache_creation_5m,cache_creation_1h,service_tier,session_id FROM v_response_usage";
/// The host whose responses carry no service tier: its reader does not keep
/// one, and OpenAI bills a request that names no tier at its default tier.
const ASSUMED_DEFAULT_TIER_HOST: &str = "codex";
const ASSUMED_DEFAULT_TIER: &str = "default";
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnpricedReason {
    MissingModel,
    UnknownModel,
    MissingServiceTier,
    UnknownServiceTier,
    MissingCounters,
    MissingPromptCounters,
    MissingCacheSplit,
    InconsistentCacheSplit,
    MissingRate,
    /// The response records no time. Only a whole-session read selects it:
    /// it is in no window, and it is counted rather than dropped.
    MissingTimestamp,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnpricedCost {
    pub model: Option<String>,
    pub service_tier: Option<String>,
    pub reason: UnpricedReason,
    pub observations: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CostSummary {
    pub selected_observations: u64,
    pub priced_observations: u64,
    pub unpriced_observations: u64,
    /// Priced observations that recorded no service tier and were priced at
    /// the host's documented default tier (Codex only); included in
    /// `priced_observations`.
    pub assumed_tier_observations: u64,
    pub total_usd: Option<f64>,
    pub priced_subtotal_usd: f64,
    /// `priced_subtotal_usd` exactly, in nano-USD, so callers can add
    /// subtotals without floating-point rounding. Not serialized.
    #[serde(skip)]
    pub priced_subtotal_nano_usd: u128,
    pub unpriced: Vec<UnpricedCost>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HostCost {
    pub host: String,
    pub cost: CostSummary,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelCost {
    pub model: Option<String>,
    pub cost: CostSummary,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SurfaceCost {
    pub host: String,
    pub surface: Option<String>,
    pub cost: CostSummary,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DayCost {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub cost: CostSummary,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CostReport {
    pub price_version: String,
    pub price_as_of: String,
    pub basis: String,
    pub total: CostSummary,
    pub by_host: Vec<HostCost>,
    pub by_model: Vec<ModelCost>,
    pub by_surface: Vec<SurfaceCost>,
    pub by_day: Vec<DayCost>,
}
pub(crate) struct Observation {
    host: String,
    model: Option<String>,
    tier: Option<String>,
    counters: [Option<u64>; 4],
    split: [Option<u64>; 2],
}
#[derive(Clone, Copy)]
pub(crate) enum Outcome {
    /// Nano-USD, and whether the tier was assumed rather than recorded.
    Priced(u128, bool),
    Unpriced(UnpricedReason),
}
/// One row of [`QUERY`] or [`SESSION_QUERY`]: its raw timestamp, host, surface
/// and the observation to price.
pub(crate) fn read_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(InstantKey, Option<String>, Observation)> {
    let raw: String = row.get(3)?;
    let ts = timestamp::parse(&raw)
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?
        .0;
    Ok((ts, row.get(2)?, read_observation(row)?))
}
/// The observation of one such row, whatever its timestamp.
fn read_observation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Observation> {
    let mut values = [None; 6];
    for (i, value) in values.iter_mut().enumerate() {
        *value = row
            .get::<_, Option<i64>>(i + 4)?
            .map(|v| {
                u64::try_from(v).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(i + 4, v))
            })
            .transpose()?;
    }
    Ok(Observation {
        host: row.get(0)?,
        model: row.get(1)?,
        tier: row.get(10)?,
        counters: [values[0], values[1], values[2], values[3]],
        split: [values[4], values[5]],
    })
}
impl Observation {
    /// The recorded model name; `None` when absent or blank, as pricing treats it.
    pub(crate) fn model(&self) -> Option<&str> {
        self.model.as_deref().filter(|s| !s.trim().is_empty())
    }
    /// The recorded output-token counter, when present.
    pub(crate) fn output_tokens(&self) -> Option<u64> {
        self.counters[1]
    }
    pub(crate) fn price(&self, catalog: &PriceCatalog) -> Result<Outcome> {
        use UnpricedReason::*;
        let unpriced = |reason| Ok(Outcome::Unpriced(reason));
        let Some(name) = self.model.as_deref().filter(|s| !s.trim().is_empty()) else {
            return unpriced(MissingModel);
        };
        let Some(model) = catalog.model(name) else {
            return unpriced(UnknownModel);
        };
        let recorded = self.tier.as_deref().filter(|s| !s.trim().is_empty());
        let assumed = recorded.is_none() && self.host == ASSUMED_DEFAULT_TIER_HOST;
        let Some(tier) = recorded.or(assumed.then_some(ASSUMED_DEFAULT_TIER)) else {
            return unpriced(MissingServiceTier);
        };
        let [input, _, read, write] = self.counters;
        if model.bands.len() > 1 && [input, read, write].iter().any(Option::is_none) {
            return unpriced(MissingPromptCounters);
        }
        let [Some(input), Some(output), Some(read), Some(write)] = self.counters else {
            return unpriced(MissingCounters);
        };
        let prompt = u128::from(input) + u128::from(read) + u128::from(write);
        // Catalog validation guarantees an ordered exhaustive partition, with an
        // unbounded final band. Prompt components are disjoint canonical counters.
        let band = model
            .bands
            .iter()
            .find(|b| {
                b.max_prompt_tokens
                    .is_none_or(|max| prompt <= u128::from(max))
            })
            .unwrap();
        let Some(tier) = band
            .tiers
            .iter()
            .find(|t| t.names.iter().any(|n| n == tier))
        else {
            // An assumed tier the catalog lacks is still a missing tier.
            return unpriced(if assumed {
                MissingServiceTier
            } else {
                UnknownServiceTier
            });
        };
        let rates = &tier.rates;
        let (Some(input_rate), Some(output_rate), Some(read_rate)) =
            (rates.input, rates.output, rates.cache_read)
        else {
            return unpriced(MissingRate);
        };
        let mut components = vec![
            (input, input_rate),
            (output, output_rate),
            (read, read_rate),
        ];
        match model.cache_write_mode {
            CacheWriteMode::Flat => {
                let Some(rate) = rates.cache_write else {
                    return unpriced(MissingRate);
                };
                components.push((write, rate));
            }
            CacheWriteMode::TtlSplit => {
                let (Some(five_rate), Some(hour_rate)) =
                    (rates.cache_write_5m, rates.cache_write_1h)
                else {
                    return unpriced(MissingRate);
                };
                if write == 0 {
                    if self.split.iter().flatten().any(|n| *n != 0) {
                        return unpriced(InconsistentCacheSplit);
                    }
                } else {
                    let [Some(five), Some(hour)] = self.split else {
                        return unpriced(MissingCacheSplit);
                    };
                    if u128::from(five) + u128::from(hour) != u128::from(write) {
                        return unpriced(InconsistentCacheSplit);
                    }
                    components.extend([(five, five_rate), (hour, hour_rate)]);
                }
            }
        }
        let mut nano_usd = 0_u128;
        for (tokens, rate) in components {
            nano_usd = nano_usd
                .checked_add(
                    u128::from(tokens)
                        .checked_mul(u128::from(rate))
                        .ok_or(Error::CounterOverflow)?,
                )
                .ok_or(Error::CounterOverflow)?;
        }
        Ok(Outcome::Priced(nano_usd, assumed))
    }
}
#[derive(Clone, Default)]
pub(crate) struct Totals {
    observations: u64,
    priced: u64,
    assumed: u64,
    nano_usd: u128,
    unpriced: BTreeMap<(Option<String>, Option<String>, UnpricedReason), u64>,
}
impl Totals {
    pub(crate) fn push(&mut self, row: &Observation, outcome: Outcome) -> Result<()> {
        self.observations += 1;
        match outcome {
            Outcome::Priced(value, assumed) => {
                self.priced += 1;
                self.assumed += u64::from(assumed);
                self.nano_usd = self
                    .nano_usd
                    .checked_add(value)
                    .ok_or(Error::CounterOverflow)?;
            }
            Outcome::Unpriced(reason) => {
                *self
                    .unpriced
                    .entry((row.model.clone(), row.tier.clone(), reason))
                    .or_default() += 1
            }
        }
        Ok(())
    }
    pub(crate) fn finish(self) -> CostSummary {
        let subtotal = self.nano_usd as f64 / 1_000_000_000.0;
        CostSummary {
            selected_observations: self.observations,
            priced_observations: self.priced,
            unpriced_observations: self.observations - self.priced,
            assumed_tier_observations: self.assumed,
            total_usd: (self.observations > 0 && self.observations == self.priced)
                .then_some(subtotal),
            priced_subtotal_usd: subtotal,
            priced_subtotal_nano_usd: self.nano_usd,
            unpriced: self
                .unpriced
                .into_iter()
                .map(
                    |((model, service_tier, reason), observations)| UnpricedCost {
                        model,
                        service_tier,
                        reason,
                        observations,
                    },
                )
                .collect(),
        }
    }
}
impl MetricsDb {
    /// Price current selected response observations once from one read snapshot.
    /// Enrichment, representative replacement and catalog changes need no stored cost.
    pub fn cost(
        &self,
        window: Window,
        zone: TimeZone,
        catalog: &PriceCatalog,
    ) -> Result<CostReport> {
        let days = window.local_days(zone)?;
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let day_ends: Vec<_> = days
            .iter()
            .map(|d| InstantKey::from_millisecond(d.window.end_ms()))
            .collect();
        let mut daily: Vec<Totals> = days.iter().map(|_| Totals::default()).collect();
        let mut total = Totals::default();
        let mut hosts = BTreeMap::<String, Totals>::new();
        let mut models = BTreeMap::<Option<String>, Totals>::new();
        let mut surfaces = BTreeMap::<(String, Option<String>), Totals>::new();
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let (ts, surface, observation) = read_row(row)?;
            if ts < start || ts >= end {
                continue;
            }
            let host = observation.host.clone();
            let priced = observation.price(catalog)?;
            total.push(&observation, priced)?;
            hosts
                .entry(host.clone())
                .or_default()
                .push(&observation, priced)?;
            models
                .entry(observation.model.clone())
                .or_default()
                .push(&observation, priced)?;
            surfaces
                .entry((host, surface))
                .or_default()
                .push(&observation, priced)?;
            daily[day_ends.partition_point(|end| end <= &ts)].push(&observation, priced)?;
        }
        Ok(CostReport {
            price_version: catalog.version().into(),
            price_as_of: catalog.as_of().into(),
            basis: COST_BASIS.into(),
            total: total.finish(),
            by_host: hosts
                .into_iter()
                .map(|(host, t)| HostCost {
                    host,
                    cost: t.finish(),
                })
                .collect(),
            by_model: models
                .into_iter()
                .map(|(model, t)| ModelCost {
                    model,
                    cost: t.finish(),
                })
                .collect(),
            by_surface: surfaces
                .into_iter()
                .map(|((host, surface), t)| SurfaceCost {
                    host,
                    surface,
                    cost: t.finish(),
                })
                .collect(),
            by_day: days
                .into_iter()
                .zip(daily)
                .map(|(d, t)| DayCost {
                    date: d.date.to_string(),
                    start_ms: d.window.start_ms(),
                    end_ms: d.window.end_ms(),
                    cost: t.finish(),
                })
                .collect(),
        })
    }
}

impl MetricsDb {
    /// The cost of exactly the named sessions' selected responses over their
    /// whole history, whenever each was: the same `v_response_usage`
    /// selection, deduplication and pricing as [`MetricsDb::cost`], restricted
    /// to a session but not to a window. A response belongs to the session
    /// that owns its record (a Claude Code sidechain belongs to its parent
    /// session; a separately indexed sub-session is its own). A response with
    /// no timestamp is in no window, so no other cost read sees it; here it is
    /// counted as unpriced ([`UnpricedReason::MissingTimestamp`]), so the
    /// session's total is a floor rather than silently short.
    ///
    /// Only indexed user sessions get an entry; an unknown identifier is
    /// absent rather than an invented zero. An indexed session with no
    /// selected response reports zero selected observations and no total.
    /// Bounded like [`MetricsDb::session_windows`].
    pub fn whole_session_costs(
        &self,
        sessions: &[&str],
        catalog: &PriceCatalog,
    ) -> Result<BTreeMap<String, CostSummary>> {
        let mut ids: Vec<&str> = sessions.to_vec();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > crate::session::MAX_SESSIONS {
            return Err(Error::TooManySessions);
        }
        if ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        // No window, so the identifiers are bound from ?1.
        let parameters = rusqlite::params_from_iter(ids.iter());
        let mut totals = BTreeMap::<String, Totals>::new();
        let mut statement = self.connection.prepare(&crate::session::by_session_from(
            crate::session::EXISTS_QUERY,
            ids.len(),
            1,
        ))?;
        let mut rows = statement.query(parameters.clone())?;
        while let Some(row) = rows.next()? {
            totals.insert(row.get(0)?, Totals::default());
        }
        drop(rows);
        drop(statement);
        if totals.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut statement = self.connection.prepare(&crate::session::by_session_from(
            WHOLE_SESSION_QUERY,
            ids.len(),
            1,
        ))?;
        let mut rows = statement.query(parameters)?;
        while let Some(row) = rows.next()? {
            let session: String = row.get(11)?;
            let Some(total) = totals.get_mut(&session) else {
                continue;
            };
            if row.get_ref(3)?.data_type() == rusqlite::types::Type::Null {
                let observation = read_observation(row)?;
                total.push(
                    &observation,
                    Outcome::Unpriced(UnpricedReason::MissingTimestamp),
                )?;
            } else {
                let (_, _, observation) = read_row(row)?;
                let priced = observation.price(catalog)?;
                total.push(&observation, priced)?;
            }
        }
        Ok(totals
            .into_iter()
            .map(|(id, total)| (id, total.finish()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use serde_json::json;
    #[test]
    fn cost_single_snapshot_preserves_all_breakdowns_and_uses_index() {
        let fixture = xt_fixtures::Fixture::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"),
        )
        .unwrap();
        let catalog = PriceCatalog::from_json(&fixture.snapshots()["prices"].to_string()).unwrap();
        let mut db = xt_fixtures::TempDb::empty().unwrap();
        let session = xt_store::SessionMeta::new("s", "codex", xt_store::SessionSource::Fixture);
        db.store_mut().upsert_session(&session, false).unwrap();
        let rows:Vec<xt_store::CanonicalRecord>=(0..2).map(|i|serde_json::from_value(json!({"uuid":format!("cost-{i}"),"type":"assistant","timestamp":"2026-09-07T12:00:00Z","message":{"model":"test-flat","usage":{"input_tokens":0,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"service_tier":"standard"}}})).unwrap()).collect();
        db.store_mut().upsert_records("s", &rows, false).unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let plan: Vec<String> = metrics
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {QUERY}"))
            .unwrap()
            .query_map([0, 1000], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(plan.iter().any(|s| s.contains("records_ts")), "{plan:?}");
        Connection::open(db.path()).unwrap().execute_batch("DROP VIEW v_response_usage; CREATE VIEW v_response_usage AS SELECT host,model,surface,cost_snapshot_write(ts) AS ts,ts_ms,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,cache_creation_5m,cache_creation_1h,service_tier FROM v_usage_records").unwrap();
        let path = db.path().to_owned();
        let wrote = std::sync::atomic::AtomicBool::new(false);
        metrics
            .connection
            .create_scalar_function(
                "cost_snapshot_write",
                1,
                FunctionFlags::SQLITE_UTF8,
                move |context| {
                    if !wrote.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        Connection::open(&path)?
                            .execute("UPDATE usage SET output_tokens=100", [])?;
                    }
                    context.get::<String>(0)
                },
            )
            .unwrap();
        let window = Window::new(1788220800000, 1788825600000).unwrap();
        let old = metrics.cost(window, TimeZone::UTC, &catalog).unwrap();
        assert_eq!(old.total.total_usd, Some(0.00002));
        assert_eq!(old.by_host[0].cost, old.total);
        assert_eq!(old.by_model[0].cost, old.total);
        assert_eq!(old.by_surface[0].cost, old.total);
        assert_eq!(old.by_day.last().unwrap().cost, old.total);
        assert_eq!(
            metrics
                .cost(window, TimeZone::UTC, &catalog)
                .unwrap()
                .total
                .total_usd,
            Some(0.002)
        );
    }
    #[test]
    fn whole_session_costs_price_every_response_whenever_it_was() {
        let fixture = xt_fixtures::Fixture::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"),
        )
        .unwrap();
        let catalog = PriceCatalog::from_json(&fixture.snapshots()["prices"].to_string()).unwrap();
        let mut db = xt_fixtures::TempDb::empty().unwrap();
        let record = |uuid: &str, ts: Option<&str>, model: &str| -> xt_store::CanonicalRecord {
            let mut value = json!({"uuid":uuid,"type":"assistant","message":{"model":model,"usage":{"input_tokens":0,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"service_tier":"standard"}}});
            if let Some(ts) = ts {
                value["timestamp"] = json!(ts);
            }
            serde_json::from_value(value).unwrap()
        };
        for (id, rows) in [
            (
                "long",
                vec![
                    // Years apart, both counted; one has no published price.
                    record("l1", Some("2020-01-01T00:00:00Z"), "test-flat"),
                    record("l2", Some("2026-09-07T12:00:00Z"), "test-flat"),
                    record("l3", Some("2026-09-07T13:00:00Z"), "not-in-catalog"),
                    // No timestamp: counted, unpriced, never dropped.
                    record("l4", None, "test-flat"),
                ],
            ),
            (
                "other",
                vec![record("o1", Some("2026-09-07T12:00:00Z"), "test-flat")],
            ),
            (
                "unpriced",
                vec![record("u1", Some("2026-09-07T12:00:00Z"), "not-in-catalog")],
            ),
        ] {
            let session = xt_store::SessionMeta::new(id, "codex", xt_store::SessionSource::Fixture);
            db.store_mut().upsert_session(&session, false).unwrap();
            db.store_mut().upsert_records(id, &rows, false).unwrap();
        }
        let empty = xt_store::SessionMeta::new("empty", "codex", xt_store::SessionSource::Fixture);
        db.store_mut().upsert_session(&empty, false).unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let costs = metrics
            .whole_session_costs(
                &["long", "absent", "long", "other", "unpriced", "empty"],
                &catalog,
            )
            .unwrap();
        // An unknown identifier is absent, never an invented zero.
        assert_eq!(
            costs.keys().map(String::as_str).collect::<Vec<_>>(),
            ["empty", "long", "other", "unpriced"]
        );
        let long = &costs["long"];
        assert_eq!(
            (
                long.selected_observations,
                long.priced_observations,
                long.unpriced_observations,
                long.total_usd,
                long.priced_subtotal_nano_usd
            ),
            (4, 2, 2, None, 20_000)
        );
        assert_eq!(
            long.unpriced
                .iter()
                .map(|u| (u.model.as_deref(), u.reason, u.observations))
                .collect::<Vec<_>>(),
            [
                (Some("not-in-catalog"), UnpricedReason::UnknownModel, 1),
                (Some("test-flat"), UnpricedReason::MissingTimestamp, 1),
            ]
        );
        assert_eq!(
            (
                costs["empty"].selected_observations,
                costs["empty"].total_usd
            ),
            (0, None)
        );
        assert_eq!(
            (
                costs["unpriced"].priced_observations,
                costs["unpriced"].unpriced_observations
            ),
            (0, 1)
        );
        // Every timed response is the cost report's own: over a window that
        // holds them all, the sessions add up to its total exactly, in
        // nano-USD, never as floating-point sums.
        let all = Window::new(946684800000, 1893456000000).unwrap(); // 2000 through 2029.
        let total = metrics.cost(all, TimeZone::UTC, &catalog).unwrap().total;
        let sum: u128 = costs.values().map(|c| c.priced_subtotal_nano_usd).sum();
        assert_eq!(sum, total.priced_subtotal_nano_usd);
        assert_eq!(
            costs.values().map(|c| c.selected_observations).sum::<u64>(),
            total.selected_observations + 1,
            "only the untimed response is beyond every window"
        );
        // Read through the session index, not a scan of all history.
        let plan: Vec<String> = metrics
            .connection
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {}",
                crate::session::by_session_from(WHOLE_SESSION_QUERY, 1, 1)
            ))
            .unwrap()
            .query_map(["long"], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            plan.iter().any(|s| s.contains("records_session_ts")),
            "{plan:?}"
        );
    }
}
