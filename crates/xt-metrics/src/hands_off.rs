//! M-09 hands-off stretches, and the private pieces the per-session view of the
//! same metric reuses.
//!
//! Loading, timestamp health and the stretch fold live here once. The global
//! report and [`crate::session_stretches`] call exactly these, so a session's
//! stretches are the very segments M-09 already measured, never a second
//! definition that could drift from the Dashboard's number.
use crate::{Error, MetricsDb, Result, Window, stats};
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str =
    "SELECT session_id,host,surface,uuid,ts,ts_ms,type,is_human,tool_use_count,
    confirmed_automated_input FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";

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

/// One in-window work event. The fields stay private to this module: a caller
/// reads a finished [`Stretch`], never an event's raw parts.
pub(crate) struct Event {
    instant: InstantKey,
    uuid: String,
    /// The exact native spelling, kept as stored so a reported endpoint is the
    /// transcript's own instant and not a reprojection of `ms`.
    ts: String,
    ms: i64,
    human: Option<bool>,
    tools: Option<u64>,
    health_record: bool,
    /// A confirmed automated input. It is neutral to every stretch: it neither
    /// starts nor ends one, cannot be an endpoint and says nothing about tool
    /// presence. It still sits on the session's clock, so timestamp health and
    /// a stretch's M-05 timeline keep it, as M-05 itself does.
    automated: bool,
}

pub(crate) type SurfaceSessions = BTreeMap<(String, Option<String>), BTreeMap<String, Vec<Event>>>;

/// Every in-window work event of the selected window, once, grouped by raw
/// `(host, surface)` and then by session, each session in chronological order.
///
/// Exact native instants decide half-open membership; equal instants order by
/// UUID. Missing timestamps cannot be assigned to a window, as in the other
/// event metrics.
pub(crate) fn load(connection: &rusqlite::Connection, window: Window) -> Result<SurfaceSessions> {
    let start = InstantKey::from_millisecond(window.start_ms());
    let end = InstantKey::from_millisecond(window.end_ms());
    let mut surfaces = SurfaceSessions::new();
    let mut statement = connection.prepare(QUERY)?;
    let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
    while let Some(row) = rows.next()? {
        let ts: String = row.get(4)?;
        let instant = timestamp::parse(&ts)
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
                u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(8, value))
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
                ts,
                ms: row.get(5)?,
                human: row.get(7)?,
                tools,
                health_record: kind == "user" || kind == "assistant",
                automated: row.get(9)?,
            });
    }
    drop(rows);
    drop(statement);
    for sessions in surfaces.values_mut() {
        for events in sessions.values_mut() {
            events.sort_unstable_by(|a, b| a.instant.cmp(&b.instant).then(a.uuid.cmp(&b.uuid)));
        }
    }
    Ok(surfaces)
}

