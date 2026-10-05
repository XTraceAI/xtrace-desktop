//! One session's M-09 hands-off stretches, for a session detail view.
//!
//! Nothing is redefined here. The events, the raw-surface timestamp health and
//! the stretch fold are [`crate::hands_off`]'s own, so a listed stretch is one
//! of the segments the Dashboard's M-09 count already measured, with the same
//! duration on the same stored millisecond projection.
//!
//! Two things are deliberately narrower than the global report:
//!
//! * **Health is the surface's, not the session's.** Timestamp health is judged
//!   over every session the raw `(host, surface)` has in the whole selected
//!   window, exactly as M-09 judges it, and only then is the answer narrowed to
//!   the requested session. One session can never make its own surface healthy.
//! * **An unknown is the session's own.** The global report cannot publish a
//!   distribution while any in-window session is unmeasured. Here a different
//!   session's unknown classification says nothing about this one, so an
//!   unrelated unknown never blanks the requested session.
//!
//! The optional first-tool locator is a structural position only. The native
//! `tool_use` identifier is not persisted anywhere, and `tool_uses.id` is a
//! database surrogate that is never the transcript's identifier, so this names
//! the record and the content block index instead and leaves resolving that
//! position against verified content to whatever reads the content. No content,
//! tool input or source file is read here. It is also only ever the call the
//! stretch actually started with: when the stored blocks cannot establish which
//! call that was, the locator is unknown rather than the earliest call that
//! happens to still be stored.

use crate::{MetricsDb, Result, Window, hands_off, session};
use rusqlite::OptionalExtension;
use serde::Serialize;
use std::collections::BTreeMap;

/// Stored block positions only, and only for record-bound assistant tool calls.
/// A row that states no position is not a locator. Both the number of stored
/// blocks and the lowest of them are needed: the count is what decides whether
/// the stored evidence accounts for every call a record states.
pub(crate) const TOOL_QUERY: &str = "SELECT uuid,count(*),min(block_index) FROM tool_uses
     WHERE session_id=?1 AND uuid IS NOT NULL AND block_index IS NOT NULL
     GROUP BY uuid";

/// What one record's stored tool-call blocks amount to.
struct StoredBlocks {
    /// Distinct stored block positions, which `UNIQUE(uuid, block_index)` keeps
    /// one per call.
    count: u64,
    lowest: u64,
}

/// The stretch's first tool call, when the stored evidence can establish which
/// call that actually was.
///
/// Walking the stretch's records in order: a record stating zero calls made
/// none and is passed over. The first record stating at least one call is the
/// record that made the stretch's first call, and its lowest stored block is
/// that call — but only when its stored blocks account for every call it
/// states. Anything less leaves the first call unknown, and unknown is the
/// answer: the next record's call is a different call, and reporting it here
/// would name a tool the stretch did not start with.
fn locate(
    stretch: &hands_off::Stretch<'_>,
    blocks: &BTreeMap<String, StoredBlocks>,
) -> Option<ToolBlock> {
    for (uuid, stated) in stretch.records() {
        match stated {
            // Nothing called here; the first call is still ahead.
            Some(0) => continue,
            // This record called first. Name the block only if every call it
            // states is accounted for, so a partly stored record cannot
            // promote its own later block either.
            Some(stated) => {
                let stored = blocks.get(uuid)?;
                return (stored.count == stated).then(|| ToolBlock {
                    record_uuid: uuid.to_owned(),
                    block_index: stored.lowest,
                });
            }
            // Whether this record called at all is unknown, so it cannot be
            // ruled out as the one that called first.
            None => return None,
        }
    }
    None
}

/// Where a stretch's first stored tool call sits: the record that carried it and
/// that record's content block index. This is a position, not an identity — see
/// the module documentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolBlock {
    pub record_uuid: String,
    pub block_index: u64,
}

/// One M-09 stretch of one session. The endpoints are exact native timestamps
/// as stored; `duration_ms` is the metric's own duration on the stored POSIX
/// millisecond projection and is not recomputed from those two spellings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionStretch {
    /// The explicitly human record the stretch starts at.
    pub start_uuid: String,
    /// The last explicitly non-human record before the next human record,
    /// including a tool-result carrier.
    pub end_uuid: String,
    pub start: String,
    pub end: String,
    pub duration_ms: u64,
    /// `None` whenever the stored blocks cannot establish which call came
    /// first — none are stored, the record that called first has only some of
    /// its blocks stored, or an earlier record's call count is unknown. A
    /// stretch qualifies on the records' own tool counts, so it keeps its
    /// duration and its place in the list either way; what is unknown is only
    /// where its first call sits. A later call is never reported in place of
    /// an earlier one that cannot be accounted for.
    pub first_tool: Option<ToolBlock>,
}

