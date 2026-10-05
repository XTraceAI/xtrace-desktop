//! M-17 environment usage: how often each structural tool identity was called,
//! and how that compares with the inventory a host registry reports.
//!
//! Every count comes from the `tool_uses` rows ingest already classified in
//! [`xt_store::tool_use`], so no content, tool input, text head or transcript is
//! read here and no name is parsed a second time. Each stored row is counted
//! exactly once, in one of two shapes:
//!
//! * A **record-bound** row is one `tool_use` block of one canonical record. It
//!   is timed by that record's own timestamp and carries the same work
//!   exclusions every other metric applies, because it is joined through
//!   `v_records`. Response-usage deduplication is deliberately not used: that
//!   projection keeps one usage row per API response and would drop the other
//!   blocks of the same response. `native_record_copies` is deliberately not
//!   joined either, so a record repeated in several native files still
//!   contributes its blocks once.
//! * A **structural** row is a slash command or a stop-hook summary. Neither is
//!   an assistant tool call, so neither is in any record's `tool_use_count`, and
//!   `command` and `hook` stay their own kinds here. Such a row carries its own
//!   `event_ts`. A slash command is backed by the user record that stated it, so
//!   that record's exclusions decide; a hook summary is its own native record
//!   with no canonical row, so only its owning session's user/judge
//!   classification can exclude it.
//!
//! A hook count is a count of summary events, exactly as M-17 defines it, and
//! never of the individual hook executions one summary describes. Every summary
//! carries the one identity `stop_hook_summary` and never names the hook that
//! ran, so it can be compared with no named installed hook. It is observed and
//! counted like any other call, and it is unresolved for inventory purposes:
//! it is neither credited to an installed item nor reported as
//! called-but-not-installed, because it names no executable to install.
//!
//! Every identity row carries the same local-day buckets the surface row does,
//! in the same window order, and an installed item that was never called still
//! carries every reported day at 0, so a consumer never recomputes a count.
//!
//! Installation is a host fact supplied by the inventory owner. It is never
//! surface-specific and is never inferred from an observed call, which is why
//! the observed surface/day breakdown is returned beside the inventory join
//! rather than folded into it.

use crate::{DayBucket, Error, MetricsDb, Result, Window};
use jiff::tz::TimeZone;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use xt_store::{
    ingest::ToolKind,
    timestamp::{self, InstantKey},
    tool_use::HOOK_EVENT_NAME,
};

/// One block of one counted record. `v_records` owns the work exclusions.
pub(crate) const RECORD_QUERY: &str =
    "SELECT t.kind,t.name,t.server,t.tool,t.skill,v.host,v.surface,v.ts
     FROM tool_uses t JOIN v_records v ON v.uuid=t.uuid
     WHERE t.uuid IS NOT NULL AND v.ts_ms>=?1 AND v.ts_ms<?2
       AND (?3=0 OR t.kind IS NOT ?4)";
/// The same rows whose record states no timestamp: no window can hold them.
pub(crate) const RECORD_UNTIMED_QUERY: &str =
    "SELECT v.host,count(*) FROM tool_uses t JOIN v_records v ON v.uuid=t.uuid
     WHERE t.uuid IS NOT NULL AND v.ts_ms IS NULL
       AND (?1=0 OR t.kind IS NOT ?2) GROUP BY v.host";
/// Slash commands and hook summaries. `s.kind` is the owning session's
/// user/judge classification, which is all a standalone summary has. A
/// record-backed identity additionally has to survive `v_records`, so the
/// record exclusions are stated once, where they already live.
pub(crate) const STRUCTURAL_QUERY: &str =
    "SELECT t.kind,t.name,t.server,t.tool,t.skill,t.event_ts,s.host,s.surface
     FROM tool_uses t LEFT JOIN records r ON r.uuid=t.source_event_id
     JOIN sessions s ON s.session_id=coalesce(r.session_id,t.session_id)
     WHERE t.uuid IS NULL AND s.kind='user' AND (?1=0 OR t.kind IS NOT ?2)
       AND (r.uuid IS NULL OR EXISTS(SELECT 1 FROM v_records v WHERE v.uuid=t.source_event_id))";

