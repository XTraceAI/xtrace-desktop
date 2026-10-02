use crate::{MetricsDb, Result, Window};
use jiff::tz::TimeZone;
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str =
    "SELECT session_id,ts FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DaySessions {
    pub date: String,
    pub sessions: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SessionsPerDay {
    pub days: Vec<DaySessions>,
    pub total_sessions: u64,
    pub max_per_day: u64,
    pub mean_per_day: f64,
}
impl MetricsDb {
    /// Count each session on its first exact in-window event. The mean divides
    /// by reported local date buckets, including zero and partial calendar days;
    /// DST-short/long days each contribute one bucket, not elapsed 24-hour units.
    pub fn sessions_per_day(&self, window: Window, zone: TimeZone) -> Result<SessionsPerDay> {
        let buckets = window.local_days(zone)?;
        let ends: Vec<_> = buckets
            .iter()
            .map(|day| InstantKey::from_millisecond(day.window.end_ms()))
            .collect();
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut first = BTreeMap::<String, InstantKey>::new();
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(1)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant < start || instant >= end {
                continue;
            }
            let session: String = row.get(0)?;
            match first.entry(session) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(instant);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if instant < *entry.get() {
                        entry.insert(instant);
                    }
                }
            }
        }
        let total_sessions = first.len() as u64;
        let mut days: Vec<_> = buckets
            .into_iter()
            .map(|day| DaySessions {
                date: day.date.to_string(),
                sessions: 0,
            })
            .collect();
        for instant in first.values() {
            let day = ends.partition_point(|end| end <= instant);
            days[day].sessions += 1;
        }
        Ok(SessionsPerDay {
            total_sessions,
            max_per_day: days.iter().map(|day| day.sessions).max().unwrap_or(0),
            mean_per_day: total_sessions as f64 / days.len() as f64,
            days,
        })
    }
}
