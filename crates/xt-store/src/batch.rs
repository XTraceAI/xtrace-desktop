//! Restricted storage composition for ingestion. Supplied coverage remains an
//! observation of the caller's payload; this module never generates a digest or
//! returns a plugin acknowledgement. Results escape only after the sole commit.

use crate::{
    CanonicalRecord, Error, Result, SessionMeta, SessionSource, Store, WriteStats,
    ingest::{
        self, CaptureReceipt, RecordCoverage, RecordSourceObservation, SessionSourceObservation,
    },
    write,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;

/// One canonical session and its submitted facts. Empty record batches may
/// update metadata/cursors, but cannot manufacture receipt coverage.
/// UUID-level evidence requires every occurrence of that UUID to be accepted;
/// mixed accepted/rejected occurrences cannot identify the evidence's input.
pub struct IngestBatch<'a> {
    pub session: &'a SessionMeta,
    pub records: &'a [CanonicalRecord],
    pub keep_content: bool,
    pub namespace: Option<&'a str>,
    /// Empty means unknown; otherwise exactly one native identity per input.
    pub identities: &'a [crate::model::RecordIdentity],
    pub session_sources: &'a [SessionSourceObservation],
    pub record_sources: &'a [RecordSourceObservation],
    pub receipt: Option<SubmittedReceipt<'a>>,
    /// Exact retry matching happens under this batch's write transaction.
    pub receipt_replay: ReceiptReplay,
    /// Writer mode: rejected identities contribute no source or receipt evidence.
    pub evidence_policy: EvidencePolicy,
    pub cursor: Option<&'a SourceCursor>,
    /// A discovered identity that fills under this batch's transaction, so a
    /// rejected batch leaves no label behind.
    pub discovery: Option<&'a crate::ingest::DiscoveredSession>,
}

impl<'a> IngestBatch<'a> {
    pub fn new(
        session: &'a SessionMeta,
        records: &'a [CanonicalRecord],
        keep_content: bool,
    ) -> Self {
        Self {
            session,
            records,
            keep_content,
            namespace: None,
            identities: &[],
            session_sources: &[],
            record_sources: &[],
            receipt: None,
            receipt_replay: ReceiptReplay::Reject,
            evidence_policy: EvidencePolicy::RequireAll,
            cursor: None,
            discovery: None,
        }
    }
}

/// Precomputed receipt facts. Reusing a sealed receipt ID fails atomically;
/// idempotent delivery and acknowledgement policy belong to the ingest writer.
pub struct SubmittedReceipt<'a> {
    pub receipt: &'a CaptureReceipt,
    pub coverage: &'a [RecordCoverage],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReceiptReplay {
    #[default]
    Reject,
    /// Parent facts and the complete unordered coverage set must match exactly.
    MatchExact,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EvidencePolicy {
    #[default]
    RequireAll,
    AcceptedOnly,
}

/// Incremental positions are monotonically nondecreasing within a source/key.
/// A complete scan from the beginning may replace its observation through
/// `record_completed_source_scan`; it must not reuse an old offset to skip input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCursor {
    pub source: SessionSource,
    pub cursor_key: String,
    pub position: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordDisposition {
    Inserted,
    /// A missing record field or session metadata field was filled.
    Enriched,
    /// Same canonical identity; known conflicting values may set a sticky flag.
    Duplicate,
    RejectedOwnership,
    RejectedType,
    DroppedMissingUuid,
}

impl RecordDisposition {
    pub fn is_accepted(self) -> bool {
        matches!(self, Self::Inserted | Self::Enriched | Self::Duplicate)
    }
}

/// One result per input, in input order. An index disambiguates repeated UUIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordOutcome {
    pub input_index: usize,
    /// None for missing/blank UUIDs; accepted/rejected valid identities are exact.
    pub uuid: Option<String>,
    pub disposition: RecordDisposition,
    /// Conflicting known measurement fields from this input under the write lock.
    pub conflict_fields: i64,
    /// Sticky row conflict state immediately after this input. A later input of
    /// the same UUID can add a conflict; this is not a per-field provenance mask.
    pub stored_has_conflict: Option<bool>,
}