/// One structural tool identity exactly as ingest stored it. A `None` detail is
/// a detail the source never stated: it is never aliased to the name, to an
/// empty string or to another identity's value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ToolIdentity {
    /// `builtin`, `mcp`, `skill`, `hook`, `command` or `subagent`; `None` for a
    /// row stored before ingest classified kinds.
    pub kind: Option<String>,
    pub name: String,
    pub server: Option<String>,
    pub tool: Option<String>,
    pub skill: Option<String>,
}

/// What a host registry reports. `Known` is the complete installed set for that
/// host; an empty `Known` list is a measured empty inventory, not a missing one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inventory {
    Known(Vec<ToolIdentity>),
    Unknown,
}

/// Supplied by the inventory owner, one entry per host it looked at. Nothing
/// here reads a registry, a file or a host command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInventory {
    pub host: String,
    pub installed: Inventory,
}

/// One identity and the calls counted for it inside the selected window,
/// with the same local-day buckets the window was split into. `by_day` holds
/// one entry per reported day, in window order, and sums to `calls`; an
/// identity with no call on a day still carries that day at 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentityCalls {
    pub identity: ToolIdentity,
    pub calls: u64,
    pub by_day: Vec<DayCalls>,
}

/// One local calendar-day bucket of the selected window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DayCalls {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub calls: u64,
}

/// Observed calls for one host and one raw source surface. The surface is the
/// value the source stated, preserved verbatim, and `None` stays unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SurfaceCalls {
    pub host: String,
    pub surface: Option<String>,
    pub calls: u64,
    pub by_day: Vec<DayCalls>,
    pub by_identity: Vec<IdentityCalls>,
}

/// The inventory join for one host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryJoin {
    Known {
        /// Every supplied installed item, its calls aggregated over every
        /// surface of this host. `calls` of 0 means no call matching that exact
        /// identity was measured *in this window*; it is not a claim that the
        /// item was never used. Such an item still carries every reported day
        /// at 0, so the day series of every row has the same shape.
        installed: Vec<IdentityCalls>,
        /// Identities that were called and that the inventory does not list,
        /// aggregated over every surface of this host the same way.
        called_not_installed: Vec<IdentityCalls>,
    },
    /// No inventory for this host, so installation is unknown: neither
    /// installed-but-never-called nor called-but-not-installed can be stated.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostEnvironment {
    pub host: String,
    pub inventory: InventoryJoin,
    /// Calls at this host that no exact identity match can account for. While
    /// this is above zero, an installed item's 0 is "no matched call in this
    /// window", and cannot establish that the item was never called.
    pub unresolved_calls: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    /// The call's canonical timestamp is absent, so no window can hold it.
    MissingTimestamp,
    /// The stored kind is absent, so the identity cannot be compared at all.
    UnknownKind,
    /// An MCP call whose name did not state both a server and a tool.
    UnknownMcpDetail,
    /// A skill call whose caller did not state the skill name.
    UnknownSkillName,
    /// A stop-hook summary. M-17 counts one event per summary, and a summary
    /// states only that hooks ran, never which installed hook ran, so it can
    /// match no named hook item. It is counted as an observed call and is not
    /// offered as an executable: it is neither credited to an installed item
    /// nor listed as called-but-not-installed.
    HookAttribution,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UnresolvedCalls {
    pub host: String,
    pub reason: UnresolvedReason,
    pub calls: u64,
}

/// Observation and inventory stay side by side: `observed` is what was measured
/// and says nothing about installation, `hosts` is what the supplied inventory
/// says, and `unresolved` is what neither can account for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EnvUsage {
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub observed: Vec<SurfaceCalls>,
    pub hosts: Vec<HostEnvironment>,
    pub unresolved: Vec<UnresolvedCalls>,
}

