//! M-20 repeated-call density (circling), per M-09 stretch of one session.
//!
//! Nothing about a stretch is redefined here. Which sessions the window holds,
//! which raw surfaces its clock excludes and where each stretch begins and ends
//! are [`crate::hands_off`]'s own answers, called exactly as
//! [`crate::session_stretches`] calls them, so a stretch counted here is one of
//! the segments the Dashboard's M-09 already measured.
//!
//! Two durations sit side by side, and they are different measurements:
//!
//! * `duration_ms` is M-09's elapsed time — the stretch's last instant minus
//!   its first, on the stored POSIX millisecond projection.
//! * `active_duration_ms` is M-05's gap rule applied to the records the stretch
//!   selects: the same fold, the same twenty-minute threshold, the same axis.
//!   A stretch that waited an hour between two calls spent none of that hour
//!   active, and M-20's threshold is about active time. This is a fold over one
//!   stretch's own records, so it is not a session's M-05 total and not a
//!   window's: parallel sessions still add in M-05 and are still measured apart
//!   here, one stretch at a time.
//!
//! Grouping is the store's, not this module's. A call's comparison key is
//! derived from its arguments at write time, before content retention decides
//! about them ([`xt_store::repeat_key`]), and all that is read here is whether
//! two calls carry the same key. No argument, command, path or pattern is read,
//! and no key leaves this crate: a group is reported by the tool's stored name,
//! how many calls it holds, and where a representative one of them sits. The
//! argument labels a detail view shows come from the original source, later and
//! transiently.
//!
//! An unknown is never a zero. A stretch reports a repeat count only when every
//! call it contains is accounted for: every record says how many calls it made,
//! the store holds that many blocks for it, and each block carries a key this
//! build derives and that no second observation contradicted. Anything less and
//! the stretch says which of those it could not establish, because a partial
//! count would understate exactly the circling this metric exists to find.

use crate::{
    Error, MetricsDb, Result, Window, hands_off, session, session_stretches::ToolBlock, spans,
};
use rusqlite::OptionalExtension;
use serde::Serialize;
use std::collections::BTreeMap;

/// Stored grouping evidence for one session's record-bound calls. Names and
/// keys only: no tool input, no record content and no source file is read.
pub(crate) const BLOCK_QUERY: &str =
    "SELECT uuid,block_index,name,group_version,group_key,group_conflict FROM tool_uses
     WHERE session_id=?1 AND uuid IS NOT NULL AND block_index IS NOT NULL";

/// M-20's two thresholds. Both are settings in the metric's own definition;
/// this build supplies no persistence and no settings surface for them, so a
/// caller states them explicitly or takes the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RepeatThresholds {
    active_ms: u64,
    repeats: u64,
}

/// Four active minutes.
pub const DEFAULT_ACTIVE_MS: u64 = 240_000;
/// Five repeats.
pub const DEFAULT_REPEATS: u64 = 5;

impl Default for RepeatThresholds {
    fn default() -> Self {
        Self {
            active_ms: DEFAULT_ACTIVE_MS,
            repeats: DEFAULT_REPEATS,
        }
    }
}

impl RepeatThresholds {
    /// Both thresholds must be positive. A zero repeat threshold would call
    /// every measured stretch circling, including one whose calls were all
    /// different, and a zero active threshold would drop the duration half of
    /// the rule; neither is a setting of this metric.
    pub fn new(active_ms: u64, repeats: u64) -> Result<Self> {
        (active_ms > 0 && repeats > 0)
            .then_some(Self { active_ms, repeats })
            .ok_or(Error::InvalidRepeatThresholds)
    }
    pub fn active_ms(&self) -> u64 {
        self.active_ms
    }
    pub fn repeats(&self) -> u64 {
        self.repeats
    }
}

/// Why a stretch's repeat count could not be established. Each names a missing
/// piece of evidence, never a measured absence of repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownRepeats {
    /// A record in the stretch never stated how many calls it made.
    CallCount,
    /// A record stated calls the store does not hold every block for, so the
    /// stretch's calls cannot all be counted.
    MissingBlocks,
    /// A stored call carries no comparison key: an older build wrote it, or the
    /// observation could not say which call it was.
    MissingKey,
    /// A stored call's key comes from a derivation this build cannot compare
    /// with its own.
    UnsupportedVersion,
    /// Two observations of one record put different calls at a stored call's
    /// position — another key, another tool name, or no call at all — so which
    /// call it was is contested.
    ConflictingKey,
}

/// One group of identical calls inside a stretch, named without its key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RepeatGroup {
    /// The tool name exactly as stored beside the calls. Every call in the
    /// group shares it, because the name is part of what the key compares.
    pub tool_name: String,
    pub count: u64,
    /// Where one of the group's calls sits: the record that carried it and that
    /// record's content block index — the same structural position
    /// [`crate::session_stretches`] uses, and the earliest of the group's own
    /// calls, so ties between equally repeated groups resolve the same way on
    /// every run.
    pub representative: ToolBlock,
}

