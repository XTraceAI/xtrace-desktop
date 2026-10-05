use crate::{Error, MetricsDb, Result, Window};
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str =
    "SELECT session_id,uuid,ts,role,is_human,human_text_len,tool_use_count,
    confirmed_automated_input,human_is_eligible FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";

/// Positive characters per minute for the explicitly estimated typing metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypingRate(u32);
impl TypingRate {
    pub fn new(characters_per_minute: u32) -> Result<Self> {
        if characters_per_minute == 0 {
            return Err(Error::InvalidTypingRate);
        }
        Ok(Self(characters_per_minute))
    }
    pub fn characters_per_minute(self) -> u32 {
        self.0
    }
}
impl Default for TypingRate {
    fn default() -> Self {
        Self(200)
    }
}

/// Local actual-work counts. No private record or session identifiers leave this
/// summary. None means a required stored measurement was unknown, never zero.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Counts {
    pub sessions: u64,
    pub human_messages: Option<u64>,
    pub assistant_turns: Option<u64>,
    pub assistant_records: u64,
    pub tool_calls: Option<u64>,
    pub human_chars: Option<u64>,
    pub typing_minutes_est: Option<f64>,
}

struct Event {
    instant: InstantKey,
    uuid: String,
    human: Option<bool>,
    /// A confirmed automated input: never a human message, and never the
    /// response that closes a turn or an interruption of one.
    automated: bool,
}

pub(crate) fn add(total: Option<u64>, value: Option<u64>) -> Result<Option<u64>> {
    match (total, value) {
        (Some(a), Some(b)) => a.checked_add(b).map(Some).ok_or(Error::CounterOverflow),
        _ => Ok(None),
    }
}
pub(crate) fn counter(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(column)?
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
        })
        .transpose()
}

impl MetricsDb {
    /// Filter exact native instants before per-session segmentation. Every
    /// structural block survives response-usage deduplication. Leading agent
    /// records and trailing unanswered humans are not responding turns.
    /// Classification remains the ingest owner's responsibility: this reader
    /// never reparses content or infers a missing is_human value from role.
    /// A confirmed automated input is an actual record but not a human message:
    /// it adds no human message or characters, and neither answers a pending
    /// turn nor starts one.
    pub fn counts(&self, window: Window, typing_rate: TypingRate) -> Result<Counts> {
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut sessions = BTreeMap::<String, Vec<Event>>::new();
        let mut counts = Counts {
            sessions: 0,
            human_messages: Some(0),
            assistant_turns: Some(0),
            assistant_records: 0,
            tool_calls: Some(0),
            human_chars: Some(0),
            typing_minutes_est: Some(0.0),
        };
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let raw_ts: String = row.get(2)?;
            let instant = timestamp::parse(&raw_ts)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant < start || instant >= end {
                continue;
            }
            let human: Option<bool> = row.get(4)?;
            // Agent turns retain the existing classification and its unknowns,
            // independently of Human-only exclusions or retained lengths.
            if human.is_none() {
                counts.assistant_turns = None;
            }
            counts.assistant_records +=
                u64::from(row.get::<_, Option<String>>(3)?.as_deref() == Some("assistant"));
            counts.tool_calls = add(counts.tool_calls, counter(row, 6)?)?;
            match row.get::<_, Option<bool>>(8)? {
                Some(true) => {
                    counts.human_messages = add(counts.human_messages, Some(1))?;
                    counts.human_chars = add(counts.human_chars, counter(row, 5)?)?;
                }
                Some(false) => {}
                None => {
                    counts.human_messages = None;
                    counts.human_chars = None;
                }
            }
            sessions.entry(row.get(0)?).or_default().push(Event {
                instant,
                uuid: row.get(1)?,
                human,
                automated: row.get(7)?,
            });
        }
        counts.sessions = sessions.len() as u64;
        for events in sessions.values_mut() {
            events.sort_unstable_by(|a, b| {
                a.instant.cmp(&b.instant).then_with(|| a.uuid.cmp(&b.uuid))
            });
            let mut awaiting_response = false;
            for event in events.iter().filter(|event| !event.automated) {
                match event.human {
                    Some(true) => awaiting_response = true,
                    Some(false) if awaiting_response => {
                        counts.assistant_turns = add(counts.assistant_turns, Some(1))?;
                        awaiting_response = false;
                    }
                    _ => {}
                }
            }
        }
        counts.typing_minutes_est = counts
            .human_chars
            .map(|chars| chars as f64 / f64::from(typing_rate.0));
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_candidates_use_the_event_timestamp_index() {
        let db = xt_fixtures::TempDb::empty().unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let plan: Vec<String> = metrics
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {QUERY}"))
            .unwrap()
            .query_map([0, 1000], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            plan.iter().any(|line| line.contains("records_ts")),
            "{plan:?}"
        );
    }
}
