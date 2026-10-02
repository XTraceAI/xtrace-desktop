use crate::{Error, MetricsDb, Result, Window};
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str =
    "SELECT session_id,host,ts_ms,ts,uuid FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";

const MAX_GAP_MS: i64 = 20 * 60 * 1000;

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
            let mut span = ActiveSpan {
                session_id,
                host,
                start_ms: events[0].0,
                end_ms: events[0].0,
            };
            for (ms, _, _) in events.into_iter().skip(1) {
                let gap = ms.checked_sub(span.end_ms).ok_or(Error::CounterOverflow)?;
                if gap > MAX_GAP_MS {
                    spans.push(span.clone());
                    span.start_ms = ms;
                } else {
                    active_ms = active_ms
                        .checked_add(gap as u64)
                        .ok_or(Error::CounterOverflow)?;
                }
                span.end_ms = ms;
            }
            spans.push(span);
        }
        Ok(ActiveSpanReport { spans, active_ms })
    }
}