/// What M-20 can say about the calls inside one stretch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RepeatDensity {
    Unknown {
        reason: UnknownRepeats,
    },
    /// The sum of `count - 1` over the groups with more than one member, and
    /// the largest of those groups. A stretch whose calls were all different
    /// measures zero repeats and names no worst group.
    Measured {
        repeats: u64,
        worst: Option<RepeatGroup>,
    },
}

/// One M-09 stretch with its repeat measurement. The endpoints and
/// `duration_ms` are M-09's, unchanged and not recomputed here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StretchRepeats {
    pub start_uuid: String,
    pub end_uuid: String,
    pub start: String,
    pub end: String,
    /// M-09's elapsed duration of the stretch.
    pub duration_ms: u64,
    /// M-05's active time over the stretch's own records: M-05's fold, on
    /// M-05's millisecond axis, so a gap over its twenty minutes contributes
    /// nothing. It is a separate measurement from `duration_ms`, not a bound
    /// on it or by it — the two order their records differently around a leap
    /// second, so no ordering between them is promised.
    pub active_duration_ms: u64,
    pub repeats: RepeatDensity,
    /// Whether this stretch is circling: at least the active threshold and at
    /// least the repeat threshold, both inclusive. `None` whenever the repeat
    /// measurement is unknown — a stretch whose repeats were never counted is
    /// neither circling nor not circling, and saying otherwise from the
    /// duration alone would answer a question only half of the evidence was
    /// available for.
    pub circling: Option<bool>,
}

/// What M-20 can say about one session inside the selected window. The three
/// states are M-09's own, for the same reasons [`crate::session_stretches`]
/// distinguishes them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionRepeats {
    /// No indexed user session owns this identifier.
    Missing,
    /// The session is indexed, but M-09 states no stretches for it.
    Unmeasured {
        excluded_surface: Option<hands_off::ExcludedSurface>,
    },
    /// Every stretch the session has in the window, in chronological order,
    /// and the thresholds these answers were judged against.
    Measured {
        thresholds: RepeatThresholds,
        stretches: Vec<StretchRepeats>,
    },
}

/// One stored call: its position, the tool's stored name, and the grouping
/// evidence. `key` is `None` when the row states none.
struct StoredCall {
    block_index: u64,
    name: String,
    key: Option<(u32, String)>,
    conflict: bool,
}

/// A group under construction. `first` is the position of its earliest call in
/// the stretch: the record's index in the stretch's own order, then the block
/// index inside that record.
struct Group {
    count: u64,
    name: String,
    first: (usize, u64),
    record_uuid: String,
}

/// Count one stretch's calls by comparison key, or say which evidence was
/// missing. Records are walked in the stretch's own chronological order.
fn density(
    stretch: &hands_off::Stretch<'_>,
    blocks: &BTreeMap<String, Vec<StoredCall>>,
) -> std::result::Result<RepeatDensity, UnknownRepeats> {
    let mut groups = BTreeMap::<String, Group>::new();
    for (order, (uuid, stated)) in stretch.records().enumerate() {
        // A record that never stated its call count could have made calls this
        // stretch's total does not include.
        let stated = stated.ok_or(UnknownRepeats::CallCount)?;
        if stated == 0 {
            continue;
        }
        // Every call the record states must be accounted for. A record with
        // some of its blocks stored would silently contribute fewer calls than
        // it made, which is exactly an understated repeat count.
        let calls = blocks.get(uuid).ok_or(UnknownRepeats::MissingBlocks)?;
        if calls.len() as u64 != stated {
            return Err(UnknownRepeats::MissingBlocks);
        }
        for call in calls {
            if call.conflict {
                return Err(UnknownRepeats::ConflictingKey);
            }
            let (version, key) = call.key.as_ref().ok_or(UnknownRepeats::MissingKey)?;
            if *version != xt_store::repeat_key::VERSION {
                return Err(UnknownRepeats::UnsupportedVersion);
            }
            groups
                .entry(key.clone())
                .or_insert_with(|| Group {
                    count: 0,
                    name: call.name.clone(),
                    first: (order, call.block_index),
                    record_uuid: uuid.to_owned(),
                })
                .count += 1;
        }
    }
    // Every group contributes `count - 1`, which is zero for the groups that
    // hold one call, so this is the sum over the repeated groups.
    let repeats = groups.values().map(|group| group.count - 1).sum();
    // The largest repeated group, and on a tie the one whose earliest call came
    // first in the stretch. Both parts are positions in a fixed order, so the
    // answer does not depend on how the keys happened to sort.
    let worst = groups
        .into_values()
        .filter(|group| group.count > 1)
        .min_by_key(|group| (std::cmp::Reverse(group.count), group.first))
        .map(|group| RepeatGroup {
            tool_name: group.name,
            count: group.count,
            representative: ToolBlock {
                record_uuid: group.record_uuid,
                block_index: group.first.1,
            },
        });
    Ok(RepeatDensity::Measured { repeats, worst })
}

