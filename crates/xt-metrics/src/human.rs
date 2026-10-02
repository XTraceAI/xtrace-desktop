use crate::{Error, MetricsDb, Result, TypingRate, Window};
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str = "SELECT session_id,uuid,ts,ts_ms,is_human,text_len
    FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";
const MAX_GAP_MS: i128 = 30 * 60 * 1000;

/// Human wall time is estimated and unioned across sessions. The comparison sums
/// each session's union, not its overlapping raw intervals. Unknown classification
/// or a needed typing length leaves both human totals and the ratio unknown.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HumanTime {
    pub human_minutes_est: Option<f64>,
    pub summed_session_minutes_est: Option<f64>,
    pub agent_minutes: f64,
    pub agent_to_human_ratio: Option<f64>,
}

struct Event {
    instant: InstantKey,
    uuid: String,
    ms: i64,
    human: Option<bool>,
    chars: Option<i64>,
}

// Coordinates are milliseconds multiplied by the common positive typing rate.
// A typing duration is therefore exactly chars * 60_000 in this private domain.
// Subtraction/union happens before conversion to f64, even at large epochs.
type Interval = (i128, i128);
fn union_duration(intervals: &mut [Interval]) -> Result<i128> {
    intervals.sort_unstable();
    let mut end = None;
    let mut total = 0_i128;
    for &(start, stop) in intervals.iter() {
        let start = end.map_or(start, |previous| start.max(previous));
        if stop > start {
            total = total
                .checked_add(stop.checked_sub(start).ok_or(Error::CounterOverflow)?)
                .ok_or(Error::CounterOverflow)?;
        }
        end = Some(end.map_or(stop, |previous| stop.max(previous)));
    }
    Ok(total)
}

impl MetricsDb {
    /// Both human intervals and the existing active-span numerator observe one
    /// read-only snapshot. If a caller already owns a transaction, reuse it.
    pub fn human_time(&self, window: Window, typing_rate: TypingRate) -> Result<HumanTime> {
        let snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let agent_minutes = self.active_spans(window)?.active_ms as f64 / 60_000.0;
        let human = self.human_durations(window, typing_rate)?;
        let scale = f64::from(typing_rate.characters_per_minute()) * 60_000.0;
        let human_minutes_est = human.map(|(wall, _)| wall as f64 / scale);
        let summed_session_minutes_est = human.map(|(_, summed)| summed as f64 / scale);
        let agent_to_human_ratio = human_minutes_est
            .filter(|minutes| *minutes > 0.0)
            .map(|minutes| agent_minutes / minutes);
        let report = HumanTime {
            human_minutes_est,
            summed_session_minutes_est,
            agent_minutes,
            agent_to_human_ratio,
        };
        if let Some(snapshot) = snapshot {
            snapshot.commit()?;
        }
        Ok(report)
    }