/// What M-09 can say about one session inside the selected window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionStretches {
    /// No indexed user session owns this identifier, so nothing was measured.
    /// This is distinct from an indexed session with no stretches in the window.
    Missing,
    /// The session is indexed, but M-09 states no stretches for it.
    /// `excluded_surface` names the raw surface whose timestamp health excluded
    /// it, when that is the reason; it is `None` when the session's own
    /// in-window records leave a boundary or a segment's tool presence unknown.
    Unmeasured {
        excluded_surface: Option<hands_off::ExcludedSurface>,
    },
    /// Every stretch the session has in the window, in chronological order.
    /// An indexed session with no qualifying segment has an honest empty list.
    Measured { stretches: Vec<SessionStretch> },
}

impl MetricsDb {
    /// M-09's stretches for exactly one session over one explicit window.
    ///
    /// Metadata, events and block positions are read inside one snapshot, so
    /// the surface a session is on, the records health was judged from and the
    /// tool blocks a stretch names all describe the same committed state while
    /// the native writer keeps appending on its own connection. An outer
    /// [`MetricsDb::read_snapshot`] still owns its own snapshot; this one joins
    /// it rather than opening a second.
    pub fn session_stretches(&self, window: Window, session_id: &str) -> Result<SessionStretches> {
        self.read_snapshot(|db| {
            // Whether an indexed user session owns this identifier, and which
            // raw surface it belongs to, are metadata facts. Both are read
            // before any measurement, so an empty window can never be mistaken
            // for an absent session, and a session with no in-window record at
            // all is still judged against its own surface's health.
            let Some((host, surface)) = db
                .connection
                .query_row(
                    &format!("{} AND session_id=?1", session::EXISTS_QUERY),
                    [session_id],
                    |row| Ok((row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)),
                )
                .optional()?
            else {
                return Ok(SessionStretches::Missing);
            };

            let surfaces = hands_off::load(&db.connection, window)?;
            let sessions = surfaces.get(&(host.clone(), surface.clone()));
            if let Some(excluded) = sessions.and_then(|s| hands_off::health(&host, &surface, s)) {
                return Ok(SessionStretches::Unmeasured {
                    excluded_surface: Some(excluded),
                });
            }
            let Some(events) = sessions.and_then(|s| s.get(session_id)) else {
                return Ok(SessionStretches::Measured { stretches: vec![] });
            };
            let Some(collected) = hands_off::collect(events)? else {
                return Ok(SessionStretches::Unmeasured {
                    excluded_surface: None,
                });
            };

            let blocks = if collected.is_empty() {
                BTreeMap::new()
            } else {
                db.stored_tool_blocks(session_id)?
            };
            let stretches = collected
                .iter()
                .map(|stretch| {
                    let (start_uuid, start) = stretch.start();
                    let (end_uuid, end) = stretch.end();
                    SessionStretch {
                        start_uuid: start_uuid.to_owned(),
                        end_uuid: end_uuid.to_owned(),
                        start: start.to_owned(),
                        end: end.to_owned(),
                        duration_ms: stretch.duration_ms(),
                        first_tool: locate(stretch, &blocks),
                    }
                })
                .collect();
            Ok(SessionStretches::Measured { stretches })
        })
    }

    /// How many tool-call blocks each record of one session has stored, and the
    /// lowest of them. Metadata only: no tool input, no record content and no
    /// source file is read.
    fn stored_tool_blocks(&self, session_id: &str) -> Result<BTreeMap<String, StoredBlocks>> {
        let mut stored = BTreeMap::<String, StoredBlocks>::new();
        let mut statement = self.connection.prepare(TOOL_QUERY)?;
        let mut rows = statement.query([session_id])?;
        while let Some(row) = rows.next()? {
            let count = row.get::<_, i64>(1)?;
            let lowest = row.get::<_, i64>(2)?;
            stored.insert(
                row.get(0)?,
                StoredBlocks {
                    count: u64::try_from(count)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, count))?,
                    lowest: u64::try_from(lowest)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, lowest))?,
                },
            );
        }
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locator_lookup_reads_no_content() {
        for forbidden in ["content_json", "input_json", "name", "kind"] {
            assert!(!TOOL_QUERY.contains(forbidden), "{forbidden}");
        }
        assert!(session::EXISTS_QUERY.contains("kind='user'"));
    }
}