impl MetricsDb {
    /// M-20 for exactly one session over one explicit window.
    ///
    /// Metadata, events and stored call evidence are read inside one snapshot,
    /// so the surface a session is on, the records its stretches were folded
    /// from and the calls those stretches count all describe the same committed
    /// state while the native writer keeps appending on its own connection.
    pub fn session_repeats(
        &self,
        window: Window,
        session_id: &str,
        thresholds: RepeatThresholds,
    ) -> Result<SessionRepeats> {
        self.read_snapshot(|db| {
            // Whether an indexed user session owns this identifier, and which
            // raw surface it belongs to, are metadata facts read before any
            // measurement — as in `session_stretches`, and for the same reason:
            // an empty window is not an absent session.
            let Some((host, surface)) = db
                .connection
                .query_row(
                    &format!("{} AND session_id=?1", session::EXISTS_QUERY),
                    [session_id],
                    |row| Ok((row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)),
                )
                .optional()?
            else {
                return Ok(SessionRepeats::Missing);
            };
            let surfaces = hands_off::load(&db.connection, window)?;
            let sessions = surfaces.get(&(host.clone(), surface.clone()));
            if let Some(excluded) = sessions.and_then(|s| hands_off::health(&host, &surface, s)) {
                return Ok(SessionRepeats::Unmeasured {
                    excluded_surface: Some(excluded),
                });
            }
            let Some(events) = sessions.and_then(|s| s.get(session_id)) else {
                return Ok(SessionRepeats::Measured {
                    thresholds,
                    stretches: vec![],
                });
            };
            let Some(collected) = hands_off::collect(events)? else {
                return Ok(SessionRepeats::Unmeasured {
                    excluded_surface: None,
                });
            };

            let blocks = if collected.is_empty() {
                BTreeMap::new()
            } else {
                db.stored_calls(session_id)?
            };
            let stretches = collected
                .iter()
                .map(|stretch| {
                    let (start_uuid, start) = stretch.start();
                    let (end_uuid, end) = stretch.end();
                    // M-05's fold, over this stretch's own records.
                    let active_duration_ms = spans::fold(&stretch.timeline(), |_, _| {})?;
                    let repeats = density(stretch, &blocks)
                        .unwrap_or_else(|reason| RepeatDensity::Unknown { reason });
                    let circling = match &repeats {
                        RepeatDensity::Unknown { .. } => None,
                        RepeatDensity::Measured { repeats, .. } => Some(
                            active_duration_ms >= thresholds.active_ms
                                && *repeats >= thresholds.repeats,
                        ),
                    };
                    Ok(StretchRepeats {
                        start_uuid: start_uuid.to_owned(),
                        end_uuid: end_uuid.to_owned(),
                        start: start.to_owned(),
                        end: end.to_owned(),
                        duration_ms: stretch.duration_ms(),
                        active_duration_ms,
                        repeats,
                        circling,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(SessionRepeats::Measured {
                thresholds,
                stretches,
            })
        })
    }

    /// One session's stored record-bound calls, by record UUID. Metadata only:
    /// the position, the stored tool name and the grouping evidence.
    fn stored_calls(&self, session_id: &str) -> Result<BTreeMap<String, Vec<StoredCall>>> {
        let mut stored = BTreeMap::<String, Vec<StoredCall>>::new();
        let mut statement = self.connection.prepare(BLOCK_QUERY)?;
        let mut rows = statement.query([session_id])?;
        while let Some(row) = rows.next()? {
            let block_index = row.get::<_, i64>(1)?;
            let version = row
                .get::<_, Option<i64>>(3)?
                .map(|value| {
                    u32::try_from(value)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(3, value))
                })
                .transpose()?;
            let digest = row.get::<_, Option<String>>(4)?;
            stored.entry(row.get(0)?).or_default().push(StoredCall {
                block_index: u64::try_from(block_index)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, block_index))?,
                name: row.get(2)?,
                key: version.zip(digest),
                conflict: row.get(5)?,
            });
        }
        for calls in stored.values_mut() {
            calls.sort_unstable_by_key(|call| call.block_index);
        }
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_grouping_read_touches_no_content_and_no_argument() {
        for forbidden in [
            "content_json",
            "input_json",
            "command",
            "file_path",
            "pattern",
        ] {
            assert!(!BLOCK_QUERY.contains(forbidden), "{forbidden}");
        }
    }
    #[test]
    fn both_thresholds_must_be_positive_and_default_to_the_metric_s_own() {
        let default = RepeatThresholds::default();
        assert_eq!((default.active_ms(), default.repeats()), (240_000, 5));
        assert_eq!(RepeatThresholds::new(240_000, 5).unwrap(), default);
        for (active, repeats) in [(0, 5), (240_000, 0), (0, 0)] {
            assert!(matches!(
                RepeatThresholds::new(active, repeats),
                Err(Error::InvalidRepeatThresholds)
            ));
        }
    }
}