    fn human_durations(
        &self,
        window: Window,
        typing_rate: TypingRate,
    ) -> Result<Option<(i128, i128)>> {
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let rate = i128::from(typing_rate.characters_per_minute());
        let scaled = |ms: i64| {
            i128::from(ms)
                .checked_mul(rate)
                .ok_or(Error::CounterOverflow)
        };
        let lower = scaled(window.start_ms())?;
        let upper = scaled(window.end_ms())?;
        let mut sessions = BTreeMap::<String, Vec<Event>>::new();
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(2)?;
            let instant = timestamp::parse(&raw)
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
            sessions.entry(row.get(0)?).or_default().push(Event {
                instant,
                uuid: row.get(1)?,
                ms: row.get(3)?,
                human: row.get(4)?,
                chars: row.get(5)?,
            });
        }
        let mut all = Vec::new();
        let mut summed = 0_i128;
        for events in sessions.values_mut() {
            events.sort_unstable_by(|a, b| {
                a.instant.cmp(&b.instant).then_with(|| a.uuid.cmp(&b.uuid))
            });
            let mut previous_agent = None;
            let mut intervals = Vec::new();
            for event in events {
                match event.human {
                    None => return Ok(None),
                    Some(false) => previous_agent = Some(event.ms),
                    Some(true) => {
                        let duration = if let Some(previous) = previous_agent {
                            i128::from(event.ms)
                                .checked_sub(i128::from(previous))
                                .ok_or(Error::CounterOverflow)?
                                .clamp(0, MAX_GAP_MS)
                                .checked_mul(rate)
                                .ok_or(Error::CounterOverflow)?
                        } else {
                            let Some(chars) = event.chars else {
                                return Ok(None);
                            };
                            let chars = u64::try_from(chars)
                                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, chars))?;
                            i128::from(chars)
                                .checked_mul(60_000)
                                .ok_or(Error::CounterOverflow)?
                        };
                        let stop = scaled(event.ms)?;
                        let begin = stop
                            .checked_sub(duration)
                            .ok_or(Error::CounterOverflow)?
                            .max(lower);
                        let stop = stop.min(upper);
                        if stop > begin {
                            intervals.push((begin, stop));
                        }
                    }
                }
            }
            summed = summed
                .checked_add(union_duration(&mut intervals)?)
                .ok_or(Error::CounterOverflow)?;
            all.extend(intervals);
        }
        Ok(Some((union_duration(&mut all)?, summed)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::path::PathBuf;
    fn fixture() -> xt_fixtures::TempDb {
        xt_fixtures::Fixture::load(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"),
        )
        .unwrap()
        .build_db(false)
        .unwrap()
    }
    fn window() -> Window {
        Window::new(1788220800000, 1788825600000).unwrap()
    }
    #[test]
    fn human_intervals_snapshot_keeps_both_reads_consistent_during_writer_commit() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let writer = Connection::open(db.path()).unwrap();
        writer.execute_batch("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT session_id,host,ts_ms,human_snapshot_write(ts) AS ts,uuid,is_human,text_len,role,tool_use_count FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
        let path = db.path().to_owned();
        let written = std::sync::atomic::AtomicBool::new(false);
        metrics
            .connection
            .create_scalar_function(
                "human_snapshot_write",
                1,
                FunctionFlags::SQLITE_UTF8,
                move |context| {
                    if !written.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        Connection::open(&path)?.execute("UPDATE records SET is_human=0", [])?;
                    }
                    context.get::<String>(0)
                },
            )
            .unwrap();
        let report = metrics.human_time(window(), TypingRate::default()).unwrap();
        assert_eq!(report.human_minutes_est, Some(8.135));
        assert_eq!(report.agent_to_human_ratio, Some(23.0 / 8.135));
        assert!(metrics.connection.is_autocommit());
        assert_eq!(
            metrics
                .human_time(window(), TypingRate::default())
                .unwrap()
                .human_minutes_est,
            Some(0.0)
        );
    }
    #[test]
    fn human_intervals_reuses_existing_read_transaction_without_ending_it() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let transaction = metrics.connection.unchecked_transaction().unwrap();
        let count: i64 = transaction
            .query_row("SELECT count(*) FROM records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 25);
        Connection::open(db.path())
            .unwrap()
            .execute("UPDATE records SET is_human=0", [])
            .unwrap();
        assert_eq!(
            metrics
                .human_time(window(), TypingRate::default())
                .unwrap()
                .human_minutes_est,
            Some(8.135)
        );
        assert!(!metrics.connection.is_autocommit());
        transaction.commit().unwrap();
        assert_eq!(
            metrics
                .human_time(window(), TypingRate::default())
                .unwrap()
                .human_minutes_est,
            Some(0.0)
        );
    }
    #[test]
    fn union_adjacent_nested_disjoint_and_checked_overflow() {
        assert_eq!(
            union_duration(&mut [(0, 10), (5, 8), (10, 20), (30, 40)]).unwrap(),
            30
        );
        assert_eq!(union_duration(&mut []).unwrap(), 0);
        assert!(matches!(
            union_duration(&mut [(i128::MIN, i128::MAX)]),
            Err(Error::CounterOverflow)
        ));
    }
}