/// A different session whose owned record was marked conflicted by this batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffectedSession {
    pub session_id: String,
    pub surface: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestBatchOutcome {
    /// Surface from the committed session merge, not the sparse request.
    pub session_surface: Option<String>,
    pub affected_owners: Vec<AffectedSession>,
    pub receipt_committed: bool,
    pub session_changed: bool,
    /// Legacy per-input counters: `ignored` also includes ownership/type rejects.
    pub stats: WriteStats,
    pub records: Vec<RecordOutcome>,
}

impl Store {
    /// Apply one fixed batch under one immediate transaction. Any validation,
    /// write or commit failure returns no successful outcome and rolls back all
    /// participating facts. There is no callback, nested commit or event hook.
    pub fn apply_ingest_batch(&mut self, batch: &IngestBatch<'_>) -> Result<IngestBatchOutcome> {
        write::validate_session(batch.session)?;
        if !batch.identities.is_empty() && batch.identities.len() != batch.records.len() {
            return Err(Error::InvalidInput(
                "native identities must align with batch inputs",
            ));
        }
        if batch
            .session_sources
            .iter()
            .any(|observation| observation.session_id != batch.session.session_id)
            || batch
                .receipt
                .as_ref()
                .is_some_and(|submitted| submitted.receipt.session_id != batch.session.session_id)
        {
            return Err(Error::InvalidInput(
                "batch facts must belong to its canonical session",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let keep_content = crate::retention::allows_content(&transaction, batch.keep_content)?;
        // Receipt-bearing writer batches may reject every record. Keep their
        // destination writes provisional while still recording original-owner conflicts.
        let provisional =
            batch.receipt.is_some() && batch.evidence_policy == EvidencePolicy::AcceptedOnly;
        if provisional {
            transaction.execute_batch("SAVEPOINT destination_import")?;
        }
        if let Some(discovery) = batch.discovery {
            ingest::observe_discovered_session(&transaction, discovery)?;
        }
        let session_changed =
            write::upsert_session(&transaction, batch.session, keep_content, batch.namespace)?;
        let mut outcome = write::upsert_records(
            &transaction,
            &batch.session.session_id,
            batch.records,
            keep_content,
            batch.identities,
        )?;
        outcome.session_changed |= session_changed;
        let rejected = outcome
            .records
            .iter()
            .filter(|record| !record.disposition.is_accepted())
            .filter_map(|record| record.uuid.as_deref())
            .collect::<BTreeSet<_>>();
        let accepted = outcome
            .records
            .iter()
            .filter(|record| record.disposition.is_accepted())
            .filter_map(|record| record.uuid.as_deref())
            .filter(|uuid| !rejected.contains(uuid))
            .collect::<BTreeSet<_>>();
        // A stored UUID outside this submission (or rejected by the merger) must
        // not acquire this batch's source/receipt evidence just because it exists.
        // Any rejected occurrence makes UUID-level evidence ambiguous, even if
        // another occurrence was accepted. Known-field conflicts remain accepted.
        if batch.evidence_policy == EvidencePolicy::RequireAll
            && (batch
                .record_sources
                .iter()
                .any(|observation| !accepted.contains(observation.uuid.as_str()))
                || batch.receipt.as_ref().is_some_and(|submitted| {
                    submitted
                        .coverage
                        .iter()
                        .any(|coverage| !accepted.contains(coverage.record_uuid.as_str()))
                }))
        {
            return Err(Error::InvalidInput(
                "record facts require a submitted UUID with no rejected occurrences",
            ));
        }
        for observation in batch.session_sources {
            ingest::observe_session_source(&transaction, observation)?;
        }
        let mut conflicts = std::collections::BTreeMap::new();
        for record in &outcome.records {
            if let Some(uuid) = record.uuid.as_deref() {
                *conflicts.entry(uuid).or_insert(0) |= record.conflict_fields;
            }
        }
        for observation in batch
            .record_sources
            .iter()
            .filter(|observation| accepted.contains(observation.uuid.as_str()))
        {
            let mut observation = observation.clone();
            observation.conflict_flags |= conflicts
                .get(observation.uuid.as_str())
                .copied()
                .unwrap_or(0)
                & observation.field_presence;
            ingest::observe_record_source(&transaction, &observation)?;
        }
        if let Some(submitted) = &batch.receipt {
            let coverage = submitted
                .coverage
                .iter()
                .filter(|item| accepted.contains(item.record_uuid.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            outcome.receipt_committed = ingest::insert_or_match_receipt(
                &transaction,
                submitted.receipt,
                &coverage,
                batch.receipt_replay == ReceiptReplay::MatchExact,
                batch.evidence_policy == EvidencePolicy::AcceptedOnly,
            )?;
        }
        if provisional {
            if !outcome
                .records
                .iter()
                .any(|row| row.disposition.is_accepted())
            {
                transaction
                    .execute_batch("ROLLBACK TO destination_import; RELEASE destination_import")?;
                for row in &outcome.records {
                    if matches!(
                        row.disposition,
                        RecordDisposition::RejectedOwnership | RecordDisposition::RejectedType
                    ) {
                        transaction.execute(
                            "UPDATE records SET has_conflict=1 WHERE uuid=?1",
                            [&row.uuid],
                        )?;
                    }
                }
                outcome.session_changed = false;
                outcome.session_surface = transaction
                    .query_row(
                        "SELECT surface FROM sessions WHERE session_id=?1",
                        [&batch.session.session_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten();
                if outcome
                    .records
                    .iter()
                    .any(|row| row.disposition == RecordDisposition::RejectedType)
                {
                    outcome.affected_owners.push(AffectedSession {
                        session_id: batch.session.session_id.clone(),
                        surface: outcome.session_surface.clone(),
                    });
                }
                transaction.commit()?;
                return Ok(outcome);
            }
            transaction.execute_batch("RELEASE destination_import")?;
        }
        if let Some(cursor) = batch.cursor {
            advance_cursor(&transaction, cursor)?;
        }
        transaction.commit()?;
        Ok(outcome)
    }

    pub fn source_cursor(&self, source: SessionSource, key: &str) -> Result<Option<SourceCursor>> {
        Ok(self.connection.query_row(
            "SELECT source,cursor_key,position,updated_at FROM source_cursors WHERE source=?1 AND cursor_key=?2",
            params![source,key], |row| Ok(SourceCursor { source: row.get(0)?, cursor_key: row.get(1)?, position: row.get(2)?, updated_at: row.get(3)? }),
        ).optional()?)
    }
}

impl Store {
    /// Record a source position on its own, after the rows it covers have
    /// committed; a position can never regress.
    pub fn advance_source_cursor(&mut self, cursor: &SourceCursor) -> Result<()> {
        advance_cursor(&self.connection, cursor)
    }

    /// Replace the observation after a complete scan from the source's beginning.
    /// Unlike incremental advancement, a full scan can observe a shorter restored
    /// file. The caller must finish all writes and reject partial coverage first.
    pub fn record_completed_source_scan(&mut self, cursor: &SourceCursor) -> Result<()> {
        self.connection.execute(
            "INSERT INTO source_cursors(source,cursor_key,position,updated_at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(source,cursor_key) DO UPDATE SET position=excluded.position,
                 updated_at=excluded.updated_at WHERE excluded.updated_at > source_cursors.updated_at",
            params![
                cursor.source,
                cursor.cursor_key,
                cursor.position,
                cursor.updated_at
            ],
        )?;
        Ok(())
    }

    /// Reset only an existing observation, retaining its timestamp to reject late scans.
    pub fn reset_existing_source_cursor(
        &mut self,
        source: SessionSource,
        key: &str,
        observed_at: i64,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE source_cursors SET position=0, updated_at=?3
             WHERE source=?1 AND cursor_key=?2 AND updated_at < ?3",
            params![source, key, observed_at],
        )?;
        Ok(())
    }
}

fn advance_cursor(connection: &Connection, cursor: &SourceCursor) -> Result<()> {
    let changed = connection.execute(
        "INSERT INTO source_cursors(source,cursor_key,position,updated_at) VALUES (?1,?2,?3,?4)
         ON CONFLICT(source,cursor_key) DO UPDATE SET position=excluded.position,
             updated_at=max(updated_at,excluded.updated_at) WHERE excluded.position >= position",
        params![
            cursor.source,
            cursor.cursor_key,
            cursor.position,
            cursor.updated_at
        ],
    )?;
    if changed == 0 {
        return Err(Error::InvalidInput("source cursor cannot regress"));
    }
    Ok(())
}
