use crate::{DayBucket, Error, MetricsDb, Result, Window, window::allocate};
use jiff::tz::TimeZone;
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str =
    "SELECT session_id,host,ts_ms,ts,uuid FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";

pub(crate) const MAX_GAP_MS: i64 = 20 * 60 * 1000;

/// The single M-05 gap rule: one session's timeline, already ordered on the
/// POSIX millisecond axis, folded into spans. A gap strictly greater than
/// twenty minutes starts a new span and contributes no active time.
pub(crate) fn fold(
    ordered: &[(i64, InstantKey, String)],
    mut span: impl FnMut(i64, i64),
) -> Result<u64> {
    let mut active_ms = 0_u64;
    let (mut start, mut end) = (ordered[0].0, ordered[0].0);
    for (ms, _, _) in ordered.iter().skip(1) {
        let gap = ms.checked_sub(end).ok_or(Error::CounterOverflow)?;
        if gap > MAX_GAP_MS {
            span(start, end);
            start = *ms;
        } else {
            active_ms = active_ms
                .checked_add(gap as u64)
                .ok_or(Error::CounterOverflow)?;
        }
        end = *ms;
    }
    span(start, end);
    Ok(active_ms)
}

/// Endpoints on the stored POSIX millisecond axis. Singletons and ties have zero
/// duration; endpoints are not expanded to the selected window's boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActiveSpan {
    pub session_id: String,
    pub host: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActiveSpanReport {
    pub spans: Vec<ActiveSpan>,
    /// Sum of span durations across sessions, including parallel activity.
    pub active_ms: u64,
}

/// One local calendar-day bucket of the selected window and the active time
/// allocated to it. `active_ms` sums to the report's `active_ms`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DayActive {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub active_ms: u64,
}

impl ActiveSpanReport {
    /// Split the spans this report already measured at the window's interior
    /// local-day boundaries, so each reported day carries the part of every
    /// span that falls inside it, parallel sessions still adding as M-05
    /// defines. Nothing is re-read and no span is re-folded: the same endpoints
    /// are allocated, so the days sum to `active_ms` exactly.
    ///
    /// A selected leap-second event projects onto the POSIX axis after the
    /// window's end, so a span can reach past the final bucket. That remainder
    /// is allocated to the last reported day; see `window::allocate`.
    pub fn by_day(&self, window: Window, zone: TimeZone) -> Result<Vec<DayActive>> {
        let days = window.local_days(zone)?;
        let ends: Vec<i128> = days
            .iter()
            .map(|day| i128::from(day.window.end_ms()))
            .collect();
        let mut totals = vec![0_i128; days.len()];
        for span in &self.spans {
            allocate(
                i128::from(window.start_ms()),
                &ends,
                (i128::from(span.start_ms), i128::from(span.end_ms)),
                &mut totals,
            )?;
        }
        day_active(&days, &totals)
    }
}

/// Pair the allocated durations with the buckets they were allocated over. The
/// two are the same length by construction and stay in window order.
fn day_active(days: &[DayBucket], totals: &[i128]) -> Result<Vec<DayActive>> {
    days.iter()
        .zip(totals)
        .map(|(day, active)| {
            Ok(DayActive {
                date: day.date.to_string(),
                start_ms: day.window.start_ms(),
                end_ms: day.window.end_ms(),
                active_ms: u64::try_from(*active).map_err(|_| Error::CounterOverflow)?,
            })
        })
        .collect()
}

impl MetricsDb {
    /// Exact native instants determine half-open window membership first. Only
    /// those events form each session's timeline, ordered by (ts_ms, precise
    /// instant, UUID). Gaps strictly greater than 20 minutes start a new span.
    /// Durations use the POSIX millisecond projection, not physical leap-second
    /// elapsed time: sorting on that axis prevents negative leap-adjacent gaps.
    /// All canonical event classes participate, including sidechain/tool rows.
    pub fn active_spans(&self, window: Window) -> Result<ActiveSpanReport> {
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut sessions = BTreeMap::<(String, String), Vec<(i64, InstantKey, String)>>::new();
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(3)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant >= start && instant < end {
                sessions
                    .entry((row.get(0)?, row.get(1)?))
                    .or_default()
                    .push((row.get(2)?, instant, row.get(4)?));
            }
        }
        let mut spans = Vec::new();
        let mut active_ms = 0_u64;
        for ((session_id, host), mut events) in sessions {
            events.sort_unstable();
            active_ms = active_ms
                .checked_add(fold(&events, |start_ms, end_ms| {
                    spans.push(ActiveSpan {
                        session_id: session_id.clone(),
                        host: host.clone(),
                        start_ms,
                        end_ms,
                    })
                })?)
                .ok_or(Error::CounterOverflow)?;
        }
        Ok(ActiveSpanReport { spans, active_ms })
    }

    /// Agent time on the last local day of `window`, measured exactly as a
    /// longer range's day bars measure that day: spans are folded with the
    /// events of up to one gap before `window` too (a span joins an earlier
    /// event only within the gap, so no older event can change them), then
    /// [`ActiveSpanReport::by_day`] splits them at local midnight. A span
    /// that began before the day counts from the day's start. Returns that
    /// day and the spans that reach into it.
    pub fn active_last_day(
        &self,
        window: Window,
        zone: TimeZone,
    ) -> Result<(DayActive, Vec<ActiveSpan>)> {
        let read = Window::new(
            window
                .start_ms()
                .checked_sub(MAX_GAP_MS)
                .ok_or(Error::InvalidWindow)?,
            window.end_ms(),
        )?;
        let report = self.active_spans(read)?;
        let day = report
            .by_day(read, zone)?
            .pop()
            .ok_or(Error::InvalidWindow)?;
        let spans = report
            .spans
            .into_iter()
            .filter(|span| span.end_ms >= day.start_ms)
            .collect();
        Ok((day, spans))
    }
}
