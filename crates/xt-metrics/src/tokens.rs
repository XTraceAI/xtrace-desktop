use crate::{Error, MetricsDb, Result, Window};
use jiff::tz::TimeZone;
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str = "SELECT session_id,host,model,surface,ts,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens
             FROM v_response_usage WHERE ts_ms>=?1 AND ts_ms<?2";

// Chrono's coarse projection puts a leap second into the following POSIX
// second. Include that overlap, then filter exact instants before accumulation.
fn candidate_end_ms(window: Window) -> Result<i64> {
    window
        .end_ms()
        .checked_add(1000)
        .ok_or(Error::InvalidWindow)
}

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
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let day_ends: Vec<_> = days
            .iter()
            .map(|day| InstantKey::from_millisecond(day.window.end_ms()))
            .collect();
        let mut daily: Vec<Totals> = days.iter().map(|_| Totals::default()).collect();
        let mut total = Totals::default();
        let mut hosts = BTreeMap::<String, Totals>::new();
        let mut models = BTreeMap::<Option<String>, Totals>::new();
        let mut surfaces = BTreeMap::<(String, Option<String>), Totals>::new();
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), candidate_end_ms(window)?])?;
        while let Some(row) = rows.next()? {
            let raw_ts: String = row.get(4)?;
            let ts = timestamp::parse(&raw_ts)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if ts < start || ts >= end {
                continue;
            }
            let session: String = row.get(0)?;
            let host: String = row.get(1)?;
            let model: Option<String> = row.get(2)?;
            let surface: Option<String> = row.get(3)?;
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
            let day = day_ends.partition_point(|end| end <= &ts);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn tokens_narrow_window_work_is_independent_of_irrelevant_history() {
        let mut steps = Vec::new();
        for count in [100, 1000] {
            let fixture = xt_fixtures::Fixture::load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"),
            )
            .unwrap();
            let mut db = xt_fixtures::TempDb::empty().unwrap();
            let session = &fixture.sessions()[0].metadata;
            db.store_mut().upsert_session(session, false).unwrap();
            let row: xt_store::CanonicalRecord = serde_json::from_value(json!({"uuid":"row","type":"assistant","timestamp":"2026-09-07T12:00:00Z","message":{"role":"assistant","usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}})).unwrap();
            db.store_mut()
                .upsert_records(&session.session_id, std::slice::from_ref(&row), false)
                .unwrap();
            let window = Window::new(1788220800000, 1788825600000).unwrap();
            let bounds = [window.start_ms(), candidate_end_ms(window).unwrap()];
            let history: Vec<_> = (0..count)
                .map(|i| {
                    let mut record = row.clone();
                    record.uuid = Some(format!("history-{i:04}"));
                    record.api_message_id = Some(format!("response-{i:04}"));
                    record.request_id = Some("history-request".into());
                    record.timestamp = Some("2026-08-01T00:00:00Z".into());
                    record
                })
                .collect();
            db.store_mut()
                .upsert_records(&session.session_id, &history, false)
                .unwrap();
            let c = rusqlite::Connection::open(db.path()).unwrap();
            xt_store::timestamp::register_sqlite(&c).unwrap();
            c.execute(
                "UPDATE records SET api_message_id='candidate',request_id='candidate' WHERE uuid='row'",
                [],
            ).unwrap();
            let plan: Vec<String> = c
                .prepare(&format!("EXPLAIN QUERY PLAN {QUERY}"))
                .unwrap()
                .query_map(bounds, |row| row.get(3))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            assert!(
                plan.iter().any(|line| line.contains("records_ts")),
                "{plan:?}"
            );
            assert!(
                plan.iter().any(|line| line.contains("records_response")),
                "{plan:?}"
            );
            let mut statement = c.prepare(QUERY).unwrap();
            let selected: Vec<String> = statement
                .query_map(bounds, |r| r.get(0))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            assert_eq!(selected, std::slice::from_ref(&session.session_id));
            steps.push(statement.get_status(rusqlite::StatementStatus::VmStep));
        }
        eprintln!("100/1000 irrelevant observations: {steps:?} VM steps");
        assert!(
            steps[1] <= steps[0] * 2,
            "narrow window scanned irrelevant history: {steps:?}"
        );
    }
}
