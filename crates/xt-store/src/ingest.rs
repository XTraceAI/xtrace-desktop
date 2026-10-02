//! Structural ingestion facts. Observation/discovery writes never create capture
//! receipts. Receipt digests and masks are supplied by the future ingest writer;
//! this module stores their exact values and seals each submitted coverage set.

use crate::{Error, Host, Result, SessionSource, Store, model::text_enum, timestamp};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSourceObservation {
    pub session_id: String,
    pub source: SessionSource,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordSourceObservation {
    pub uuid: String,
    pub source: SessionSource,
    pub field_presence: i64,
    pub conflict_flags: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureReceipt {
    pub receipt_id: String,
    pub session_id: String,
    pub surface: Option<String>,
    /// Explicit UTC milliseconds supplied by the caller, not a fixture wall clock.
    pub received_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordCoverage {
    pub record_uuid: String,
    /// Nonnegative SQLite integer; named bits are owned by the digest schema.
    pub metric_field_mask: i64,
    /// Canonical measurement digest supplied by ingestion, never recomputed from
    /// a richer stored record. Exactly 64 lowercase SHA-256 hexadecimal digits.
    pub measurement_revision: String,
    pub digest_schema_version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredSession {
    pub host: Host,
    pub native_session_id: String,
    /// May be known before any canonical session has been imported.
    pub conversation_id: Option<String>,
    pub surface: Option<String>,
    pub started_at_ms: Option<i64>,
    pub last_observed_at: i64,
    pub discovery_complete: bool,
}

text_enum!(ToolKind {
    Builtin => "builtin", Mcp => "mcp", Skill => "skill", Hook => "hook",
    Command => "command", Subagent => "subagent"
});

/// A classified structural event that needs no fabricated user/assistant row.
/// This shape deliberately has no command, transcript, input or output payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolEvent {
    pub session_id: String,
    pub source: SessionSource,
    pub source_event_id: String,
    /// Native RFC3339 timestamp, preserved verbatim. Absence stays unknown;
    /// session start and receipt time are not substitutes for an event time.
    pub timestamp: Option<String>,
    pub name: String,
    pub kind: ToolKind,
    pub server: Option<String>,
    pub tool: Option<String>,
    pub skill: Option<String>,
}

impl Store {
    /// Monotonic observation interval; this is not a delivery receipt.
    pub fn observe_session_source(&mut self, observation: &SessionSourceObservation) -> Result<()> {
        observe_session_source(&self.connection, observation)
    }

    pub fn session_sources(&self, session_id: &str) -> Result<Vec<SessionSourceObservation>> {
        Ok(self.connection.prepare(
            "SELECT session_id,source,first_seen_at,last_seen_at FROM session_sources WHERE session_id=?1 ORDER BY source"
        )?.query_map([session_id], |row| Ok(SessionSourceObservation {
            session_id: row.get(0)?, source: row.get(1)?, first_seen_at: row.get(2)?, last_seen_at: row.get(3)?,
        }))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Accumulate supplied presence/conflict bits without reading stored content
    /// or claiming that these observations were covered by a plugin receipt.
    pub fn observe_record_source(&mut self, observation: &RecordSourceObservation) -> Result<()> {
        observe_record_source(&self.connection, observation)
    }

    pub fn record_sources(&self, uuid: &str) -> Result<Vec<RecordSourceObservation>> {
        Ok(self.connection.prepare(
            "SELECT uuid,source,field_presence,conflict_flags FROM record_sources WHERE uuid=?1 ORDER BY source"
        )?.query_map([uuid], |row| Ok(RecordSourceObservation {
            uuid: row.get(0)?, source: row.get(1)?, field_presence: row.get(2)?, conflict_flags: row.get(3)?,
        }))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Insert and seal one nonempty submitted set atomically. Duplicate receipt
    /// IDs, duplicate child UUIDs and cross-session coverage fail without leaving
    /// partial facts. Retry arbitration and digest generation belong to ingestion.
    pub fn insert_capture_receipt(
        &mut self,
        receipt: &CaptureReceipt,
        coverage: &[RecordCoverage],
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_receipt(&transaction, receipt, coverage)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn capture_receipts(&self, session_id: &str) -> Result<Vec<CaptureReceipt>> {
        Ok(self
            .connection
            .prepare(
                "SELECT receipt_id,session_id,surface,received_at FROM capture_receipts
             WHERE session_id=?1 AND coverage_sealed=1 ORDER BY received_at,receipt_id",
            )?
            .query_map([session_id], |row| {
                Ok(CaptureReceipt {
                    receipt_id: row.get(0)?,
                    session_id: row.get(1)?,
                    surface: row.get(2)?,
                    received_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn capture_coverage(&self, receipt_id: &str) -> Result<Vec<RecordCoverage>> {
        coverage(&self.connection, receipt_id)
    }

    /// Discovery exists independently of canonical ingestion. Known identities
    /// fill once; conflicting non-null values fail. A newer probe can change
    /// completeness, while a stale result cannot replace the current status.
    pub fn observe_discovered_session(&mut self, discovery: &DiscoveredSession) -> Result<()> {
        observe_discovered_session(&self.connection, discovery)
    }

    pub fn discovered_sessions(&self, host: Host) -> Result<Vec<DiscoveredSession>> {
        Ok(self.connection.prepare(
            "SELECT host,native_session_id,conversation_id,surface,started_at_ms,last_observed_at,discovery_complete
             FROM discovered_sessions WHERE host=?1 ORDER BY native_session_id"
        )?.query_map([host], |row| Ok(DiscoveredSession {
            host: row.get(0)?, native_session_id: row.get(1)?, conversation_id: row.get(2)?, surface: row.get(3)?,
            started_at_ms: row.get(4)?, last_observed_at: row.get(5)?, discovery_complete: row.get(6)?,
        }))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Store only the classified event identity and names, with no record FK
    /// when the native event is not a canonical user/assistant record.
    /// Idempotent for the event's own stable identity, and bound to the session
    /// that owns that identity rather than the supplied one: see
    /// `insert_tool_event` and `structural_owner`.
    pub fn insert_tool_event(&mut self, event: &ToolEvent) -> Result<i64> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = insert_tool_event(&transaction, event)?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn tool_events(&self, session_id: &str) -> Result<Vec<ToolEvent>> {
        Ok(self.connection.prepare(
            "SELECT session_id,source,source_event_id,name,kind,server,tool,skill,event_ts FROM tool_uses
             WHERE session_id=?1 AND uuid IS NULL ORDER BY source,source_event_id"
        )?
        .query_map([session_id], |row| tool_event_from_row(row, 0))?
        .collect::<rusqlite::Result<_>>()?)
    }
}

/// The session a structural identity is stored under, so that a native fork
/// repeating the same line in several files yields one event rather than one
/// per file. Resolved in this order:
///
/// 1. The canonical record carrying this UUID owns it. That is the existing
///    ownership rule `upsert_records` enforces, so a copied context resolves to
///    the record's owner however many files repeat the line. This step is real
///    provenance: the owner is where the work record is actually stored.
/// 2. Otherwise, a structural event already stored under this identity keeps
///    its session. This step is a stable storage convention, not provenance. A
///    hook summary is no record, so nothing here establishes which session
///    originally produced it; whichever context was scanned first is the one
///    that holds it, and a later context does not move it. The value is that
///    the identity has exactly one home and that home does not change, not that
///    the home is the summary's origin.
/// 3. Otherwise the identity is new and the caller's session holds it.
pub(crate) fn structural_owner(
    connection: &Connection,
    source_event_id: &str,
) -> Result<Option<String>> {
    if let Some(owner) = connection
        .query_row(
            "SELECT session_id FROM records WHERE uuid=?1",
            [source_event_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(Some(owner));
    }
    Ok(connection
        .query_row(
            "SELECT session_id FROM tool_uses WHERE source_event_id=?1 AND uuid IS NULL",
            [source_event_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?)
}

/// Insert one structural event, or recognise the one already stored under the
/// same stable identity. The event is bound to the session `structural_owner`
/// resolves, not to the one the caller supplied, so a copied native context
/// adds no second row. Within that session the search is by `source_event_id`
/// **regardless of source**, so the same native event delivered by the plugin
/// and by the native scan, in either order, is one row and a plain retry adds
/// nothing. An identity that arrives again with different facts is a conflict:
/// it fails, leaving the stored event untouched.
///
/// Callers inside a batch pass that batch's transaction; the public wrapper
/// opens an immediate one, so the read and the insert cannot race a writer.
pub(crate) fn insert_tool_event(connection: &Connection, event: &ToolEvent) -> Result<i64> {
    if event.name.trim().is_empty() {
        return Err(Error::InvalidInput("tool event name must be nonempty"));
    }
    // An arrival-ordered identity would make retries look like new events.
    if event.source_event_id.trim().is_empty() {
        return Err(Error::InvalidInput(
            "structural tool event requires a stable source event ID",
        ));
    }
    if let Some(value) = &event.timestamp {
        timestamp::parse(value)?;
    }
    let owner = structural_owner(connection, &event.source_event_id)?
        .unwrap_or_else(|| event.session_id.clone());
    let stored = connection
        .query_row(
            "SELECT id,session_id,source,source_event_id,name,kind,server,tool,skill,event_ts
             FROM tool_uses WHERE session_id=?1 AND source_event_id=?2 AND uuid IS NULL",
            params![owner, event.source_event_id],
            |row| Ok((row.get::<_, i64>(0)?, tool_event_from_row(row, 1)?)),
        )
        .optional()?;
    // Compare against the identity as owned, so a copied context is recognised
    // as the same event instead of being read as a conflicting session.
    let event = &ToolEvent {
        session_id: owner,
        ..event.clone()
    };
    if let Some((id, saved)) = stored {
        // Source is deliberately excluded: which adapter delivered the event
        // first is not part of what the event says happened.
        if (
            &saved.session_id,
            &saved.source_event_id,
            &saved.name,
            saved.kind,
            &saved.server,
            &saved.tool,
            &saved.skill,
            &saved.timestamp,
        ) != (
            &event.session_id,
            &event.source_event_id,
            &event.name,
            event.kind,
            &event.server,
            &event.tool,
            &event.skill,
            &event.timestamp,
        ) {
            return Err(Error::InvalidInput(
                "structural tool event identity replays with conflicting facts",
            ));
        }
        return Ok(id);
    }
    connection.execute(
        "INSERT INTO tool_uses(session_id,source,source_event_id,name,kind,server,tool,skill,event_ts)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            event.session_id,
            event.source,
            event.source_event_id,
            event.name,
            event.kind,
            event.server,
            event.tool,
            event.skill,
            event.timestamp
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn tool_event_from_row(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<ToolEvent> {
    Ok(ToolEvent {
        session_id: row.get(start)?,
        source: row.get(start + 1)?,
        source_event_id: row.get(start + 2)?,
        name: row.get(start + 3)?,
        kind: row.get(start + 4)?,
        server: row.get(start + 5)?,
        tool: row.get(start + 6)?,
        skill: row.get(start + 7)?,
        timestamp: row.get(start + 8)?,
    })
}

pub(crate) fn observe_session_source(
    connection: &Connection,
    observation: &SessionSourceObservation,
) -> Result<()> {
    connection.execute(
        "INSERT INTO session_sources(session_id,source,first_seen_at,last_seen_at) VALUES (?1,?2,?3,?4)
         ON CONFLICT(session_id,source) DO UPDATE SET
             first_seen_at=min(first_seen_at,excluded.first_seen_at),
             last_seen_at=max(last_seen_at,excluded.last_seen_at)",
        params![observation.session_id, observation.source, observation.first_seen_at, observation.last_seen_at],
    )?;
    Ok(())
}

pub(crate) fn observe_record_source(
    connection: &Connection,
    observation: &RecordSourceObservation,
) -> Result<()> {
    connection.execute(
        "INSERT INTO record_sources(uuid,source,field_presence,conflict_flags) VALUES (?1,?2,?3,?4)
         ON CONFLICT(uuid,source) DO UPDATE SET field_presence=field_presence | excluded.field_presence,
             conflict_flags=conflict_flags | excluded.conflict_flags",
        params![observation.uuid, observation.source, observation.field_presence, observation.conflict_flags],
    )?;
    Ok(())
}

pub(crate) fn insert_receipt(
    connection: &Connection,
    receipt: &CaptureReceipt,
    coverage: &[RecordCoverage],
) -> Result<()> {
    if coverage.is_empty() {
        return Err(Error::InvalidInput(
            "capture receipt requires submitted record coverage",
        ));
    }
    connection.execute(
        "INSERT INTO capture_receipts(receipt_id,session_id,surface,received_at) VALUES (?1,?2,?3,?4)",
        params![receipt.receipt_id, receipt.session_id, receipt.surface, receipt.received_at],
    )?;
    for item in coverage {
        connection.execute(
            "INSERT INTO capture_record_coverage(receipt_id,record_uuid,session_id,metric_field_mask,measurement_revision,digest_schema_version)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![receipt.receipt_id, item.record_uuid, receipt.session_id, item.metric_field_mask, item.measurement_revision, item.digest_schema_version],
        )?;
    }
    connection.execute(
        "UPDATE capture_receipts SET coverage_sealed=1 WHERE receipt_id=?1",
        [&receipt.receipt_id],
    )?;
    Ok(())
}

fn coverage(connection: &Connection, receipt_id: &str) -> Result<Vec<RecordCoverage>> {
    Ok(connection.prepare(
            "SELECT c.record_uuid,c.metric_field_mask,c.measurement_revision,c.digest_schema_version
             FROM capture_record_coverage c JOIN capture_receipts r ON r.receipt_id=c.receipt_id
             WHERE c.receipt_id=?1 AND r.coverage_sealed=1 ORDER BY c.record_uuid"
        )?.query_map([receipt_id], |row| Ok(RecordCoverage {
            record_uuid: row.get(0)?, metric_field_mask: row.get(1)?, measurement_revision: row.get(2)?, digest_schema_version: row.get(3)?,
        }))?.collect::<rusqlite::Result<_>>()?)
}

// Called only inside the owning immediate write transaction. An existing ID
// cannot race another writer between comparison and canonical/cursor commit.
pub(crate) fn insert_or_match_receipt(
    connection: &Connection,
    receipt: &CaptureReceipt,
    submitted: &[RecordCoverage],
    allow_exact: bool,
    ignore_new_empty: bool,
) -> Result<bool> {
    let saved = connection.query_row(
        "SELECT session_id,surface,received_at,coverage_sealed FROM capture_receipts WHERE receipt_id=?1",
        [&receipt.receipt_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,i64>(2)?,row.get::<_,bool>(3)?)),
    ).optional()?;
    if let Some((session, surface, received, sealed)) = saved {
        let mut expected = submitted.to_vec();
        expected.sort_by(|a, b| a.record_uuid.cmp(&b.record_uuid));
        if !allow_exact
            || !sealed
            || expected.is_empty()
            || session != receipt.session_id
            || surface != receipt.surface
            || received != receipt.received_at
            || coverage(connection, &receipt.receipt_id)? != expected
        {
            return Err(Error::InvalidInput(
                "receipt retry does not match immutable submitted facts",
            ));
        }
        return Ok(true);
    }
    if ignore_new_empty && submitted.is_empty() {
        return Ok(false);
    }
    insert_receipt(connection, receipt, submitted)?;
    Ok(true)
}

/// The discovered-session upsert shared by direct observation and the batch
/// transaction, so a batch can fill an identity atomically with its rows.
pub(crate) fn observe_discovered_session(
    connection: &Connection,
    discovery: &DiscoveredSession,
) -> Result<()> {
    let changed = connection.execute(
        "INSERT INTO discovered_sessions(host,native_session_id,conversation_id,surface,started_at_ms,last_observed_at,discovery_complete)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(host,native_session_id) DO UPDATE SET
             conversation_id=coalesce(conversation_id,excluded.conversation_id),
             surface=coalesce(surface,excluded.surface),started_at_ms=coalesce(started_at_ms,excluded.started_at_ms),
             discovery_complete=CASE WHEN excluded.last_observed_at >= last_observed_at THEN excluded.discovery_complete ELSE discovery_complete END,
             last_observed_at=max(last_observed_at,excluded.last_observed_at)
         WHERE (conversation_id IS NULL OR excluded.conversation_id IS NULL OR conversation_id=excluded.conversation_id)
             AND (surface IS NULL OR excluded.surface IS NULL OR surface=excluded.surface)
             AND (started_at_ms IS NULL OR excluded.started_at_ms IS NULL OR started_at_ms=excluded.started_at_ms)",
        params![discovery.host, discovery.native_session_id, discovery.conversation_id, discovery.surface, discovery.started_at_ms, discovery.last_observed_at, discovery.discovery_complete],
    )?;
    if changed == 0 {
        return Err(Error::InvalidInput(
            "conflicting discovered session identity",
        ));
    }
    Ok(())
}
