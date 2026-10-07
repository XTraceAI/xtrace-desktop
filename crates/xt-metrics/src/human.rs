use crate::{ActiveSpanReport, Error, MetricsDb, Result, TypingRate, Window};
use jiff::tz::TimeZone;
use serde::Serialize;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str = "SELECT ts,human_is_eligible,human_text_len,human_excluded
    FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";

/// Prompt-effort estimate: counted characters divided by the configured typing
/// rate. This is not measured attention or a claim that retained text was typed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HumanTime {
    pub human_minutes_est: Option<f64>,
    /// Compatibility field: character sums are additive across sessions, so
    /// this is always the same estimate as `human_minutes_est`.
    pub summed_session_minutes_est: Option<f64>,
    pub agent_minutes: f64,
    /// Each message belongs to the local day containing its exact timestamp.
    /// A missing classification or counted length makes every bucket unknown.
    pub by_day: Vec<DayHuman>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DayHuman {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub minutes_est: Option<f64>,
}

impl MetricsDb {
    /// Character totals and measured agent spans share one read-only snapshot.
    /// Exact source instants decide membership, including submillisecond and
    /// leap-second boundaries. No duration is clipped, capped or unioned.
    pub fn human_time(
        &self,
        window: Window,
        typing_rate: TypingRate,
        zone: TimeZone,
    ) -> Result<HumanTime> {
        self.read_snapshot(|db| db.read_human_time(window, typing_rate, zone, None))
    }

    /// [`MetricsDb::human_time`] inside the caller's snapshot. `measured` is
    /// this window's [`MetricsDb::active_spans`] already read in that same
    /// snapshot; without it the spans are read here.
    pub(crate) fn read_human_time(
        &self,
        window: Window,
        typing_rate: TypingRate,
        zone: TimeZone,
        measured: Option<&ActiveSpanReport>,
    ) -> Result<HumanTime> {
        let days = window.local_days(zone)?;
        let active_ms = match measured {
            Some(spans) => spans.active_ms,
            None => self.active_spans(window)?.active_ms,
        };
        let agent_minutes = active_ms as f64 / 60_000.0;
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let day_ends: Vec<_> = days
            .iter()
            .map(|day| InstantKey::from_millisecond(day.window.end_ms()))
            .collect();
        let mut total = Some(0_u64);
        let mut daily = vec![0_u64; days.len()];
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(0)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant < start || instant >= end || row.get::<_, bool>(3)? {
                continue;
            }
            match row.get::<_, Option<bool>>(1)? {
                Some(false) => {}
                None => total = None,
                Some(true) => {
                    let Some(chars) = crate::counts::counter(row, 2)? else {
                        total = None;
                        continue;
                    };
                    total = crate::counts::add(total, Some(chars))?;
                    let day = day_ends.partition_point(|end| *end <= instant);
                    daily[day] = daily[day]
                        .checked_add(chars)
                        .ok_or(Error::CounterOverflow)?;
                }
            }
        }
        let rate = f64::from(typing_rate.characters_per_minute());
        let human_minutes_est = total.map(|chars| chars as f64 / rate);
        Ok(HumanTime {
            human_minutes_est,
            summed_session_minutes_est: human_minutes_est,
            agent_minutes,
            by_day: days
                .iter()
                .zip(daily)
                .map(|(day, chars)| DayHuman {
                    date: day.date.to_string(),
                    start_ms: day.window.start_ms(),
                    end_ms: day.window.end_ms(),
                    minutes_est: total.map(|_| chars as f64 / rate),
                })
                .collect(),
        })
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
        writer.execute_batch("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT session_id,host,ts_ms,human_snapshot_write(ts) AS ts,uuid,is_human,text_len,human_is_eligible,human_text_len,human_excluded,role,tool_use_count,confirmed_automated_input FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
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
        let report = metrics
            .human_time(window(), TypingRate::default(), TimeZone::UTC)
            .unwrap();
        assert_eq!(report.human_minutes_est, Some(0.675));
        assert_eq!(report.agent_minutes, 23.0);
        assert!(metrics.connection.is_autocommit());
        assert_eq!(
            metrics
                .human_time(window(), TypingRate::default(), TimeZone::UTC)
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
                .human_time(window(), TypingRate::default(), TimeZone::UTC)
                .unwrap()
                .human_minutes_est,
            Some(0.675)
        );
        assert!(!metrics.connection.is_autocommit());
        transaction.commit().unwrap();
        assert_eq!(
            metrics
                .human_time(window(), TypingRate::default(), TimeZone::UTC)
                .unwrap()
                .human_minutes_est,
            Some(0.0)
        );
    }
}