impl ToolIdentity {
    /// Why this identity cannot be compared with an inventory item, if it
    /// cannot. Matching is exact, so a detail the source never stated leaves
    /// the identity unmatchable rather than being guessed at or ignored.
    fn unmatchable(&self) -> Option<UnresolvedReason> {
        let Some(kind) = self.kind.as_deref() else {
            return Some(UnresolvedReason::UnknownKind);
        };
        if kind == ToolKind::Mcp.as_str() && (self.server.is_none() || self.tool.is_none()) {
            return Some(UnresolvedReason::UnknownMcpDetail);
        }
        if kind == ToolKind::Skill.as_str() && self.skill.is_none() {
            return Some(UnresolvedReason::UnknownSkillName);
        }
        if kind == ToolKind::Hook.as_str() && self.name == HOOK_EVENT_NAME {
            return Some(UnresolvedReason::HookAttribution);
        }
        None
    }
}

fn identity(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolIdentity> {
    Ok(ToolIdentity {
        kind: row.get(0)?,
        name: row.get(1)?,
        server: row.get(2)?,
        tool: row.get(3)?,
        skill: row.get(4)?,
    })
}

/// A running total and one counter per local day bucket, in window order. The
/// day vector is allocated at its final length, so a bucket with no call keeps
/// its 0 rather than being absent from the series.
struct Tally {
    calls: u64,
    by_day: Vec<u64>,
}

impl Tally {
    fn empty(buckets: usize) -> Self {
        Self {
            calls: 0,
            by_day: vec![0; buckets],
        }
    }

    fn count(&mut self, day: usize) {
        self.calls += 1;
        self.by_day[day] += 1;
    }

    fn resolve(self, identity: ToolIdentity, days: &[DayBucket]) -> IdentityCalls {
        IdentityCalls {
            identity,
            calls: self.calls,
            by_day: day_calls(&self.by_day, days),
        }
    }
}

/// Pair the accumulated counters with the buckets they were accumulated over.
/// The two are the same length by construction and stay in window order.
fn day_calls(counts: &[u64], days: &[DayBucket]) -> Vec<DayCalls> {
    days.iter()
        .zip(counts)
        .map(|(day, calls)| DayCalls {
            date: day.date.to_string(),
            start_ms: day.window.start_ms(),
            end_ms: day.window.end_ms(),
            calls: *calls,
        })
        .collect()
}

struct SurfaceTotals {
    total: Tally,
    identities: BTreeMap<ToolIdentity, Tally>,
}

/// Accumulates one pass over the stored rows. Exact half-open membership is
/// decided on the precise native instant, never on the millisecond projection.
struct Observation {
    start: InstantKey,
    end: InstantKey,
    day_ends: Vec<InstantKey>,
    surfaces: BTreeMap<(String, Option<String>), SurfaceTotals>,
    /// Matched calls of one host, aggregated over every surface of that host,
    /// because installation is a host fact and never a surface one.
    matched: BTreeMap<String, BTreeMap<ToolIdentity, Tally>>,
    unresolved: BTreeMap<(String, UnresolvedReason), u64>,
}

impl Observation {
    fn new(window: Window, days: &[DayBucket]) -> Self {
        Self {
            start: InstantKey::from_millisecond(window.start_ms()),
            end: InstantKey::from_millisecond(window.end_ms()),
            day_ends: days
                .iter()
                .map(|day| InstantKey::from_millisecond(day.window.end_ms()))
                .collect(),
            surfaces: BTreeMap::new(),
            matched: BTreeMap::new(),
            unresolved: BTreeMap::new(),
        }
    }

    fn unresolve(&mut self, host: &str, reason: UnresolvedReason, calls: u64) {
        *self
            .unresolved
            .entry((host.to_owned(), reason))
            .or_default() += calls;
    }

    /// Count one stored row. A row outside the window belongs to another
    /// window and is silently not this window's work; a row with no timestamp
    /// belongs to none, and is reported with its reason instead.
    fn push(
        &mut self,
        host: String,
        surface: Option<String>,
        identity: ToolIdentity,
        timestamp: Option<String>,
    ) -> rusqlite::Result<()> {
        let Some(raw) = timestamp else {
            self.unresolve(&host, UnresolvedReason::MissingTimestamp, 1);
            return Ok(());
        };
        let instant = timestamp::parse(&raw)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?
            .0;
        if instant < self.start || instant >= self.end {
            return Ok(());
        }
        let day = self.day_ends.partition_point(|end| *end <= instant);
        let buckets = self.day_ends.len();
        let totals = self
            .surfaces
            .entry((host.clone(), surface))
            .or_insert_with(|| SurfaceTotals {
                total: Tally::empty(buckets),
                identities: BTreeMap::new(),
            });
        totals.total.count(day);
        totals
            .identities
            .entry(identity.clone())
            .or_insert_with(|| Tally::empty(buckets))
            .count(day);
        match identity.unmatchable() {
            Some(reason) => self.unresolve(&host, reason, 1),
            None => self
                .matched
                .entry(host)
                .or_default()
                .entry(identity)
                .or_insert_with(|| Tally::empty(buckets))
                .count(day),
        }
        Ok(())
    }

    fn finish(
        self,
        window: Window,
        days: &[DayBucket],
        supplied: &BTreeMap<&str, &Inventory>,
        exclude_builtin: bool,
    ) -> EnvUsage {
        let Self {
            surfaces,
            mut matched,
            unresolved,
            ..
        } = self;
        let mut per_host = BTreeMap::<&str, u64>::new();
        for ((host, _), calls) in &unresolved {
            *per_host.entry(host.as_str()).or_default() += calls;
        }
        let mut names = BTreeSet::<String>::new();
        names.extend(supplied.keys().map(|host| (*host).to_owned()));
        names.extend(matched.keys().cloned());
        names.extend(unresolved.keys().map(|(host, _)| host.clone()));
        let hosts = names
            .into_iter()
            .map(|host| {
                let calls = matched.remove(&host).unwrap_or_default();
                let inventory = match supplied.get(host.as_str()) {
                    // A repeated supplied item is one installed item, so the
                    // same call is never credited to it twice.
                    Some(Inventory::Known(items)) => {
                        // An item with no matched call still carries the whole
                        // reported day series at 0, so a consumer reads a count
                        // per day off every row without computing one.
                        let mut installed: BTreeMap<ToolIdentity, Tally> = items
                            .iter()
                            .filter(|item| {
                                !exclude_builtin
                                    || item.kind.as_deref() != Some(ToolKind::Builtin.as_str())
                            })
                            .map(|item| (item.clone(), Tally::empty(days.len())))
                            .collect();
                        let mut called_not_installed = Vec::new();
                        for (identity, tally) in calls {
                            match installed.get_mut(&identity) {
                                Some(matched) => *matched = tally,
                                None => called_not_installed.push(tally.resolve(identity, days)),
                            }
                        }
                        InventoryJoin::Known {
                            installed: installed
                                .into_iter()
                                .map(|(identity, tally)| tally.resolve(identity, days))
                                .collect(),
                            called_not_installed,
                        }
                    }
                    Some(Inventory::Unknown) | None => InventoryJoin::Unknown,
                };
                HostEnvironment {
                    unresolved_calls: per_host.get(host.as_str()).copied().unwrap_or(0),
                    host,
                    inventory,
                }
            })
            .collect();
        EnvUsage {
            window_start_ms: window.start_ms(),
            window_end_ms: window.end_ms(),
            observed: surfaces
                .into_iter()
                .map(|((host, surface), totals)| SurfaceCalls {
                    host,
                    surface,
                    calls: totals.total.calls,
                    by_day: day_calls(&totals.total.by_day, days),
                    by_identity: totals
                        .identities
                        .into_iter()
                        .map(|(identity, tally)| tally.resolve(identity, days))
                        .collect(),
                })
                .collect(),
            hosts,
            unresolved: unresolved
                .into_iter()
                .map(|((host, reason), calls)| UnresolvedCalls {
                    host,
                    reason,
                    calls,
                })
                .collect(),
        }
    }
}

impl MetricsDb {
    /// Aggregate structural calls over one read snapshot and join them to the
    /// supplied host inventory. The caller owns the zone, the window and the
    /// inventory; this reader discovers nothing and ranks nothing.
    pub fn environment(
        &self,
        window: Window,
        zone: TimeZone,
        inventory: &[HostInventory],
    ) -> Result<EnvUsage> {
        self.environment_with_builtin_filter(window, zone, inventory, false)
    }

    /// The desktop Environment report omits stored built-in tool calls while
    /// retaining unknown and future kinds. Other metrics readers keep all kinds.
    pub fn environment_without_builtins(
        &self,
        window: Window,
        zone: TimeZone,
        inventory: &[HostInventory],
    ) -> Result<EnvUsage> {
        self.environment_with_builtin_filter(window, zone, inventory, true)
    }

    fn environment_with_builtin_filter(
        &self,
        window: Window,
        zone: TimeZone,
        inventory: &[HostInventory],
        exclude_builtin: bool,
    ) -> Result<EnvUsage> {
        let mut supplied = BTreeMap::<&str, &Inventory>::new();
        for entry in inventory {
            if supplied
                .insert(entry.host.as_str(), &entry.installed)
                .is_some()
            {
                return Err(Error::DuplicateInventoryHost);
            }
        }
        let days = window.local_days(zone)?;
        let observation =
            self.read_snapshot(|db| db.observe_environment(window, &days, exclude_builtin))?;
        Ok(observation.finish(window, &days, &supplied, exclude_builtin))
    }

    fn observe_environment(
        &self,
        window: Window,
        days: &[DayBucket],
        exclude_builtin: bool,
    ) -> Result<Observation> {
        let mut observation = Observation::new(window, days);
        let mut statement = self.connection.prepare(RECORD_QUERY)?;
        let mut rows = statement.query(rusqlite::params![
            window.start_ms(),
            window.candidate_end_ms()?,
            exclude_builtin,
            ToolKind::Builtin.as_str(),
        ])?;
        while let Some(row) = rows.next()? {
            observation.push(row.get(5)?, row.get(6)?, identity(row)?, row.get(7)?)?;
        }
        let mut statement = self.connection.prepare(RECORD_UNTIMED_QUERY)?;
        let mut rows = statement.query(rusqlite::params![
            exclude_builtin,
            ToolKind::Builtin.as_str()
        ])?;
        while let Some(row) = rows.next()? {
            let host: String = row.get(0)?;
            let calls: i64 = row.get(1)?;
            let calls = u64::try_from(calls)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, calls))?;
            observation.unresolve(&host, UnresolvedReason::MissingTimestamp, calls);
        }
        let mut statement = self.connection.prepare(STRUCTURAL_QUERY)?;
        let mut rows = statement.query(rusqlite::params![
            exclude_builtin,
            ToolKind::Builtin.as_str()
        ])?;
        while let Some(row) = rows.next()? {
            observation.push(row.get(6)?, row.get(7)?, identity(row)?, row.get(5)?)?;
        }
        Ok(observation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Structural inspection of the only SQL this metric runs: it names the
    /// classified columns and the shared work projection, and no column that
    /// could carry transcript content, tool input or a text head.
    #[test]
    fn environment_queries_read_structure_and_never_name_a_content_column() {
        for query in [RECORD_QUERY, RECORD_UNTIMED_QUERY, STRUCTURAL_QUERY] {
            for forbidden in [
                "content_json",
                "raw_json",
                "input_json",
                "text_head",
                "excerpt",
                "title",
            ] {
                assert!(!query.contains(forbidden), "{forbidden}: {query}");
            }
            assert!(query.contains("v_records"), "{query}");
        }
        for column in ["t.kind", "t.name", "t.server", "t.tool", "t.skill"] {
            assert!(RECORD_QUERY.contains(column), "{column}");
            assert!(STRUCTURAL_QUERY.contains(column), "{column}");
        }
        assert!(STRUCTURAL_QUERY.contains("t.event_ts"));
        assert!(RECORD_QUERY.contains("v.ts"));
        // The deduplicated response projection and the copied-context union are
        // the two joins this metric must not make.
        for query in [RECORD_QUERY, RECORD_UNTIMED_QUERY, STRUCTURAL_QUERY] {
            assert!(!query.contains("v_response_usage"), "{query}");
            assert!(!query.contains("native_record_copies"), "{query}");
            assert!(!query.contains("session_work_records"), "{query}");
        }
    }
}
