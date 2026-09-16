use crate::{Error, MetricsDb, Result, Window};
use jiff::tz::TimeZone;
use serde::Serialize;
use std::collections::BTreeMap;

pub(crate) const QUERY: &str = "SELECT session_id,host,model,surface,ts_ms,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens
             FROM v_response_usage WHERE ts_ms>=?1 AND ts_ms<?2";

/// None is unknown, not zero. Components are independently measurable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenCounters {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenSummary {
    pub selected_responses: u64,
    pub measured_responses: u64,
    pub sessions: u64,
    pub measured_sessions: u64,
    pub counters: TokenCounters,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostTokens {
    pub host: String,
    pub tokens: TokenSummary,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelTokens {
    pub model: Option<String>,
    pub tokens: TokenSummary,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SurfaceTokens {
    pub host: String,
    pub surface: Option<String>,
    pub tokens: TokenSummary,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DayTokens {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub tokens: TokenSummary,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenReport {
    pub total: TokenSummary,
    pub by_host: Vec<HostTokens>,
    pub by_model: Vec<ModelTokens>,
    pub by_surface: Vec<SurfaceTokens>,
    pub by_day: Vec<DayTokens>,
}

#[derive(Default)]
struct Totals {
    values: [Option<u64>; 4],
    responses: u64,
    measured: u64,
    sessions: BTreeMap<String, bool>,
}
fn add(a: Option<u64>, b: Option<u64>) -> Result<Option<u64>> {
    match (a, b) {
        (Some(a), Some(b)) => a.checked_add(b).map(Some).ok_or(Error::CounterOverflow),
        _ => Ok(None),
    }
}
impl Totals {
    fn push(&mut self, session: &str, values: [Option<u64>; 4]) -> Result<()> {
        let measured = values.iter().all(Option::is_some);
        if self.responses == 0 {
            self.values = [Some(0); 4];
        }
        self.responses += 1;
        self.measured += u64::from(measured);
        self.sessions
            .entry(session.to_owned())
            .and_modify(|v| *v &= measured)
            .or_insert(measured);
        for (total, value) in self.values.iter_mut().zip(values) {
            *total = add(*total, value)?;
        }
        Ok(())
    }
    fn finish(self) -> Result<TokenSummary> {
        let [
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
        ] = self.values;
        let total_tokens = self.values.into_iter().try_fold(Some(0), add)?;
        Ok(TokenSummary {
            selected_responses: self.responses,
            measured_responses: self.measured,
            sessions: self.sessions.len() as u64,
            measured_sessions: self.sessions.values().filter(|v| **v).count() as u64,
            counters: TokenCounters {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                total_tokens,
            },
        })
    }
}
impl MetricsDb {
    /// Select response snapshots before applying the event window. All breakdowns
    /// share one SQLite statement/snapshot and one checked, nullable accumulator.
    pub fn tokens(&self, window: Window, zone: TimeZone) -> Result<TokenReport> {
        let days = window.local_days(zone)?;
        let mut daily: Vec<Totals> = days.iter().map(|_| Totals::default()).collect();
        let mut total = Totals::default();
        let mut hosts = BTreeMap::<String, Totals>::new();
        let mut models = BTreeMap::<Option<String>, Totals>::new();
        let mut surfaces = BTreeMap::<(String, Option<String>), Totals>::new();
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.end_ms()])?;
        while let Some(row) = rows.next()? {
            let session: String = row.get(0)?;
            let host: String = row.get(1)?;
            let model: Option<String> = row.get(2)?;
            let surface: Option<String> = row.get(3)?;
            let ts: i64 = row.get(4)?;
            let mut values = [None; 4];
            for (i, value) in values.iter_mut().enumerate() {
                *value = row
                    .get::<_, Option<i64>>(i + 5)?
                    .map(|v| {
                        u64::try_from(v)
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(i + 5, v))
                    })
                    .transpose()?;
            }
            total.push(&session, values)?;
            hosts
                .entry(host.clone())
                .or_default()
                .push(&session, values)?;
            models.entry(model).or_default().push(&session, values)?;
            surfaces
                .entry((host, surface))
                .or_default()
                .push(&session, values)?;
            let day = days.partition_point(|d| d.window.end_ms() <= ts);
            daily[day].push(&session, values)?;
        }
        Ok(TokenReport {
            total: total.finish()?,
            by_host: hosts
                .into_iter()
                .map(|(host, t)| {
                    Ok(HostTokens {
                        host,
                        tokens: t.finish()?,
                    })
                })
                .collect::<Result<_>>()?,
            by_model: models
                .into_iter()
                .map(|(model, t)| {
                    Ok(ModelTokens {
                        model,
                        tokens: t.finish()?,
                    })
                })
                .collect::<Result<_>>()?,
            by_surface: surfaces
                .into_iter()
                .map(|((host, surface), t)| {
                    Ok(SurfaceTokens {
                        host,
                        surface,
                        tokens: t.finish()?,
                    })
                })
                .collect::<Result<_>>()?,
            by_day: days
                .into_iter()
                .zip(daily)
                .map(|(d, t)| {
                    Ok(DayTokens {
                        date: d.date.to_string(),
                        start_ms: d.window.start_ms(),
                        end_ms: d.window.end_ms(),
                        tokens: t.finish()?,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }
}