/// M-09 timestamp health for one raw `(host, surface)`, judged over every
/// session that surface has in the selected window.
///
/// Only in-window user/assistant records participate. A session qualifies with
/// at least five of them and is degenerate when its distinct precise instants
/// multiplied by three do not exceed its record count. The surface is excluded
/// only with at least three qualifying sessions and strictly more than 30%
/// degenerate ones. A caller that is interested in one session still asks about
/// the whole surface: the judgement is about the surface's clock, not about
/// whichever session is being looked at.
pub(crate) fn health(
    host: &str,
    surface: &Option<String>,
    sessions: &BTreeMap<String, Vec<Event>>,
) -> Option<ExcludedSurface> {
    let mut qualifying = 0_u64;
    let mut degenerate = 0_u64;
    for events in sessions.values() {
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
    (qualifying >= 3 && u128::from(degenerate) * 10 > u128::from(qualifying) * 3).then(|| {
        ExcludedSurface {
            host: host.to_owned(),
            surface: surface.clone(),
            qualifying_sessions: qualifying,
            degenerate_sessions: degenerate,
        }
    })
}

impl MetricsDb {
    /// Health and stretches consume one read snapshot. Exact instants determine
    /// membership and chronology; positive durations use the POSIX ms projection.
    pub fn hands_off(&self, window: Window) -> Result<HandsOff> {
        let surfaces = load(&self.connection, window)?;
        let mut excluded_surfaces = Vec::new();
        let mut durations = Vec::new();
        let mut measured = true;
        for ((host, surface), sessions) in &surfaces {
            if let Some(excluded) = health(host, surface, sessions) {
                excluded_surfaces.push(excluded);
                continue;
            }
            for events in sessions.values() {
                measured &= stretches(events, |stretch| durations.push(stretch.duration_ms()))?;
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

/// One measured stretch: the contiguous run of events from its human record
/// through the last explicitly non-human record before the next human one.
pub(crate) struct Stretch<'a> {
    events: &'a [Event],
    duration_ms: u64,
}
impl<'a> Stretch<'a> {
    /// The difference of the two endpoints' stored POSIX millisecond
    /// projections, which is the duration M-09 already measured.
    pub(crate) fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
    /// `(uuid, exact native timestamp)` of the human record it starts at.
    pub(crate) fn start(&self) -> (&'a str, &'a str) {
        let event = &self.events[0];
        (&event.uuid, &event.ts)
    }
    /// `(uuid, exact native timestamp)` of the non-human record it ends at.
    pub(crate) fn end(&self) -> (&'a str, &'a str) {
        let event = &self.events[self.events.len() - 1];
        (&event.uuid, &event.ts)
    }
    /// The stretch's own records on the shape M-05 folds: the stored POSIX
    /// millisecond projection, the exact native instant and the UUID, sorted
    /// the way [`crate::spans`] sorts a session's timeline. Ordering on the
    /// millisecond axis is M-05's own rule and is what keeps a leap-adjacent
    /// pair from producing a negative gap. A confirmed automated input inside
    /// the stretch stays on this timeline, because M-05 keeps every event.
    pub(crate) fn timeline(&self) -> Vec<(i64, InstantKey, String)> {
        let mut rows: Vec<_> = self
            .events
            .iter()
            .map(|event| (event.ms, event.instant.clone(), event.uuid.clone()))
            .collect();
        rows.sort_unstable();
        rows
    }
    /// Every work record the stretch covers, in chronological order, paired
    /// with the number of tool calls that record states. `None` is a record
    /// whose count the source never stated, not a record that called nothing.
    /// A confirmed automated input is not the stretch's work and is skipped.
    pub(crate) fn records(&self) -> impl Iterator<Item = (&'a str, Option<u64>)> {
        self.events
            .iter()
            .filter(|event| !event.automated)
            .map(|event| (event.uuid.as_str(), event.tools))
    }
}

#[derive(Default)]
struct Segment {
    start: usize,
    end: Option<usize>,
    known_tool: bool,
    unknown_tool: bool,
}
impl Segment {
    fn observe_tools(&mut self, tools: Option<u64>) {
        self.known_tool |= tools.is_some_and(|n| n > 0);
        self.unknown_tool |= tools.is_none();
    }
    fn finish<'a>(self, events: &'a [Event], emit: &mut dyn FnMut(Stretch<'a>)) -> Result<bool> {
        let Some(end) = self.end else {
            return Ok(true);
        };
        let duration = events[end]
            .ms
            .checked_sub(events[self.start].ms)
            .ok_or(Error::CounterOverflow)?;
        if duration <= 0 {
            return Ok(true);
        }
        if self.known_tool {
            emit(Stretch {
                events: &events[self.start..=end],
                duration_ms: duration as u64,
            });
        }
        Ok(self.known_tool || !self.unknown_tool)
    }
}

/// Fold one session's ordered events into M-09 stretches, reporting each
/// measured one. The returned flag is false when this session's sample is
/// unmeasured: it is the caller's, and only this session's, unknown.
fn stretches<'a>(events: &'a [Event], mut emit: impl FnMut(Stretch<'a>)) -> Result<bool> {
    // An unknown classification can move either segment boundary. Do not infer
    // it from role or turn a partially known sample into a measured distribution.
    if events.iter().any(|e| e.human.is_none()) {
        return Ok(false);
    }
    let mut segment: Option<Segment> = None;
    let mut measured = true;
    for (index, event) in events.iter().enumerate() {
        // Neutral: no boundary, no endpoint, no tool evidence either way.
        if event.automated {
            continue;
        }
        if event.human == Some(true) {
            if let Some(previous) = segment.take() {
                measured &= previous.finish(events, &mut emit)?;
            }
            segment = Some(Segment {
                start: index,
                ..Default::default()
            });
        } else if let Some(active) = &mut segment {
            active.end = Some(index);
        }
        if let Some(active) = &mut segment {
            active.observe_tools(event.tools);
        }
    }
    if let Some(last) = segment {
        measured &= last.finish(events, &mut emit)?;
    }
    Ok(measured)
}

/// The same fold, for one session, collecting the stretches themselves. The
/// global report only needs each duration; the per-session view needs the
/// endpoints and the records in between, so it takes them from here rather
/// than walking the events a second time.
pub(crate) fn collect(events: &[Event]) -> Result<Option<Vec<Stretch<'_>>>> {
    let mut collected = Vec::new();
    let measured = stretches(events, |stretch| collected.push(stretch))?;
    Ok(measured.then_some(collected))
}
