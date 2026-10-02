use crate::{Error, MetricsDb, Result, Window, stats};
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str =
    "SELECT session_id,host,surface,uuid,ts,ts_ms,type,is_human,tool_use_count
    FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExcludedSurface {
    pub host: String,
    pub surface: Option<String>,
    pub qualifying_sessions: u64,
    pub degenerate_sessions: u64,
}

/// Null count/percentiles mean an in-window measurement could change the sample.
/// Empty measured samples have n=0 and null percentiles. Missing timestamps cannot
/// be assigned to a window; this report does not substitute for source coverage.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HandsOff {
    pub n: Option<u64>,
    pub median_min: Option<f64>,
    pub p90_min: Option<f64>,
    pub excluded_surfaces: Vec<ExcludedSurface>,
}

struct Event {
    instant: InstantKey,
    uuid: String,
    ms: i64,
    human: Option<bool>,
    tools: Option<u64>,
    health_record: bool,
}

type SurfaceSessions = BTreeMap<(String, Option<String>), BTreeMap<String, Vec<Event>>>;

impl MetricsDb {
    /// Health and stretches consume one read snapshot. Exact instants determine
    /// membership and chronology; positive durations use the POSIX ms projection.
    pub fn hands_off(&self, window: Window) -> Result<HandsOff> {
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut surfaces = SurfaceSessions::new();
        let mut statement = self.connection.prepare(QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(4)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant < start || instant >= end {
                continue;
            }
            let tools = row
                .get::<_, Option<i64>>(8)?
                .map(|value| {
                    u64::try_from(value)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(8, value))
                })
                .transpose()?;
            let kind: String = row.get(6)?;
            surfaces
                .entry((row.get(1)?, row.get(2)?))
                .or_default()
                .entry(row.get(0)?)
                .or_default()
                .push(Event {
                    instant,
                    uuid: row.get(3)?,
                    ms: row.get(5)?,
                    human: row.get(7)?,
                    tools,
                    health_record: kind == "user" || kind == "assistant",
                });
        }
        let mut excluded_surfaces = Vec::new();
        let mut durations = Vec::new();
        let mut measured = true;
        for ((host, surface), sessions) in &mut surfaces {
            let mut qualifying = 0_u64;
            let mut degenerate = 0_u64;
            for events in sessions.values_mut() {
                events.sort_unstable_by(|a, b| a.instant.cmp(&b.instant).then(a.uuid.cmp(&b.uuid)));
                let health: Vec<_> = events.iter().filter(|e| e.health_record).collect();
                if health.len() >= 5 {
                    qualifying += 1;
                    let distinct = 1 + health
                        .windows(2)
                        .filter(|p| p[0].instant != p[1].instant)
                        .count();
                    if distinct <= health.len() / 3 {
                        degenerate += 1;
                    }
                }
            }
            if qualifying >= 3 && u128::from(degenerate) * 10 > u128::from(qualifying) * 3 {
                excluded_surfaces.push(ExcludedSurface {
                    host: host.clone(),
                    surface: surface.clone(),
                    qualifying_sessions: qualifying,
                    degenerate_sessions: degenerate,
                });
                continue;
            }
            for events in sessions.values() {
                measured &= stretches(events, &mut durations)?;
            }
        }
        let n = measured.then_some(durations.len() as u64);
        let percentiles = measured
            .then(|| stats::median_p90(&mut durations))
            .flatten();
        Ok(HandsOff {
            n,
            median_min: percentiles.map(|(median, _)| median / 60000.0),
            p90_min: percentiles.map(|(_, p90)| p90 as f64 / 60000.0),
            excluded_surfaces,
        })
    }
}

#[derive(Default)]
struct Segment {
    start: i64,
    end: Option<i64>,
    known_tool: bool,
    unknown_tool: bool,
}
impl Segment {
    fn observe_tools(&mut self, tools: Option<u64>) {
        self.known_tool |= tools.is_some_and(|n| n > 0);
        self.unknown_tool |= tools.is_none();
    }
    fn finish(self, durations: &mut Vec<u64>) -> Result<bool> {
        let Some(end) = self.end else {
            return Ok(true);
        };
        let duration = end.checked_sub(self.start).ok_or(Error::CounterOverflow)?;
        if duration <= 0 {
            return Ok(true);
        }
        if self.known_tool {
            durations.push(duration as u64);
        }
        Ok(self.known_tool || !self.unknown_tool)
    }
}

fn stretches(events: &[Event], durations: &mut Vec<u64>) -> Result<bool> {
    // An unknown classification can move either segment boundary. Do not infer
    // it from role or turn a partially known sample into a measured distribution.
    if events.iter().any(|e| e.human.is_none()) {
        return Ok(false);
    }
    let mut segment: Option<Segment> = None;
    let mut measured = true;
    for event in events {
        if event.human == Some(true) {
            if let Some(previous) = segment.take() {
                measured &= previous.finish(durations)?;
            }
            segment = Some(Segment {
                start: event.ms,
                ..Default::default()
            });
        } else if let Some(active) = &mut segment {
            active.end = Some(event.ms);
        }
        if let Some(active) = &mut segment {
            active.observe_tools(event.tools);
        }
    }
    if let Some(last) = segment {
        measured &= last.finish(durations)?;
    }
    Ok(measured)
}
